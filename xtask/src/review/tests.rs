use serde_json::json;

use super::*;

const TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn finding(severity: &str, verdict: &str, resolution: &str) -> Value {
    json!({"rule": "K-03", "severity": severity, "verdict": verdict, "resolution": resolution})
}

fn accepted(severity: &str, verdict: &str, reason: &str) -> Value {
    json!({"rule": "K-03", "severity": severity, "verdict": verdict, "resolution": "accepted",
           "accepted_reason": reason})
}

fn problems_for(findings: Vec<Value>) -> Vec<String> {
    open_blockers(&json!({"tree": TREE, "findings": findings}), TREE)
}

/// Prefix of the format problem (addendum F5).
fn format_problem(index: usize, field: &str) -> String {
    format!("finding {index}: `{field}` is missing or unknown (")
}

fn assert_no_format_problem(problems: &[String], what: &str) {
    assert!(!problems.iter().any(|p| p.contains("is missing or unknown")), "{what}: {problems:?}");
}

// ADR-0041: confirmed, open blocker/major findings block, one problem each.
#[test]
fn open_confirmed_blockers_and_majors_block() {
    let record = json!({"tree": TREE, "findings": [
        finding("blocker", "confirmed", "open"),
        finding("major", "confirmed", "open"),
    ]});
    assert_eq!(open_blockers(&record, TREE).len(), 2);
}

// ADR-0041 (addendum F5): minor findings, refuted findings and accepted
// findings with a reason do not block.
#[test]
fn harmless_findings_do_not_block() {
    for f in [
        finding("minor", "confirmed", "open"),
        finding("blocker", "refuted", "open"),
        finding("major", "refuted", "open"),
        finding("major", "refuted", "fixed"),
        accepted("major", "confirmed", "upstream bug, tracked in #12"),
        accepted("blocker", "confirmed", "deliberate, see ADR-0031"),
        accepted("major", "unverified", "cannot be reproduced without hardware"),
    ] {
        let record = json!({"tree": TREE, "findings": [f.clone()]});
        assert!(open_blockers(&record, TREE).is_empty(), "{f}");
    }
    let empty = json!({"tree": TREE, "findings": []});
    assert!(open_blockers(&empty, TREE).is_empty());
}

// ADR-0041 (addendum F5): only the blocking findings of a mixed list are
// reported. Blocking: blocker/open, major/fixed (a fix needs a new review),
// major/open, blocker/accepted without a reason = 4.
#[test]
fn counts_only_blocking_findings_in_mixed_list() {
    let record = json!({"tree": TREE, "findings": [
        finding("minor", "confirmed", "open"),
        finding("blocker", "confirmed", "open"),
        finding("blocker", "refuted", "open"),
        finding("major", "confirmed", "fixed"),
        finding("major", "confirmed", "open"),
        finding("blocker", "confirmed", "accepted"),
        accepted("major", "confirmed", "documented in ADR-0031"),
    ]});
    assert_eq!(open_blockers(&record, TREE).len(), 4);
}

// ADR-0041: a review applies to exactly one tree; another or missing tree is
// exactly one problem, regardless of the findings.
#[test]
fn record_for_another_tree_is_one_problem() {
    let other = json!({"tree": "deadbeef", "findings": [
        finding("blocker", "confirmed", "open"),
        finding("major", "confirmed", "open"),
    ]});
    assert_eq!(open_blockers(&other, TREE).len(), 1);
    let missing = json!({"findings": []});
    assert_eq!(open_blockers(&missing, TREE).len(), 1);
}

// ADR-0041: a record without a `findings` array is exactly one problem.
#[test]
fn record_without_findings_is_one_problem() {
    for record in [
        json!({"tree": TREE}),
        json!({"tree": TREE, "findings": {"rule": "K-01"}}),
        json!({"tree": TREE, "findings": null}),
    ] {
        assert_eq!(open_blockers(&record, TREE).len(), 1, "{record}");
    }
}

