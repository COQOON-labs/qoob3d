//! Critic records (ADR-0041) and the review plan (ADR-0052). The critic itself
//! is a model and not deterministic; what is checked here is that the reviews
//! of exactly the pushed tree cover the plan of the pushed range and leave no
//! blocking finding unresolved.
//!
//! The plan follows from the commits of the range (including what merge
//! commits change themselves), the tip's committer date and the escape log
//! `docs/escapes.md` as of the tip, the range's boundary commits and the
//! commits the range excludes, each log read with its own header: tier
//! `none` (neither production code nor the harness changed), `standard` or
//! `high` (harness, OCCT boundary, format, size or a recent escape).
//!
//! Record (version 2): `target/critic/<tree>.json`; `head` is the pushed tip
//! commit, `base` one of the range's boundary commits (null without one).
//! `{"version": 2, "tree": "<sha>", "base": "<sha>", "head": "<sha>",
//!   "tier": "standard|high", "previous_tree": "<tree>|null",
//!   "reviews": [{"id": "r1", "model": "opus|fable|sonnet",
//!     "lens": "general|tests|security|robustness",
//!     "round": 1, "scope": "diff|files"}],
//!   "findings": [{"rule": "K-03", "severity": "blocker|major|minor",
//!     "verdict": "confirmed|refuted|unverified", "resolution": "open|fixed|accepted",
//!     "accepted_reason": "...", "found_by": ["r1"], ...}]}`
//!
//! `scope` is absent or `diff` for a review of the whole diff, `files` for a
//! review of one file group; only whole-diff `general` reviews count as the
//! general samples (coverage, rotation, stopping rule).

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::paths::{self, Kind};
use crate::{docs, git};

pub(crate) const RECORD_DIR: &str = "target/critic";

/// The escape log (ADR-0052 decision 5).
const ESCAPES: &str = "docs/escapes.md";
/// An escape row counts if it is dated less than this many days before the
/// tip, or after it (a branch committed before the row landed on main).
const ESCAPE_WINDOW_DAYS: i64 = 180;
/// The models a review may name: the Agent tool's names the skill uses.
const MODELS: &[&str] = &["opus", "fable", "sonnet"];
/// More changed production code lines than this make a diff high-risk.
const MAX_STANDARD_LINES: usize = 400;

/// Review depth of a pushed range (ADR-0052 decision 1), in rising order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tier {
    None,
    Standard,
    High,
}

impl Tier {
    const ALL: [Tier; 3] = [Tier::None, Tier::Standard, Tier::High];

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

    /// The lens review the trigger requires beside `tests`.
    fn lens(self) -> Option<Lens> {
        match self {
            Trigger::Harness => Some(Lens::Security),
            Trigger::OcctBoundary | Trigger::Format => Some(Lens::Robustness),
            Trigger::Size | Trigger::Escape => None,
        }
    }
}

/// What a critic looks at (ADR-0052 decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Lens {
    General,
    Tests,
    Security,
    Robustness,
}

impl Lens {
    const ALL: [Lens; 4] = [Lens::General, Lens::Tests, Lens::Security, Lens::Robustness];

    fn name(self) -> &'static str {
        match self {
            Lens::General => "general",
            Lens::Tests => "tests",
            Lens::Security => "security",
            Lens::Robustness => "robustness",
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
    /// Added + deleted lines of `Code` files over the range (size trigger).
    pub(crate) code_lines: usize,
}

impl Plan {
    /// The lens reviews required beside the general ones.
    fn lenses(&self) -> BTreeSet<Lens> {
        if self.tier != Tier::High {
            return BTreeSet::new();
        }
        let triggered = self.reasons.iter().filter_map(|r| r.trigger.lens());
        std::iter::once(Lens::Tests).chain(triggered).collect()
    }
}

/// The `--plan` output: tier, reasons and the required reviews.
impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "tier: {}", self.tier.name())?;
        writeln!(f, "code lines: {}", self.code_lines)?;
        for reason in &self.reasons {
            writeln!(f, "reason: {} ({})", reason.trigger.name(), reason.path)?;
        }
        match self.tier {
            Tier::None => writeln!(f, "required: no review"),
            Tier::Standard => writeln!(f, "required: 1 review with lens general"),
            Tier::High => {
                let lenses: Vec<&str> = self.lenses().into_iter().map(Lens::name).collect();
                writeln!(
                    f,
                    "required: 2 reviews with lens general on different models; lenses {}",
                    lenses.join(", ")
                )
            }
        }
    }
}

