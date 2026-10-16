use serde_json::json;

use super::*;

const TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
/// The tree of the reviewed state before the last fix (rotation, ADR-0052).
const PREV: &str = "1111111111111111111111111111111111111111";
const PANELS: &str = "crates/qoob3d-app/src/panels.rs";
/// The tip date every pure plan in these tests is computed for.
const TIP_DATE: &str = "2026-10-16";
const MODEL_A: &str = "claude-opus-5-5";
const MODEL_B: &str = "claude-sonnet-5";
const MODEL_C: &str = "fable-1";

// ---------------------------------------------------------------------------
// Builders: escape log, plans and version-2 records (ADR-0052 spec §1, §2).
// ---------------------------------------------------------------------------

/// The header of `docs/escapes.md` (ADR-0052 decision 5) with an intro that
/// contains backticks, followed by `rows`.
fn escapes(rows: &[String]) -> String {
    let mut text = String::from(
        "# Escape log (ADR-0052)\n\n\
         Status: `open` (class not covered yet), `covered` (a check exists).\n\n\
         | Date | Found by | What escaped | Where | Missed by | Class | Check for the class | Status |\n\
         | --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for row in rows {
        text.push_str(row);
        text.push('\n');
    }
    text
}

/// An escape-log row dated `date` whose `Where` cell is `place`.
fn escape_row(date: &str, place: &str) -> String {
    format!("| {date} | CI Linux | a defect | {place} | critic | a class | a check | open |")
}

/// The plan of `changes` (path, changed lines of one commit) with an empty
/// escape log.
fn plan_of(changes: &[(&str, usize)]) -> Plan {
    plan(changes, &escapes(&[]), TIP_DATE).expect("an empty escape log is valid")
}

/// Small app change: `standard` (spec acceptance criterion 1).
fn standard_plan() -> Plan {
    plan_of(&[(PANELS, 10)])
}

/// 401 changed code lines: `high` by size only, so the plan requires two
/// general reviews on different models and lens `tests`.
fn size_plan() -> Plan {
    plan_of(&[(PANELS, 401)])
}

/// `xtask/` is a harness path: `high`, additionally lens `security`.
fn harness_plan() -> Plan {
    plan_of(&[("xtask/src/main.rs", 10)])
}

/// The project format: `high`, additionally lens `robustness`.
fn format_plan() -> Plan {
    plan_of(&[("crates/qoob3d-core/src/document/mod.rs", 10)])
}

/// The OCCT boundary: `high`, additionally lens `robustness`.
fn occt_plan() -> Plan {
    plan_of(&[("crates/qoob3d-kernel/src/occt.rs", 10)])
}

fn triggers(plan: &Plan) -> Vec<Trigger> {
    plan.reasons.iter().map(|r| r.trigger).collect()
}

fn review(id: &str, model: &str, lens: &str) -> Value {
    json!({"id": id, "model": model, "lens": lens, "round": 1})
}

/// A version-2 record of `TREE` (spec §2), first round.
fn record(tier: &str, reviews: Vec<Value>, findings: Vec<Value>) -> Value {
    json!({
        "version": 2, "tree": TREE, "base": "a".repeat(40), "head": "b".repeat(40),
        "tier": tier, "previous_tree": null, "reviews": reviews, "findings": findings
    })
}

/// The minimal complete standard record: one general review `r1`.
fn standard_record(findings: Vec<Value>) -> Value {
    record("standard", vec![review("r1", MODEL_A, "general")], findings)
}

/// The minimal complete record for [`size_plan`]: two general reviews on
/// different models and a tests lens (on any model, here the first one).
fn high_reviews() -> Vec<Value> {
    vec![
        review("r1", MODEL_A, "general"),
        review("r2", MODEL_B, "general"),
        review("r3", MODEL_A, "tests"),
    ]
}

fn high_record(findings: Vec<Value>) -> Value {
    record("high", high_reviews(), findings)
}

fn with(mut record: Value, key: &str, value: Value) -> Value {
    record[key] = value;
    record
}

fn without(mut record: Value, key: &str) -> Value {
    record.as_object_mut().unwrap().remove(key);
    record
}

/// A directory that holds no records.
fn no_records() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("qoob3d-review-no-records-{}", std::process::id()))
}

fn problems(record: &Value, plan: &Plan) -> Vec<String> {
    record_problems(record, TREE, plan, &no_records())
}

fn assert_problem(problems: &[String], needle: &str, what: &str) {
    assert!(
        problems.iter().any(|p| p.contains(needle)),
        "{what}: no problem naming `{needle}`: {problems:?}"
    );
}

/// A finding of review `r1` (ADR-0041 fields plus `found_by`).
fn finding(severity: &str, verdict: &str, resolution: &str) -> Value {
    json!({"rule": "K-03", "severity": severity, "verdict": verdict, "resolution": resolution,
           "found_by": ["r1"]})
}

fn accepted(severity: &str, verdict: &str, reason: &str) -> Value {
    json!({"rule": "K-03", "severity": severity, "verdict": verdict, "resolution": "accepted",
           "accepted_reason": reason, "found_by": ["r1"]})
}

/// A non-blocking major finding that counts for the stopping rule: not
/// refuted, accepted with a reason.
fn counted(found_by: &[&str]) -> Value {
    json!({"rule": "K-09", "severity": "major", "verdict": "confirmed", "resolution": "accepted",
           "accepted_reason": "tracked in #7", "found_by": found_by,
           "file": "crates/qoob3d-app/src/panels.rs", "line": 1, "summary": "s"})
}

// ===========================================================================
// ADR-0041 behaviour, on version-2 records (spec acceptance: "Existing
// ADR-0041 behaviour still holds for v2 records"). Every count below is the
// ADR-0041 count; a complete v2 record adds no problem of its own.
// ===========================================================================

fn problems_for(findings: Vec<Value>) -> Vec<String> {
    problems(&standard_record(findings), &standard_plan())
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
    let problems = problems_for(vec![
        finding("blocker", "confirmed", "open"),
        finding("major", "confirmed", "open"),
    ]);
    assert_eq!(problems.len(), 2, "{problems:?}");
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
        let problems = problems_for(vec![f.clone()]);
        assert!(problems.is_empty(), "{f}: {problems:?}");
    }
    assert!(problems_for(vec![]).is_empty());
}

// ADR-0041 (addendum F5): only the blocking findings of a mixed list are
// reported. Blocking: blocker/open, major/fixed (a fix needs a new review),
// major/open, blocker/accepted without a reason = 4.
#[test]
fn counts_only_blocking_findings_in_mixed_list() {
    let problems = problems_for(vec![
        finding("minor", "confirmed", "open"),
        finding("blocker", "confirmed", "open"),
        finding("blocker", "refuted", "open"),
        finding("major", "confirmed", "fixed"),
        finding("major", "confirmed", "open"),
        finding("blocker", "confirmed", "accepted"),
        accepted("major", "confirmed", "documented in ADR-0031"),
    ]);
    assert_eq!(problems.len(), 4, "{problems:?}");
}

// ADR-0041: a review applies to exactly one tree; another or missing tree is
// exactly one problem, regardless of the findings.
#[test]
fn record_for_another_tree_is_one_problem() {
    let blocking =
        vec![finding("blocker", "confirmed", "open"), finding("major", "confirmed", "open")];
    let other = with(standard_record(blocking.clone()), "tree", json!("deadbeef"));
    assert_eq!(problems(&other, &standard_plan()).len(), 1);
    let missing = without(standard_record(blocking), "tree");
    assert_eq!(problems(&missing, &standard_plan()).len(), 1);
}

// ADR-0041: a record without a `findings` array is exactly one problem.
#[test]
fn record_without_findings_is_one_problem() {
    for record in [
        without(standard_record(vec![]), "findings"),
        with(standard_record(vec![]), "findings", json!({"rule": "K-01"})),
        with(standard_record(vec![]), "findings", json!(null)),
    ] {
        assert_eq!(problems(&record, &standard_plan()).len(), 1, "{record}");
    }
}

// ADR-0041 (addendum F5): a major finding without a verdict is a format
// problem (fails closed instead of being skipped).
#[test]
fn f5_major_without_verdict_is_a_format_problem() {
    let f = json!({"rule": "K-03", "severity": "major", "resolution": "open", "found_by": ["r1"]});
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
        json!({"rule": "K-03", "verdict": "confirmed", "resolution": "open", "found_by": ["r1"]}),
        finding("critical", "confirmed", "open"),
        finding("", "confirmed", "open"),
        json!({"rule": "K-03", "severity": 2, "verdict": "confirmed", "resolution": "open",
               "found_by": ["r1"]}),
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
    let f =
        json!({"rule": "K-03", "severity": "blocker", "verdict": "refuted", "found_by": ["r1"]});
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
               "resolution": "accepted", "accepted_reason": null, "found_by": ["r1"]}),
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
        json!({"rule": "K-03", "severity": "minor", "found_by": ["r1"]}),
        finding("minor", "whatever", "later"),
    ] {
        let problems = problems_for(vec![f.clone()]);
        assert!(problems.is_empty(), "{f}: {problems:?}");
    }
}

// ===========================================================================
// Spec §1: the review plan (ADR-0052 decision 1), pure part.
// ===========================================================================

// ADR-0052 §1 / acceptance 1: one app file, 10 lines, no escape row.
#[test]
fn plan_of_a_small_app_change_is_standard() {
    let plan = standard_plan();
    assert_eq!(plan.tier, Tier::Standard);
    assert!(plan.reasons.is_empty(), "{:?}", plan.reasons);
}