// ADR-0041 (addendum F5): a major finding without a verdict is a format
// problem (fails closed instead of being skipped).
#[test]
fn f5_major_without_verdict_is_a_format_problem() {
    let f = json!({"rule": "K-03", "severity": "major", "resolution": "open"});
    let problems = problems_for(vec![f]);
    assert!(!problems.is_empty(), "missing verdict passed");
    assert!(problems.iter().any(|p| p.contains(&format_problem(1, "verdict"))), "{problems:?}");
}

// ADR-0041 (addendum F5): severity is case-sensitive; `Major` and a missing
// severity are format problems that name the field (and the bad value).
#[test]
fn f5_unknown_or_missing_severity_is_a_format_problem() {
    let problems = problems_for(vec![finding("Major", "confirmed", "open")]);
    let problem = problems.iter().find(|p| p.contains(&format_problem(1, "severity")));
    assert!(problem.is_some_and(|p| p.contains("Major")), "{problems:?}");

    for f in [
        json!({"rule": "K-03", "verdict": "confirmed", "resolution": "open"}),
        finding("critical", "confirmed", "open"),
        finding("", "confirmed", "open"),
        json!({"rule": "K-03", "severity": 2, "verdict": "confirmed", "resolution": "open"}),
    ] {
        let problems = problems_for(vec![f.clone()]);
        assert!(
            problems.iter().any(|p| p.contains(&format_problem(1, "severity"))),
            "{f}: {problems:?}"
        );
    }
}

// ADR-0041 (addendum F5): an unknown verdict or resolution of a blocker/major
// finding is a format problem naming the field and the value.
#[test]
fn f5_unknown_verdict_or_resolution_is_a_format_problem() {
    for (f, field, value) in [
        (finding("major", "maybe", "open"), "verdict", "maybe"),
        (finding("blocker", "Confirmed", "open"), "verdict", "Confirmed"),
        (finding("major", "confirmed", "done"), "resolution", "done"),
        (finding("blocker", "confirmed", "Open"), "resolution", "Open"),
    ] {
        let problems = problems_for(vec![f.clone()]);
        let problem = problems.iter().find(|p| p.contains(&format_problem(1, field)));
        assert!(problem.is_some_and(|p| p.contains(value)), "{f}: {problems:?}");
    }
    // A missing resolution is a format problem even for a refuted finding:
    // the rule requires both fields on every blocker/major finding.
    let f = json!({"rule": "K-03", "severity": "blocker", "verdict": "refuted"});
    let problems = problems_for(vec![f]);
    assert!(problems.iter().any(|p| p.contains(&format_problem(1, "resolution"))), "{problems:?}");
}

// ADR-0041 (addendum F5): the index in a format problem is 1-based.
#[test]
fn f5_format_problem_index_is_one_based() {
    let problems = problems_for(vec![
        finding("minor", "confirmed", "open"),
        finding("major", "refuted", "open"),
        finding("major", "confirmed", "unknown-resolution"),
    ]);
    assert!(problems.iter().any(|p| p.contains(&format_problem(3, "resolution"))), "{problems:?}");
    assert!(
        !problems.iter().any(|p| p.contains("finding 1:") || p.contains("finding 2:")),
        "{problems:?}"
    );
}

// ADR-0041 (addendum F5): `unverified` is a valid verdict, but only
// `refuted` unblocks, so an unverified open major blocks (not as a format
// problem).
#[test]
fn f5_unverified_major_blocks() {
    for resolution in ["open", "fixed"] {
        let problems = problems_for(vec![finding("major", "unverified", resolution)]);
        assert_eq!(problems.len(), 1, "{resolution}: {problems:?}");
        assert_no_format_problem(&problems, resolution);
    }
}

// ADR-0041 (addendum F5): `fixed` does not unblock; a fix changes the tree,
// which needs a new review.
#[test]
fn f5_fixed_major_still_blocks() {
    for severity in ["major", "blocker"] {
        let problems = problems_for(vec![finding(severity, "confirmed", "fixed")]);
        assert_eq!(problems.len(), 1, "{severity}: {problems:?}");
        assert_no_format_problem(&problems, severity);
    }
}