/// `cargo xtask review [--plan] <tip> <rev-list args…>`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Invocation<'a> {
    Plan { tip: &'a str, revs: &'a [String] },
    Gate { tip: &'a str, revs: &'a [String] },
}

/// The arguments after `review`; `None` is a usage error. `--plan` is never
/// a tip.
pub(crate) fn parse_args(args: &[String]) -> Option<Invocation<'_>> {
    match args {
        [flag, tip, revs @ ..] if flag == "--plan" && tip != "--plan" && !revs.is_empty() => {
            Some(Invocation::Plan { tip, revs })
        }
        [tip, revs @ ..] if tip != "--plan" && !revs.is_empty() => {
            Some(Invocation::Gate { tip, revs })
        }
        _ => None,
    }
}

/// The plan of `changes` (path and changed lines, one entry per file and
/// commit) with the escape log `escapes` as of a tip committed on `tip_date`.
/// Every table in `escapes` is read with its own header, so several logs may
/// be joined.
pub(crate) fn plan(
    changes: &[(&str, usize)],
    escapes: &str,
    tip_date: &str,
) -> Result<Plan, Vec<String>> {
    let tip = day_number(tip_date)
        .ok_or_else(|| vec![format!("tip date `{tip_date}` is not YYYY-MM-DD")])?;
    let escaped = recent_escapes(escapes, tip)?;
    let code: Vec<(&str, usize)> =
        changes.iter().copied().filter(|(path, _)| paths::kind(path) == Kind::Code).collect();
    let code_lines: usize = code.iter().map(|c| c.1).sum();
    let first = |hit: fn(&str) -> bool| changes.iter().map(|c| c.0).find(|path| hit(path));
    let harness = first(paths::is_harness);
    if code.is_empty() && harness.is_none() {
        return Ok(Plan { tier: Tier::None, reasons: Vec::new(), code_lines });
    }
    let reasons: Vec<Reason> = [
        (Trigger::Harness, harness),
        (Trigger::OcctBoundary, first(paths::is_occt_boundary)),
        (Trigger::Format, first(paths::is_format)),
        (Trigger::Size, code.first().map(|c| c.0).filter(|_| code_lines > MAX_STANDARD_LINES)),
        (Trigger::Escape, code.iter().map(|c| c.0).find(|path| paths::listed(&escaped, path))),
    ]
    .into_iter()
    .filter_map(|(trigger, path)| Some(Reason { trigger, path: path?.to_owned() }))
    .collect();
    let tier = if reasons.is_empty() { Tier::Standard } else { Tier::High };
    Ok(Plan { tier, reasons, code_lines })
}

/// The plan of the commits in `rev_list_args`, pushed as `tip`.
pub(crate) fn plan_range(
    root: &Path,
    tip: &str,
    rev_list_args: &[String],
) -> Result<Plan, Vec<String>> {
    planned_range(root, tip, rev_list_args).map(|(plan, _)| plan)
}

/// [`plan_range`] and the boundary commits of the range.
fn planned_range(
    root: &Path,
    tip: &str,
    rev_list_args: &[String],
) -> Result<(Plan, Vec<String>), Vec<String>> {
    let git = |args: &[&str]| git(root, args).map_err(|e| vec![e]);
    let with_revs = |args: &[&'static str]| {
        let mut args = args.to_vec();
        args.extend(rev_list_args.iter().map(String::as_str));
        git(&args)
    };
    // `-z`: paths unquoted, as they are (git quotes non-ASCII, tab, `"`, `\`).
    // `--no-renames`: a rename changes the old path too. `remerge`: what a
    // merge commit changes beyond merging its parents (conflict resolutions).
    let numstat = with_revs(&[
        "log",
        "--format=",
        "--numstat",
        "--no-renames",
        "-z",
        "--diff-merges=remerge",
    ])?;
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
    let boundaries: Vec<String> = with_revs(&["rev-list", "--boundary"])?
        .lines()
        .filter_map(|line| line.strip_prefix('-'))
        .map(str::to_owned)
        .collect();
    // Neither the range nor a branch forked before a row landed on main may
    // drop the row that makes it high-risk: the logs at the tip, the
    // boundaries and the excluded commits (`--not`, `^`, `a..b`, globs) count.
    let parsed = with_revs(&["rev-parse"])?;
    let excluded = parsed.lines().filter_map(|line| line.strip_prefix('^'));
    let mut logs = BTreeSet::new();
    for commit in std::iter::once(tip).chain(boundaries.iter().map(String::as_str)).chain(excluded)
    {
        logs.insert(escape_log(root, commit)?);
    }
    let escapes = logs.into_iter().collect::<Vec<_>>().join("\n\n");
    Ok((plan(&changes, &escapes, tip_date.trim())?, boundaries))
}