// ADR-0052 §1, fix round 1 ("harness-only changes need a review"): tier
// `none` only when neither a `Code` file nor a harness path changed, even
// for test files under the format prefix, non-harness files of other kinds
// and big diffs.
#[test]
fn plan_without_code_or_harness_is_none() {
    let cases: &[&[(&str, usize)]] = &[
        &[],
        &[("docs/adr/0052-x.md", 5000)],
        &[("crates/qoob3d-core/src/document/tests.rs", 50)],
        &[("crates/qoob3d-app/tests/modeling.rs", 1000)],
        &[("third_party/opencascade-sys/README.md", 1)],
    ];
    for changes in cases {
        assert_eq!(plan_of(changes).tier, Tier::None, "{changes:?}");
    }
}

// ADR-0052 fix round 1: a range that changes only harness files (of any
// kind: hook, workflow, checklist, harness tests, root config) is `high` with
// reason harness and the harness file as example.
#[test]
fn plan_harness_only_range_is_high() {
    for path in [
        ".githooks/pre-push",
        ".github/workflows/ci.yml",
        "harness/critic-checklist.md",
        "xtask/src/review/tests.rs",
        "Cargo.toml",
    ] {
        let plan = plan_of(&[(path, 1)]);
        assert_eq!(plan.tier, Tier::High, "{path}");
        assert_eq!(triggers(&plan), [Trigger::Harness], "{path}");
        assert_eq!(plan.reasons[0].path, path);
    }
}

// ADR-0052 §1 size: more than 400 changed code lines, summed over all
// entries (one entry per file and commit). 400 is not more than 400.
#[test]
fn plan_size_counts_code_lines_over_400() {
    let high: &[&[(&str, usize)]] = &[
        &[(PANELS, 401)],
        &[(PANELS, 200), (PANELS, 201)],
        &[(PANELS, 300), ("crates/qoob3d-render/src/lib.rs", 101)],
    ];
    for changes in high {
        let plan = plan_of(changes);
        assert_eq!(plan.tier, Tier::High, "{changes:?}");
        assert_eq!(triggers(&plan), [Trigger::Size], "{changes:?}");
    }
    // Test and doc lines do not count: 10 code lines + 1000 others.
    let low: &[&[(&str, usize)]] = &[
        &[(PANELS, 400)],
        &[(PANELS, 200), (PANELS, 200)],
        &[(PANELS, 10), ("crates/qoob3d-app/tests/modeling.rs", 1000)],
        &[(PANELS, 10), ("docs/architecture.md", 1000)],
    ];
    for changes in low {
        assert_eq!(plan_of(changes).tier, Tier::Standard, "{changes:?}");
    }
}

// ADR-0052 §1 size: the reason names the trigger and a code file as example.
#[test]
fn plan_size_reason_names_a_code_file() {
    let plan = plan_of(&[("crates/qoob3d-app/tests/modeling.rs", 900), (PANELS, 401)]);
    let reason = plan.reasons.iter().find(|r| r.trigger == Trigger::Size);
    assert!(reason.is_some_and(|r| r.path == PANELS), "{:?}", plan.reasons);
}

// ADR-0052 §1 harness / acceptance 3: a harness path of any kind triggers,
// as long as some code changed; its example is the harness path.
#[test]
fn plan_harness_trigger() {
    for (changes, example) in [
        (vec![("xtask/src/main.rs", 10)], "xtask/src/main.rs"),
        (vec![(PANELS, 1), ("xtask/src/review/tests.rs", 1)], "xtask/src/review/tests.rs"),
        (vec![(".github/workflows/ci.yml", 1), (PANELS, 1)], ".github/workflows/ci.yml"),
        (vec![(PANELS, 1), ("Cargo.toml", 1)], "Cargo.toml"),
    ] {
        let plan = plan_of(&changes);
        assert_eq!(plan.tier, Tier::High, "{changes:?}");
        assert_eq!(triggers(&plan), [Trigger::Harness], "{changes:?}");
        assert_eq!(plan.reasons[0].path, example);
    }
}

// ADR-0052 §1 occt-boundary: `third_party/`, `crates/qoob3d-kernel/cpp/`,
// the file `crates/qoob3d-kernel/src/occt.rs`, any `build.rs`, C++ sources
// and headers.
#[test]
fn plan_occt_boundary_trigger() {
    for path in [
        "third_party/opencascade-sys/src/lib.rs",
        "crates/qoob3d-kernel/cpp/qoob3d_occt.cpp",
        "crates/qoob3d-kernel/cpp/qoob3d_occt.hxx",
        "crates/qoob3d-kernel/src/occt.rs",
        "crates/qoob3d-kernel/build.rs",
        "crates/qoob3d-app/build.rs",
        "crates/qoob3d-render/src/shim.cpp",
        "crates/qoob3d-render/src/shim.hpp",
        "crates/qoob3d-render/src/shim.h",
    ] {
        let plan = plan_of(&[(path, 1)]);
        assert_eq!(plan.tier, Tier::High, "{path}");
        assert_eq!(triggers(&plan), [Trigger::OcctBoundary], "{path}");
        assert_eq!(plan.reasons[0].path, path);
    }
    // A path under `third_party/` triggers whatever its kind.
    let plan = plan_of(&[(PANELS, 1), ("third_party/opencascade/README.md", 1)]);
    assert_eq!(triggers(&plan), [Trigger::OcctBoundary]);
}

// ADR-0052 §1 occt-boundary: names close to the boundary do not trigger
// (`rebuild.rs` is not a `build.rs`, `occt_shapes.rs` is not `occt.rs`).
#[test]
fn plan_occt_boundary_lookalikes_are_standard() {
    for path in [
        "crates/qoob3d-kernel/src/lib.rs",
        "crates/qoob3d-kernel/src/occt_shapes.rs",
        "crates/qoob3d-kernel/src/model.rs",
        "crates/qoob3d-app/src/rebuild.rs",
        "crates/qoob3d-render/src/shader.wgsl",
    ] {
        let plan = plan_of(&[(path, 1)]);
        assert_eq!(plan.tier, Tier::Standard, "{path}: {:?}", plan.reasons);
    }
}

// ADR-0052 §1 format / acceptance 4: the document module and the exporters
// trigger and require lens `robustness`; their test files do not.
#[test]
fn plan_format_trigger() {
    for path in [
        "crates/qoob3d-core/src/document/mod.rs",
        "crates/qoob3d-core/src/document/sketch/arc.rs",
        "crates/qoob3d-core/src/export.rs",
        "crates/qoob3d-core/src/export/threemf.rs",
    ] {
        let plan = plan_of(&[(path, 1)]);
        assert_eq!(plan.tier, Tier::High, "{path}");
        assert_eq!(triggers(&plan), [Trigger::Format], "{path}");
        assert_eq!(plan.reasons[0].path, path);
    }
    for path in [
        "crates/qoob3d-core/src/document/tests.rs",
        "crates/qoob3d-core/src/document/sketch/tests.rs",
        "crates/qoob3d-core/src/export/tests.rs",
        "crates/qoob3d-core/src/exporter.rs",
        "crates/qoob3d-core/src/expr.rs",
    ] {
        let plan = plan_of(&[(PANELS, 1), (path, 1)]);
        assert_eq!(plan.tier, Tier::Standard, "{path}: {:?}", plan.reasons);
    }
}

// ADR-0052 §1: every trigger that holds is its own reason, once, with one
// example path.
#[test]
fn plan_reports_each_trigger_once() {
    let changes = [
        ("xtask/src/main.rs", 300),
        ("xtask/src/paths.rs", 1),
        ("crates/qoob3d-core/src/document/mod.rs", 101),
        ("crates/qoob3d-core/src/export.rs", 1),
        ("crates/qoob3d-kernel/src/occt.rs", 1),
        ("crates/qoob3d-kernel/build.rs", 1),
        (PANELS, 1),
    ];
    let log = escapes(&[escape_row("2026-10-01", &format!("`{PANELS}`"))]);
    let plan = plan(&changes, &log, TIP_DATE).unwrap();
    assert_eq!(plan.tier, Tier::High);
    for trigger in
        [Trigger::Harness, Trigger::OcctBoundary, Trigger::Format, Trigger::Size, Trigger::Escape]
    {
        let n = plan.reasons.iter().filter(|r| r.trigger == trigger).count();
        assert_eq!(n, 1, "{trigger:?}: {:?}", plan.reasons);
    }
    assert_eq!(plan.reasons.len(), 5, "{:?}", plan.reasons);
}

/// The tier of a change to `PANELS` with one escape row `(date, where)` and
/// the tip date `tip`.
fn escape_tier(date: &str, place: &str, tip: &str) -> (Tier, Vec<Trigger>) {
    let p = plan(&[(PANELS, 10)], &escapes(&[escape_row(date, place)]), tip).unwrap();
    (p.tier, triggers(&p))
}

// ADR-0052 §1 escape / acceptance 5: rows within the 180 days up to and
// including the tip date count. Day counts by month lengths:
// 2026-04-20 → 2026-10-16 = 10 (Apr) + 31 + 30 + 31 + 31 + 30 + 16 = 179;
// 2026-04-19 is 180 and 2026-04-18 181 days before. The window is
// 0 ≤ tip − row ≤ 179 days (coordinator decision 2026-10-16), so 180 is out.
// A row dated after the tip is not "up to the tip". 2028 is a leap year:
// 2027-09-04 → 2028-03-01 = 26 + 31 + 30 + 31 + 31 + 29 + 1 = 179,
// 2027-09-03 is 180 and 2027-09-02 181 days before.
#[test]
fn plan_escape_window() {
    let row = format!("`{PANELS}`");
    for (date, tip) in [
        ("2026-04-20", "2026-10-16"),
        ("2026-10-16", "2026-10-16"),
        ("2026-10-15", "2026-10-16"),
        ("2027-09-04", "2028-03-01"),
    ] {
        assert_eq!(
            escape_tier(date, &row, tip),
            (Tier::High, vec![Trigger::Escape]),
            "{date}/{tip}"
        );
    }
    for (date, tip) in [
        ("2026-04-19", "2026-10-16"),
        ("2026-04-18", "2026-10-16"),
        ("2025-10-16", "2026-10-16"),
        ("2026-10-17", "2026-10-16"),
        ("2027-09-03", "2028-03-01"),
        ("2027-09-02", "2028-03-01"),
    ] {
        assert_eq!(escape_tier(date, &row, tip), (Tier::Standard, vec![]), "{date}/{tip}");
    }
}

