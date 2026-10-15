//! Critic records (ADR-0041). The critic itself is a model and not
//! deterministic; what is checked here is that a review of exactly the pushed
//! tree exists and leaves no blocking finding unresolved.
//!
//! Record: `target/critic/<tree>.json`
//! `{"tree": "<sha>", "findings": [{"rule": "K-03", "severity": "blocker|major|minor",
//!   "verdict": "confirmed|refuted|unverified", "resolution": "open|fixed|accepted",
//!   "accepted_reason": "...", ...}]}`

use std::path::Path;

use serde_json::Value;

use crate::git;
use crate::paths::{self, Kind};

pub(crate) const RECORD_DIR: &str = "target/critic";

/// Requires a record for `tip` if any commit in `rev_list_args` changes production code.
pub(crate) fn check(root: &Path, tip: &str, rev_list_args: &[String]) -> Result<(), Vec<String>> {
    // `-z`: paths unquoted, as they are (git quotes non-ASCII, tab, `"`, `\`).
    let mut args = vec!["log", "--format=", "--name-only", "-z"];
    args.extend(rev_list_args.iter().map(String::as_str));
    let changed = git(root, &args).map_err(|e| vec![e])?;
    if !changed.split('\0').any(|f| paths::kind(f) == Kind::Code) {
        return Ok(());
    }
    let tree = git(root, &["rev-parse", &format!("{tip}^{{tree}}")]).map_err(|e| vec![e])?;
    let tree = tree.trim();
    let file = root.join(RECORD_DIR).join(format!("{tree}.json"));
    let text = std::fs::read_to_string(&file).map_err(|_| {
        vec![format!(
            "no critic review for this state ({}) – run `/critic-review` (ADR-0041)",
            file.display()
        )]
    })?;
    let record: Value = serde_json::from_str(&text)
        .map_err(|e| vec![format!("{}: not valid JSON: {e}", file.display())])?;
    let problems = open_blockers(&record, tree);
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// Blocker/major findings that are neither refuted nor accepted with a
/// reason, plus format errors; the record fails closed.
pub(crate) fn open_blockers(record: &Value, tree: &str) -> Vec<String> {
    if record["tree"].as_str() != Some(tree) {
        return vec![format!("review belongs to state {}, not to {tree}", record["tree"])];
    }
    let Some(findings) = record["findings"].as_array() else {
        return vec!["review without a `findings` list".into()];
    };
    findings.iter().enumerate().filter_map(|(i, f)| blocking(i + 1, f)).collect()
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