/// The escape log at `commit`; empty where it did not exist yet.
fn escape_log(root: &Path, commit: &str) -> Result<String, Vec<String>> {
    let git = |args: &[&str]| git(root, args).map_err(|e| vec![e]);
    if git(&["ls-tree", "--name-only", commit, ESCAPES])?.is_empty() {
        return Ok(String::new());
    }
    git(&["show", &format!("{commit}:{ESCAPES}")])
}

/// The backticked `Where` paths of the escape-log rows dated within the
/// window of the tip day `tip`. Each table (consecutive `|` lines) is read
/// with its own header; the line after the header is the separator. A row
/// with another cell count than its header, no backticked path or an
/// unparsable date fails closed.
fn recent_escapes(escapes: &str, tip: i64) -> Result<Vec<&str>, Vec<String>> {
    let mut lines = escapes.lines().map(str::trim).peekable();
    let mut places = Vec::new();
    let mut errors = Vec::new();
    while lines.peek().is_some() {
        let table: Vec<&str> = lines
            .by_ref()
            .skip_while(|l| !l.starts_with('|'))
            .take_while(|l| l.starts_with('|'))
            .collect();
        let [header, _separator, rows @ ..] = table.as_slice() else {
            continue;
        };
        let header = cells(header);
        let column = |name: &str| {
            header
                .iter()
                .position(|cell| *cell == name)
                .ok_or_else(|| format!("{ESCAPES}: table without a `{name}` column"))
        };
        let (date_column, where_column) = match (column("Date"), column("Where")) {
            (Ok(date), Ok(place)) => (date, place),
            (date, place) => {
                errors.extend(date.err().into_iter().chain(place.err()));
                continue;
            }
        };
        for row in rows {
            let cells = cells(row);
            if cells.len() != header.len() {
                errors.push(format!(
                    "{ESCAPES}: row with {} cells, the header has {} – {row}",
                    cells.len(),
                    header.len()
                ));
                continue;
            }
            let paths: Vec<&str> = cells[where_column]
                .split('`')
                .skip(1)
                .step_by(2)
                .filter(|p| !p.is_empty())
                .collect();
            if paths.is_empty() {
                errors.push(format!("{ESCAPES}: row without a backticked path in `Where` – {row}"));
                continue;
            }
            let date = cells[date_column];
            let Some(day) = day_number(date) else {
                errors.push(format!(
                    "{ESCAPES}: row date `{date}` is not a valid YYYY-MM-DD date – {row}"
                ));
                continue;
            };
            if tip - day < ESCAPE_WINDOW_DAYS {
                places.extend(paths);
            }
        }
    }
    if errors.is_empty() { Ok(places) } else { Err(errors) }
}

/// The trimmed cells of a table row, split on every `|` not escaped as `\|`.
fn cells(row: &str) -> Vec<&str> {
    let mut cells = Vec::new();
    let mut start = 0;
    for (i, _) in row.match_indices('|') {
        if !row[..i].ends_with('\\') {
            cells.push(row[start..i].trim());
            start = i + 1;
        }
    }
    cells.push(row[start..].trim());
    cells
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
/// `rev_list_args` changes production code or the harness.
pub(crate) fn check(root: &Path, tip: &str, rev_list_args: &[String]) -> Result<(), Vec<String>> {
    let (plan, boundaries) = planned_range(root, tip, rev_list_args)?;
    if plan.tier == Tier::None {
        return Ok(());
    }
    let resolve = |rev: String| git(root, &["rev-parse", &rev]).map_err(|e| vec![e]);
    let commit = resolve(format!("{tip}^{{commit}}"))?;
    let tree = resolve(format!("{tip}^{{tree}}"))?;
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
    let mut problems = record_problems(&record, tree, &plan, &records);
    problems.extend(predecessor_problem(root, &record, tree, rev_list_args, &records)?);
    problems.extend(range_problems(&record, commit.trim(), &boundaries));
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// Rotation is not opt-in (ADR-0052 decision 2): the earlier records of the
/// branch are the version-2 `*.json` files in `records` of another tree whose
/// `head` the range selects. If there are any, `previous_tree` names one with
/// the highest round. Other files are ignored and cannot block a push.
fn predecessor_problem(
    root: &Path,
    record: &Value,
    tree: &str,
    rev_list_args: &[String],
    records: &Path,
) -> Result<Option<String>, Vec<String>> {
    let mut args = vec!["rev-list"];
    args.extend(rev_list_args.iter().map(String::as_str));
    let commits = git(root, &args).map_err(|e| vec![e])?;
    let commits: BTreeSet<&str> = commits.lines().collect();
    let entries = std::fs::read_dir(records)
        .map_err(|e| vec![format!("{}: cannot be listed: {e}", records.display())])?;
    let predecessors: BTreeSet<(u64, String)> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "json" {
                return None;
            }
            let earlier: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            let other = earlier["tree"].as_str().filter(|other| *other != tree)?;
            (earlier["version"].as_u64() == Some(2)
                && earlier["head"].as_str().is_some_and(|head| commits.contains(head)))
            .then(|| (highest_round(&earlier), other.to_owned()))
        })
        .collect();
    let Some((round, _)) = predecessors.last() else {
        return Ok(None);
    };
    let latest: Vec<&str> =
        predecessors.iter().filter(|p| p.0 == *round).map(|p| p.1.as_str()).collect();
    if latest.iter().any(|t| record["previous_tree"] == *t) {
        return Ok(None);
    }
    Ok(Some(format!(
        "previous_tree {} must be {} (the latest earlier record of this branch, round {round}) \
         (ADR-0052)",
        record["previous_tree"],
        latest.join(" or ")
    )))
}