// ADR-0052 §1 escape: `Where` holds backticked paths; one ending in `/` is a
// prefix, any other must match exactly; every listed path counts.
#[test]
fn plan_escape_where_matching() {
    for place in [
        "`crates/qoob3d-app/src/`",
        "`crates/qoob3d-app/`",
        "`docs/` and `crates/qoob3d-app/src/panels.rs`",
        "`.githooks/`, `crates/qoob3d-app/src/panels.rs`",
    ] {
        assert_eq!(escape_tier("2026-10-01", place, TIP_DATE).0, Tier::High, "{place}");
    }
    for place in [
        "`crates/qoob3d-app/src`",
        "`crates/qoob3d-app/src/pan`",
        "`crates/qoob3d-app/src/panels.rs.bak`",
        "`crates/qoob3d-app/src/panels/`",
        "`docs/`",
    ] {
        assert_eq!(escape_tier("2026-10-01", place, TIP_DATE).0, Tier::Standard, "{place}");
    }
}

// ADR-0052 §1 escape: only changed `Code` files match, and only the `Where`
// column counts (not a path named in another column).
#[test]
fn plan_escape_matches_only_code_in_the_where_column() {
    let tests_only = escapes(&[escape_row("2026-10-01", "`crates/qoob3d-app/src/`")]);
    let changes =
        [("crates/qoob3d-app/src/panels/tests.rs", 10), ("crates/qoob3d-render/src/lib.rs", 10)];
    assert_eq!(plan(&changes, &tests_only, TIP_DATE).unwrap().tier, Tier::Standard);

    let other_column = escapes(&[format!(
        "| 2026-10-01 | user | broke `{PANELS}` | `docs/` | critic | x | lint `{PANELS}` | open |"
    )]);
    assert_eq!(plan(&[(PANELS, 10)], &other_column, TIP_DATE).unwrap().tier, Tier::Standard);
}

// ADR-0052 §1 escape: the row's status does not matter, a `covered` or
// `accepted` row within the window still counts (the spec names no filter).
#[test]
fn plan_escape_ignores_status() {
    for status in ["covered", "accepted (user decision)"] {
        let row = format!("| 2026-10-01 | CI | x | `{PANELS}` | critic | c | k | {status} |");
        let plan = plan(&[(PANELS, 10)], &escapes(&[row]), TIP_DATE).unwrap();
        assert_eq!(triggers(&plan), [Trigger::Escape], "{status}");
    }
}

// ADR-0052 §1 escape: a row with an unparsable date is a plan error (fail
// closed), even if its `Where` does not match any changed file.
#[test]
fn plan_escape_row_with_unparsable_date_fails_closed() {
    for date in ["2026-13-01", "15.10.2026", "soon", ""] {
        let log = escapes(&[escape_row("2026-10-01", "`docs/`"), escape_row(date, "`docs/`")]);
        let result = plan(&[(PANELS, 10)], &log, TIP_DATE);
        let Err(errors) = result else {
            panic!("{date:?}: unparsable date passed: {result:?}");
        };
        assert!(!errors.is_empty(), "{date:?}");
        if !date.is_empty() {
            assert_problem(&errors, date, date);
        }
    }
}

// ADR-0052 fix round 1 (escape table parsing fails closed): an escaped `\|`
// does not split a cell, so the `Where` column stays in place.
#[test]
fn plan_escape_row_with_escaped_pipe() {
    let row = format!("| 2026-10-01 | CI | `a \\| b` broke | `{PANELS}` | critic | c | k | open |");
    let plan = plan(&[(PANELS, 10)], &escapes(&[row]), TIP_DATE);
    assert!(plan.as_ref().is_ok_and(|p| triggers(p) == [Trigger::Escape]), "{plan:?}");
}

// ADR-0052 fix round 1: a row with more or fewer cells than the header is a
// plan error naming the row (a shifted `Where` must not be read silently).
#[test]
fn plan_escape_row_with_wrong_cell_count_fails_closed() {
    for (marker, row) in [
        (
            "extra-cell",
            format!("| 2026-10-01 | CI | a | b extra-cell | `{PANELS}` | x | c | k | open |"),
        ),
        ("short-row", format!("| 2026-10-01 | CI | short-row | `{PANELS}` | x | c | open |")),
        (
            "unescaped",
            "| 2026-10-01 | CI | a | unescaped | `docs/` | x | c | k | open |".to_owned(),
        ),
    ] {
        let result = plan(&[(PANELS, 10)], &escapes(&[row]), TIP_DATE);
        let Err(errors) = result else { panic!("{marker}: passed: {result:?}") };
        assert_problem(&errors, marker, marker);
    }
}

// ADR-0052 §1: `--plan` prints the tier, one line per reason (trigger and
// example path) and the required reviews.
#[test]
fn plan_text_names_tier_reasons_and_required_reviews() {
    let text = harness_plan().to_string();
    for needle in ["high", "harness", "xtask/src/main.rs", "general", "tests", "security"] {
        assert!(text.contains(needle), "`{needle}` missing:\n{text}");
    }
    assert!(!text.contains("robustness"), "{text}");

    let text = format_plan().to_string();
    for needle in ["high", "format", "crates/qoob3d-core/src/document/mod.rs", "robustness"] {
        assert!(text.contains(needle), "`{needle}` missing:\n{text}");
    }
    assert!(!text.contains("security"), "{text}");

    let text = standard_plan().to_string();
    assert!(text.contains("standard") && text.contains("general"), "{text}");
    assert!(!text.contains("security") && !text.contains("robustness"), "{text}");

    assert!(plan_of(&[]).to_string().contains("none"));
}

// ===========================================================================
// Spec §3.7: the Chapman estimate of undiscovered defects (ADR-0052
// decision 4). R̂ = (n₁+1)(n₂+1)/(m+1) − 1 − (n₁+n₂−m). All values are
// small dyadic rationals, exact in f64; 1e-9 only absorbs the formula order.
// ===========================================================================

#[test]
fn chapman_examples_from_the_spec() {
    for ((n1, n2, m), expected) in [
        ((0, 0, 0), 0.0),
        ((1, 0, 0), 0.0),
        ((2, 2, 1), 0.5),
        ((2, 1, 0), 2.0),
        ((3, 1, 0), 3.0),
        // 2·2/1 − 1 − 2 = 1: exactly the threshold.
        ((1, 1, 0), 1.0),
        // 2·2/2 − 1 − 1 = 0: both found the same defect.
        ((1, 1, 1), 0.0),
        // 1001² − 1 − 2000 = 1 000 000.
        ((1000, 1000, 0), 1_000_000.0),
    ] {
        let r = undiscovered(n1, n2, m);
        assert!((r - expected).abs() < 1e-9, "({n1},{n2},{m}): {r} != {expected}");
    }
}

// Chapman is symmetric in the two samples, and an empty sample estimates
// nothing undiscovered: (n+1)·1/1 − 1 − n = 0.
#[test]
fn chapman_is_symmetric_and_zero_for_an_empty_sample() {
    for (n1, n2, m) in [(2, 1, 0), (3, 1, 0), (5, 2, 1), (7, 4, 3)] {
        assert!((undiscovered(n1, n2, m) - undiscovered(n2, n1, m)).abs() < 1e-9);
    }
    for n in [0, 1, 5, 40] {
        assert!(undiscovered(n, 0, 0).abs() < 1e-9, "{n}");
        assert!(undiscovered(0, n, 0).abs() < 1e-9, "{n}");
    }
}

// ===========================================================================
// Spec §3: the version-2 gate on records (pure part).
// ===========================================================================

// ADR-0052 §3.1: the ADR-0041 record format (version 1) and any version but
// the integer 2 are rejected with a request to re-run `/critic-review`.
#[test]
fn version_1_record_is_rejected() {
    let v1 = json!({"tree": TREE, "findings": []});
    let found = problems(&v1, &standard_plan());
    assert_problem(&found, "/critic-review", "v1");
    assert_problem(&found, "ADR-0052", "v1");

    for version in [json!(1), json!("2"), json!(3), json!(null), json!(2.5)] {
        let record = with(standard_record(vec![]), "version", version.clone());
        let found = problems(&record, &standard_plan());
        assert_problem(&found, "/critic-review", &version.to_string());
    }
    let record = without(standard_record(vec![]), "version");
    assert_problem(&problems(&record, &standard_plan()), "/critic-review", "no version");
}

// ADR-0052 §3: complete records for each tier pass (controls for every
// rejection below).
#[test]
fn complete_records_pass() {
    assert_eq!(problems(&standard_record(vec![]), &standard_plan()), Vec::<String>::new());
    assert_eq!(problems(&high_record(vec![]), &size_plan()), Vec::<String>::new());
    let mut reviews = high_reviews();
    reviews.push(review("r4", MODEL_B, "security"));
    assert_eq!(problems(&record("high", reviews, vec![]), &harness_plan()), Vec::<String>::new());
    for plan in [format_plan(), occt_plan()] {
        let mut reviews = high_reviews();
        reviews.push(review("r4", MODEL_C, "robustness"));
        assert_eq!(problems(&record("high", reviews, vec![]), &plan), Vec::<String>::new());
    }
}