// ADR-0041 (addendum F5): `accepted` unblocks only with a non-empty
// `accepted_reason`; missing, empty or non-string reasons block.
#[test]
fn f5_accepted_needs_a_reason() {
    for f in [
        finding("major", "confirmed", "accepted"),
        accepted("major", "confirmed", ""),
        accepted("blocker", "unverified", ""),
        json!({"rule": "K-03", "severity": "major", "verdict": "confirmed",
               "resolution": "accepted", "accepted_reason": null}),
    ] {
        let problems = problems_for(vec![f.clone()]);
        assert_eq!(problems.len(), 1, "{f}: {problems:?}");
    }
    assert!(problems_for(vec![accepted("major", "confirmed", "known OCCT limitation")]).is_empty());
}

// ADR-0041 (addendum F5): `refuted` unblocks whatever the resolution.
#[test]
fn f5_refuted_passes() {
    for severity in ["major", "blocker"] {
        for resolution in ["open", "fixed", "accepted"] {
            let problems = problems_for(vec![finding(severity, "refuted", resolution)]);
            assert!(problems.is_empty(), "{severity}/{resolution}: {problems:?}");
        }
    }
}

// ADR-0041 (addendum F5): minor findings never block, and verdict/resolution
// are only required on blocker/major findings.
#[test]
fn f5_minor_never_blocks() {
    for f in [
        finding("minor", "confirmed", "open"),
        finding("minor", "unverified", "fixed"),
        finding("minor", "confirmed", "accepted"),
        json!({"rule": "K-03", "severity": "minor"}),
        finding("minor", "whatever", "later"),
    ] {
        let problems = problems_for(vec![f.clone()]);
        assert!(problems.is_empty(), "{f}: {problems:?}");
    }
}

// ---------------------------------------------------------------------------
// Paths git quotes (critic finding N3 on 0600f48, user decision 2026-10-15):
// `git log --name-only` prints a non-ASCII, tab, `"` or `\` path in C quotes
// under `core.quotePath` (git's default, set locally here so a global `false`
// cannot hide the case). `cargo xtask review` must still see a production
// code change and require a critic record.
// ---------------------------------------------------------------------------

/// Runs git as Alice in `root` without global/system config or signing (a
/// fresh repository has no active hooks).
fn git_in(root: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Alice")
        .env("GIT_AUTHOR_EMAIL", "alice@example.com")
        .env("GIT_COMMITTER_NAME", "Alice")
        .env("GIT_COMMITTER_EMAIL", "alice@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A fresh repository with `core.quotePath=true` and one commit of `file`.
fn repo_with_commit(test: &str, file: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("qoob3d-review-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    git_in(&root, &["init", "-q", "-b", "main"]);
    git_in(&root, &["config", "core.quotePath", "true"]);
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "fn f() {}\n").unwrap();
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "Add code"]);
    root
}

// ADR-0041 (critic finding N3 on 0600f48): a code change whose path git quotes
// needs a critic record like any other. The plain name and the name with a
// space (never quoted) are controls that the setup reaches the check.
#[test]
fn quoted_code_paths_need_a_critic_record() {
    for (i, file) in [
        "crates/qoob3d-core/src/plain.rs",
        "crates/qoob3d-core/src/a b.rs",
        "crates/qoob3d-core/src/\u{e4}.rs",
        "crates/qoob3d-core/src/a\tb.rs",
        "crates/qoob3d-core/src/\"q\".rs",
        "crates/qoob3d-core/src/a\\b.rs",
    ]
    .into_iter()
    .enumerate()
    {
        let root = repo_with_commit(&format!("quoted_{i}"), file);
        let Err(problems) = check(&root, "HEAD", &["HEAD".to_owned()]) else {
            panic!("{file:?}: passed without a critic record");
        };
        assert!(problems.iter().any(|p| p.contains("no critic review")), "{file:?}: {problems:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
