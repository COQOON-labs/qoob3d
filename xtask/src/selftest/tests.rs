use super::*;

/// Set for the child process of
/// `prepare_and_run_case_mean_the_work_copy_even_with_hook_git_variables`:
/// the directory holding the source repository `root/` and the work copy.
const CHILD_BASE: &str = "QOOB3D_XTASK_TEST_SELFTEST_CHILD_BASE";

/// The variables a git hook exports; they point git at another repository,
/// work tree or index than the one the command runs in.
const HOOK_GIT_ENV: [&str; 3] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"];

/// Author and committer of the commits in the work copy.
const IDENTITY: &str = "Selftest Author <selftest@example.com>";

/// A patch creating `b.txt` with the line `b`.
const NEW_FILE_PATCH: &str = "diff --git a/b.txt b/b.txt\nnew file mode 100644\n\
                              --- /dev/null\n+++ b/b.txt\n@@ -0,0 +1 @@\n+b\n";

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

// One git helper (critic K-01 on ef1aa48): the selftest's git calls mean the
// work copy even when the process inherited the variables a git hook exports
// (`cargo xtask check` runs inside the pre-push hook), pointing at a
// repository `elsewhere/` that does not exist. The test runs itself as a
// child with those variables set (the process environment cannot be changed
// safely in-process); the child runs `prepare` and one `run_case` and checks
// the result (see `child_prepares_and_runs_a_case_in_the_work_copy`). With
// the inherited GIT_DIR, `git ls-files` in `copy_tree` fails ("not a git
// repository") and `git init` would create the git dir under `elsewhere/`.
#[test]
fn prepare_and_run_case_mean_the_work_copy_even_with_hook_git_variables() {
    let base =
        std::env::temp_dir().join(format!("qoob3d-selftest-hook-env-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let root = base.join("root");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), "a\n").unwrap();
    for args in [&["init", "-q"][..], &["add", "a.txt"]] {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(&root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        for key in HOOK_GIT_ENV {
            command.env_remove(key);
        }
        assert!(command.status().unwrap().success(), "git {args:?}");
    }
    let elsewhere = base.join("elsewhere");
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "selftest::tests::child_prepares_and_runs_a_case_in_the_work_copy",
            "--nocapture",
        ])
        .env(CHILD_BASE, &base)
        // No user configuration (signing, hooks path) in the work copy.
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_DIR", elsewhere.join(".git"))
        .env("GIT_WORK_TREE", &elsewhere)
        .env("GIT_INDEX_FILE", elsewhere.join("index"))
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    assert!(text.contains("1 passed"), "the child test did not run:\n{text}");
    fs::remove_dir_all(&base).unwrap();
}

/// The child half of the test above; does nothing unless started by it.
/// `prepare` copies the staged `a.txt` into `work/` and commits it as
/// `baseline` (tagged); `run_case` resets to it, applies the patch creating
/// `b.txt`, commits it as `sabotage` and runs `ls no-such-file`, which fails
/// naming the file (the expected text), so the case counts as rejected.
#[test]
fn child_prepares_and_runs_a_case_in_the_work_copy() {
    let Some(base) = std::env::var_os(CHILD_BASE) else { return };
    let base = PathBuf::from(base);
    let (root, work, target) = (base.join("root"), base.join("work"), base.join("target"));
    assert_eq!(prepare(&root, &work, IDENTITY), Ok(()));
    let case = Case {
        command: "ls no-such-file".to_owned(),
        expect: "no-such-file".to_owned(),
        message: "sabotage".to_owned(),
        patch: Some(NEW_FILE_PATCH.to_owned()),
        ..Case::default()
    };
    assert_eq!(run_case(&work, &target, &case, IDENTITY), Ok(()));
    assert_eq!(crate::git(&work, &["ls-files"]), Ok("a.txt\nb.txt\n".to_owned()));
    assert_eq!(crate::git(&work, &["log", "--format=%s"]), Ok("sabotage\nbaseline\n".to_owned()));
    assert_eq!(
        crate::git(&work, &["log", "-1", "--format=%an <%ae>", "baseline"]),
        Ok(format!("{IDENTITY}\n"))
    );
    let top = crate::git(&work, &["rev-parse", "--show-toplevel"]).unwrap();
    assert_eq!(fs::canonicalize(top.trim()).unwrap(), fs::canonicalize(&work).unwrap());
    assert!(!base.join("elsewhere").exists(), "git wrote the repository named by GIT_DIR");
}