// ADR-0052 §3.2, fix round 1 F2.1: the record's tier must be at least the
// plan's (none < standard < high); lower, missing or unknown tiers are
// rejected (tier names are fixed strings, case-sensitive).
#[test]
fn tier_must_be_at_least_the_plan() {
    let cases = [
        (standard_record(vec![]), size_plan()),
        (with(high_record(vec![]), "tier", json!("standard")), size_plan()),
        (with(standard_record(vec![]), "tier", json!("none")), standard_plan()),
        (with(standard_record(vec![]), "tier", json!("Standard")), standard_plan()),
        (with(standard_record(vec![]), "tier", json!("medium")), standard_plan()),
        (with(high_record(vec![]), "tier", json!("High")), size_plan()),
        (without(standard_record(vec![]), "tier"), standard_plan()),
    ];
    for (record, plan) in cases {
        assert_problem(&problems(&record, &plan), "tier", &record["tier"].to_string());
    }
}

// ADR-0052 fix round 1 F2.1: a `high` record for a `standard` plan passes;
// coverage and the stopping rule follow the plan, so one general review
// suffices and (2,1,0) needs no third model. It must still cover the
// standard requirement (a general review).
#[test]
fn high_record_for_a_standard_plan_follows_the_plan() {
    assert_eq!(problems(&high_record(vec![]), &standard_plan()), Vec::<String>::new());
    let one_general = record("high", vec![review("r1", MODEL_A, "general")], vec![]);
    assert_eq!(problems(&one_general, &standard_plan()), Vec::<String>::new());
    let findings = vec![counted(&["r1"]), counted(&["r1"]), counted(&["r2"])];
    assert_eq!(problems(&high_record(findings), &standard_plan()), Vec::<String>::new());

    let lens_only = record("high", vec![review("r1", MODEL_A, "tests")], vec![]);
    assert_problem(&problems(&lens_only, &standard_plan()), "general", "high without general");
}

// ADR-0052 §3.3: `reviews` must be a non-empty list; the problem names the
// field (test-quality finding of fix round 1: the specific problem).
#[test]
fn reviews_must_be_a_non_empty_list() {
    for record in [
        without(standard_record(vec![]), "reviews"),
        with(standard_record(vec![]), "reviews", json!([])),
        with(standard_record(vec![]), "reviews", json!({"id": "r1"})),
        with(standard_record(vec![]), "reviews", json!(null)),
    ] {
        assert_problem(&problems(&record, &standard_plan()), "`reviews`", &record.to_string());
    }
}

/// A standard record with the complete general review `r1` and `extra`, so
/// the only problem can be `extra`.
fn standard_with(extra: Value) -> Value {
    record("standard", vec![review("r1", MODEL_A, "general"), extra], vec![])
}

// ADR-0052 §3.3: every review has a non-empty id, a non-empty model, a known
// lens (case-sensitive, like the ADR-0041 fields) and an integer round ≥ 1.
#[test]
fn every_review_is_complete() {
    let cases = [
        (json!({"model": MODEL_B, "lens": "tests", "round": 1}), "id"),
        (json!({"id": "", "model": MODEL_B, "lens": "tests", "round": 1}), "id"),
        (json!({"id": "r2", "lens": "tests", "round": 1}), "model"),
        (json!({"id": "r2", "model": "", "lens": "tests", "round": 1}), "model"),
        (json!({"id": "r2", "model": MODEL_B, "round": 1}), "lens"),
        (json!({"id": "r2", "model": MODEL_B, "lens": "style", "round": 1}), "style"),
        (json!({"id": "r2", "model": MODEL_B, "lens": "General", "round": 1}), "General"),
        (json!({"id": "r2", "model": MODEL_B, "lens": "tests"}), "round"),
        (json!({"id": "r2", "model": MODEL_B, "lens": "tests", "round": 0}), "round"),
        (json!({"id": "r2", "model": MODEL_B, "lens": "tests", "round": -1}), "round"),
        (json!({"id": "r2", "model": MODEL_B, "lens": "tests", "round": 1.5}), "round"),
        (json!({"id": "r2", "model": MODEL_B, "lens": "tests", "round": "1"}), "round"),
    ];
    for (extra, needle) in cases {
        let problems = problems(&standard_with(extra.clone()), &standard_plan());
        assert_problem(&problems, needle, &extra.to_string());
    }
    // Control: the same extra review, complete, passes; so do all lenses.
    // (Round 1: a first-round record has only round-1 reviews, fix round 1.)
    for lens in ["general", "tests", "security", "robustness"] {
        let extra = json!({"id": "r2", "model": MODEL_B, "lens": lens, "round": 1});
        assert!(problems(&standard_with(extra), &standard_plan()).is_empty(), "{lens}");
    }
}

// ADR-0052 fix round 1 F3: `scope` is absent, `diff` (whole diff) or `files`
// (one file group); any other value is a problem naming `scope`.
#[test]
fn review_scope_is_diff_or_files() {
    for scope in ["diff", "files"] {
        let extra = json!({"id": "r2", "model": MODEL_B, "lens": "general", "round": 1,
                           "scope": scope});
        let found = problems(&standard_with(extra), &standard_plan());
        assert!(found.is_empty(), "{scope}: {found:?}");
    }
    for scope in [json!("all"), json!("Files"), json!(""), json!(1), json!(["files"])] {
        let extra = json!({"id": "r2", "model": MODEL_B, "lens": "general", "round": 1,
                           "scope": scope.clone()});
        assert_problem(
            &problems(&standard_with(extra), &standard_plan()),
            "scope",
            &scope.to_string(),
        );
    }
}

/// A review of one file group (F3).
fn files_review(id: &str, model: &str, lens: &str) -> Value {
    json!({"id": id, "model": model, "lens": lens, "round": 1, "scope": "files"})
}

// ADR-0052 fix round 1 F3: only whole-diff general reviews cover the
// general requirement of tier standard and the two models of tier high.
// File-group reviews may still be cited in `found_by`.
#[test]
fn file_group_reviews_do_not_count_as_whole_diff() {
    let standard = record("standard", vec![files_review("r1", MODEL_A, "general")], vec![]);
    assert_problem(&problems(&standard, &standard_plan()), "general", "files only");

    let reviews = vec![
        review("r1", MODEL_A, "general"),
        files_review("r2", MODEL_B, "general"),
        review("r3", MODEL_A, "tests"),
    ];
    let high = record("high", reviews, vec![]);
    assert_problem(&problems(&high, &size_plan()), "general", "second model only on files");

    let reviews = vec![review("r1", MODEL_A, "general"), files_review("r2", MODEL_B, "general")];
    let finding = json!({"rule": "K-03", "severity": "minor", "found_by": ["r2"]});
    let cited = record("standard", reviews, vec![finding]);
    let found = problems(&cited, &standard_plan());
    assert!(found.is_empty(), "{found:?}");
}

// ADR-0052 fix round 1 ("rounds are consistent"): without `previous_tree`
// every review has round 1.
#[test]
fn first_round_reviews_have_round_1() {
    for round in [2, 3] {
        let extra = json!({"id": "r2", "model": MODEL_B, "lens": "tests", "round": round});
        assert_problem(&problems(&standard_with(extra), &standard_plan()), "round", "first round");
    }
}

// ADR-0052 §3.3: review ids are unique within the record.
#[test]
fn review_ids_are_unique() {
    let record = record(
        "standard",
        vec![review("dup", MODEL_A, "general"), review("dup", MODEL_B, "tests")],
        vec![],
    );
    assert_problem(&problems(&record, &standard_plan()), "dup", "duplicate id");
}

// ADR-0052 §3.5: every finding (any severity) has a non-empty `found_by`
// whose ids all name reviews of the record.
#[test]
fn found_by_names_reviews_of_the_record() {
    let base = json!({"rule": "K-03", "severity": "minor"});
    for (found_by, needle) in [
        (None, "found_by"),
        (Some(json!([])), "found_by"),
        (Some(json!("r1")), "found_by"),
        (Some(json!([1])), "found_by"),
        (Some(json!(["r9"])), "r9"),
        (Some(json!(["r1", "r7"])), "r7"),
    ] {
        let mut f = base.clone();
        if let Some(ids) = found_by {
            f["found_by"] = ids;
        }
        let problems = problems(&standard_record(vec![f.clone()]), &standard_plan());
        assert_problem(&problems, needle, &f.to_string());
    }
    let ok = with(base, "found_by", json!(["r1"]));
    assert!(problems(&standard_record(vec![ok]), &standard_plan()).is_empty());
}

// ADR-0052 §1/§3.4 / acceptance 1: tier standard needs a general review; a
// lens review alone does not cover it.
#[test]
fn standard_needs_a_general_review() {
    let record = record("standard", vec![review("r1", MODEL_A, "tests")], vec![]);
    assert_problem(&problems(&record, &standard_plan()), "general", "only lens tests");
}

// ADR-0052 §1/§3.4: tier high needs two general reviews on two different
// models; the same model twice or one general review is rejected.
#[test]
fn high_needs_general_reviews_on_two_models() {
    for reviews in [
        vec![review("r1", MODEL_A, "general"), review("r2", MODEL_A, "general")],
        vec![review("r1", MODEL_A, "general")],
    ] {
        let mut reviews = reviews;
        reviews.push(review("r3", MODEL_B, "tests"));
        let record = record("high", reviews, vec![]);
        assert_problem(&problems(&record, &size_plan()), "general", &record.to_string());
    }
}

