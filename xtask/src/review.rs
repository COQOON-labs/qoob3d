//! Critic records (ADR-0041) and the review plan (ADR-0052). The critic itself
//! is a model and not deterministic; what is checked here is that the reviews
//! of exactly the pushed tree cover the plan of the pushed range and leave no
//! blocking finding unresolved.
//!
//! The plan follows from the commits of the range, the tip's committer date
//! and `docs/escapes.md` as of the tip: tier `none` (no production code),
//! `standard` or `high` (harness, OCCT boundary, format, size or a recent
//! escape).
//!
//! Record (version 2): `target/critic/<tree>.json`
//! `{"version": 2, "tree": "<sha>", "base": "<sha>", "head": "<sha>",
//!   "tier": "standard|high", "previous_tree": "<tree>|null",
//!   "reviews": [{"id": "r1", "model": "...", "lens": "general|tests|security|robustness",
//!     "round": 1}],
//!   "findings": [{"rule": "K-03", "severity": "blocker|major|minor",
//!     "verdict": "confirmed|refuted|unverified", "resolution": "open|fixed|accepted",
//!     "accepted_reason": "...", "found_by": ["r1"], ...}]}`

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::paths::{self, Kind};
use crate::{docs, git};

pub(crate) const RECORD_DIR: &str = "target/critic";

/// The escape log, read as of the tip (ADR-0052 decision 5).
const ESCAPES: &str = "docs/escapes.md";
/// An escape row counts for this many days up to and including the tip date.
const ESCAPE_WINDOW_DAYS: i64 = 180;
/// More changed production code lines than this make a diff high-risk.
const MAX_STANDARD_LINES: usize = 400;
const LENSES: &[&str] = &["general", "tests", "security", "robustness"];

/// Review depth of a pushed range (ADR-0052 decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tier {
    None,
    Standard,
    High,
}

impl Tier {
    fn name(self) -> &'static str {
        match self {
            Tier::None => "none",
            Tier::Standard => "standard",
            Tier::High => "high",
        }
    }
}

/// What makes a range high-risk (ADR-0052 decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    Harness,
    OcctBoundary,
    Format,
    Size,
    Escape,
}

impl Trigger {
    fn name(self) -> &'static str {
        match self {
            Trigger::Harness => "harness",
            Trigger::OcctBoundary => "occt-boundary",
            Trigger::Format => "format",
            Trigger::Size => "size",
            Trigger::Escape => "escape",
        }
    }
}

/// A trigger that holds, with one changed path as example.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reason {
    pub(crate) trigger: Trigger,
    pub(crate) path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) tier: Tier,
    pub(crate) reasons: Vec<Reason>,
}

impl Plan {
    /// The lens reviews required beside the general ones.
    fn lenses(&self) -> Vec<&'static str> {
        if self.tier != Tier::High {
            return Vec::new();
        }
        let holds = |trigger| self.reasons.iter().any(|r| r.trigger == trigger);
        let mut lenses = vec!["tests"];
        if holds(Trigger::Harness) {
            lenses.push("security");
        }
        if holds(Trigger::OcctBoundary) || holds(Trigger::Format) {
            lenses.push("robustness");
        }
        lenses
    }
}

/// The `--plan` output: tier, reasons and the required reviews.
impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "tier: {}", self.tier.name())?;
        for reason in &self.reasons {
            writeln!(f, "reason: {} ({})", reason.trigger.name(), reason.path)?;
        }
        match self.tier {
            Tier::None => writeln!(f, "required: no review"),
            Tier::Standard => writeln!(f, "required: 1 review with lens general"),
            Tier::High => writeln!(
                f,
                "required: 2 reviews with lens general on different models; lenses {}",
                self.lenses().join(", ")
            ),
        }
    }
}

/// The plan of `changes` (path and changed lines, one entry per file and
/// commit) with the escape log `escapes` as of a tip committed on `tip_date`.
pub(crate) fn plan(
    changes: &[(&str, usize)],
    escapes: &str,
    tip_date: &str,
) -> Result<Plan, Vec<String>> {
    let escaped = recent_escapes(escapes, tip_date)?;
    let code: Vec<(&str, usize)> =
        changes.iter().copied().filter(|(path, _)| paths::kind(path) == Kind::Code).collect();
    let Some(&(first_code, _)) = code.first() else {
        return Ok(Plan { tier: Tier::None, reasons: Vec::new() });
    };
    let first = |hit: fn(&str) -> bool| changes.iter().map(|c| c.0).find(|path| hit(path));
    let lines: usize = code.iter().map(|c| c.1).sum();
    let reasons: Vec<Reason> = [
        (Trigger::Harness, first(paths::is_harness)),
        (Trigger::OcctBoundary, first(paths::is_occt_boundary)),
        (Trigger::Format, first(paths::is_format)),
        (Trigger::Size, (lines > MAX_STANDARD_LINES).then_some(first_code)),
        (Trigger::Escape, code.iter().map(|c| c.0).find(|path| paths::listed(&escaped, path))),
    ]
    .into_iter()
    .filter_map(|(trigger, path)| Some(Reason { trigger, path: path?.to_owned() }))
    .collect();
    let tier = if reasons.is_empty() { Tier::Standard } else { Tier::High };
    Ok(Plan { tier, reasons })
}

