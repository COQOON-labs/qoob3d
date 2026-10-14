use super::*;

// ADR-0042: a full case: comments and blank lines ignored, `\n` in `message`
// becomes a newline, everything after `--- patch` (without that line) is the
// patch, verbatim, including lines that look like header lines.
#[test]
fn parses_full_case() {
    let patch = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1 +1,3 @@\n \
                 context\n+# not a comment\n+command: not a header\n\n";
    let text = format!(
        "# inline test module\n\ncommand: cargo xtask hygiene\nexpect: inline\n\
         # another comment\nmessage: first\\nsecond\nstdin: {{\"tool_name\":\"Bash\"}}\n--- patch\n{patch}"
    );
    let expected = Case {
        command: "cargo xtask hygiene".to_owned(),
        expect: "inline".to_owned(),
        message: "first\nsecond".to_owned(),
        stdin: Some("{\"tool_name\":\"Bash\"}".to_owned()),
        patch: Some(patch.to_owned()),
        env: vec![],
        identity: None,
    };
    assert_eq!(parse(&text), Ok(expected));
}

// ADR-0042: `message` defaults to "sabotage"; no stdin, no patch, no env,
// no identity (spec section 3).
#[test]
fn defaults_without_optional_keys() {
    let expected = Case {
        command: "cargo xtask commit-msg".to_owned(),
        expect: "ADR-0038".to_owned(),
        message: "sabotage".to_owned(),
        stdin: None,
        patch: None,
        env: vec![],
        identity: None,
    };
    assert_eq!(parse("command: cargo xtask commit-msg\nexpect: ADR-0038\n"), Ok(expected));
}

// ADR-0042: a colon inside the value belongs to the value.
#[test]
fn colon_in_value_belongs_to_value() {
    let case = parse("command: cargo xtask guard\nexpect: a: b\n").unwrap();
    assert_eq!(case.expect, "a: b");
}

// ADR-0042: `command` and `expect` are required.
#[test]
fn missing_required_keys_fail() {
    let error = parse("expect: x\n").unwrap_err();
    assert!(error.contains("`command` and `expect` are required"), "{error}");
    assert!(parse("command: cargo xtask hygiene\n").is_err());
    assert!(parse("").is_err());
    assert!(parse("# only a comment\n--- patch\ncommand: x\nexpect: y\n").is_err());
}

// ADR-0042: an unknown key is an error (catches typos in the corpus).
#[test]
fn unknown_key_fails() {
    let error = parse("command: cargo xtask hygiene\nexpect: x\nexpected: y\n").unwrap_err();
    assert!(error.contains("unknown key `expected`"), "{error}");
}

// ADR-0042 (spec section 3): a line without `key:` is an error naming the line.
#[test]
fn line_without_key_fails() {
    let error = parse("command: cargo xtask hygiene\nexpect: x\njust text\n").unwrap_err();
    assert!(error.contains("line without `key:`") && error.contains("just text"), "{error}");
}

// ADR-0042/ADR-0044 (spec section 3): `env:` may repeat, keeps the order and
// splits at the first `=`; `identity:` is trimmed.
#[test]
fn parses_env_and_identity() {
    let text = "command: cargo xtask commits --message m\nexpect: ADR-0044\n\
                env: QOOB3D_HARNESS_UNLOCK=1\nenv:  GIT_X=a=b \n\
                identity:  Mallory <mallory@example.com> \n";
    let case = parse(text).unwrap();
    assert_eq!(
        case.env,
        [
            ("QOOB3D_HARNESS_UNLOCK".to_owned(), "1".to_owned()),
            ("GIT_X".to_owned(), "a=b".to_owned())
        ]
    );
    assert_eq!(case.identity.as_deref(), Some("Mallory <mallory@example.com>"));
    assert_eq!(case.command, "cargo xtask commits --message m");
    assert_eq!(case.expect, "ADR-0044");
}

// ADR-0042 (spec section 3): `env:` without `=` is a parse error.
#[test]
fn env_without_equals_fails() {
    for line in ["env: QOOB3D_HARNESS_UNLOCK", "env: ", "env: KEY:VALUE"] {
        let text = format!("command: cargo xtask guard\nexpect: x\n{line}\n");
        assert!(parse(&text).is_err(), "{line:?}");
    }
}