// ADR-0052 §1/§3.4 / acceptance 3, 4: tier high always needs lens `tests`;
// the harness trigger adds `security`, occt-boundary and format add
// `robustness`.
#[test]
fn high_needs_the_lenses_of_its_triggers() {
    let general = || vec![review("r1", MODEL_A, "general"), review("r2", MODEL_B, "general")];
    let without_tests = record("high", general(), vec![]);
    assert_problem(&problems(&without_tests, &size_plan()), "tests", "size");

    assert_problem(&problems(&high_record(vec![]), &harness_plan()), "security", "harness");
    for (plan, what) in [(format_plan(), "format"), (occt_plan(), "occt-boundary")] {
        assert_problem(&problems(&high_record(vec![]), &plan), "robustness", what);
    }
    // A lens of another trigger does not stand in: robustness for harness.
    let mut reviews = high_reviews();
    reviews.push(review("r4", MODEL_B, "robustness"));
    let record = record("high", reviews, vec![]);
    assert_problem(&problems(&record, &harness_plan()), "security", "robustness for harness");
}

/// Writes `previous` as the record of `PREV` into a fresh directory named
/// after `test`.
fn records_with_previous(test: &str, previous: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("qoob3d-review-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{PREV}.json")), previous).unwrap();
    dir
}

/// A record of the previous round (round 1) with whole-diff general reviews
/// by `models`, in order, and a tests lens on `MODEL_C`.
fn previous_record(tier: &str, models: &[&str]) -> String {
    let mut reviews: Vec<Value> =
        models.iter().enumerate().map(|(i, m)| review(&format!("p{i}"), m, "general")).collect();
    reviews.push(review("pt", MODEL_C, "tests"));
    previous_with(record(tier, reviews, vec![]))
}

/// `record` as the record of `PREV`.
fn previous_with(mut previous: Value) -> String {
    previous["tree"] = json!(PREV);
    previous.to_string()
}

/// `record` as the round after `PREV`: round 2, one more than the previous
/// records of these tests (all round 1).
fn after_previous(mut record: Value) -> Value {
    for review in record["reviews"].as_array_mut().into_iter().flatten() {
        review["round"] = json!(2);
    }
    with(record, "previous_tree", json!(PREV))
}

/// The problems of `record` with the records in `dir`.
fn problems_in(dir: &Path, record: &Value, plan: &Plan) -> Vec<String> {
    record_problems(record, TREE, plan, dir)
}

/// The rotation problem: it names the previous tree and a model of the
/// previous whole-diff general set (`model`), which no missing-record or
/// format problem can.
fn assert_rotation_problem(found: &[String], model: &str, what: &str) {
    assert!(
        found.iter().any(|p| p.contains(PREV) && p.contains(model)),
        "{what}: no rotation problem naming {PREV} and {model}: {found:?}"
    );
}