/// The plan of the commits in `rev_list_args`, pushed as `tip`.
pub(crate) fn plan_range(
    root: &Path,
    tip: &str,
    rev_list_args: &[String],
) -> Result<Plan, Vec<String>> {
    let git = |args: &[&str]| git(root, args).map_err(|e| vec![e]);
    // `-z`: paths unquoted, as they are (git quotes non-ASCII, tab, `"`, `\`).
    // `--no-renames`: a rename changes the old path too.
    let mut args = vec!["log", "--format=", "--numstat", "--no-renames", "-z"];
    args.extend(rev_list_args.iter().map(String::as_str));
    let numstat = git(&args)?;
    let changes: Vec<(&str, usize)> = numstat
        .split('\0')
        .filter_map(|entry| {
            let mut fields = entry.splitn(3, '\t');
            let (added, deleted, path) = (fields.next()?, fields.next()?, fields.next()?);
            // Binary files have `-` instead of line counts.
            let count = |n: &str| n.parse::<usize>().unwrap_or(0);
            Some((path, count(added) + count(deleted)))
        })
        .collect();
    let tip_date = git(&["log", "-1", "--format=%cs", tip, "--"])?;
    // States before the escape log have none.
    let escapes = if git(&["ls-tree", "--name-only", tip, ESCAPES])?.is_empty() {
        String::new()
    } else {
        git(&["show", &format!("{tip}:{ESCAPES}")])?
    };
    plan(&changes, &escapes, tip_date.trim())
}

/// The backticked `Where` paths of the escape-log rows dated within the
/// window up to `tip_date`. A row with an unparsable date fails closed.
fn recent_escapes<'a>(escapes: &'a str, tip_date: &str) -> Result<Vec<&'a str>, Vec<String>> {
    let tip = day_number(tip_date)
        .ok_or_else(|| vec![format!("tip date `{tip_date}` is not YYYY-MM-DD")])?;
    let mut places = Vec::new();
    let mut errors = Vec::new();
    for row in escapes.lines().filter(|l| l.trim_start().starts_with('|')) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let [_, date, _, _, place, ..] = cells.as_slice() else {
            errors.push(format!("{ESCAPES}: row without a `Where` column – {row}"));
            continue;
        };
        let separator = !date.is_empty() && date.trim_matches(['-', ':']).is_empty();
        if *date == "Date" || separator {
            continue;
        }
        let Some(day) = day_number(date) else {
            errors.push(format!("{ESCAPES}: row date `{date}` is not a valid YYYY-MM-DD date"));
            continue;
        };
        if (0..ESCAPE_WINDOW_DAYS).contains(&(tip - day)) {
            places.extend(place.split('`').skip(1).step_by(2));
        }
    }
    if errors.is_empty() { Ok(places) } else { Err(errors) }
}