/// The record names the reviewed range: `head` is the tip commit, `base` one
/// of the range's boundary commits (absent or null for a range without one).
pub(crate) fn range_problems(record: &Value, tip: &str, boundaries: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    if record["head"].as_str() != Some(tip) {
        problems
            .push(format!("record head {} is not the pushed tip {tip} (ADR-0052)", record["head"]));
    }
    let base = &record["base"];
    let known = if boundaries.is_empty() {
        base.is_null()
    } else {
        base.as_str().is_some_and(|base| boundaries.iter().any(|b| b == base))
    };
    if !known {
        let expected = if boundaries.is_empty() { "none".into() } else { boundaries.join(", ") };
        problems.push(format!(
            "record base {base} is not a boundary commit of the pushed range ({expected}) \
             (ADR-0052)"
        ));
    }
    problems
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
    let tier = Tier::ALL.into_iter().find(|t| record["tier"] == t.name());
    if tier.is_none_or(|tier| tier < plan.tier) {
        problems.push(format!(
            "record tier {} is below the tier `{}` of the pushed range (ADR-0052)",
            record["tier"],
            plan.tier.name()
        ));
    }
    let reviews = reviews_of(record);
    if reviews.is_empty() {
        problems.push("review record without a non-empty `reviews` list".into());
    }
    // The round follows from the previous record; unknown if that is broken.
    let round = match previous_record(record, records) {
        Ok(None) => Some(1),
        Ok(Some(previous)) => {
            problems.extend(rotation_problem(record, &previous));
            Some(highest_round(&previous) + 1)
        }
        Err(problem) => {
            problems.push(problem);
            None
        }
    };
    problems.extend(review_problems(reviews, round));
    problems.extend(coverage_problems(reviews, plan));
    problems.extend(found_by_problems(findings, reviews));
    if plan.tier == Tier::High {
        problems.extend(stopping_problem(reviews, findings));
    }
    problems
}

fn reviews_of(record: &Value) -> &[Value] {
    record["reviews"].as_array().map_or(&[], Vec::as_slice)
}

/// The highest `round` of the record's reviews, 0 without one.
fn highest_round(record: &Value) -> u64 {
    reviews_of(record).iter().filter_map(|r| r["round"].as_u64()).max().unwrap_or(0)
}

/// The `general` reviews of the whole diff (no `files` or unknown scope).
fn whole_diff_general(reviews: &[Value]) -> impl Iterator<Item = &Value> {
    reviews
        .iter()
        .filter(|r| r["lens"] == Lens::General.name())
        .filter(|r| r.get("scope").is_none_or(|scope| scope == "diff"))
}

/// The two general samples (ADR-0052 decision 4): the first whole-diff
/// general review and the first one on another model, as far as present.
fn samples(reviews: &[Value]) -> Vec<&Value> {
    let mut samples: Vec<&Value> = Vec::new();
    for review in whole_diff_general(reviews) {
        if samples.len() < 2 && samples.iter().all(|s| s["model"] != review["model"]) {
            samples.push(review);
        }
    }
    samples
}

