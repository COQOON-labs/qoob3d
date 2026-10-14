use std::path::PathBuf;

use super::*;

fn ids_and_checks(rules: &[Rule]) -> Vec<(&str, Vec<&str>)> {
    rules.iter().map(|r| (r.id.as_str(), r.checks.iter().map(String::as_str).collect())).collect()
}

// ADR-0040: rule rows start with `R-` and have at least three cells; the last
// cell lists the checks, comma-separated, trimmed, without backticks, empty
// entries dropped. Header, separator and prose are ignored.
#[test]
fn parses_rule_rows() {
    let text = "# Rules\n\
                \n\
                Prose with R-01 and | pipe.\n\
                \n\
                | ID | Rule | Check |\n\
                | --- | --- | --- |\n\
                | R-01 | Done means check green | `xtask:check`, `ci:check` |\n\
                | R-02 | No AI attribution | `githook:commit-msg`,  , `xtask:commits`, |\n\
                | R-03 | only two cells |\n\
                | X-01 | not a rule | `xtask:check` |\n\
                | R-04 | Readability | manual |\n\
                | R-05 | four cells | a note | `test:parses_rule_rows` |\n";
    let rules = parse(text);
    assert_eq!(
        ids_and_checks(&rules),
        [
            ("R-01", vec!["xtask:check", "ci:check"]),
            ("R-02", vec!["githook:commit-msg", "xtask:commits"]),
            ("R-04", vec!["manual"]),
            ("R-05", vec!["test:parses_rule_rows"]),
        ]
    );
}

// ADR-0040: a text without rule rows yields no rules.
#[test]
fn empty_register_has_no_rules() {
    assert!(parse("").is_empty());
    assert!(parse("| ID | Rule | Check |\n| --- | --- | --- |\n").is_empty());
}

