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