/// Every review has a unique id, a model, a known lens, a known scope and
/// `round` (if known, else any round >= 1).
fn review_problems(reviews: &[Value], round: Option<u64>) -> Vec<String> {
    let mut problems = Vec::new();
    let mut ids = BTreeSet::new();
    for (i, review) in reviews.iter().enumerate() {
        let index = i + 1;
        if let Some(id) = review["id"].as_str().filter(|id| !id.is_empty()) {
            if !ids.insert(id) {
                problems.push(format!("review id `{id}` is not unique"));
            }
        } else {
            problems.push(format!("review {index}: `id` is missing or empty"));
        }
        if !review["model"].as_str().is_some_and(|model| MODELS.contains(&model)) {
            problems.push(format!(
                "review {index}: `model` is missing or unknown (`{}`)",
                review["model"]
            ));
        }
        if !Lens::ALL.iter().any(|lens| review["lens"] == lens.name()) {
            problems.push(format!(
                "review {index}: `lens` is missing or unknown (`{}`)",
                review["lens"]
            ));
        }
        if review.get("scope").is_some_and(|scope| scope != "diff" && scope != "files") {
            problems.push(format!(
                "review {index}: `scope` is neither `diff` nor `files` (`{}`)",
                review["scope"]
            ));
        }
        let actual = review["round"].as_u64();
        if round.map_or(actual.is_none_or(|r| r < 1), |round| actual != Some(round)) {
            let expected = round.map_or("an integer >= 1".into(), |round| round.to_string());
            problems.push(format!(
                "review {index}: `round` is `{}`, expected {expected}",
                review["round"]
            ));
        }
    }
    problems
}

/// The reviews cover the general reviews and lenses the plan requires.
fn coverage_problems(reviews: &[Value], plan: &Plan) -> Vec<String> {
    let mut problems = Vec::new();
    let samples = samples(reviews).len();
    if plan.tier == Tier::High && samples < 2 {
        problems
            .push("tier high needs whole-diff `general` reviews on two different models".into());
    } else if samples == 0 {
        problems.push("the record needs a whole-diff `general` review".into());
    }
    for lens in plan.lenses() {
        if !reviews.iter().any(|r| r["lens"] == lens.name()) {
            problems.push(format!("tier high needs a review with lens `{}`", lens.name()));
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

/// The record of the previous round, `None` for a first round. A previous
/// tree that is no object id, or a previous record that is missing, broken
/// or has no whole-diff general review, is one problem (fail closed).
fn previous_record(record: &Value, records: &Path) -> Result<Option<Value>, String> {
    let id = match &record["previous_tree"] {
        Value::Null => return Ok(None),
        Value::String(id)
            if matches!(id.len(), 40 | 64)
                && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
        {
            id
        }
        other => return Err(format!("`previous_tree` {other} is no tree id (ADR-0052)")),
    };
    let file = records.join(format!("{id}.json"));
    std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|previous| {
            previous["version"].as_u64() == Some(2)
                && previous["tree"] == id.as_str()
                && !samples(reviews_of(previous)).is_empty()
        })
        .map(Some)
        .ok_or_else(|| {
            format!(
                "previous critic record {} is missing or not a version-2 record of {id} with a \
                 whole-diff general review – the model rotation cannot be checked (ADR-0052)",
                file.display()
            )
        })
}

/// After a fix, a whole-diff general review uses a model outside the
/// previous round's two general samples (ADR-0052 decision 2).
fn rotation_problem(record: &Value, previous: &Value) -> Option<String> {
    let used: BTreeSet<&str> =
        samples(reviews_of(previous)).iter().filter_map(|r| r["model"].as_str()).collect();
    let rotated = whole_diff_general(reviews_of(record))
        .filter_map(|r| r["model"].as_str())
        .any(|model| !used.contains(model));
    (!rotated).then(|| {
        format!(
            "no whole-diff `general` review uses a model outside the general samples {used:?} \
             of the previous round {} (ADR-0052)",
            previous["tree"]
        )
    })
}

/// The stopping rule (ADR-0052 decision 4): the two general samples; if they
/// leave at least one severe finding undiscovered by estimate, a whole-diff
/// general review on a third model is required.
fn stopping_problem(reviews: &[Value], findings: &[Value]) -> Option<String> {
    let [a, b] = samples(reviews)[..] else {
        return None;
    };
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
    let third =
        whole_diff_general(reviews).any(|r| r["model"] != a["model"] && r["model"] != b["model"]);
    (estimate >= 1.0 && !third).then(|| {
        format!(
            "stopping rule: an estimated {estimate:.1} blocker/major findings are undiscovered – \
             run a third whole-diff `general` review on a model other than {} and {} (ADR-0052)",
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