fn temp_root(test: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("qoob3d-rules-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("harness")).unwrap();
    root
}

// ADR-0040 (spec section 6): the register and the critic checklist have
// English file names.
#[test]
fn register_file_is_english() {
    assert_eq!(REGISTER, "harness/rules.md");
}

// ADR-0040 (spec section 6): the manual check kind is `manual`; the German
// `manuell` no longer exists.
#[test]
fn manual_check_kind_is_english() {
    let root = temp_root("manual");
    assert_eq!(exists(&root, "manual"), Ok(()));
    assert!(exists(&root, "manuell").is_err());
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0041 (spec section 6): `critic:` checks look up the rule id in
// `harness/critic-checklist.md`.
#[test]
fn critic_checks_use_the_critic_checklist() {
    let root = temp_root("critic");
    fs::write(root.join("harness/kritik.md"), "| K-01 | old |\n").unwrap();
    assert!(exists(&root, "critic:K-01").is_err());
    fs::write(root.join("harness/critic-checklist.md"), "| K-01 | new |\n").unwrap();
    assert_eq!(exists(&root, "critic:K-01"), Ok(()));
    assert!(exists(&root, "critic:K-02").is_err());
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0040 (spec section 6): a missing check and a rule without checks are
// reported in English.
#[test]
fn register_problems_are_english() {
    let root = temp_root("register");
    let register = "| ID | Rule | Check |\n| --- | --- | --- |\n\
                    | R-01 | a | `xtask:no-such-step` |\n\
                    | R-02 | b |  |\n\
                    | R-03 | c | manual |\n";
    fs::write(root.join(REGISTER), register).unwrap();
    let problems = check(&root).unwrap_err();
    assert!(
        problems.iter().any(|p| p.contains("R-01: check `xtask:no-such-step` does not exist")),
        "{problems:?}"
    );
    assert!(problems.iter().any(|p| p.contains("R-02: no check given")), "{problems:?}");
    assert_eq!(problems.len(), 2, "{problems:?}");
    fs::remove_dir_all(&root).unwrap();
}

// ---------------------------------------------------------------------------
// Review G4: `xtask:<step>` follows the real steps of `xtask/src/main.rs`
// (subcommands of main's dispatch and named steps of `check()`), so removing
// a step there turns its rules red. The tests copy the real `main.rs` into a
// temp root (as `<root>/xtask/src/main.rs`) and edit the copy.
// ---------------------------------------------------------------------------

/// The real `xtask/src/main.rs` of this build.
const MAIN_RS: &str = include_str!("../main.rs");

/// Named steps of `check()` in the real `main.rs` (read from it by hand).
const CHECK_STEPS: &[&str] = &[
    "fmt", "clippy", "test", "hooks", "hygiene", "rules", "arch", "docs", "i18n", "models", "deny",
    "smoke",
];
/// Subcommands of main's dispatch in the real `main.rs` (read by hand).
const SUBCOMMANDS: &[&str] = &[
    "check",
    "arch",
    "docs",
    "screenshot",
    "i18n",
    "hygiene",
    "commits",
    "setup",
    "guard",
    "rules",
    "selftest",
    "review",
    "models",
    "printers",
];

/// A temp root with `main_rs` as `xtask/src/main.rs` and `register`.
fn root_with_main(test: &str, main_rs: &str, register: &str) -> PathBuf {
    let root = temp_root(test);
    fs::create_dir_all(root.join("xtask/src")).unwrap();
    fs::write(root.join("xtask/src/main.rs"), main_rs).unwrap();
    fs::write(root.join(REGISTER), register).unwrap();
    root
}

/// One register row per check, ids R-01, R-02, ….
fn register_of(checks: &[String]) -> String {
    let mut text = String::from("| ID | Rule | Check |\n| --- | --- | --- |\n");
    for (i, check) in checks.iter().enumerate() {
        text.push_str(&format!("| R-{:02} | rule | `{check}` |\n", i + 1));
    }
    text
}

/// The real `main.rs` with `from` replaced by `to`; `from` must occur.
fn edited_main(from: &str, to: &str) -> String {
    assert!(MAIN_RS.contains(from), "main.rs no longer contains {from:?}; adapt the test");
    MAIN_RS.replace(from, to)
}

// ADR-0040 (review G4): with the real `main.rs`, every subcommand and check
// step exists, including subcommands the hand-kept list lacked
// (`setup`, `screenshot`, `printers`).
#[test]
fn every_real_step_exists() {
    let checks: Vec<String> =
        CHECK_STEPS.iter().chain(SUBCOMMANDS).map(|s| format!("xtask:{s}")).collect();
    let root = root_with_main("real_steps", MAIN_RS, &register_of(&checks));
    assert_eq!(check(&root), Ok(()));
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0040 (review G4): other string literals of `main.rs` (cargo and git
// arguments, flags) are no steps.
#[test]
fn other_literals_of_main_are_no_steps() {
    let root = root_with_main("other_literals", MAIN_RS, "");
    for name in ["cargo", "git", "--fast", "--fix", "--message", "warnings", "error", "nope"] {
        assert!(exists(&root, &format!("xtask:{name}")).is_err(), "xtask:{name}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0040 (review G4, the spec's example): removing `deny` from `check()`
// makes `cargo xtask rules` report exactly "R-10: check `xtask:deny` does not
// exist"; the other steps still exist.
#[test]
fn step_removed_from_check_turns_its_rule_red() {
    let main_rs = edited_main("\"deny\"", "\"withdrawn-step\"");
    let mut checks: Vec<String> = (1..=9).map(|_| "xtask:fmt".to_owned()).collect();
    checks.push("xtask:deny".into());
    checks.push("xtask:smoke".into());
    let root = root_with_main("removed_deny", &main_rs, &register_of(&checks));
    assert_eq!(check(&root), Err(vec!["R-10: check `xtask:deny` does not exist".to_owned()]));
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0040 (review G4): removing a subcommand from main's dispatch turns its
// rule red.
#[test]
fn subcommand_removed_from_dispatch_turns_its_rule_red() {
    let main_rs = edited_main("Some(\"guard\")", "Some(\"withdrawn-step\")");
    let checks = ["xtask:guard".to_owned(), "xtask:selftest".to_owned()];
    let root = root_with_main("removed_guard", &main_rs, &register_of(&checks));
    assert_eq!(check(&root), Err(vec!["R-01: check `xtask:guard` does not exist".to_owned()]));
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0040 (review G4): a step that is both a subcommand and a check step
// (`rules`) exists as long as either remains, and not when both are gone.
#[test]
fn step_in_dispatch_or_check_exists() {
    let register = register_of(&["xtask:rules".to_owned()]);
    let only_check = edited_main("Some(\"rules\")", "Some(\"withdrawn-step\")");
    let only_dispatch = edited_main("(\"rules\", Step::", "(\"withdrawn-step\", Step::");
    let neither = edited_main("\"rules\"", "\"withdrawn-step\"");
    for (name, main_rs, expected) in [
        ("rules_check", only_check, Ok(())),
        ("rules_dispatch", only_dispatch, Ok(())),
        (
            "rules_neither",
            neither,
            Err(vec!["R-01: check `xtask:rules` does not exist".to_owned()]),
        ),
    ] {
        let root = root_with_main(name, &main_rs, &register);
        assert_eq!(check(&root), expected, "{name}");
        fs::remove_dir_all(&root).unwrap();
    }
}

// ADR-0040 (review G4): the hand-kept copy of the step names is gone from
// `rules.rs`; a step name literal there would be such a copy.
#[test]
fn rules_keeps_no_copy_of_the_step_names() {
    let source = include_str!("../rules.rs");
    assert!(!source.contains("XTASK_STEPS"), "XTASK_STEPS still exists");
    for step in ["\"selftest\"", "\"smoke\"", "\"clippy\""] {
        assert!(!source.contains(step), "rules.rs names the step {step}");
    }
}