// ADR-0052 §3.6 / acceptance "Rotation": after a fix, a whole-diff general
// review must use a model outside the previous whole-diff general set. Lens
// reviews neither rotate nor block the rotation.
#[test]
fn rotation_needs_a_new_general_model() {
    let dir = records_with_previous("rotation", &previous_record("standard", &[MODEL_A]));
    let rotated = |reviews: Vec<Value>| after_previous(record("standard", reviews, vec![]));
    let same = rotated(vec![review("r1", MODEL_A, "general")]);
    assert_rotation_problem(&problems_in(&dir, &same, &standard_plan()), MODEL_A, "same model");
    let lens_only = rotated(vec![review("r1", MODEL_A, "general"), review("r2", MODEL_B, "tests")]);
    let found = problems_in(&dir, &lens_only, &standard_plan());
    assert_rotation_problem(&found, MODEL_A, "lens rotated");
    // The previous tests lens used MODEL_C; that does not block MODEL_C now.
    for model in [MODEL_B, MODEL_C] {
        let fresh = rotated(vec![review("r1", model, "general")]);
        let found = problems_in(&dir, &fresh, &standard_plan());
        assert!(found.is_empty(), "{model}: {found:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

// ADR-0052 §3.6: in tier high one new general model suffices; repeating both
// previous models does not.
#[test]
fn rotation_in_tier_high() {
    let dir = records_with_previous("rotation-high", &previous_record("high", &[MODEL_A, MODEL_B]));
    let same = after_previous(high_record(vec![]));
    assert_rotation_problem(&problems_in(&dir, &same, &size_plan()), MODEL_A, "same models");
    let reviews = vec![
        review("r1", MODEL_A, "general"),
        review("r2", MODEL_C, "general"),
        review("r3", MODEL_A, "tests"),
    ];
    let fresh = after_previous(record("high", reviews, vec![]));
    let found = problems_in(&dir, &fresh, &size_plan());
    assert!(found.is_empty(), "{found:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ADR-0052 fix round 1 F1: the previous set is only the a/b pair of the
// stopping rule. Previous a/b = A/C with a third (stopping-rule) review on B:
// a new whole-diff general review on B passes; only A and C fails.
#[test]
fn rotation_ignores_the_previous_third_review() {
    let previous = previous_record("high", &[MODEL_A, MODEL_C, MODEL_B]);
    let dir = records_with_previous("rotation-third", &previous);
    let reviews = vec![
        review("r1", MODEL_B, "general"),
        review("r2", MODEL_A, "general"),
        review("r3", MODEL_A, "tests"),
    ];
    let fresh = after_previous(record("high", reviews, vec![]));
    let found = problems_in(&dir, &fresh, &size_plan());
    assert!(found.is_empty(), "{found:?}");

    let reviews = vec![
        review("r1", MODEL_A, "general"),
        review("r2", MODEL_C, "general"),
        review("r3", MODEL_A, "tests"),
    ];
    let same = after_previous(record("high", reviews, vec![]));
    assert_rotation_problem(&problems_in(&dir, &same, &size_plan()), MODEL_A, "only A and C");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ADR-0052 fix round 1 F1: the previous pair is the first whole-diff general
// review and the first one on a different model, in record order; a repeat
// of the first model and file-group reviews are not in the set. Previous:
// A, A again, B (pair A/B), C only on a file group.
#[test]
fn rotation_set_is_the_first_whole_diff_pair() {
    let previous = record(
        "high",
        vec![
            review("p1", MODEL_A, "general"),
            review("p2", MODEL_A, "general"),
            files_review("p3", MODEL_C, "general"),
            review("p4", MODEL_B, "general"),
            review("pt", MODEL_A, "tests"),
        ],
        vec![],
    );
    let dir = records_with_previous("rotation-pair", &previous_with(previous));
    let rotated = |model: &str| {
        let reviews = vec![
            review("r1", MODEL_A, "general"),
            review("r2", model, "general"),
            review("r3", MODEL_A, "tests"),
        ];
        after_previous(record("high", reviews, vec![]))
    };
    let found = problems_in(&dir, &rotated(MODEL_C), &size_plan());
    assert!(found.is_empty(), "C was only a file-group review: {found:?}");
    assert_rotation_problem(&problems_in(&dir, &rotated(MODEL_B), &size_plan()), MODEL_A, "A/B");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ADR-0052 fix round 1 F1/F3: the new model must be on a whole-diff general
// review; a file-group general review on a new model does not rotate.
#[test]
fn rotation_needs_the_new_model_on_the_whole_diff() {
    let dir = records_with_previous("rotation-files", &previous_record("standard", &[MODEL_A]));
    let reviews = vec![review("r1", MODEL_A, "general"), files_review("r2", MODEL_B, "general")];
    let record = after_previous(record("standard", reviews, vec![]));
    assert_rotation_problem(&problems_in(&dir, &record, &standard_plan()), MODEL_A, "files");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ADR-0052 §3.6, fix round 1: a missing or unreadable previous record fails
// closed with exactly one problem naming the previous tree; `null` or no
// `previous_tree` is a first round.
#[test]
fn rotation_fails_closed_without_the_previous_record() {
    let record = after_previous(standard_record(vec![]));
    let found = problems(&record, &standard_plan());
    assert!(found.len() == 1 && found[0].contains(PREV), "missing previous record: {found:?}");

    let dir = records_with_previous("rotation-broken", "{not json");
    let found = problems_in(&dir, &record, &standard_plan());
    assert!(found.len() == 1 && found[0].contains(PREV), "unreadable previous record: {found:?}");
    std::fs::remove_dir_all(&dir).unwrap();

    let first = without(standard_record(vec![]), "previous_tree");
    assert!(problems(&first, &standard_plan()).is_empty());
}

// ADR-0052 fix round 1 (previous record validation): the previous record
// must have version 2, `tree` equal to `previous_tree` and a whole-diff
// general review; otherwise exactly one problem naming the previous tree.
#[test]
fn previous_record_is_validated() {
    let general = || vec![review("p1", MODEL_A, "general")];
    let cases = [
        ("v1", previous_with(json!({"findings": [], "reviews": general()}))),
        (
            "version 3",
            previous_with(with(record("standard", general(), vec![]), "version", json!(3))),
        ),
        (
            "other tree",
            with(record("standard", general(), vec![]), "tree", json!(TREE)).to_string(),
        ),
        ("no tree", without(record("standard", general(), vec![]), "tree").to_string()),
        ("no reviews", previous_with(without(record("standard", vec![], vec![]), "reviews"))),
        (
            "lens only",
            previous_with(record("standard", vec![review("p1", MODEL_A, "tests")], vec![])),
        ),
        (
            "files only",
            previous_with(record("standard", vec![files_review("p1", MODEL_A, "general")], vec![])),
        ),
    ];
    let fresh = after_previous(record("standard", vec![review("r1", MODEL_B, "general")], vec![]));
    for (what, previous) in cases {
        let dir = records_with_previous(&format!("previous-{}", what.replace(' ', "-")), &previous);
        let found = problems_in(&dir, &fresh, &standard_plan());
        assert!(found.len() == 1 && found[0].contains(PREV), "{what}: {found:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

// ADR-0052 fix round 1: `previous_tree` must be a 40- or 64-character
// lowercase hex object id (no path); otherwise exactly one problem naming
// `previous_tree`. A path to a valid record outside the record directory
// must not be read.
#[test]
fn previous_tree_must_be_an_object_id() {
    let dir = records_with_previous("previous-id", &previous_record("standard", &[MODEL_A]));
    // A complete previous record one level up whose `tree` is the path: only
    // the id check keeps `../<TREE>` from passing.
    let outside = dir.parent().unwrap().join(format!("{TREE}.json"));
    let mut escaped = record("standard", vec![review("p1", MODEL_A, "general")], vec![]);
    escaped["tree"] = json!(format!("../{TREE}"));
    std::fs::write(&outside, escaped.to_string()).unwrap();
    let fresh =
        || after_previous(record("standard", vec![review("r1", MODEL_B, "general")], vec![]));
    for id in [
        json!(5),
        json!(true),
        json!(""),
        json!(PREV.to_uppercase().replace('1', "A")),
        json!(&PREV[..39]),
        json!(format!("{PREV}0")),
        json!(format!("../{TREE}")),
        json!("g".repeat(40)),
    ] {
        let found =
            problems_in(&dir, &with(fresh(), "previous_tree", id.clone()), &standard_plan());
        assert!(found.len() == 1 && found[0].contains("previous_tree"), "{id}: {found:?}");
    }
    std::fs::remove_file(&outside).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

// ADR-0052 fix round 1: a 64-character id (SHA-256 repositories) is valid.
#[test]
fn previous_tree_may_be_a_sha256_id() {
    let id = "ab".repeat(32);
    let dir = records_with_previous("previous-sha256", "");
    let mut previous = record("standard", vec![review("p1", MODEL_A, "general")], vec![]);
    previous["tree"] = json!(id);
    std::fs::write(dir.join(format!("{id}.json")), previous.to_string()).unwrap();
    let fresh = after_previous(record("standard", vec![review("r1", MODEL_B, "general")], vec![]));
    let found = problems_in(&dir, &with(fresh, "previous_tree", json!(id)), &standard_plan());
    assert!(found.is_empty(), "{found:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ADR-0052 fix round 1 ("rounds are consistent"): with `previous_tree`,
// every review has round = highest previous round + 1.
#[test]
fn rounds_follow_the_previous_record() {
    let mut previous = record("standard", vec![review("p1", MODEL_A, "general")], vec![]);
    previous["reviews"][0]["round"] = json!(2);
    let dir = records_with_previous("rounds", &previous_with(previous));
    let at = |rounds: &[u64]| {
        let models = [MODEL_B, MODEL_C];
        let reviews: Vec<Value> = rounds
            .iter()
            .zip(models)
            .enumerate()
            .map(|(i, (round, model))| {
                json!({"id": format!("r{i}"), "model": model, "lens": "general", "round": round})
            })
            .collect();
        with(record("standard", reviews, vec![]), "previous_tree", json!(PREV))
    };
    let found = problems_in(&dir, &at(&[3, 3]), &standard_plan());
    assert!(found.is_empty(), "{found:?}");
    for rounds in [&[2, 2][..], &[1][..], &[4][..], &[3, 2][..]] {
        let found = problems_in(&dir, &at(rounds), &standard_plan());
        assert_problem(&found, "round", &format!("{rounds:?}"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A size-plan record with the reviews of [`high_reviews`] plus `extra`.
fn high_with(extra: Vec<Value>, findings: Vec<Value>) -> Value {
    let mut reviews = high_reviews();
    reviews.extend(extra);
    record("high", reviews, findings)
}

// ADR-0052 §3.7 / acceptance "Stopping rule": (n₁,n₂,m) = (2,1,0) gives
// R̂ = 2 ≥ 1, so a third general review on a model other than those of a and
// b is required; the problem states R̂ with one decimal.
#[test]
fn stopping_rule_needs_a_third_model() {
    let findings = || vec![counted(&["r1"]), counted(&["r1"]), counted(&["r2"])];
    let found = problems(&high_record(findings()), &size_plan());
    assert_problem(&found, "2.0", "(2,1,0) without a third review");

    for model in [MODEL_A, MODEL_B] {
        let record = high_with(vec![review("r4", model, "general")], findings());
        assert_problem(&problems(&record, &size_plan()), "2.0", model);
    }
    let record = high_with(vec![review("r4", MODEL_C, "general")], findings());
    let found = problems(&record, &size_plan());
    assert!(found.is_empty(), "{found:?}");
}

// ADR-0052 §3.7: (3,1,0) → R̂ = 3; (1,1,0) → R̂ = 1 is on the threshold
// (≥ 1) and needs the third review too; (2,2,1) → R̂ = 0.5 does not.
#[test]
fn stopping_rule_threshold() {
    let three = vec![counted(&["r1"]), counted(&["r1"]), counted(&["r1"]), counted(&["r2"])];
    assert_problem(&problems(&high_record(three), &size_plan()), "3.0", "(3,1,0)");
    let one = vec![counted(&["r1"]), counted(&["r2"])];
    assert_problem(&problems(&high_record(one), &size_plan()), "1.0", "(1,1,0)");
    let half = vec![counted(&["r1"]), counted(&["r2"]), counted(&["r1", "r2"])];
    let found = problems(&high_record(half), &size_plan());
    assert!(found.is_empty(), "(2,2,1): {found:?}");
}

/// (2,2,1) of r1/r2, R̂ = 0.5: passes alone.
fn two_two_one() -> Vec<Value> {
    vec![counted(&["r1"]), counted(&["r2"]), counted(&["r1", "r2"])]
}

// ADR-0052 §3.7: only blocker/major findings that are not refuted count, and
// only those of a and b. Each addition below would make (3,2,1) → R̂ = 1 if
// it were counted for r1.
#[test]
fn stopping_rule_counts_only_unrefuted_severe_findings_of_a_and_b() {
    let refuted = json!({"rule": "K-09", "severity": "major", "verdict": "refuted",
                         "resolution": "open", "found_by": ["r1"]});
    let minor = json!({"rule": "K-09", "severity": "minor", "verdict": "confirmed",
                       "resolution": "open", "found_by": ["r1"]});
    for extra in [refuted, minor, counted(&["r3"])] {
        let mut findings = two_two_one();
        findings.push(extra.clone());
        let found = problems(&high_record(findings), &size_plan());
        assert!(found.is_empty(), "{extra}: {found:?}");
    }
    // Blocker and unverified findings do count: (3,2,1) → R̂ = 1.
    for verdict in ["confirmed", "unverified"] {
        let mut findings = two_two_one();
        findings.push(json!({"rule": "K-09", "severity": "blocker", "verdict": verdict,
                             "resolution": "accepted", "accepted_reason": "tracked",
                             "found_by": ["r1"]}));
        assert_problem(&problems(&high_record(findings), &size_plan()), "1.0", verdict);
    }
}

// ADR-0052 §3.7: a and b are the first two general reviews with different
// models in record order. Here a = r1 (A), b = r3 (B); r2 (A) is skipped.
// (2,1,0) of r1/r3 → R̂ = 2, and r2 is no third model. Pairing r2 with r3
// would give (0,1,0) → R̂ = 0 and pass.
#[test]
fn stopping_rule_takes_the_first_two_models_in_record_order() {
    let reviews = vec![
        review("r1", MODEL_A, "general"),
        review("r2", MODEL_A, "general"),
        review("r3", MODEL_B, "general"),
        review("r4", MODEL_A, "tests"),
    ];
    let findings = vec![counted(&["r1"]), counted(&["r1"]), counted(&["r3"])];
    let found = problems(&record("high", reviews, findings), &size_plan());
    assert_problem(&found, "2.0", "a = r1, b = r3");
}

// ADR-0052 fix round 1 F3: a and b and the third model are whole-diff
// general reviews only. r2 (B) reviews a file group, so a = r1 (A),
// b = r3 (C): (2,1,0) → R̂ = 2. Taking r2 as b would give (2,0,0) → R̂ = 0.
// The third model must be a whole-diff review: B on a file group is not.
#[test]
fn stopping_rule_uses_whole_diff_reviews_only() {
    let reviews = |third: Value| {
        vec![
            review("r1", MODEL_A, "general"),
            files_review("r2", MODEL_B, "general"),
            review("r3", MODEL_C, "general"),
            review("r4", MODEL_A, "tests"),
            third,
        ]
    };
    let findings = || vec![counted(&["r1"]), counted(&["r1"]), counted(&["r3"])];
    let record_with = |third| record("high", reviews(third), findings());
    let files_third = record_with(files_review("r5", MODEL_B, "general"));
    assert_problem(&problems(&files_third, &size_plan()), "2.0", "third model on a file group");
    let diff_third = record_with(review("r5", MODEL_B, "general"));
    let found = problems(&diff_third, &size_plan());
    assert!(found.is_empty(), "{found:?}");
}

// ADR-0052 §3.7: only one extra review is required; its findings do not
// re-trigger the rule.
#[test]
fn stopping_rule_third_review_findings_do_not_retrigger() {
    let mut findings = vec![counted(&["r1"]), counted(&["r1"]), counted(&["r2"])];
    findings.extend((0..5).map(|_| counted(&["r4"])));
    let record = high_with(vec![review("r4", MODEL_C, "general")], findings);
    let found = problems(&record, &size_plan());
    assert!(found.is_empty(), "{found:?}");
}

// ADR-0052 §3.7: the stopping rule applies to tier high only.
#[test]
fn stopping_rule_not_in_tier_standard() {
    let reviews = vec![review("r1", MODEL_A, "general"), review("r2", MODEL_B, "general")];
    let findings = vec![counted(&["r1"]), counted(&["r1"]), counted(&["r2"])];
    let found = problems(&record("standard", reviews, findings), &standard_plan());
    assert!(found.is_empty(), "{found:?}");
}

// ---------------------------------------------------------------------------
// Real ranges in temporary repositories (spec §1: plan from the commits of
// the range, the tip's committer date and `docs/escapes.md` as of the tip).
// ---------------------------------------------------------------------------

/// A fixed commit date, far from today, so no test depends on the clock.
const DATE: &str = "2025-03-01T12:00:00+00:00";

/// Runs git as Alice in `root` with fixed dates, without global/system config
/// or signing (a fresh repository has no active hooks). Returns stdout.
fn git_at(root: &Path, author_date: &str, committer_date: &str, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Alice")
        .env("GIT_AUTHOR_EMAIL", "alice@example.com")
        .env("GIT_COMMITTER_NAME", "Alice")
        .env("GIT_COMMITTER_EMAIL", "alice@example.com")
        .env("GIT_AUTHOR_DATE", author_date)
        .env("GIT_COMMITTER_DATE", committer_date)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn git_in(root: &Path, args: &[&str]) -> String {
    git_at(root, DATE, DATE, args)
}

fn write_file(root: &Path, file: &str, content: &str) {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
}

fn commit_all(root: &Path, message: &str) -> String {
    git_in(root, &["add", "-A"]);
    git_in(root, &["commit", "-q", "-m", message]);
    git_in(root, &["rev-parse", "HEAD"])
}

/// `n` distinct lines.
fn lines(n: usize) -> String {
    (0..n).map(|i| format!("// line {i}\n")).collect()
}

/// A fresh repository with `core.quotePath=true` and a base commit with an
/// empty escape log (plus `files`); returns the root and the base sha.
fn repo_with_base(test: &str, files: &[(&str, &str)]) -> (std::path::PathBuf, String) {
    let root = std::env::temp_dir().join(format!("qoob3d-review-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    git_in(&root, &["init", "-q", "-b", "main"]);
    git_in(&root, &["config", "core.quotePath", "true"]);
    write_file(&root, "docs/escapes.md", &escapes(&[]));
    for (file, content) in files {
        write_file(&root, file, content);
    }
    let base = commit_all(&root, "Base");
    (root, base)
}

/// A fresh repository with an escape log and one commit of `file`.
fn repo_with_commit(test: &str, file: &str) -> std::path::PathBuf {
    let (root, _) = repo_with_base(test, &[(file, "fn f() {}\n")]);
    root
}

fn range_plan(root: &Path, base: &str) -> Plan {
    plan_range(root, "HEAD", &[format!("{base}..HEAD")]).unwrap()
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

// ADR-0052 §1 format, with git quoting (N3): quoted paths under the document
// module still trigger `format`; the plain name is the control.
#[test]
fn plan_range_sees_quoted_format_paths() {
    for (i, file) in [
        "crates/qoob3d-core/src/document/plain.rs",
        "crates/qoob3d-core/src/document/\u{e4}.rs",
        "crates/qoob3d-core/src/document/a\tb.rs",
    ]
    .into_iter()
    .enumerate()
    {
        let (root, base) = repo_with_base(&format!("plan-quoted-{i}"), &[]);
        write_file(&root, file, "fn f() {}\n");
        commit_all(&root, "Add");
        let plan = range_plan(&root, &base);
        assert_eq!(triggers(&plan), [Trigger::Format], "{file:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}

// ADR-0052 §1 size: lines are summed over the commits of the range, not taken
// from the net diff. 300 lines added, then deleted: net 0, summed 600 > 400.
// Control: 10 lines in one commit stay standard.
#[test]
fn plan_range_sums_changed_lines_over_the_commits() {
    let (root, base) = repo_with_base("plan-size", &[(PANELS, "fn f() {}\n")]);
    write_file(&root, PANELS, &format!("fn f() {{}}\n{}", lines(300)));
    commit_all(&root, "Grow");
    write_file(&root, PANELS, "fn f() {}\n");
    commit_all(&root, "Shrink");
    let plan = range_plan(&root, &base);
    assert_eq!((plan.tier, triggers(&plan)), (Tier::High, vec![Trigger::Size]));

    let shrunk = git_in(&root, &["rev-parse", "HEAD"]);
    write_file(&root, PANELS, &format!("fn f() {{}}\n{}", lines(10)));
    commit_all(&root, "Small");
    let plan = range_plan(&root, &shrunk);
    assert_eq!(plan.tier, Tier::Standard, "{:?}", plan.reasons);
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 §1 occt-boundary/format: a rename or deletion changes the old path
// too. Moving a file out of the document module, or deleting one, is a
// format change.
#[test]
fn plan_range_sees_renamed_and_deleted_paths() {
    let moved = "crates/qoob3d-core/src/document/feature.rs";
    let deleted = "crates/qoob3d-core/src/document/old.rs";
    let content = format!("fn f() {{}}\n{}", lines(20));
    let (root, base) =
        repo_with_base("plan-rename", &[(moved, content.as_str()), (deleted, content.as_str())]);
    git_in(&root, &["mv", moved, "crates/qoob3d-core/src/feature.rs"]);
    commit_all(&root, "Move");
    let plan = range_plan(&root, &base);
    assert_eq!(triggers(&plan), [Trigger::Format], "rename out of document/");

    let before = git_in(&root, &["rev-parse", "HEAD"]);
    std::fs::remove_file(root.join(deleted)).unwrap();
    commit_all(&root, "Delete");
    let plan = plan_range(&root, "HEAD", &[format!("{before}..HEAD")]).unwrap();
    assert_eq!(triggers(&plan), [Trigger::Format], "deletion in document/");
    std::fs::remove_dir_all(&root).unwrap();
}

/// A repository whose tip changes `PANELS`, committed on `committer_date`
/// with a different `author_date`, and whose escape log at the tip has `row`.
fn repo_with_dated_tip(
    test: &str,
    row: &str,
    author: &str,
    committer: &str,
) -> (std::path::PathBuf, String) {
    let (root, base) = repo_with_base(test, &[(PANELS, "fn f() {}\n")]);
    write_file(&root, "docs/escapes.md", &escapes(&[row.to_owned()]));
    write_file(&root, PANELS, "fn g() {}\n");
    git_in(&root, &["add", "-A"]);
    git_at(&root, author, committer, &["commit", "-q", "-m", "Tip"]);
    (root, base)
}

// ADR-0052 §1 escape / acceptance 5, on a real range: the window ends at the
// tip's committer date (not the author date, not today). Committer date
// 2025-03-01: 2024-09-03 is 179 days before (27 + 31 + 30 + 31 + 31 + 28 + 1),
// 2024-09-01 is 181. The author dates are chosen so that counting from them
// gives the opposite answer: 2025-03-03 is 181 days after 2024-09-03,
// 2025-02-27 is 179 days after 2024-09-01.
#[test]
fn plan_range_uses_the_tip_committer_date() {
    let row = escape_row("2024-09-03", &format!("`{PANELS}`"));
    let (root, base) = repo_with_dated_tip("plan-date-in", &row, "2025-03-03T12:00:00+00:00", DATE);
    let plan = range_plan(&root, &base);
    assert_eq!((plan.tier, triggers(&plan)), (Tier::High, vec![Trigger::Escape]));
    std::fs::remove_dir_all(&root).unwrap();

    let row = escape_row("2024-09-01", &format!("`{PANELS}`"));
    let (root, base) =
        repo_with_dated_tip("plan-date-out", &row, "2025-02-27T12:00:00+00:00", DATE);
    let plan = range_plan(&root, &base);
    assert_eq!(plan.tier, Tier::Standard, "{:?}", plan.reasons);
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 §1: the escape log is read as of the tip, not from the working
// tree: removing the row in the work tree keeps `high`, adding a matching
// row in the work tree keeps `standard`.
#[test]
fn plan_range_reads_escapes_as_of_the_tip() {
    let row = escape_row("2025-02-01", &format!("`{PANELS}`"));
    let (root, base) = repo_with_dated_tip("plan-escapes-tip", &row, DATE, DATE);
    write_file(&root, "docs/escapes.md", &escapes(&[]));
    assert_eq!(range_plan(&root, &base).tier, Tier::High);
    std::fs::remove_dir_all(&root).unwrap();

    let (root, base) = repo_with_base("plan-escapes-tree", &[(PANELS, "fn f() {}\n")]);
    write_file(&root, PANELS, "fn g() {}\n");
    commit_all(&root, "Tip");
    write_file(&root, "docs/escapes.md", &escapes(&[row]));
    let plan = range_plan(&root, &base);
    assert_eq!(plan.tier, Tier::Standard, "{:?}", plan.reasons);
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 §1: a tip without `docs/escapes.md` (older states predate it) has
// an empty escape log, not a plan error (coordinator decision 2026-10-16): a
// small app change stays `standard`.
#[test]
fn plan_range_without_escape_log_at_the_tip_is_an_empty_log() {
    let (root, base) = repo_with_base("plan-no-escapes", &[(PANELS, "fn f() {}\n")]);
    git_in(&root, &["rm", "-q", "docs/escapes.md"]);
    write_file(&root, PANELS, "fn g() {}\n");
    commit_all(&root, "Tip without escape log");
    let plan = plan_range(&root, "HEAD", &[format!("{base}..HEAD")]);
    assert!(plan.as_ref().is_ok_and(|p| p.tier == Tier::Standard), "{plan:?}");
    std::fs::remove_dir_all(&root).unwrap();
}

/// Stores `record` as the critic record of `HEAD`'s tree in `root`.
fn store_record(root: &Path, record: &Value) {
    let tree = git_in(root, &["rev-parse", "HEAD^{tree}"]);
    let mut record = record.clone();
    record["tree"] = json!(tree);
    write_file(root, &format!("{RECORD_DIR}/{tree}.json"), &record.to_string());
}

// ADR-0052 §3.2/§3.8: `cargo xtask review` uses the plan of the pushed range:
// a harness range needs a high record with lens security; a standard record
// is rejected for its tier.
#[test]
fn check_requires_a_record_for_the_plan_of_the_range() {
    let (root, base) = repo_with_base("check-harness", &[]);
    write_file(&root, "xtask/src/main.rs", "fn main() {}\n");
    commit_all(&root, "Harness");
    let range = [format!("{base}..HEAD")];
    let Err(found) = check(&root, "HEAD", &range) else { panic!("passed without a record") };
    assert_problem(&found, "no critic review", "no record");

    store_record(&root, &standard_record(vec![]));
    let Err(found) = check(&root, "HEAD", &range) else { panic!("standard record passed") };
    assert_problem(&found, "tier", "standard record for a harness range");

    let mut reviews = high_reviews();
    reviews.push(review("r4", MODEL_B, "security"));
    store_record(&root, &record("high", reviews, vec![]));
    assert_eq!(check(&root, "HEAD", &range), Ok(()));
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 §3: a small app range passes with a complete standard record and
// is rejected with a version-1 record.
#[test]
fn check_standard_range_with_v2_and_v1_records() {
    let (root, base) = repo_with_base("check-standard", &[(PANELS, "fn f() {}\n")]);
    write_file(&root, PANELS, "fn g() {}\n");
    commit_all(&root, "Small");
    let range = [format!("{base}..HEAD")];
    store_record(&root, &standard_record(vec![]));
    assert_eq!(check(&root, "HEAD", &range), Ok(()));

    store_record(&root, &json!({"findings": []}));
    let Err(found) = check(&root, "HEAD", &range) else { panic!("v1 record passed") };
    assert_problem(&found, "/critic-review", "v1 record");
    assert_problem(&found, "ADR-0052", "v1 record");
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 §1: an escape row with an unparsable date at the tip fails the
// gate closed, even with a complete record.
#[test]
fn check_fails_closed_on_an_unparsable_escape_date() {
    let (root, base) = repo_with_base("check-bad-date", &[(PANELS, "fn f() {}\n")]);
    write_file(&root, "docs/escapes.md", &escapes(&[escape_row("16.10.2026", "`docs/`")]));
    write_file(&root, PANELS, "fn g() {}\n");
    commit_all(&root, "Tip");
    store_record(&root, &standard_record(vec![]));
    let Err(found) = check(&root, "HEAD", &[format!("{base}..HEAD")]) else {
        panic!("unparsable escape date passed");
    };
    assert_problem(&found, "16.10.2026", "bad date");
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0041/ADR-0052 tier none: a range without code needs no record.
#[test]
fn check_range_without_code_needs_no_record() {
    let (root, base) = repo_with_base("check-docs", &[]);
    write_file(&root, "docs/architecture.md", "# Architecture\n");
    write_file(&root, "crates/qoob3d-core/src/document/tests.rs", "#[test]\nfn t() {}\n");
    commit_all(&root, "Docs");
    assert_eq!(check(&root, "HEAD", &[format!("{base}..HEAD")]), Ok(()));
    assert_eq!(range_plan(&root, &base).tier, Tier::None);
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 acceptance: the pre-push hook calls the gate as
// `review <tip> <rev-list args…>` with the pushed tip, without `--plan`. (Fix
// round 1 F2.2 changes the range; `xtask/src/worktree/tests.rs` runs the hook
// and checks the range itself.)
#[test]
fn pre_push_hook_calls_the_review_gate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let hook = std::fs::read_to_string(root.join(".githooks/pre-push")).unwrap();
    let calls = hook
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#') && l.contains("cargo xtask review"))
        .collect::<Vec<_>>();
    assert!(!calls.is_empty(), "{hook}");
    for call in calls {
        assert!(call.starts_with(r#"cargo xtask review "$local_sha" "#), "{call}");
    }
    assert!(!hook.contains("--plan"), "{hook}");
}

// ADR-0052 fix round 1: a range that changes only a harness file (here the
// pre-push hook) needs a critic record.
#[test]
fn check_harness_only_range_needs_a_record() {
    let (root, base) = repo_with_base("check-harness-only", &[]);
    write_file(&root, ".githooks/pre-push", "#!/usr/bin/env bash\nexit 0\n");
    commit_all(&root, "Hook");
    let Err(found) = check(&root, "HEAD", &[format!("{base}..HEAD")]) else {
        panic!("harness-only range passed without a record");
    };
    assert_problem(&found, "no critic review", "harness only");
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 fix round 1 F5: escape rows are the union of the log at the tip
// and at every boundary commit, so a range cannot drop the row that makes it
// high. Both range forms the hook uses are covered.
#[test]
fn plan_range_keeps_escape_rows_the_range_removes() {
    let row = escape_row("2025-02-01", &format!("`{PANELS}`"));
    let log = escapes(&[row]);
    let (root, base) =
        repo_with_base("plan-escape-union", &[(PANELS, "fn f() {}\n"), ("docs/escapes.md", &log)]);
    write_file(&root, "docs/escapes.md", &escapes(&[]));
    write_file(&root, PANELS, "fn g() {}\n");
    commit_all(&root, "Drop the escape row");
    for range in [vec![format!("{base}..HEAD")], vec!["HEAD".into(), "--not".into(), base.clone()]]
    {
        let plan = plan_range(&root, "HEAD", &range).unwrap();
        assert_eq!(triggers(&plan), [Trigger::Escape], "{range:?}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 fix round 1 F5: a boundary commit without `docs/escapes.md`
// contributes no rows and no error.
#[test]
fn plan_range_with_boundary_without_escape_log() {
    let (root, _) = repo_with_base("plan-boundary-no-log", &[(PANELS, "fn f() {}\n")]);
    git_in(&root, &["rm", "-q", "docs/escapes.md"]);
    let boundary = commit_all(&root, "Drop the log");
    write_file(&root, PANELS, "fn g() {}\n");
    commit_all(&root, "Tip");
    let plan = plan_range(&root, "HEAD", &[format!("{boundary}..HEAD")]);
    assert!(plan.as_ref().is_ok_and(|p| p.tier == Tier::Standard), "{plan:?}");
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 fix round 1 (merge commits): code a merge commit itself
// introduces counts; a plain merge of main does not count main's changes.
// main gets 500 lines in the document module (size + format if counted); the
// branch changes `PANELS` and merges main.
#[test]
fn plan_range_counts_what_a_merge_itself_changes() {
    let (root, _) = repo_with_base("plan-merge", &[(PANELS, "fn f() {}\n")]);
    git_in(&root, &["checkout", "-q", "-b", "feat"]);
    write_file(&root, PANELS, &format!("fn f() {{}}\n{}", lines(10)));
    let branch = commit_all(&root, "Branch");
    git_in(&root, &["checkout", "-q", "main"]);
    write_file(&root, "crates/qoob3d-core/src/document/main_only.rs", &lines(500));
    commit_all(&root, "Main");

    git_in(&root, &["checkout", "-q", "feat"]);
    git_in(&root, &["merge", "-q", "--no-ff", "--no-edit", "main"]);
    let plan = plan_range(&root, "HEAD", &["HEAD".into(), "--not".into(), "main".into()]).unwrap();
    assert_eq!(plan.tier, Tier::Standard, "plain merge: {:?}", plan.reasons);

    let evil = "crates/qoob3d-core/src/document/evil.rs";
    git_in(&root, &["checkout", "-q", "-b", "evil", &branch]);
    git_in(&root, &["merge", "-q", "--no-ff", "--no-commit", "main"]);
    write_file(&root, evil, "fn evil() {}\n");
    git_in(&root, &["add", "-A"]);
    git_in(&root, &["commit", "-q", "-m", "Merge main with a change of its own"]);
    let plan = plan_range(&root, "HEAD", &["HEAD".into(), "--not".into(), "main".into()]).unwrap();
    assert_eq!(triggers(&plan), [Trigger::Format], "evil merge: {:?}", plan.reasons);
    assert_eq!(plan.reasons[0].path, evil);
    std::fs::remove_dir_all(&root).unwrap();
}

// ADR-0052 fix round 1 (test quality): a binary code file (numstat `-`) is
// still a changed code file, without a plan error.
#[test]
fn plan_range_with_a_binary_code_file() {
    let (root, base) = repo_with_base("plan-binary", &[]);
    write_file(&root, "crates/qoob3d-app/src/blob.rs", "\0\u{1}\u{2}binary\0");
    commit_all(&root, "Binary");
    let plan = plan_range(&root, "HEAD", &[format!("{base}..HEAD")]);
    assert!(plan.as_ref().is_ok_and(|p| p.tier == Tier::Standard), "{plan:?}");
    std::fs::remove_dir_all(&root).unwrap();
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|&a| a.to_owned()).collect()
}

// ADR-0052 fix round 1 (test quality): `review [--plan] <tip> <revs…>`
// argument handling. `--plan` is never a tip, and revs are required.
#[test]
fn review_arguments() {
    let args = strings(&["--plan", "abc", "x", "--not", "--remotes=origin"]);
    assert_eq!(parse_args(&args), Some(Invocation::Plan { tip: "abc", revs: &args[2..] }));
    let args = strings(&["--plan", "abc", "a..b"]);
    assert_eq!(parse_args(&args), Some(Invocation::Plan { tip: "abc", revs: &args[2..] }));
    let args = strings(&["abc", "a..b"]);
    assert_eq!(parse_args(&args), Some(Invocation::Gate { tip: "abc", revs: &args[1..] }));
    let args = strings(&["abc", "x", "--not", "--remotes=origin"]);
    assert_eq!(parse_args(&args), Some(Invocation::Gate { tip: "abc", revs: &args[1..] }));
    for args in [
        &[][..],
        &["abc"][..],
        &["--plan"][..],
        &["--plan", "a..b"][..],
        &["--plan", "--plan", "a..b"][..],
    ] {
        assert_eq!(parse_args(&strings(args)), None, "{args:?}");
    }
}