/// Days since 1970-01-01 of a valid `YYYY-MM-DD` date (proleptic Gregorian;
/// `days_from_civil` by Howard Hinnant).
fn day_number(date: &str) -> Option<i64> {
    if !docs::is_date(date) {
        return None;
    }
    let field = |range: std::ops::Range<usize>| date[range].parse::<i64>().ok();
    let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let lengths = [31, 28 + i64::from(leap), 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let length = *lengths.get(usize::try_from(month).ok()?.checked_sub(1)?)?;
    if !(1..=length).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// Requires a record covering the plan for `tip` if any commit in
/// `rev_list_args` changes production code.
pub(crate) fn check(root: &Path, tip: &str, rev_list_args: &[String]) -> Result<(), Vec<String>> {
    let plan = plan_range(root, tip, rev_list_args)?;
    if plan.tier == Tier::None {
        return Ok(());
    }
    let tree = git(root, &["rev-parse", &format!("{tip}^{{tree}}")]).map_err(|e| vec![e])?;
    let tree = tree.trim();
    let records = root.join(RECORD_DIR);
    let file = records.join(format!("{tree}.json"));
    let text = std::fs::read_to_string(&file).map_err(|_| {
        vec![format!(
            "no critic review for this state ({}) – run `/critic-review` (ADR-0041)",
            file.display()
        )]
    })?;
    let record: Value = serde_json::from_str(&text)
        .map_err(|e| vec![format!("{}: not valid JSON: {e}", file.display())])?;
    let problems = record_problems(&record, tree, &plan, &records);
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// Everything that keeps `record` from passing for `tree` under `plan`;
/// `records` holds the record of the previous round. Fails closed.
pub(crate) fn record_problems(
    record: &Value,
    tree: &str,
    plan: &Plan,
    records: &Path,
) -> Vec<String> {
    if record["version"].as_u64() != Some(2) {
        return vec![
            "critic record without `\"version\": 2` – re-run `/critic-review` (ADR-0052)".into(),
        ];
    }
    if record["tree"].as_str() != Some(tree) {
        return vec![format!("review belongs to state {}, not to {tree}", record["tree"])];
    }
    let Some(findings) = record["findings"].as_array() else {
        return vec!["review without a `findings` list".into()];
    };
    let mut problems: Vec<String> =
        findings.iter().enumerate().filter_map(|(i, f)| blocking(i + 1, f)).collect();
    if record["tier"].as_str() != Some(plan.tier.name()) {
        problems.push(format!(
            "record tier {} differs from the tier `{}` of the pushed range (ADR-0052)",
            record["tier"],
            plan.tier.name()
        ));
    }
    let reviews = reviews(record);
    if reviews.is_empty() {
        problems.push("review record without a non-empty `reviews` list".into());
    }
    problems.extend(review_problems(reviews));
    problems.extend(coverage_problems(reviews, plan));
    problems.extend(found_by_problems(findings, reviews));
    problems.extend(rotation_problem(record, records));
    if plan.tier == Tier::High {
        problems.extend(stopping_problem(reviews, findings));
    }
    problems
}

fn reviews(record: &Value) -> &[Value] {
    record["reviews"].as_array().map_or(&[], Vec::as_slice)
}

fn general(reviews: &[Value]) -> impl Iterator<Item = &Value> {
    reviews.iter().filter(|r| r["lens"] == "general")
}

/// Every review has a unique id, a model, a known lens and a round.
fn review_problems(reviews: &[Value]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut ids = BTreeSet::new();
    for (i, review) in reviews.iter().enumerate() {
        let index = i + 1;
        let text = |name: &str| review[name].as_str().filter(|s| !s.is_empty());
        if let Some(id) = text("id") {
            if !ids.insert(id) {
                problems.push(format!("review id `{id}` is not unique"));
            }
        } else {
            problems.push(format!("review {index}: `id` is missing or empty"));
        }
        if text("model").is_none() {
            problems.push(format!("review {index}: `model` is missing or empty"));
        }
        if !review["lens"].as_str().is_some_and(|lens| LENSES.contains(&lens)) {
            problems.push(format!(
                "review {index}: `lens` is missing or unknown (`{}`)",
                review["lens"]
            ));
        }
        if !review["round"].as_u64().is_some_and(|round| round >= 1) {
            problems.push(format!(
                "review {index}: `round` is not an integer >= 1 (`{}`)",
                review["round"]
            ));
        }
    }
    problems
}

/// The reviews cover the general reviews and lenses the plan requires.
fn coverage_problems(reviews: &[Value], plan: &Plan) -> Vec<String> {
    let mut problems = Vec::new();
    let models: BTreeSet<&str> = general(reviews).filter_map(|r| r["model"].as_str()).collect();
    if plan.tier == Tier::High && models.len() < 2 {
        problems.push("tier high needs `general` reviews on two different models".into());
    } else if models.is_empty() {
        problems.push("the record needs a `general` review".into());
    }
    for lens in plan.lenses() {
        if !reviews.iter().any(|r| r["lens"] == lens) {
            problems.push(format!("tier high needs a review with lens `{lens}`"));
        }
    }
    problems
}

/// Every finding names the reviews that found it.
fn found_by_problems(findings: &[Value], reviews: &[Value]) -> Vec<String> {
    let mut problems = Vec::new();
    for (i, finding) in findings.iter().enumerate() {
        let index = i + 1;
        let Some(ids) = finding["found_by"].as_array().filter(|ids| !ids.is_empty()) else {
            problems.push(format!(
                "finding {index}: `found_by` is missing or empty (`{}`)",
                finding["found_by"]
            ));
            continue;
        };
        for id in ids.iter().filter(|id| !reviews.iter().any(|r| r["id"] == **id)) {
            problems
                .push(format!("finding {index}: `found_by` names no review of the record ({id})"));
        }
    }
    problems
}

/// After a fix, a general review uses a model that no general review of the
/// previous round used (ADR-0052 decision 2). Fails closed without the
/// previous record.
fn rotation_problem(record: &Value, records: &Path) -> Option<String> {
    let previous_tree = match &record["previous_tree"] {
        Value::Null => return None,
        Value::String(tree) => tree,
        other => return Some(format!("`previous_tree` is neither a tree nor null ({other})")),
    };
    let file = records.join(format!("{previous_tree}.json"));
    let previous = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let Some(previous) = previous else {
        return Some(format!(
            "previous critic record {} is missing or not valid JSON – the model rotation \
             cannot be checked (ADR-0052)",
            file.display()
        ));
    };
    let used: BTreeSet<&str> =
        general(reviews(&previous)).filter_map(|r| r["model"].as_str()).collect();
    let rotated = general(reviews(record))
        .filter_map(|r| r["model"].as_str())
        .any(|model| !used.contains(model));
    (!rotated).then(|| {
        format!(
            "no `general` review uses a model the previous round {previous_tree} did not use \
             ({used:?}) (ADR-0052)"
        )
    })
}

/// The stopping rule (ADR-0052 decision 4): the first two general reviews on
/// different models are two samples; if they leave at least one severe
/// finding undiscovered by estimate, a general review on a third model is
/// required.
fn stopping_problem(reviews: &[Value], findings: &[Value]) -> Option<String> {
    let generals: Vec<&Value> = general(reviews).collect();
    let a = *generals.first()?;
    let b = *generals.iter().find(|r| r["model"] != a["model"])?;
    let found_by = |finding: &Value, review: &Value| {
        finding["found_by"].as_array().is_some_and(|ids| ids.contains(&review["id"]))
    };
    let severe: Vec<&Value> = findings
        .iter()
        .filter(|f| matches!(f["severity"].as_str(), Some("blocker" | "major")))
        .filter(|f| f["verdict"] != "refuted")
        .collect();
    let count = |hit: &dyn Fn(&Value) -> bool| severe.iter().filter(|f| hit(f)).count();
    let estimate = undiscovered(
        count(&|f| found_by(f, a)),
        count(&|f| found_by(f, b)),
        count(&|f| found_by(f, a) && found_by(f, b)),
    );
    let third = generals.iter().any(|r| r["model"] != a["model"] && r["model"] != b["model"]);
    (estimate >= 1.0 && !third).then(|| {
        format!(
            "stopping rule: an estimated {estimate:.1} blocker/major findings are undiscovered – \
             run a third `general` review on a model other than {} and {} (ADR-0052)",
            a["model"], b["model"]
        )
    })
}

/// The Chapman estimate of undiscovered defects from two samples of `n1` and
/// `n2` defects with `m` in both (ADR-0052 decision 4).
pub(crate) fn undiscovered(n1: usize, n2: usize, m: usize) -> f64 {
    let (n1, n2, m) = (n1 as f64, n2 as f64, m as f64);
    (n1 + 1.0) * (n2 + 1.0) / (m + 1.0) - 1.0 - (n1 + n2 - m)
}

/// The problem of the `index`-th finding, if it blocks or is malformed.
fn blocking(index: usize, finding: &Value) -> Option<String> {
    let field = |name: &str, allowed: &[&str]| match finding[name].as_str() {
        Some(value) if allowed.contains(&value) => Ok(value),
        _ => Err(format!("finding {index}: `{name}` is missing or unknown (`{}`)", finding[name])),
    };
    let checked = field("severity", &["blocker", "major", "minor"]).and_then(|severity| {
        if severity == "minor" {
            return Ok(None);
        }
        let verdict = field("verdict", &["confirmed", "refuted", "unverified"])?;
        let resolution = field("resolution", &["open", "fixed", "accepted"])?;
        let reason = finding["accepted_reason"].as_str().is_some_and(|r| !r.trim().is_empty());
        Ok((verdict != "refuted" && !(resolution == "accepted" && reason)).then(|| {
            format!(
                "unresolved finding {} {}:{} – {}",
                finding["rule"].as_str().unwrap_or("?"),
                finding["file"].as_str().unwrap_or("?"),
                finding["line"],
                finding["summary"].as_str().unwrap_or("")
            )
        }))
    });
    checked.unwrap_or_else(Some)
}

#[cfg(test)]
mod tests;
