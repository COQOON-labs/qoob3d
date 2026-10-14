use serde_json::json;

use super::*;

const FIXTURE: &str = "crates/qoob3d-core/tests/fixtures/v1.qbd";
const CODE: &str = "crates/qoob3d-core/src/expr.rs";
const TEST: &str = "crates/qoob3d-core/tests/expr.rs";
const OTHER: &str = "docs/x.md";

/// The root and home used by the examples in the spec (§1.4). They are plain
/// strings for `check_shell`; nothing is read from or written to them.
const SPEC_ROOT: &str = "/Users/ben/Development/coqoon/qoob3d";
const SPEC_HOME: &str = "/Users/ben";

/// Message fragments (spec §1.3–§1.5).
const MSG_HARNESS_SHELL: &str = "could modify the harness file `";
const MSG_SELF_UNLOCK: &str = "can set the environment of Claude Code hooks";
const MSG_BYPASS: &str = "bypasses the quality gates";
const MSG_LAUNCHCTL: &str = "`launchctl setenv` changes the environment of future sessions";

/// A fresh repository root with `.git/` and one existing fixture.
fn repo(test: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("qoob3d-guard-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join(".git")).unwrap();
    let fixture = root.join(FIXTURE);
    fs::create_dir_all(fixture.parent().unwrap()).unwrap();
    fs::write(fixture, "{}").unwrap();
    root
}

/// A fresh home directory (with an empty `.claude/`) outside the repository.
fn home_dir(test: &str) -> PathBuf {
    let home =
        std::env::temp_dir().join(format!("qoob3d-guard-home-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join(".claude")).unwrap();
    home
}

/// A home that is never created; for tests where the home does not matter.
fn unused_home() -> PathBuf {
    std::env::temp_dir().join(format!("qoob3d-guard-unused-home-{}", std::process::id()))
}

fn session(unlocked: bool, home: &Path) -> Session {
    Session { unlocked, home: home.to_path_buf() }
}

fn locked() -> Session {
    session(false, &unused_home())
}

fn abs(root: &Path, rel: &str) -> String {
    root.join(rel).to_str().unwrap().to_owned()
}

fn write(
    tool: &str,
    file: &str,
    session: &str,
    agent_id: Option<&str>,
    agent_type: Option<&str>,
) -> Value {
    let mut input = json!({
        "tool_name": tool,
        "tool_input": {"file_path": file, "content": "x"},
        "session_id": session,
    });
    if let Some(id) = agent_id {
        input["agent_id"] = json!(id);
    }
    if let Some(kind) = agent_type {
        input["agent_type"] = json!(kind);
    }
    input
}

fn bash(command: &str) -> Value {
    json!({"tool_name": "Bash", "tool_input": {"command": command}, "session_id": "s"})
}

/// `check_shell` with the spec's root and home.
fn spec_shell(unlocked: bool, command: &str) -> Result<(), String> {
    check_shell(Path::new(SPEC_ROOT), &session(unlocked, Path::new(SPEC_HOME)), command)
}

/// Asserts a denial whose message contains every needle.
fn assert_denied(result: Result<(), String>, needles: &[&str], what: &str) {
    match result {
        Ok(()) => panic!("{what}: passed, expected a denial"),
        Err(message) => {
            for needle in needles {
                assert!(message.contains(needle), "{what}: {message:?} lacks {needle:?}");
            }
        }
    }
}

/// Commands that write a harness file through a repo-relative path, with the
/// harness fragment each one names (spec §1.4, MUST BE DENIED, plus the
/// harness writes denied before this change).
const HARNESS_WRITES: &[(&str, &str)] = &[
    ("sed -i '' s/a/b/ Cargo.toml", "Cargo.toml"),
    ("cp a .claude/settings.json", ".claude/"),
    ("cp a ./xtask/src/main.rs", "xtask/"),
    ("echo x >Cargo.toml", "Cargo.toml"),
    ("echo x > .claude/settings.json", ".claude/"),
    ("echo x >> .cargo/config.toml", ".cargo/"),
    ("sed -i '' s/100/200/ clippy.toml", "clippy.toml"),
    ("cat /tmp/x | tee deny.toml", "deny.toml"),
    ("mv /tmp/main.rs xtask/src/main.rs", "xtask/"),
    ("cp /tmp/ci.yml .github/workflows/ci.yml", ".github/"),
    ("rm .githooks/pre-push", ".githooks"),
    ("git checkout -- Cargo.toml", "Cargo.toml"),
    ("python3 -c 'open(\"rustfmt.toml\", \"w\")'", "rustfmt.toml"),
    ("cat <<EOF > harness/rules.md", "harness/"),
    ("cp /tmp/fake target/debug/xtask", "target/debug/xtask"),
    ("rm rust-toolchain.toml", "rust-toolchain"),
    // A null redirection next to a real write does not hide the write
    // (review5 A3): stripping `2>/dev/null`, `>/dev/null`, `&>/dev/null` and
    // `2>&1` still leaves `>`, `tee `, `cp ` or `rm `.
    ("echo x > xtask/src/guard.rs 2>/dev/null", "xtask/"),
    ("cat a 2>/dev/null >Cargo.toml", "Cargo.toml"),
    ("cat a 2>&1 | tee harness/rules.md", "harness/"),
    ("cp /tmp/x .githooks/pre-push 2>/dev/null", ".githooks"),
    ("cat a >/dev/null >> clippy.toml", "clippy.toml"),
    ("rm -f deny.toml &>/dev/null", "deny.toml"),
];

/// Commands that must pass in a locked session (spec §1.4, MUST PASS, plus
/// the harmless commands that passed before this change).
const HARMLESS: &[&str] = &[
    "",
    "export PATH=\"/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH\"; cargo xtask check 2>&1 | tail",
    "printf x >> /Users/ben/.claude/projects/foo/MEMORY.md",
    "echo x > crates/qoob3d-core/Cargo.toml",
    "cargo xtask check",
    "cargo test -p xtask",
    "cat xtask/src/main.rs",
    "grep -n foo Cargo.toml",
    "ls .claude/agents",
    "git diff --stat Cargo.toml",
    "git push origin feature",
    "echo hi > /tmp/x",
    // Reads of harness files with a null redirection write no file
    // (review5 A3): the exempt redirections of pure reads (`2>/dev/null`,
    // `>/dev/null`, `&>/dev/null`, `2>&1`, also with whitespace after `>`)
    // are no write operation for the harness check either.
    "cat xtask/src/guard.rs 2>/dev/null",
    "grep -rn foo xtask/src 2>&1 | head",
    "ls harness/ >/dev/null",
    "cat Cargo.toml &>/dev/null",
    "ls harness/ > /dev/null",
    "cat clippy.toml 2> /dev/null",
];

// ADR-0037/ADR-0043 (spec §1.3): commands that bypass a gate or name the
// unlock/skip variables are denied.
#[test]
fn shell_bypasses_are_denied() {
    for command in [
        "git commit --no-verify -m x",
        "git -c core.hooksPath=/dev/null commit -m x",
        "git config core.hooksPath .nohooks",
        "gh pr merge 12 --admin",
        "QOOB3D_HARNESS_UNLOCK=1 claude",
        "export QOOB3D_SKIP_REVIEW=1 && git push",
    ] {
        assert!(spec_shell(false, command).is_err(), "{command}");
    }
    assert_denied(
        spec_shell(false, "git commit --no-verify -m x"),
        &["`--no-verify` bypasses the quality gates"],
        "bypass message",
    );
}

// ADR-0037 (spec §1.4): a repo-relative harness path plus a write operation is
// denied in a locked session, with the root given as a temp repo and as the
// spec's absolute string. The message names the harness fragment.
#[test]
fn shell_writes_to_harness_are_denied() {
    let root = repo("shell_harness_writes");
    for (command, fragment) in HARNESS_WRITES {
        let needle = format!("{MSG_HARNESS_SHELL}{fragment}`");
        assert_denied(spec_shell(false, command), &[&needle], command);
        assert_denied(check_shell(&root, &locked(), command), &[&needle], command);
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (spec §1.4): reading harness files, writing elsewhere and writing
// files that only share a name with a harness file pass in a locked session.
#[test]
fn harmless_shell_commands_pass() {
    let root = repo("shell_harmless");
    for command in HARMLESS {
        assert_eq!(spec_shell(false, command), Ok(()), "{command}");
        assert_eq!(check_shell(&root, &locked(), command), Ok(()), "{command}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (spec §1.4): an absolute path into this repository counts, spelled
// with the root as passed, its canonical form (macOS `/var` → `/private/var`),
// quoted, or via $CLAUDE_PROJECT_DIR / $PWD / $(pwd).
#[test]
fn absolute_harness_paths_into_this_repo_are_denied() {
    let root = repo("shell_absolute");
    let root_str = root.to_str().unwrap().to_owned();
    let canonical = fs::canonicalize(&root).unwrap().to_str().unwrap().to_owned();
    let mut prefixes = vec![
        root_str.clone(),
        canonical.clone(),
        format!("\"{root_str}\""),
        format!("'{root_str}'"),
        format!("\"{canonical}\""),
    ];
    for var in ["$CLAUDE_PROJECT_DIR", "${CLAUDE_PROJECT_DIR}", "$PWD", "${PWD}", "$(pwd)"] {
        prefixes.push(var.to_owned());
        prefixes.push(format!("\"{var}\""));
    }
    for prefix in &prefixes {
        for (command, fragment) in [
            (format!("echo x > {prefix}/xtask/src/main.rs"), "xtask/"),
            (format!("cp a {prefix}/Cargo.toml"), "Cargo.toml"),
            (format!("rm {prefix}/.githooks/pre-push"), ".githooks"),
        ] {
            let needle = format!("{MSG_HARNESS_SHELL}{fragment}`");
            assert_denied(check_shell(&root, &locked(), &command), &[&needle], &command);
        }
    }
    // A quote that opens before the root and closes after the file.
    let quoted = format!("echo x > \"{root_str}/xtask/src/main.rs\"");
    assert!(check_shell(&root, &locked(), &quoted).is_err(), "{quoted}");
    // The spec's literal examples.
    for command in [
        "echo x > /Users/ben/Development/coqoon/qoob3d/xtask/src/main.rs",
        "echo x > \"$CLAUDE_PROJECT_DIR\"/xtask/src/main.rs",
    ] {
        assert_denied(spec_shell(false, command), &[MSG_HARNESS_SHELL, "xtask/"], command);
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (spec §1.4): a fragment counts at a token start, i.e. at the start
// of the command or after whitespace, `"`, `'`, `=`, `(`, `;`, `|`, `&`, `>`,
// `<` or a backtick, and after a `./` whose dot starts a token.
#[test]
fn harness_fragment_at_token_start_counts() {
    for sep in [" ", "\t", "\"", "'", "=", "(", ";", "|", "&", ">", "<", "`", " ./", "=./"] {
        let command = format!("echo x > /tmp/out {sep}xtask/src/main.rs");
        assert_denied(spec_shell(false, &command), &[MSG_HARNESS_SHELL], &command);
    }
    // At the very start of the command.
    let command = "xtask/run.sh > /tmp/out";
    assert!(spec_shell(false, command).is_err(), "{command}");
}

// ADR-0043 (spec §1.4): a fragment inside another path (after `/`, a letter,
// `-`, or a `./` whose dot is part of a name) refers to another directory and
// passes even with a write operation.
#[test]
fn harness_fragment_inside_other_paths_passes() {
    let root = repo("shell_other_paths");
    let root_str = root.to_str().unwrap().to_owned();
    let mut commands = vec![
        "echo x > /tmp/elsewhere/xtask/src/main.rs".to_owned(),
        "cp a /Users/ben/other/Cargo.toml".to_owned(),
        "cp a crates/qoob3d-sketch/Cargo.toml".to_owned(),
        "echo x > docs/xtask/notes.md".to_owned(),
        "echo x > notes-harness/x.md".to_owned(),
        "echo x > myxtask/a.rs".to_owned(),
        "echo x > a./xtask/x".to_owned(),
        "rm -rf /tmp/qoob3d/.github/workflows".to_owned(),
        "echo x > ../other/.cargo/config.toml".to_owned(),
        // A sibling directory that shares the root as prefix.
        format!("echo x > {root_str}-old/xtask/src/main.rs"),
        format!("echo x > {root_str}x/Cargo.toml"),
    ];
    commands.extend(HARMLESS.iter().map(|c| (*c).to_owned()));
    for command in &commands {
        assert_eq!(check_shell(&root, &locked(), command), Ok(()), "{command}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (spec §1.2, §1.4): in an unlocked session the harness writes pass.
#[test]
fn unlocked_session_allows_harness_shell_writes() {
    let root = repo("shell_unlocked");
    let unlocked = session(true, &unused_home());
    for (command, _) in HARNESS_WRITES {
        assert_eq!(spec_shell(true, command), Ok(()), "{command}");
        assert_eq!(check_shell(&root, &unlocked, command), Ok(()), "{command}");
    }
    for command in [
        "echo x > /Users/ben/Development/coqoon/qoob3d/xtask/src/main.rs",
        "echo x > \"$CLAUDE_PROJECT_DIR\"/xtask/src/main.rs",
    ] {
        assert_eq!(spec_shell(true, command), Ok(()), "{command}");
    }
    for command in HARMLESS {
        assert_eq!(check_shell(&root, &unlocked, command), Ok(()), "{command}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (spec §1.2, §1.3): unlocking does not lift bypass detection,
// including both environment variable names.
#[test]
fn unlocked_session_still_denies_bypasses() {
    for command in [
        "git commit --no-verify -m x",
        "git -c core.hooksPath=/dev/null commit -m x",
        "gh pr merge 12 --admin",
        "QOOB3D_HARNESS_UNLOCK=1 claude",
        "export QOOB3D_HARNESS_UNLOCK=1",
        "export QOOB3D_SKIP_REVIEW=1 && git push",
        "QOOB3D_SKIP_REVIEW=1 git push",
    ] {
        assert_denied(spec_shell(true, command), &[MSG_BYPASS], command);
    }
}

// ADR-0043 (spec §1.3): `launchctl setenv` with any whitespace is denied,
// locked and unlocked; other launchctl subcommands are not affected.
#[test]
fn launchctl_setenv_is_denied() {
    for unlocked in [false, true] {
        for command in [
            "launchctl setenv QOOB3D_X 1",
            "launchctl  setenv QOOB3D_X 1",
            "launchctl\tsetenv FOO 1",
            "true && launchctl setenv FOO bar",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_LAUNCHCTL, "ADR-0043"],
                &format!("{command} (unlocked: {unlocked})"),
            );
        }
        assert_eq!(spec_shell(unlocked, "launchctl getenv PATH"), Ok(()));
    }
}

// ADR-0043 (spec §1.3): a write together with a protected config fragment is
// denied, locked and unlocked; the home may also be spelled out.
#[test]
fn shell_writes_to_hook_environment_files_are_denied() {
    let home = home_dir("shell_config");
    let home_str = home.to_str().unwrap().to_owned();
    let root = repo("shell_config");
    let mut commands: Vec<String> = [
        "echo 'export X=1' >> ~/.zshrc",
        "echo x >> ~/.zprofile",
        "echo x >> ~/.zshenv",
        "echo x >> ~/.bashrc",
        "echo x >> ~/.bash_profile",
        "echo x >> ~/.profile",
        "echo '{}' > ~/.claude/settings.json",
        "echo '{}' > $HOME/.claude/settings.json",
        "echo '{}' > ${HOME}/.claude/settings.local.json",
        "cp a /Users/ben/.claude/settings.json",
        "tee \"/Library/Application Support/ClaudeCode/managed-settings.json\" < x",
        "cp a /etc/claude-code/managed-settings.json",
        "echo '{}' > .vscode/settings.json",
        "cp a \"$HOME/Library/Application Support/Code/User/settings.json\"",
        "cp a ~/.config/Code/User/settings.json",
    ]
    .iter()
    .map(|c| (*c).to_owned())
    .collect();
    commands.push(format!("echo x > {home_str}/.claude/settings.json"));
    for unlocked in [false, true] {
        let s = session(unlocked, &home);
        for command in &commands {
            let what = format!("{command} (unlocked: {unlocked})");
            // SPEC_HOME spelled out only matters with SPEC_HOME as home.
            let result = if command.contains(SPEC_HOME) {
                spec_shell(unlocked, command)
            } else {
                check_shell(&root, &s, command)
            };
            assert_denied(result, &[MSG_SELF_UNLOCK], &what);
        }
        // Inside the repo `.claude/` is also harness, so locked the harness
        // message may come first; unlocked only the self-unlock rule applies.
        let local = "cp a .claude/settings.local.json";
        if unlocked {
            assert_denied(check_shell(&root, &s, local), &[MSG_SELF_UNLOCK], local);
        } else {
            assert!(check_shell(&root, &s, local).is_err(), "{local}");
        }
    }
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
}

// ADR-0043 (spec §1.3): reading the protected files is fine.
#[test]
fn reading_hook_environment_files_passes() {
    for unlocked in [false, true] {
        for command in [
            "cat ~/.zshrc",
            "cat ~/.claude/settings.json",
            "grep -n hooks .claude/settings.local.json",
            "cat .vscode/settings.json",
            "ls /etc/claude-code",
        ] {
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{command} (unlocked: {unlocked})");
        }
    }
}

// ADR-0037: absolute paths inside the root become repo-relative, even for
// files in directories that do not exist yet; relative input stays as is.
#[test]
fn relative_paths_inside_root() {
    let root = repo("relative_inside");
    assert_eq!(relative(&root, &abs(&root, FIXTURE)), Some(FIXTURE.to_owned()));
    let new = "crates/new-crate/src/deep/mod.rs";
    assert_eq!(relative(&root, &abs(&root, new)), Some(new.to_owned()));
    assert_eq!(relative(&root, "docs/x.md"), Some("docs/x.md".to_owned()));
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037: paths outside the root, including a sibling sharing the root's
// name as prefix, are `None`.
#[test]
fn relative_paths_outside_root() {
    let root = repo("relative_outside");
    assert_eq!(relative(&root, "/etc/hosts"), None);
    let parent = root.parent().unwrap().join("elsewhere.rs");
    assert_eq!(relative(&root, parent.to_str().unwrap()), None);
    let sibling = format!("{}x/src/a.rs", root.to_str().unwrap());
    assert_eq!(relative(&root, &sibling), None);
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037: a symlinked root (macOS `/var` → `/private/var`) compares
// correctly in both directions.
#[test]
fn relative_paths_through_symlinked_root() {
    let root = repo("relative_symlink");
    let canonical = fs::canonicalize(&root).unwrap();
    let new = "crates/x/src/new/file.rs";
    assert_eq!(relative(&root, &abs(&canonical, new)), Some(new.to_owned()));
    assert_eq!(relative(&canonical, &abs(&root, new)), Some(new.to_owned()));
    assert_eq!(relative(&canonical, &abs(&root, FIXTURE)), Some(FIXTURE.to_owned()));
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (spec §1.5): every file-writing tool is denied on harness files in
// a locked session, whatever the role.
#[test]
fn harness_files_are_read_only() {
    let root = repo("harness_read_only");
    for tool in ["Edit", "Write", "MultiEdit", "NotebookEdit"] {
        for rel in
            ["xtask/src/main.rs", ".claude/settings.json", "Cargo.toml", "xtask/src/guard/tests.rs"]
        {
            let input = write(tool, &abs(&root, rel), "s", None, None);
            assert!(pre_tool_use(&root, &locked(), &input).is_err(), "{tool} {rel}");
        }
    }
    let input =
        write("Edit", &abs(&root, "xtask/src/main.rs"), "s", Some("a"), Some("implementer"));
    assert_denied(
        pre_tool_use(&root, &locked(), &input),
        &["`xtask/src/main.rs` belongs to the harness and is read-only for agents (ADR-0037)"],
        "harness message",
    );
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (spec §1.2): an unlocked session may edit harness files; each file
// in its own context so that no role binding interferes.
#[test]
fn unlocked_session_allows_harness_edits() {
    let root = repo("harness_unlocked");
    let unlocked = session(true, &unused_home());
    for tool in ["Edit", "Write", "MultiEdit"] {
        for rel in ["xtask/src/main.rs", ".claude/settings.json", "Cargo.toml", "harness/rules.md"]
        {
            let context = format!("{tool}-{rel}");
            let input = write(tool, &abs(&root, rel), &context, None, None);
            assert_eq!(pre_tool_use(&root, &unlocked, &input), Ok(()), "{tool} {rel}");
        }
    }
    let input = write(
        "Edit",
        &abs(&root, "xtask/src/guard/tests.rs"),
        "t",
        Some("t1"),
        Some("test-author"),
    );
    assert_eq!(pre_tool_use(&root, &unlocked, &input), Ok(()));
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0030/ADR-0037 (spec §1.5): existing fixtures are never modified; new
// ones may be added.
#[test]
fn existing_fixtures_are_read_only() {
    let root = repo("fixtures");
    for tool in ["Edit", "Write"] {
        let input = write(tool, &abs(&root, FIXTURE), "s", Some("a"), Some("test-author"));
        assert!(pre_tool_use(&root, &locked(), &input).is_err(), "{tool}");
    }
    let input = write("Edit", &abs(&root, FIXTURE), "s", Some("a"), Some("test-author"));
    assert_denied(
        pre_tool_use(&root, &locked(), &input),
        &[&format!("`{FIXTURE}` is a format fixture and immutable (ADR-0030)")],
        "fixture message",
    );
    let new = abs(&root, "crates/qoob3d-core/tests/fixtures/v99.qbd");
    assert_eq!(
        pre_tool_use(&root, &locked(), &write("Write", &new, "s", Some("a"), Some("test-author"))),
        Ok(())
    );
    assert_eq!(pre_tool_use(&root, &locked(), &write("Write", &new, "s2", None, None)), Ok(()));
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (spec §1.2): unlocking does not lift fixture protection or the
// role separation.
#[test]
fn unlocked_session_keeps_fixture_and_role_protection() {
    let root = repo("unlocked_keeps");
    let unlocked = session(true, &unused_home());
    for tool in ["Edit", "Write"] {
        let input = write(tool, &abs(&root, FIXTURE), "s", None, None);
        assert_denied(
            pre_tool_use(&root, &unlocked, &input),
            &["is a format fixture and immutable (ADR-0030)"],
            tool,
        );
    }
    // First write binds the main context to production code ...
    let main = |rel: &str| write("Edit", &abs(&root, rel), "u1", None, None);
    assert_eq!(pre_tool_use(&root, &unlocked, &main("xtask/src/guard.rs")), Ok(()));
    // ... so a test file, also inside the harness, is denied.
    assert_denied(
        pre_tool_use(&root, &unlocked, &main("xtask/src/guard/tests.rs")),
        &["already writes production code", "(ADR-0039)", "`test-author`"],
        "role switch",
    );
    // Role agents stay fixed by type.
    let author =
        write("Edit", &abs(&root, "xtask/src/guard.rs"), "u2", Some("t"), Some("test-author"));
    assert_denied(
        pre_tool_use(&root, &unlocked, &author),
        &["already writes tests", "`implementer`"],
        "test-author on code",
    );
    let implementer = write(
        "Edit",
        &abs(&root, "xtask/src/guard/tests.rs"),
        "u2",
        Some("i"),
        Some("implementer"),
    );
    assert!(pre_tool_use(&root, &unlocked, &implementer).is_err());
    fs::remove_dir_all(&root).unwrap();
}

/// Files that can set the environment of Claude Code hooks (spec §1.3).
/// `home` is the session home, `root` the repository, `elsewhere` a directory
/// outside both.
fn self_unlock_files(home: &Path, root: &Path, elsewhere: &Path) -> Vec<String> {
    let mut files = vec![
        abs(home, ".claude/settings.json"),
        abs(home, ".claude/settings.local.json"),
        abs(home, ".claude/settings-backup.json"),
        "/Library/Application Support/ClaudeCode/managed-settings.json".to_owned(),
        "/etc/claude-code/managed-settings.json".to_owned(),
        "/etc/claude-code/sub/anything.txt".to_owned(),
        abs(elsewhere, ".claude/settings.local.json"),
        abs(root, ".vscode/settings.json"),
        ".vscode/settings.json".to_owned(),
        abs(elsewhere, ".vscode/settings.json"),
        abs(home, "Library/Application Support/Code/User/settings.json"),
        abs(home, ".config/Code/User/settings.json"),
    ];
    for rc in [".zshrc", ".zprofile", ".zshenv", ".bashrc", ".bash_profile", ".profile"] {
        files.push(abs(home, rc));
    }
    files
}

// ADR-0043 (spec §1.3): every file that can set the hook environment is
// denied for all writing tools, locked and unlocked, also when spelled via
// the canonical home (macOS `/var` → `/private/var`).
#[test]
fn hook_environment_files_are_never_written() {
    let root = repo("self_unlock");
    let home = home_dir("self_unlock");
    let elsewhere = home_dir("self_unlock_elsewhere");
    let canonical_home = fs::canonicalize(&home).unwrap();
    let mut files = self_unlock_files(&home, &root, &elsewhere);
    files.extend(self_unlock_files(&canonical_home, &root, &elsewhere));
    for unlocked in [false, true] {
        let s = session(unlocked, &home);
        for tool in ["Edit", "Write", "MultiEdit", "NotebookEdit"] {
            for (i, file) in files.iter().enumerate() {
                let input = write(tool, file, &format!("c{i}"), None, None);
                assert_denied(
                    pre_tool_use(&root, &s, &input),
                    &[MSG_SELF_UNLOCK, "ADR-0043"],
                    &format!("{tool} {file} (unlocked: {unlocked})"),
                );
            }
        }
        // `.claude/settings.local.json` in the repo, absolute and relative:
        // locked, the harness rule may answer first.
        for file in
            [abs(&root, ".claude/settings.local.json"), ".claude/settings.local.json".into()]
        {
            let result = pre_tool_use(&root, &s, &write("Write", &file, "local", None, None));
            if unlocked {
                assert_denied(result, &[MSG_SELF_UNLOCK, "ADR-0043"], &file);
            } else {
                assert!(result.is_err(), "{file}");
            }
        }
    }
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
    fs::remove_dir_all(&elsewhere).unwrap();
}

// ADR-0043 (spec §1.3): other files under `<home>/.claude/` (memory, global
// CLAUDE.md, settings files not directly in `.claude/`) and other files in
// the home pass, locked and unlocked.
#[test]
fn other_home_files_pass() {
    let root = repo("home_other");
    let home = home_dir("home_other");
    for unlocked in [false, true] {
        let s = session(unlocked, &home);
        for rel in [
            ".claude/projects/x/memory/MEMORY.md",
            ".claude/CLAUDE.md",
            ".claude/projects/x/settings.json",
            ".claude/agents/helper.md",
            "notes.txt",
            ".zshrc.d/README",
        ] {
            for tool in ["Edit", "Write"] {
                let input = write(tool, &abs(&home, rel), "s", None, None);
                assert_eq!(pre_tool_use(&root, &s, &input), Ok(()), "{tool} {rel} ({unlocked})");
            }
        }
        // VS Code files other than settings.json in the repo are ordinary files.
        let launch = write("Write", &abs(&root, ".vscode/launch.json"), "s", None, None);
        assert_eq!(pre_tool_use(&root, &s, &launch), Ok(()));
    }
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
}

// ADR-0037: files outside the repository are not the guard's business
// (unless they can set the hook environment, see above).
#[test]
fn files_outside_root_pass() {
    let root = repo("outside_root");
    let outside =
        std::env::temp_dir().join(format!("qoob3d-guard-elsewhere-{}", std::process::id()));
    for rel in ["Cargo.toml", "xtask/src/main.rs", CODE] {
        let file = outside.join(rel);
        let input = write("Write", file.to_str().unwrap(), "s", Some("a"), Some("test-author"));
        assert_eq!(pre_tool_use(&root, &locked(), &input), Ok(()), "{rel}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0039 (spec §1.5): the `test-author` agent writes only tests, the
// `implementer` only code.
#[test]
fn role_agents_are_fixed_by_type() {
    let root = repo("role_agents");
    let s = locked();
    let author = |rel: &str| write("Edit", &abs(&root, rel), "s", Some("t1"), Some("test-author"));
    assert_denied(
        pre_tool_use(&root, &s, &author(CODE)),
        &[
            "This agent context already writes tests;",
            &format!("`{CODE}` must be changed by another context (ADR-0039)"),
            "Delegate to the subagent `implementer`",
        ],
        "test-author on code",
    );
    assert!(pre_tool_use(&root, &s, &author("crates/qoob3d-kernel/build.rs")).is_err());
    assert_eq!(pre_tool_use(&root, &s, &author(TEST)), Ok(()));
    assert_eq!(pre_tool_use(&root, &s, &author("crates/qoob3d-core/src/expr/tests.rs")), Ok(()));
    assert_eq!(pre_tool_use(&root, &s, &author("models/gear.expect.json")), Ok(()));
    assert!(pre_tool_use(&root, &s, &author(CODE)).is_err());

    let implementer =
        |rel: &str| write("Write", &abs(&root, rel), "s", Some("i1"), Some("implementer"));
    assert_denied(
        pre_tool_use(&root, &s, &implementer(TEST)),
        &["This agent context already writes production code;", "`test-author`"],
        "implementer on tests",
    );
    assert_eq!(pre_tool_use(&root, &s, &implementer(CODE)), Ok(()));
    assert!(pre_tool_use(&root, &s, &implementer("crates/qoob3d-core/src/expr/tests.rs")).is_err());
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0039, critic finding K-13: the C++ header of the kernel wrappers is
// production code; the test author may not edit it, the implementer may.
#[test]
fn test_author_may_not_edit_cxx_header() {
    const HEADER: &str = "crates/qoob3d-kernel/cpp/qoob3d_occt.hxx";
    let root = repo("cxx_header");
    let s = locked();
    let author = write("Edit", &abs(&root, HEADER), "s", Some("t1"), Some("test-author"));
    assert_denied(
        pre_tool_use(&root, &s, &author),
        &[
            "This agent context already writes tests;",
            &format!("`{HEADER}` must be changed by another context (ADR-0039)"),
        ],
        "test-author on .hxx",
    );
    let implementer = write("Edit", &abs(&root, HEADER), "s", Some("i1"), Some("implementer"));
    assert_eq!(pre_tool_use(&root, &s, &implementer), Ok(()));
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0039: any other context is bound by its first write of tests or code;
// switching is denied, the same kind passes; the ledger is in `.git/qoob3d/roles/`.
#[test]
fn other_contexts_are_bound_by_first_write() {
    let root = repo("first_write");
    let s = locked();
    let main = |rel: &str| write("Edit", &abs(&root, rel), "s1", None, None);
    assert_eq!(pre_tool_use(&root, &s, &main(CODE)), Ok(()));
    assert_eq!(pre_tool_use(&root, &s, &main("crates/qoob3d-kernel/cpp/qoob3d_occt.cpp")), Ok(()));
    assert!(pre_tool_use(&root, &s, &main(TEST)).is_err());
    assert_eq!(pre_tool_use(&root, &s, &main(CODE)), Ok(()));
    let ledger = root.join(".git/qoob3d/roles");
    assert!(fs::read_dir(&ledger).unwrap().next().is_some(), "no role recorded in {ledger:?}");

    let general =
        |rel: &str| write("Write", &abs(&root, rel), "s1", Some("g1"), Some("general-purpose"));
    assert_eq!(pre_tool_use(&root, &s, &general(TEST)), Ok(()));
    assert!(pre_tool_use(&root, &s, &general(CODE)).is_err());
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0039: the binding is per (session, agent) context.
#[test]
fn contexts_are_independent() {
    let root = repo("independent_contexts");
    let s = locked();
    let edit = |rel: &str, session: &str, agent: Option<&str>| {
        pre_tool_use(&root, &s, &write("Edit", &abs(&root, rel), session, agent, None))
    };
    assert_eq!(edit(CODE, "s1", Some("a1")), Ok(()));
    assert_eq!(edit(TEST, "s2", Some("a1")), Ok(()));
    assert_eq!(edit(TEST, "s1", Some("a2")), Ok(()));
    assert_eq!(edit(TEST, "s1", None), Ok(()));
    assert!(edit(TEST, "s1", Some("a1")).is_err());
    assert!(edit(CODE, "s2", Some("a1")).is_err());
    assert!(edit(CODE, "s1", Some("a2")).is_err());
    assert!(edit(CODE, "s1", None).is_err());
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0039: other files (docs, config) always pass and bind no role.
#[test]
fn other_files_bind_no_role() {
    let root = repo("other_files");
    let s = locked();
    let edit =
        |rel: &str| pre_tool_use(&root, &s, &write("Edit", &abs(&root, rel), "s", Some("a"), None));
    assert_eq!(edit(OTHER), Ok(()));
    assert_eq!(edit("crates/qoob3d-core/Cargo.toml"), Ok(()));
    assert_eq!(edit(TEST), Ok(()));
    assert_eq!(edit(OTHER), Ok(()));
    assert!(edit(CODE).is_err());
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037/ADR-0043: Bash goes through `check_shell` with the session;
// read-only tools always pass.
#[test]
fn bash_and_read_tools() {
    let root = repo("bash_and_read");
    let s = locked();
    assert!(pre_tool_use(&root, &s, &bash("echo x > .claude/settings.json")).is_err());
    assert!(pre_tool_use(&root, &s, &bash("git commit --no-verify")).is_err());
    assert_eq!(pre_tool_use(&root, &s, &bash("cargo xtask check")), Ok(()));
    for tool in ["Read", "Grep", "Glob"] {
        let input = write(tool, &abs(&root, "xtask/src/main.rs"), "s", None, None);
        assert_eq!(pre_tool_use(&root, &s, &input), Ok(()), "{tool}");
    }
    let unlocked = session(true, &unused_home());
    assert_eq!(pre_tool_use(&root, &unlocked, &bash("echo x > .claude/settings.json")), Ok(()));
    assert!(pre_tool_use(&root, &unlocked, &bash("echo x >> ~/.zshrc")).is_err());
    assert!(pre_tool_use(&root, &unlocked, &bash("launchctl setenv A 1")).is_err());
    assert!(pre_tool_use(&root, &unlocked, &bash("git push --no-verify")).is_err());
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (spec section 1.1): the session is unlocked only for exactly
// QOOB3D_HARNESS_UNLOCK=1; `home` is the given HOME.
#[test]
fn session_from_env_values() {
    let s = Session::from_env_values(Some("1"), Some("/Users/ben"));
    assert!(s.unlocked);
    assert_eq!(s.home, PathBuf::from("/Users/ben"));
    for unlock in [None, Some(""), Some("0"), Some("true"), Some(" 1"), Some("1 "), Some("01")] {
        let s = Session::from_env_values(unlock, Some("/home/x"));
        assert!(!s.unlocked, "{unlock:?}");
        assert_eq!(s.home, PathBuf::from("/home/x"), "{unlock:?}");
    }
}

// ---------------------------------------------------------------------------
// Fixes from the critic review (spec addendum F1, F2, T1, F3).
// ---------------------------------------------------------------------------

/// Message fragments of the addendum.
const MSG_GIT_DIR: &str = "`.git/` is managed by git and the guard";
const MSG_COMMIT_N: &str = "`git commit -n` bypasses the quality gates (ADR-0037)";

// ADR-0043 (addendum F1, MUST BE DENIED): a hook-environment file named in a
// command that is not a pure read is denied, locked and unlocked. The first
// four slipped through before (no WRITES fragment, or a quote-split name).
#[test]
fn f1_non_read_commands_on_hook_environment_files_are_denied() {
    for unlocked in [false, true] {
        for command in [
            "perl -pi -e s/a/b/ ~/.zshenv",
            "echo x | tee -a ~/.zsh''env",
            "echo x >> ~/.zsh''rc",
            "awk 1 x | dd of=$HOME/.zshenv",
            "sed -i '' s/a/b/ ~/.zprofile",
            "python3 -c 'open(\"/Users/x/.bashrc\",\"a\")'",
            "cp x ~/.claude/settings.json",
            "cat x > ~/.zshrc",
            // `tee` is not read-only.
            "cat ~/.zshrc | tee ~/.zshrc.bak",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_SELF_UNLOCK, "ADR-0043"],
                &format!("{command} (unlocked: {unlocked})"),
            );
        }
    }
}

// ADR-0043 (addendum F1, MUST PASS): pure reads of hook-environment files and
// commands without such a fragment pass, locked and unlocked.
#[test]
fn f1_pure_reads_of_hook_environment_files_pass() {
    for unlocked in [false, true] {
        for command in [
            "cat ~/.zshrc",
            "grep -n PATH ~/.zshrc",
            "head -5 ~/.zprofile | wc -l",
            "ls -la ~/.claude",
            "ls ~/.claude/settings.json",
            "diff ~/.zshrc ~/.zshrc.orig",
        ] {
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{command} (unlocked: {unlocked})");
        }
    }
}

// ADR-0043 (addendum F1): every program of the read-only list may start a
// segment; segments are split on `|`, `;`, `&&`, `||`, `&` and newline.
#[test]
fn f1_every_read_only_program_passes() {
    for unlocked in [false, true] {
        for command in [
            "tail -n 3 ~/.zshrc",
            "head ~/.bash_profile",
            "rg PATH ~/.zshenv",
            "stat ~/.profile",
            "wc -l ~/.zshrc",
            "file ~/.zprofile",
            "test -f ~/.zshrc",
            "[ -f ~/.zshrc ]",
            "cat ~/.zshrc | grep PATH | wc -l",
            "cat ~/.zshrc; cat ~/.zprofile",
            "test -f ~/.zshrc && cat ~/.zshrc",
            "test -f ~/.zshrc || ls ~",
            "cat ~/.zshrc & cat ~/.bashrc",
            "cat ~/.zshrc\nhead ~/.zprofile",
            "cat ~/.claude/settings.local.json",
        ] {
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{command:?} (unlocked: {unlocked})");
        }
    }
}

// ADR-0043 (addendum F1, revised): `less` and `more` are not read-only
// (`less -o FILE` / `--log-file` writes a file), so naming a hook-environment
// file with them is denied, locked and unlocked.
#[test]
fn f1_less_and_more_are_not_read_only() {
    for unlocked in [false, true] {
        for command in [
            "cat x | less -o ~/.zshrc",
            "cat x | less --log-file=~/.zshrc",
            "less ~/.zshrc",
            "more ~/.zshrc",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_SELF_UNLOCK, "ADR-0043"],
                &format!("{command} (unlocked: {unlocked})"),
            );
        }
    }
}

// ADR-0043 (addendum F1): the read exception needs every segment to start
// with a read-only program, no `>` anywhere and no leading `VAR=value`.
#[test]
fn f1_read_exception_is_strict() {
    for unlocked in [false, true] {
        for command in [
            // A later segment is not a read (one per separator).
            "cat ~/.zshrc; rm ~/.zshrc",
            "cat ~/.zshrc && vim ~/.zshrc",
            "cat ~/.zshrc || nano ~/.zshrc",
            "cat ~/.zshrc & ed ~/.zshrc",
            "cat ~/.zshrc\nvim ~/.zshrc",
            // `>` anywhere, also with a read-only program and to another file
            // (review4 S2 exempts only the `/dev/null` and `2>&1` redirections).
            "cat ~/.zshrc > /tmp/copy",
            "grep PATH ~/.zshrc 2>/tmp/err",
            // Leading assignment.
            "FOO=1 cat ~/.zshrc",
            // A program not on the list.
            "vim ~/.zshrc",
            "open ~/.zshrc",
            "echo ~/.zshrc",
            // Backslash-split name (removed before matching).
            "echo x >> ~/.zsh\\rc",
            "echo x >> ~/.zs\"\"henv",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_SELF_UNLOCK],
                &format!("{command:?} (unlocked: {unlocked})"),
            );
        }
    }
}

// ADR-0037 (addendum F2, MUST BE DENIED): bypasses are matched lower-cased and
// with quotes and backslashes removed, locked and unlocked; the message names
// the (lower-case) entry.
#[test]
fn f2_bypass_variants_are_denied() {
    for unlocked in [false, true] {
        for (command, entry) in [
            ("git config core.hookspath /dev/null", "core.hookspath"),
            ("git -c CORE.HOOKSPATH=x commit", "core.hookspath"),
            ("env QOOB3D_SKIP_''REVIEW=1 git push", "qoob3d_skip_review"),
            ("git push --NO-VERIFY", "--no-verify"),
            ("git push --no-\\verify", "--no-verify"),
            ("gh pr merge 1 --ADMIN", "--admin"),
            ("export qoob3d_harness_unlock=1", "qoob3d_harness_unlock"),
            ("env QOOB3D_HARNESS_\"\"UNLOCK=1 claude", "qoob3d_harness_unlock"),
        ] {
            let needle = format!("`{entry}` {MSG_BYPASS}");
            assert_denied(
                spec_shell(unlocked, command),
                &[&needle],
                &format!("{command} (unlocked: {unlocked})"),
            );
        }
    }
}

// ADR-0037 (addendum F2): `git commit` with a short-flag group containing `n`
// is `--no-verify`, also after `git -c x=y`; locked and unlocked.
#[test]
fn f2_git_commit_short_n_is_denied() {
    for unlocked in [false, true] {
        for command in [
            "git commit -n -m x",
            "git commit -anm x",
            "git commit -nm x",
            "git commit -an",
            "git commit -m x -n",
            "git -c user.name=x commit -n -m x",
            "cargo test && git commit -n -m x",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_COMMIT_N],
                &format!("{command} (unlocked: {unlocked})"),
            );
        }
    }
}

// ADR-0037 (addendum F2): ordinary commits pass; the `n` must be in a word
// with exactly one leading `-`, and in the same segment as `git commit`.
#[test]
fn f2_ordinary_git_commits_pass() {
    for unlocked in [false, true] {
        for command in [
            "git commit -m x",
            "git commit --amend",
            "git commit -a -m x",
            "git commit --amend --no-edit",
            "git -c core.editor=nano commit -m x",
            "git commit -m x && ls -n",
            "git log -n 3",
        ] {
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{command} (unlocked: {unlocked})");
        }
    }
}

// ADR-0043 (addendum T1): no file-writing tool may touch anything under
// `<root>/.git/`, locked and unlocked, absolute or relative.
#[test]
fn t1_files_under_git_dir_are_never_written() {
    let root = repo("t1_git_dir");
    for unlocked in [false, true] {
        let s = session(unlocked, &unused_home());
        for tool in ["Edit", "Write", "MultiEdit", "NotebookEdit"] {
            for file in [
                abs(&root, ".git/config"),
                abs(&root, ".git/hooks/pre-commit"),
                abs(&root, ".git/qoob3d/roles/s--main"),
                ".git/config".to_owned(),
                ".git/hooks/pre-push".to_owned(),
            ] {
                let input = write(tool, &file, "t1", None, None);
                assert_denied(
                    pre_tool_use(&root, &s, &input),
                    &[MSG_GIT_DIR, "ADR-0043"],
                    &format!("{tool} {file} (unlocked: {unlocked})"),
                );
            }
        }
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (addendum T1): names that only start like `.git` are not under
// `.git/`; unlocked they pass (`.github/`, `.githooks/` are harness).
#[test]
fn t1_git_like_names_are_not_the_git_dir() {
    let root = repo("t1_git_like");
    let unlocked = session(true, &unused_home());
    for (i, rel) in [".gitignore", ".github/CODEOWNERS", ".githooks/pre-push", ".gitattributes"]
        .iter()
        .enumerate()
    {
        let input = write("Write", &abs(&root, rel), &format!("g{i}"), None, None);
        assert_eq!(pre_tool_use(&root, &unlocked, &input), Ok(()), "{rel}");
    }
    let input = write("Write", &abs(&root, ".gitignore"), "g", None, None);
    assert_eq!(pre_tool_use(&root, &locked(), &input), Ok(()));
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (addendum T1, refined by review3 R2 and review4 S1): `.git/config`
// and `.git/hooks` named in a command that is no pure read are denied with
// the `.git/` message, locked and unlocked; pure reads pass.
#[test]
fn t1_git_config_and_hooks_in_shell() {
    let root = repo("t1_shell");
    for unlocked in [false, true] {
        let s = session(unlocked, &unused_home());
        for command in [
            "echo '[core]' >> .git/config",
            "cp /tmp/hook .git/hooks/pre-commit",
            "perl -pi -e s/a/b/ .git/config",
            "chmod +x .git/hooks/pre-push",
            "ln -sf /tmp/x .git/hooks/pre-push",
        ] {
            let what = format!("{command} (unlocked: {unlocked})");
            assert_denied(check_shell(&root, &s, command), &[MSG_GIT_DIR, ADR_0043], &what);
        }
        for command in ["cat .git/config", "ls .git/hooks", "grep -n url .git/config"] {
            assert_eq!(check_shell(&root, &s, command), Ok(()), "{command} (unlocked: {unlocked})");
        }
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (addendum F3): `.` and `..` are collapsed lexically before the
// harness check, for absolute and relative paths.
#[test]
fn f3_dot_segments_do_not_hide_harness_files() {
    let root = repo("f3_harness");
    for (i, (tool, file, rel)) in [
        ("Edit", abs(&root, "nonexistent/../xtask/src/guard.rs"), "xtask/src/guard.rs"),
        ("Write", "crates/../xtask/src/main.rs".to_owned(), "xtask/src/main.rs"),
        ("Write", "./xtask/src/main.rs".to_owned(), "xtask/src/main.rs"),
        ("Write", abs(&root, "./Cargo.toml"), "Cargo.toml"),
        ("Edit", abs(&root, "crates/a/b/../../../harness/rules.md"), "harness/rules.md"),
    ]
    .into_iter()
    .enumerate()
    {
        let input = write(tool, &file, &format!("f3-{i}"), None, None);
        assert_denied(
            pre_tool_use(&root, &locked(), &input),
            &[&format!("`{rel}` belongs to the harness and is read-only for agents (ADR-0037)")],
            &format!("{tool} {file}"),
        );
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0030 (addendum F3): a `..` through a missing directory does not hide an
// existing fixture, locked and unlocked.
#[test]
fn f3_dot_segments_do_not_hide_fixtures() {
    let root = repo("f3_fixture");
    let fixture = "crates/qoob3d-core/tests/fixtures/format_v1.qbd";
    fs::write(root.join(fixture), "{}").unwrap();
    let file = abs(&root, "crates/nonexistent/../qoob3d-core/tests/fixtures/format_v1.qbd");
    for unlocked in [false, true] {
        let s = session(unlocked, &unused_home());
        let input = write("Write", &file, "f3", Some("a"), Some("test-author"));
        assert_denied(
            pre_tool_use(&root, &s, &input),
            &[&format!("`{fixture}` is a format fixture and immutable (ADR-0030)")],
            &format!("{file} (unlocked: {unlocked})"),
        );
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (addendum F3): a `..` through a missing directory in the home does
// not hide a shell startup file, locked and unlocked.
#[test]
fn f3_dot_segments_do_not_hide_hook_environment_files() {
    let root = repo("f3_home");
    let home = home_dir("f3_home");
    for unlocked in [false, true] {
        let s = session(unlocked, &home);
        for file in [abs(&home, "nonexistent/../.zshenv"), abs(&home, ".claude/x/../settings.json")]
        {
            for tool in ["Edit", "Write"] {
                let input = write(tool, &file, "f3h", None, None);
                assert_denied(
                    pre_tool_use(&root, &s, &input),
                    &[MSG_SELF_UNLOCK, "ADR-0043"],
                    &format!("{tool} {file} (unlocked: {unlocked})"),
                );
            }
        }
    }
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
}

// ADR-0039 (addendum F3: "before any check"): the role check sees the
// collapsed path, so a `tests/…/..` detour does not turn code into a test.
// (`tests/` exists in `repo`, `missing/` does not, so canonicalising the
// existing ancestor alone would keep the `tests` component.)
#[test]
fn f3_dot_segments_do_not_change_the_role_kind() {
    let root = repo("f3_role");
    let detour = abs(&root, "crates/qoob3d-core/tests/missing/../../src/expr.rs");
    let input = write("Edit", &detour, "s", Some("t1"), Some("test-author"));
    assert_denied(
        pre_tool_use(&root, &locked(), &input),
        &["This agent context already writes tests;", &format!("`{CODE}` must be changed")],
        "test-author on code via tests/..",
    );
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (addendum F3): `relative` collapses `.` and `..`; a path that
// leaves the root after collapsing is outside.
#[test]
fn f3_relative_collapses_dot_segments() {
    let root = repo("f3_relative");
    let root_str = root.to_str().unwrap().to_owned();
    assert_eq!(relative(&root, "crates/../docs/x.md"), Some("docs/x.md".to_owned()));
    assert_eq!(relative(&root, "./docs/./x.md"), Some("docs/x.md".to_owned()));
    assert_eq!(
        relative(&root, &format!("{root_str}/nonexistent/../docs/x.md")),
        Some("docs/x.md".to_owned())
    );
    assert_eq!(relative(&root, "../x"), None);
    assert_eq!(relative(&root, "crates/../../x"), None);
    assert_eq!(relative(&root, &format!("{root_str}/a/../../x")), None);
    assert_eq!(relative(&root, &format!("{root_str}/a/b/../../../x/Cargo.toml")), None);
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (addendum F3): a file that is outside the root after collapsing is
// not the guard's business, even if its tail looks like a harness path.
#[test]
fn f3_paths_leaving_the_root_pass() {
    let root = repo("f3_outside");
    let root_str = root.to_str().unwrap().to_owned();
    for file in [
        format!("{root_str}/crates/../../elsewhere/xtask/src/main.rs"),
        "../elsewhere/Cargo.toml".to_owned(),
    ] {
        let input = write("Write", &file, "s", Some("a"), Some("test-author"));
        assert_eq!(pre_tool_use(&root, &locked(), &input), Ok(()), "{file}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ---------------------------------------------------------------------------
// Fixes from the branch review (review spec G2, G3, D1, F6).
// ---------------------------------------------------------------------------

/// Message of an abbreviated `--no-verify` (review G2).
const MSG_NO_VERIFY: &str = "`--no-verify` bypasses the quality gates";
/// Every denial that protects against self-unlock names this decision.
const ADR_0043: &str = "ADR-0043";

/// Runs `f` for a locked and an unlocked session.
fn both_sessions(mut f: impl FnMut(bool)) {
    for unlocked in [false, true] {
        f(unlocked);
    }
}

// ADR-0037 (review G2): git accepts unambiguous abbreviations of long
// options, so every prefix of `--no-verify` that is at least as long as
// `--no-veri` is the same bypass. Matched lower-cased and with quotes
// removed; locked and unlocked; the message names `--no-verify`.
#[test]
fn g2_abbreviated_no_verify_is_denied() {
    both_sessions(|unlocked| {
        for command in [
            // The spec's examples.
            "git push --no-veri",
            "git commit --no-verif -m x",
            "git push origin HEAD --NO-VERI",
            // Mixed case, quoted, in a later segment.
            "git commit -m x --No-Verif",
            "git push '--no-veri'",
            "cargo test && git push --no-veri origin",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_NO_VERIFY],
                &format!("{command} (unlocked: {unlocked})"),
            );
        }
    });
}

// ADR-0037 (review G2): other `--no-…` options pass. `--no-ver` is shorter
// than `--no-veri` (ambiguous with `--no-verbose`, git rejects it), and
// `--no-verbose` is no prefix of `--no-verify`.
#[test]
fn g2_other_no_options_pass() {
    both_sessions(|unlocked| {
        for command in [
            "git commit --no-verbose -m x",
            "git log --no-merges",
            "git commit --no-ver -m x",
            "git commit --no-edit --amend",
            "git push --no-thin origin",
        ] {
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{command} (unlocked: {unlocked})");
        }
    });
}

// ADR-0043 (review G3, refined by review3 R2 and review4 S1): the guard
// binary lives in `.git/qoob3d/guard-target`, so a shell command naming
// `.git` that is no pure read and no exclusion is denied with the `.git/`
// message, locked and unlocked, relative, absolute or via
// $CLAUDE_PROJECT_DIR. The cargo target-dir cases need no special rule any
// more: `cargo build` is no read-only program.
#[test]
fn g3_shell_writes_under_git_dir_are_denied() {
    let root = repo("g3_git_shell");
    let root_str = root.to_str().unwrap().to_owned();
    let commands = [
        "cp /tmp/fake .git/qoob3d/guard-target/debug/xtask".to_owned(),
        "rm -rf .git/qoob3d/guard-target".to_owned(),
        "echo x > .git/info/exclude".to_owned(),
        "touch .git/qoob3d/roles/s--main".to_owned(),
        "mv /tmp/x .git/HEAD".to_owned(),
        // No WRITES fragment; denied because `cargo` is no read-only program.
        "CARGO_TARGET_DIR=.git/qoob3d/guard-target cargo build --package xtask".to_owned(),
        "cargo build --target-dir .git/qoob3d/guard-target -p xtask".to_owned(),
        "dd if=/tmp/fake of=.git/qoob3d/guard-target/debug/xtask".to_owned(),
        format!("cp /tmp/fake {root_str}/.git/qoob3d/guard-target/debug/xtask"),
        "cp /tmp/fake \"$CLAUDE_PROJECT_DIR\"/.git/qoob3d/guard-target/debug/xtask".to_owned(),
    ];
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in &commands {
            assert_denied(
                check_shell(&root, &s, command),
                &[MSG_GIT_DIR, ADR_0043],
                &format!("{command} (unlocked: {unlocked})"),
            );
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review G3, review3 R2): reads of `.git/` pass; names
// that only start like `.git` (`.github/`, `.githooks/`, `.gitignore`) are
// not the git directory, and git commands without a `.git/` path are
// unaffected. Unlocked, harness writes to `.github/`/`.githooks/` pass.
#[test]
fn g3_git_dir_reads_and_git_like_names_pass() {
    let root = repo("g3_git_reads");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in [
            "cat .git/HEAD",
            "ls .git/qoob3d/guard-target/debug",
            "stat .git/qoob3d/guard-target/debug/xtask",
            "grep -r main .git/refs | wc -l",
            "echo target >> .gitignore",
            "cat .github/CODEOWNERS",
            "git status",
            "git log --oneline -3",
        ] {
            assert_eq!(check_shell(&root, &s, command), Ok(()), "{command} (unlocked: {unlocked})");
        }
    });
    let unlocked = session(true, &unused_home());
    for command in ["cp /tmp/ci.yml .github/workflows/ci.yml", "rm .githooks/pre-push"] {
        assert_eq!(check_shell(&root, &unlocked, command), Ok(()), "{command}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (review G3): cargo also links the xtask binary as
// `target/debug/deps/xtask-<hash>`; that prefix is a harness path in shell
// commands. Locked denied, unlocked allowed (the unlock lifts harness paths).
#[test]
fn g3_xtask_deps_binary_in_shell_is_harness() {
    let root = repo("g3_deps_shell");
    let root_str = root.to_str().unwrap().to_owned();
    let commands = [
        "cp /tmp/fake target/debug/deps/xtask-0123abcd".to_owned(),
        "rm target/debug/deps/xtask-*".to_owned(),
        format!("cp /tmp/fake {root_str}/target/debug/deps/xtask-0123abcd"),
    ];
    let unlocked = session(true, &unused_home());
    for command in &commands {
        assert_denied(check_shell(&root, &locked(), command), &[MSG_HARNESS_SHELL], command);
        assert_eq!(check_shell(&root, &unlocked, command), Ok(()), "{command} (unlocked)");
    }
    for command in [
        "cp /tmp/x target/debug/deps/qoob3d_core-0123abcd",
        "rm target/debug/deps/xtaskish-1",
        "cargo build -p xtask",
    ] {
        assert_eq!(check_shell(&root, &locked(), command), Ok(()), "{command}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (review G3): `target/debug/deps/xtask-<hash>` is a harness path
// for the file tools too: locked denied with the harness message, unlocked
// allowed. `target/debug/xtask` stays a harness path.
#[test]
fn g3_xtask_deps_binary_is_read_only_for_file_tools() {
    let root = repo("g3_deps_write");
    let unlocked = session(true, &unused_home());
    for (i, rel) in [
        "target/debug/deps/xtask-0123abcd",
        "target/debug/deps/xtask-0123abcd.d",
        "target/debug/xtask",
    ]
    .iter()
    .enumerate()
    {
        for tool in ["Edit", "Write"] {
            let input = write(tool, &abs(&root, rel), &format!("g3-{i}"), None, None);
            assert_denied(
                pre_tool_use(&root, &locked(), &input),
                &[&format!(
                    "`{rel}` belongs to the harness and is read-only for agents (ADR-0037)"
                )],
                &format!("{tool} {rel}"),
            );
            assert_eq!(pre_tool_use(&root, &unlocked, &input), Ok(()), "{tool} {rel} (unlocked)");
        }
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review D1): macOS file systems ignore case, so hook-environment
// fragments in shell commands match case-insensitively: denied locked and
// unlocked. (`.Claude/…` inside the repo is also harness; locked, either
// message may answer.)
#[test]
fn d1_shell_hook_environment_fragments_ignore_case() {
    let root = repo("d1_shell_env");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in [
            "echo export X=1 >> ~/.ZSHENV",
            "echo {} > ~/.claude/Settings.json",
            "echo x >> ~/.ZshRc",
            "cp x ~/.CLAUDE/settings.json",
            "echo x > $HOME/.Profile",
            "echo x > .VSCode/Settings.json",
            "echo '[core]' >> .GIT/config",
            "cp /tmp/fake .Git/qoob3d/guard-target/debug/xtask",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[ADR_0043],
                &format!("{command} (unlocked: {unlocked})"),
            );
        }
        let local = "echo {} > .Claude/Settings.Local.json";
        let what = format!("{local} (unlocked: {unlocked})");
        if unlocked {
            assert_denied(check_shell(&root, &s, local), &[MSG_SELF_UNLOCK], &what);
        } else {
            assert!(check_shell(&root, &s, local).is_err(), "{what}");
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (review D1): harness fragments in shell commands match
// case-insensitively. Locked denied with the harness message (spec root and a
// temp repo), unlocked allowed.
#[test]
fn d1_shell_harness_fragments_ignore_case() {
    let root = repo("d1_shell_harness");
    let root_str = root.to_str().unwrap().to_owned();
    let commands = [
        "cp x XTASK/src/main.rs".to_owned(),
        "echo x >> CARGO.TOML".to_owned(),
        "echo x > .Cargo/config.toml".to_owned(),
        "cp x Clippy.toml".to_owned(),
        "rm .GitHooks/pre-push".to_owned(),
        "cp x Target/Debug/XTASK".to_owned(),
        format!("cp x {root_str}/Xtask/src/main.rs"),
    ];
    let unlocked = session(true, &unused_home());
    for command in &commands {
        if !command.contains(&root_str) {
            assert_denied(spec_shell(false, command), &[MSG_HARNESS_SHELL], command);
            assert_eq!(spec_shell(true, command), Ok(()), "{command} (unlocked)");
        }
        assert_denied(check_shell(&root, &locked(), command), &[MSG_HARNESS_SHELL], command);
        assert_eq!(check_shell(&root, &unlocked, command), Ok(()), "{command} (unlocked)");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review D1): the protected-file comparison of the file tools
// ignores case, also for files that do not exist yet; denied locked and
// unlocked. None of these files exists in the temp home or repo, so the
// file system cannot correct the case.
#[test]
fn d1_hook_environment_files_ignore_case() {
    let root = repo("d1_files");
    let home = home_dir("d1_files");
    let elsewhere = home_dir("d1_files_elsewhere");
    let files = [
        abs(&home, ".ZSHENV"),
        abs(&home, ".ZProfile"),
        abs(&home, ".Bash_Profile"),
        abs(&home, ".PROFILE"),
        abs(&home, ".claude/Settings.json"),
        abs(&home, ".claude/SETTINGS.LOCAL.JSON"),
        abs(&elsewhere, ".Claude/Settings.Local.json"),
        abs(&root, ".VSCode/Settings.json"),
        abs(&home, "Library/Application Support/Code/User/SETTINGS.json"),
        abs(&home, ".config/code/user/settings.json"),
        "/etc/Claude-Code/managed-settings.json".to_owned(),
        "/Library/Application Support/claudecode/managed-settings.json".to_owned(),
    ];
    both_sessions(|unlocked| {
        let s = session(unlocked, &home);
        for (i, file) in files.iter().enumerate() {
            for tool in ["Edit", "Write"] {
                let input = write(tool, file, &format!("d1-{i}"), None, None);
                assert_denied(
                    pre_tool_use(&root, &s, &input),
                    &[MSG_SELF_UNLOCK, ADR_0043],
                    &format!("{tool} {file} (unlocked: {unlocked})"),
                );
            }
        }
        // In the repo `.claude/` is also harness: locked, either rule may
        // answer; unlocked only the self-unlock rule applies.
        let local = abs(&root, ".claude/Settings.Local.json");
        let result = pre_tool_use(&root, &s, &write("Write", &local, "d1-local", None, None));
        if unlocked {
            assert_denied(result, &[MSG_SELF_UNLOCK, ADR_0043], &local);
        } else {
            assert!(result.is_err(), "{local}");
        }
    });
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&home).unwrap();
    fs::remove_dir_all(&elsewhere).unwrap();
}

// ADR-0037 (review D1): the harness check compares the repo-relative path
// case-insensitively: `Cargo.TOML` (no such file) and directories that do
// not exist in the temp repo are denied locked, allowed unlocked.
// `XTASK/new.rs` with an existing `xtask/` is listed as well (the spec says
// it is caught already).
#[test]
fn d1_harness_files_ignore_case() {
    let root = repo("d1_harness");
    fs::create_dir_all(root.join("xtask")).unwrap();
    let unlocked = session(true, &unused_home());
    for (i, (rel, normal)) in [
        ("Cargo.TOML", "cargo.toml"),
        ("CLIPPY.toml", "clippy.toml"),
        ("Harness/rules.md", "harness/rules.md"),
        (".GitHub/workflows/ci.yml", ".github/workflows/ci.yml"),
        (".Claude/agents/x.md", ".claude/agents/x.md"),
        ("XTASK/new.rs", "xtask/new.rs"),
    ]
    .iter()
    .enumerate()
    {
        let input = write("Write", &abs(&root, rel), &format!("d1h-{i}"), None, None);
        let message = pre_tool_use(&root, &locked(), &input).expect_err(rel);
        assert!(
            message.to_lowercase().contains(&format!("`{normal}` belongs to the harness")),
            "{rel}: {message}"
        );
        assert_eq!(pre_tool_use(&root, &unlocked, &input), Ok(()), "{rel} (unlocked)");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0030 (review D1): the fixture check compares the repo-relative path
// case-insensitively. Where the file system ignores case (macOS), the
// differently cased path is the existing fixture and is denied; where it
// does not, it is a new file and passes.
#[test]
fn d1_fixtures_ignore_case() {
    let root = repo("d1_fixture");
    let file = abs(&root, "crates/qoob3d-core/tests/FIXTURES/V1.qbd");
    let case_insensitive = Path::new(&file).exists();
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        let input = write("Write", &file, "d1f", Some("a"), Some("test-author"));
        let result = pre_tool_use(&root, &s, &input);
        if case_insensitive {
            let message = result.expect_err(&file);
            assert!(message.contains("is a format fixture and immutable (ADR-0030)"), "{message}");
        } else {
            assert_eq!(result, Ok(()), "{file}");
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review F6): shell startup names count only as a whole path
// component, so identifiers and file names that merely contain them pass in
// write commands, locked and unlocked.
#[test]
fn f6_startup_names_inside_identifiers_pass() {
    both_sessions(|unlocked| {
        for command in [
            "sed -i '' s/a/b/ crates/qoob3d-core/src/profile.rs",
            "cat > /tmp/x <<EOF\nlet n = self.profiles.len();\nEOF",
            "echo my.profile_x > /tmp/y",
            "sed -i '' s/self.profiles/self.regions/ crates/qoob3d-kernel/src/model.rs",
            "python3 -c 'print(self.profile)'",
            "perl -pi -e s/cfg.bashrc_path/cfg.rc/ /tmp/notes.txt",
            "echo x > /tmp/a.zshrc_backup",
        ] {
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{command:?} (unlocked: {unlocked})");
        }
    });
}

// ADR-0043 (review F6): a startup name after start, whitespace, quote, `/`,
// `=`, `~`, `>`, `<` and before end, whitespace, quote, `;`, `|`, `&`, `)`
// is a path; non-read commands naming it are denied, locked and unlocked.
#[test]
fn f6_startup_names_as_path_components_are_denied() {
    both_sessions(|unlocked| {
        for command in [
            "echo x >> ~/.profile",
            "echo x >> $HOME/.profile",
            "echo x >.profile",
            "echo x > \"/Users/x/.profile\"",
            "echo x > '/Users/x/.profile'",
            "dd if=/tmp/x of=.profile",
            "sed -i '' s/a/b/ .zprofile",
            "cat x > ~/.profile; ls",
            "echo x >> ~/.profile && ls",
            "(echo x >> ~/.profile)",
            "echo x >> ~/.profile| cat",
            "echo x\t>>\t~/.bashrc",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_SELF_UNLOCK],
                &format!("{command:?} (unlocked: {unlocked})"),
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Fixes from the second branch review (review3 spec R1, R2).
// ---------------------------------------------------------------------------

// ADR-0043 (review3 R1): a startup name counts as named when the characters
// directly before and after it are not file-name characters (ASCII letters,
// digits, `.`, `_`, `-`). Glob characters and brace expansion therefore end a
// name: `~/.zshrc*` and `~/{.zshenv,/dev/null}` expand to the startup file.
// Denied with the self-unlock message, locked and unlocked.
// Known limit, not tested: glob classes inside the name (`~/.zsh[r]c`).
#[test]
fn r1_startup_names_next_to_glob_or_brace_characters_are_denied() {
    both_sessions(|unlocked| {
        for command in [
            // The spec's examples.
            "echo x >> ~/.zshrc*",
            "echo x >> ~/.zshrc?",
            "echo x | tee -a ~/{.zshenv,/dev/null}",
            "echo x | tee -a ~/{.zshrc,.bashrc}",
            // Same boundaries, other names: `,` before, `}` after.
            "echo x | tee -a ~/{/dev/null,.profile}",
            "cp /tmp/x ~/.zprofile*",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_SELF_UNLOCK, ADR_0043],
                &format!("{command:?} (unlocked: {unlocked})"),
            );
        }
    });
}

// ADR-0043 (review3 R1): a file-name character directly before or after the
// startup name makes it part of another name, so these pass in write
// commands, locked and unlocked. Pure reads of startup files still pass.
#[test]
fn r1_startup_names_inside_other_file_names_pass() {
    both_sessions(|unlocked| {
        for command in [
            // The spec's examples (also in the F6 test above).
            "echo self.profiles > /tmp/y",
            "echo my.profile_x > /tmp/y",
            "python3 -c 'print(self.profile)'",
            "perl -pi -e s/cfg.bashrc_path/cfg.rc/ /tmp/notes.txt",
            "echo x > /tmp/a.zshrc_backup",
            // `.`, `-` or a digit after the name, a letter before it.
            "echo x > /tmp/.zshrc.bak",
            "echo x > /tmp/.zshrc-old",
            "echo x > /tmp/.zshenv2",
            "echo x > /tmp/my.bashrc",
            // Pure reads.
            "cat ~/.zshrc",
            "grep -n PATH ~/.zshenv ~/.zprofile",
        ] {
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{command:?} (unlocked: {unlocked})");
        }
    });
}

/// Commands that name the git directory (by name or by `git rev-parse`) and
/// write or set a cargo target dir (review3 R2, DENIED; still denied under
/// review4 S1: none is a pure read or a pure exclusion).
const GIT_DIR_WRITES: &[&str] = &[
    // The spec's examples.
    "echo hi > .git/hooks/pre-commit",
    "cp x .git/config",
    "cp /tmp/fake .git/qoob3d/guard-target/debug/xtask",
    "CARGO_TARGET_DIR=.git/x cargo build",
    "cargo build --target-dir .git/x",
    "echo x > {.git,x}/hooks/pre-push",
    "cp /tmp/fake \"$(git rev-parse --git-dir)/qoob3d/guard-target/debug/xtask\"",
    "cp x \"$(git rev-parse --git-path hooks)/pre-push\"",
    "rm -rf .git/qoob3d",
    // The other two git-dir options of R2, with a write.
    "cp x \"$(git rev-parse --absolute-git-dir)/hooks/pre-push\"",
    "cp x \"$(git rev-parse --git-common-dir)/config\"",
    // A cargo target dir given via `=`, and via a git-dir substitution.
    "cargo build --target-dir=.git/x",
    "CARGO_TARGET_DIR=$(git rev-parse --git-dir)/x cargo build",
    // Case-insensitive (macOS), and `.git` as the whole last component.
    "echo x > .GIT/hooks/pre-push",
    "rm -rf .git",
    // Write operations added by review3 R5.
    "perl -pi -e s/a/b/ .git/config",
    "dd if=/tmp/fake of=.git/qoob3d/guard-target/debug/xtask",
    "perl -i.bak -pe s/a/b/ .git/config",
    "dd of=.git/hooks/pre-push if=/tmp/x",
];

/// Commands that name the git directory and pass: pure reads (review4 S2) or
/// commands whose every `.git` is an exclusion (review4 S1 PASS, which
/// includes the review3 R2 PASS examples except running the guard binary from
/// `.git/qoob3d/…`, no longer required to pass).
const GIT_DIR_NON_WRITES: &[&str] = &[
    // Exclusions: `find … -path <x>/.git -prune`, `-name .git -prune`,
    // `--exclude=`, `--exclude-dir=`, `-E `, `!` (after quote removal).
    "find . -path ./.git -prune -o -name '*.rs' -print",
    "find . -name .git -prune -o -type f -print",
    "tar czf /tmp/x.tgz --exclude=.git .",
    "rsync -a --exclude=.git . /tmp/y",
    "grep -rn --exclude-dir=.git foo . | sort",
    "fd -E .git foo",
    "rg --glob '!.git' foo | head",
    // Pure reads, with the read-only programs added by S2.
    "du -sh .git",
    "ls -la .git",
    "cat .git/config",
    "git rev-parse --git-dir",
    "git rev-parse --show-toplevel",
    // Redirections S2 exempts.
    "ls .git 2>/dev/null",
    "find . -path ./.git -prune -o -name '*.rs' -print 2>/dev/null",
    "git rev-parse --git-dir 2>/dev/null",
    // Whitespace after `>` is exempt too (coordinator decision c).
    "ls .git 2> /dev/null",
    "du -sh .git > /dev/null",
    // The other git-dir options of review3 R2 with `git rev-parse`, a
    // read-only program since S2.
    "git rev-parse --absolute-git-dir",
    "git rev-parse --git-path hooks",
    "git rev-parse --git-common-dir",
];

// ADR-0043 (review3 R2, review4 S1): naming `.git` (whole name, any case) or
// substituting the git dir via `git rev-parse` in a command that writes or
// sets a cargo target dir is denied in any session with the `.git/`
// message. `.git` is no hook-environment fragment any more, so the
// self-unlock message must not answer. Spec root and a temp repo (absolute
// root spelled out as well).
#[test]
fn r2_git_dir_with_a_write_is_denied_with_the_git_message() {
    let root = repo("r2_git_writes");
    let root_str = root.to_str().unwrap().to_owned();
    let absolute = [
        format!("cp x {root_str}/.git/config"),
        format!("echo x > \"{root_str}/.git/hooks/pre-push\""),
    ];
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        let checks = GIT_DIR_WRITES
            .iter()
            .map(|c| ((*c).to_owned(), spec_shell(unlocked, c)))
            .chain(GIT_DIR_WRITES.iter().map(|c| ((*c).to_owned(), check_shell(&root, &s, c))))
            .chain(absolute.iter().map(|c| (c.clone(), check_shell(&root, &s, c))));
        for (command, result) in checks {
            let what = format!("{command:?} (unlocked: {unlocked})");
            assert_denied(result.clone(), &[MSG_GIT_DIR, ADR_0043], &what);
            let message = result.unwrap_err();
            assert!(!message.contains(MSG_SELF_UNLOCK), "{what}: {message}");
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review3 R2, review4 S1/S2): naming `.git` or the git-dir options
// in a pure read, or `.git` only as an exclusion, passes, locked and
// unlocked.
#[test]
fn s1_git_dir_reads_and_exclusions_pass() {
    let root = repo("r2_git_reads");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in GIT_DIR_NON_WRITES {
            let what = format!("{command:?} (unlocked: {unlocked})");
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{what}");
            assert_eq!(check_shell(&root, &s, command), Ok(()), "{what}");
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review3 R1 applied to `.git`): a file-name character directly
// before or after `.git` makes it another name, so writes to these pass; a
// cargo target dir that does not name `.git` passes too.
#[test]
fn r2_git_like_names_and_other_target_dirs_pass() {
    let root = repo("r2_git_like");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in [
            "echo target >> .gitignore",
            "echo '*.qbd binary' >> .gitattributes",
            "echo x > .git-blame-ignore-revs",
            "echo x > /tmp/my.git/x",
            "cargo build --target-dir /tmp/t",
            "CARGO_TARGET_DIR=/tmp/t cargo build",
        ] {
            let what = format!("{command:?} (unlocked: {unlocked})");
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{what}");
            assert_eq!(check_shell(&root, &s, command), Ok(()), "{what}");
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (review3 R5): `dd` as a whole word, `of=`, and `perl` with an
// in-place flag (`-i`, `-pi`, `-pie`, `-i.bak`, also after other flags) are
// write operations. With a harness path: locked denied with the harness
// message (naming the fragment), unlocked allowed (the unlock lifts harness
// paths). Spec root and a temp repo.
#[test]
fn r5_dd_and_perl_in_place_write_harness_files() {
    let root = repo("r5_harness");
    let unlocked = session(true, &unused_home());
    for (command, fragment) in [
        // The spec's examples.
        ("dd if=/tmp/x of=xtask/src/main.rs", "xtask/"),
        ("perl -pi -e s/a/b/ Cargo.toml", "Cargo.toml"),
        // Other in-place spellings and operand orders.
        ("perl -i -pe s/a/b/ Cargo.toml", "Cargo.toml"),
        ("perl -pie s/a/b/ clippy.toml", "clippy.toml"),
        ("perl -i.bak -pe s/a/b/ deny.toml", "deny.toml"),
        ("perl -0777 -pi -e s/a/b/ rustfmt.toml", "rustfmt.toml"),
        ("dd of=harness/rules.md if=/tmp/x", "harness/"),
        ("cat /tmp/x | dd of=.githooks/pre-push", ".githooks"),
    ] {
        let needle = format!("{MSG_HARNESS_SHELL}{fragment}`");
        assert_denied(spec_shell(false, command), &[&needle], command);
        assert_denied(check_shell(&root, &locked(), command), &[&needle], command);
        assert_eq!(spec_shell(true, command), Ok(()), "{command} (unlocked)");
        assert_eq!(check_shell(&root, &unlocked, command), Ok(()), "{command} (unlocked)");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (review3 R5): `dd` inside a word is no write operation, and `perl`
// without an in-place flag does not write the files it names, so these pass
// in a locked session.
#[test]
fn r5_add_commands_and_perl_without_in_place_pass() {
    let root = repo("r5_pass");
    for command in [
        // The spec's examples.
        "git add xtask/src/main.rs",
        "cargo add serde",
        // `dd` inside other words, next to harness paths.
        "git add -p Cargo.toml .github/workflows/ci.yml",
        "cargo add --path xtask serde",
        // `perl` that only reads.
        "perl -ne print Cargo.toml",
    ] {
        assert_eq!(spec_shell(false, command), Ok(()), "{command}");
        assert_eq!(check_shell(&root, &locked(), command), Ok(()), "{command}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ---------------------------------------------------------------------------
// Third branch review (review4 spec S1–S4).
// ---------------------------------------------------------------------------

/// The spec's DENIED examples of review4 S1: writers missing from the review3
/// WRITES list, cargo target dirs spelled other than `CARGO_TARGET_DIR` /
/// `--target-dir`, and `git rev-parse` git-dir queries with options in
/// between. Each names the git dir and is neither a pure read nor an
/// exclusion, so S1 denies it.
const S1_GIT_DIR_DENIED: &[&str] = &[
    "install -m 755 /tmp/fake .git/qoob3d/guard-target/debug/xtask",
    "rsync /tmp/fake .git/qoob3d/guard-target/debug/xtask",
    "ditto /tmp/fake .git/hooks/pre-push",
    "curl -sSo .git/hooks/pre-push https://example.com/x",
    "tar xf h.tar -C .git/hooks",
    "git config --file .git/config include.path /tmp/evil",
    "/usr/bin/perl -i -pe s/a/b/ .git/config",
    "CARGO_BUILD_TARGET_DIR=.git/qoob3d/guard-target cargo build -p xtask",
    "cargo build -p xtask --config build.target-dir=\".git/qoob3d/guard-target\"",
    "cp /tmp/x \"$(git rev-parse -q --git-dir)/hooks/pre-push\"",
    "cp /tmp/x \"$(git rev-parse --path-format=absolute --git-dir)/hooks/pre-push\"",
];

/// Consequences of the S1 rule that are not spec examples. Each names the
/// git dir; the comment says why it is no pure read (S2) and no exclusion.
const S1_GIT_DIR_DENIED_DERIVED: &[&str] = &[
    // The program is not read-only.
    "vim .git/config",
    "sh .git/hooks/pre-push",
    "find .git -name '*.lock' -delete",
    // A redirection other than the exempt ones (`2>/dev/null`, `>/dev/null`,
    // `&>/dev/null`, `2>&1`) remains after stripping those.
    "cat .git/config > /tmp/copy",
    "cat /tmp/fake 2>/dev/null >.git/hooks/pre-push",
    // `tee` and `xargs` are not read-only, also after a read-only segment.
    "cat /tmp/fake 2>&1 | tee .git/hooks/pre-push",
    "git rev-parse --git-dir | xargs -I{} cp /tmp/x {}/hooks/pre-push",
    // `sort` with an output file, also in a short-flag group, is no read.
    "sort -o .git/info/exclude /tmp/x",
    "sort --output=.git/info/exclude /tmp/x",
    "sort -uo .git/info/exclude /tmp/x",
    // `cargo` is no read-only program and `-v ` is no exclusion prefix (the
    // spec's former PASS example, withdrawn as a spec mistake).
    "cargo test -p xtask 2>&1 | grep -v .git",
    // A command substitution is no pure read (review3 R2 listed this as PASS).
    "ls \"$(git rev-parse --git-common-dir)\"",
    // Not every `.git` is an exclusion.
    "tar czf /tmp/x.tgz --exclude=.git .git",
    "rsync -a --exclude=.git /tmp/fake/ .git/hooks/",
    // Every `.git` is an exclusion, but the command has a git-dir query.
    "rsync -a --exclude=.git /tmp/fake/ \"$(git rev-parse --git-dir)/hooks/\"",
];

// ADR-0043 (review4 S1): a command naming the git dir — `.git` by name or a
// `git rev-parse` query with options between `rev-parse` and the git-dir
// option — is denied unless it is a pure read or names `.git` only as an
// exclusion. Locked and unlocked, spec root and a temp repo, with the `.git/`
// message and never the self-unlock message.
#[test]
fn s1_git_dir_outside_reads_and_exclusions_is_denied() {
    let root = repo("s1_git_denied");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in S1_GIT_DIR_DENIED.iter().chain(S1_GIT_DIR_DENIED_DERIVED) {
            for result in [spec_shell(unlocked, command), check_shell(&root, &s, command)] {
                let what = format!("{command:?} (unlocked: {unlocked})");
                assert_denied(result.clone(), &[MSG_GIT_DIR, ADR_0043], &what);
                let message = result.unwrap_err();
                assert!(!message.contains(MSG_SELF_UNLOCK), "{what}: {message}");
            }
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review4 S2): the read-only list also has `du`, `sort` (without
// an output file), `uniq`, `cut`, `tr`, `nl`, `basename`, `dirname`,
// `realpath`, `md5`, `shasum` and segments starting with `git rev-parse`;
// `2>/dev/null`, `>/dev/null` (also with whitespace after `>`), `&>/dev/null`
// and `2>&1` keep a read a read; they are removed before segmenting.
// So these name hook-environment files and pass, locked and unlocked.
#[test]
fn s2_more_read_only_programs_and_null_redirections_pass() {
    both_sessions(|unlocked| {
        for command in [
            // The spec's example.
            "cat ~/.zshrc 2>/dev/null",
            // The other exempt redirections, also with whitespace after `>`.
            "cat ~/.zshrc >/dev/null",
            "cat ~/.zshrc > /dev/null",
            "cat ~/.zshrc 2> /dev/null",
            // Exempt redirections are removed before splitting into
            // segments, so their `&` splits nothing.
            "grep -q PATH ~/.zshrc &>/dev/null",
            "cat ~/.zshrc 2>&1 | head",
            "grep -c PATH ~/.zshrc &>/dev/null && cat ~/.zprofile 2>&1",
            // Every added program.
            "du -h ~/.zshrc",
            "sort ~/.zshrc | uniq -c",
            "cut -c1-20 ~/.zshenv",
            "cat ~/.zshrc | tr a-z A-Z | nl",
            "basename ~/.zprofile",
            "dirname ~/.zprofile",
            "realpath ~/.zshrc",
            "md5 ~/.zshrc",
            "shasum -a 256 ~/.claude/settings.json",
            "git rev-parse --show-toplevel && cat ~/.zshrc",
        ] {
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{command:?} (unlocked: {unlocked})");
        }
    });
}

// ADR-0043 (review4 S2): `sort` with `--output` or a short-flag group
// containing `o` (`-o`, `-uo`, `-o<file>`), any other `>`, and a
// `git` segment that does not start with `git rev-parse` are no reads, so
// naming a hook-environment file with them is denied, locked and unlocked.
#[test]
fn s2_read_exception_stays_strict() {
    both_sessions(|unlocked| {
        for command in [
            "sort -o ~/.zshrc /tmp/x",
            "sort /tmp/x -o ~/.zshrc",
            "sort --output=~/.zshrc /tmp/x",
            "sort --output ~/.zshrc /tmp/x",
            "sort -uo ~/.zshrc /tmp/x",
            "sort -o~/.zshrc /tmp/x",
            "cat ~/.zshrc 2>/dev/null > /tmp/copy",
            "cat ~/.zshrc > /dev/null > /tmp/copy",
            "cat /tmp/x 2>&1 >~/.zshrc",
            "cat ~/.zshrc 2>/tmp/err",
            "git config --global x.y z; cat ~/.zshrc",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_SELF_UNLOCK, ADR_0043],
                &format!("{command:?} (unlocked: {unlocked})"),
            );
        }
    });
}

// ADR-0037 (review4 S3): `perl` is recognised by the last path component of
// the program word, and an in-place flag is a short-flag group with a
// lower-case `i`. With a harness path: locked denied with the harness
// message naming the fragment, unlocked allowed. Spec root and a temp repo.
#[test]
fn s3_perl_by_path_with_in_place_flag_writes_harness_files() {
    let root = repo("s3_perl_denied");
    let unlocked = session(true, &unused_home());
    for (command, fragment) in [
        // The spec's example.
        ("/usr/bin/perl -pi -e s/a/b/ Cargo.toml", "Cargo.toml"),
        // Another path, `-i.bak`; `-M`/`-I` groups next to a real `-i`.
        ("/opt/homebrew/bin/perl -i.bak -pe s/a/b/ deny.toml", "deny.toml"),
        ("perl -Mstrict -pi -e s/a/b/ clippy.toml", "clippy.toml"),
        ("perl -Ilib -i -pe s/a/b/ rustfmt.toml", "rustfmt.toml"),
    ] {
        let needle = format!("{MSG_HARNESS_SHELL}{fragment}`");
        assert_denied(spec_shell(false, command), &[&needle], command);
        assert_denied(check_shell(&root, &locked(), command), &[&needle], command);
        assert_eq!(spec_shell(true, command), Ok(()), "{command} (unlocked)");
        assert_eq!(check_shell(&root, &unlocked, command), Ok(()), "{command} (unlocked)");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (review4 S3): `-M<module>` and `-I<dir>` are no in-place flags,
// even when the lower-cased word contains an `i` (`-mstrict`, `-ilib`), so
// these read harness files and pass in a locked session.
#[test]
fn s3_perl_module_and_include_flags_are_not_in_place() {
    let root = repo("s3_perl_pass");
    for command in [
        // The spec's examples.
        "perl -Mstrict -ne print Cargo.toml",
        "perl -Ilib -ne print Cargo.toml",
        // By path, a module name with `i`, a directory with `i`.
        "/usr/bin/perl -Mwarnings -ne print Cargo.toml",
        "perl -I/tmp/lib -Mstrict -ne print xtask/src/main.rs",
    ] {
        assert_eq!(spec_shell(false, command), Ok(()), "{command}");
        assert_eq!(check_shell(&root, &locked(), command), Ok(()), "{command}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (review4 S4): `install`, `rsync`, `ditto`, `curl` with
// `-o`/`-O`/`--output`, `wget`, `tar` with an extract flag and
// `git config --file` are write operations. With a harness path: locked
// denied with the harness message naming the fragment, unlocked allowed.
#[test]
fn s4_more_write_programs_write_harness_files() {
    let root = repo("s4_writes");
    let unlocked = session(true, &unused_home());
    for (command, fragment) in [
        // The spec's examples.
        ("install -m 644 /tmp/x xtask/src/main.rs", "xtask/"),
        ("rsync /tmp/x Cargo.toml", "Cargo.toml"),
        ("curl -so clippy.toml https://example.com/x", "clippy.toml"),
        // The other writers and spellings of S4.
        ("ditto /tmp/x deny.toml", "deny.toml"),
        ("curl --output rustfmt.toml https://example.com/x", "rustfmt.toml"),
        ("curl -o harness/rules.md https://example.com/x", "harness/"),
        ("cd xtask/src && curl -sO https://example.com/main.rs", "xtask/"),
        ("wget -P .github/workflows https://example.com/ci.yml", ".github/"),
        ("wget -O .github/workflows/ci.yml https://example.com/x", ".github/"),
        ("tar xzf /tmp/h.tgz xtask/src/main.rs", "xtask/"),
        ("tar -xf /tmp/h.tar Cargo.toml", "Cargo.toml"),
        ("git config --file .cargo/config.toml alias.b build", ".cargo/"),
    ] {
        let needle = format!("{MSG_HARNESS_SHELL}{fragment}`");
        assert_denied(spec_shell(false, command), &[&needle], command);
        assert_denied(check_shell(&root, &locked(), command), &[&needle], command);
        assert_eq!(spec_shell(true, command), Ok(()), "{command} (unlocked)");
        assert_eq!(check_shell(&root, &unlocked, command), Ok(()), "{command} (unlocked)");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0037 (review4 S4): the new writers need their trigger: `install` as a
// whole word and without a harness path, `tar` only with an extract flag
// (`c`/`t` are no extract flags; the `x` in `--exclude` or `/tmp/x.tgz` is
// none either), `curl` only with an output option, `git config` only with
// `--file`. These pass in a locked session.
#[test]
fn s4_commands_without_a_write_trigger_pass() {
    let root = repo("s4_pass");
    for command in [
        // The spec's examples.
        "cargo install cargo-deny",
        "tar czf /tmp/x.tgz --exclude=target .",
        // Harness paths with a non-writing use of the new programs.
        "tar czf /tmp/x.tgz --exclude=target xtask/",
        "tar tzf /tmp/x.tgz xtask/src/main.rs",
        "curl -sS https://example.com/x | grep -c Cargo.toml",
        "git config --get user.name && cat Cargo.toml",
        "brew reinstall cmake && cat Cargo.toml",
    ] {
        assert_eq!(spec_shell(false, command), Ok(()), "{command}");
        assert_eq!(check_shell(&root, &locked(), command), Ok(()), "{command}");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ---------------------------------------------------------------------------
// Stricter rules accepted after review4 (coordinator decisions 1–4).
// ---------------------------------------------------------------------------

// ADR-0043 (review4, decisions 1, 2, 4): `find` is a read only without action
// primaries (`-exec*`, `-ok*`, `-delete`, `-fprint*`, `-fls`), even when it
// also prunes `.git`; `-E ` is an exclusion only in an `fd` segment; `perl`
// is recognised in any case. Denied with the `.git/` message (never the
// self-unlock message), locked and unlocked, spec root and a temp repo.
#[test]
fn s5_find_actions_foreign_e_flags_and_upper_case_perl_on_git_dir_are_denied() {
    let root = repo("s5_git_denied");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in [
            // Decision 1: action primaries, before or after the prune.
            "find . -name .git -prune -exec cp /tmp/x {}/hooks/pre-push \\;",
            "find . -exec cp /tmp/x {}/hooks/pre-push \\; -name .git -prune -o -print",
            "find .git -name '*.lock' -delete",
            // The other action primaries.
            "find . -path ./.git -prune -o -execdir rm {} +",
            "find . -path ./.git -prune -o -ok rm {} \\;",
            "find .git -fprint /tmp/list",
            "find .git -fls /tmp/list",
            // Decision 2: `-E ` outside `fd`.
            "sed -i s/a/b/ -E .git/config",
            "rsync /tmp/fake -E .git/hooks/pre-push",
            // Decision 4: `perl` in upper case.
            "PERL -pi -e s/a/b/ .git/config",
        ] {
            for result in [spec_shell(unlocked, command), check_shell(&root, &s, command)] {
                let what = format!("{command:?} (unlocked: {unlocked})");
                assert_denied(result.clone(), &[MSG_GIT_DIR, ADR_0043], &what);
                let message = result.unwrap_err();
                assert!(!message.contains(MSG_SELF_UNLOCK), "{what}: {message}");
            }
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review4, decision 1): `find` without action primaries is a pure
// read, so it may name `.git`, locked and unlocked; `fd -E .git` stays an
// exclusion (decision 2).
#[test]
fn s5_find_without_actions_and_fd_exclusions_pass() {
    let root = repo("s5_git_pass");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in [
            "find .git/hooks -type f",
            "find .git -name '*.lock' | wc -l",
            "find . -path ./.git -prune -o -type f -print",
            "fd -E .git foo",
            "fd -H -E .git -e rs main",
        ] {
            let what = format!("{command:?} (unlocked: {unlocked})");
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{what}");
            assert_eq!(check_shell(&root, &s, command), Ok(()), "{what}");
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review4, decisions 3, 4): `uniq` with a second operand writes it
// (`uniq IN OUT`), and git-style abbreviations of `--output` (`--out=`) are
// `--output`. Naming a hook-environment file this way is denied with the
// self-unlock message, locked and unlocked. One operand stays a read.
#[test]
fn s5_uniq_output_operand_and_abbreviated_sort_output_are_denied() {
    both_sessions(|unlocked| {
        for command in [
            "uniq /tmp/x ~/.zshrc",
            "uniq -c /tmp/x ~/.zshenv",
            "sort --out=~/.zshrc /tmp/x",
            "sort --outp ~/.zshrc /tmp/x",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_SELF_UNLOCK, ADR_0043],
                &format!("{command:?} (unlocked: {unlocked})"),
            );
        }
        for command in ["uniq ~/.zshrc", "uniq -c ~/.zshrc", "sort ~/.zshrc | uniq"] {
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{command:?} (unlocked: {unlocked})");
        }
    });
}

// ADR-0037 (review4, decision 4): the long and short spellings `curl
// --remote-name`, `tar --extract` and `git config -f` are write operations.
// With a harness path: locked denied with the harness message naming the
// fragment, unlocked allowed. Spec root and a temp repo.
#[test]
fn s5_long_and_short_writer_options_write_harness_files() {
    let root = repo("s5_writers");
    let unlocked = session(true, &unused_home());
    for (command, fragment) in [
        ("cd xtask/src && curl --remote-name https://example.com/main.rs", "xtask/"),
        ("tar --extract -f /tmp/h.tar Cargo.toml", "Cargo.toml"),
        ("tar --extract --file=/tmp/h.tar harness/rules.md", "harness/"),
        ("git config -f .cargo/config.toml alias.b build", ".cargo/"),
    ] {
        let needle = format!("{MSG_HARNESS_SHELL}{fragment}`");
        assert_denied(spec_shell(false, command), &[&needle], command);
        assert_denied(check_shell(&root, &locked(), command), &[&needle], command);
        assert_eq!(spec_shell(true, command), Ok(()), "{command} (unlocked)");
        assert_eq!(check_shell(&root, &unlocked, command), Ok(()), "{command} (unlocked)");
    }
    fs::remove_dir_all(&root).unwrap();
}

// ---------------------------------------------------------------------------
// Fifth branch review (review5 A1–A3; A3 is in HARNESS_WRITES and HARMLESS).
// ---------------------------------------------------------------------------

/// `rg` with a preprocessor option runs the given program on every searched
/// file (`--pre CMD`; `--pre-glob GLOB` selects the files for it), so it is no
/// pure read (review5 A1). All four spellings of the spec.
const RG_PRE_ON_GIT_DIR: &[&str] = &[
    // The spec's examples.
    "rg --pre=rm x .git/hooks/pre-push",
    "rg --pre rm x .git/config",
    // `--pre-glob` in both spellings, alone and with `--pre`.
    "rg --pre-glob=* x .git/config",
    "rg --pre-glob '*' --pre=sh x .git/hooks/pre-push",
    // After another option and behind a read-only segment.
    "rg -n --pre=/tmp/w.sh x .git/config",
    "cat /tmp/x | rg --pre sh x .git/config",
];

/// The same with shell startup files (review5 A1).
const RG_PRE_ON_STARTUP_FILES: &[&str] = &[
    // The spec's examples.
    "rg --pre=/tmp/w.sh x ~/.zshrc",
    "rg --pre-glob '*' --pre=sh x ~/.zprofile",
    // The other spellings.
    "rg --pre sh x ~/.zshenv",
    "rg --pre-glob=*.gz x ~/.bashrc",
    "rg --pre-glob *.gz x ~/.profile",
    "rg -n x ~/.zshrc; rg --pre=sh x ~/.zshrc",
];

// ADR-0043 (review5 A1): `rg --pre`/`--pre-glob` runs a program, so naming
// `.git` with it is denied with the `.git/` message (never the self-unlock
// message), locked and unlocked, spec root and a temp repo.
#[test]
fn a1_rg_with_a_preprocessor_on_git_dir_is_denied() {
    let root = repo("a1_rg_git");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in RG_PRE_ON_GIT_DIR {
            for result in [spec_shell(unlocked, command), check_shell(&root, &s, command)] {
                let what = format!("{command:?} (unlocked: {unlocked})");
                assert_denied(result.clone(), &[MSG_GIT_DIR, ADR_0043], &what);
                let message = result.unwrap_err();
                assert!(!message.contains(MSG_SELF_UNLOCK), "{what}: {message}");
            }
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review5 A1): `rg --pre`/`--pre-glob` naming a shell startup file
// is denied with the self-unlock message, locked and unlocked.
#[test]
fn a1_rg_with_a_preprocessor_on_startup_files_is_denied() {
    both_sessions(|unlocked| {
        for command in RG_PRE_ON_STARTUP_FILES {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_SELF_UNLOCK, ADR_0043],
                &format!("{command:?} (unlocked: {unlocked})"),
            );
        }
    });
}

// ADR-0043 (review5 A1): `rg` without a preprocessor stays a pure read: a
// pattern `pre`, the different flag `--pretty` and other options do not
// count, on startup files and on `.git`, locked and unlocked.
#[test]
fn a1_rg_without_a_preprocessor_stays_a_read() {
    let root = repo("a1_rg_read");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in [
            // The spec's examples.
            "rg -n foo ~/.zshrc",
            "rg -n pre ~/.zshrc",
            "rg --pretty x ~/.zshrc",
            // The same on `.git`, and `pre` inside other words.
            "rg -n url .git/config",
            "rg --pretty x .git/config",
            "rg -n pre-push .git/config",
            "rg -n prefix ~/.zprofile | head",
        ] {
            let what = format!("{command:?} (unlocked: {unlocked})");
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{what}");
            assert_eq!(check_shell(&root, &s, command), Ok(()), "{what}");
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review5 A2): GNU `sort --compress-program=PROG` runs PROG on its
// temporary files, so it is no pure read. GNU accepts unambiguous
// abbreviations of long options; the only other `sort` option starting with
// `--c` is `--check`, so every prefix from `--co` on means
// `--compress-program`. With a startup file: the self-unlock message; with
// `.git`: the `.git/` message. Locked and unlocked.
#[test]
fn a2_sort_with_a_compress_program_is_denied() {
    let root = repo("a2_sort_git");
    both_sessions(|unlocked| {
        for command in [
            // The spec's example.
            "sort --compress-program=sh ~/.zshrc",
            // Separate argument and abbreviations.
            "sort --compress-program sh ~/.zshenv",
            "sort --compress-prog=sh ~/.zshrc",
            "sort --compress=sh ~/.zprofile",
            "sort -u --compress-prog sh ~/.bashrc",
            "sort --co=sh ~/.profile",
            "cat /tmp/x | sort --compress-program=sh ~/.zshrc",
        ] {
            assert_denied(
                spec_shell(unlocked, command),
                &[MSG_SELF_UNLOCK, ADR_0043],
                &format!("{command:?} (unlocked: {unlocked})"),
            );
        }
        let s = session(unlocked, &unused_home());
        for command in [
            // The spec's example.
            "sort --compress-prog sh .git/config",
            "sort --compress-program=sh .git/info/exclude",
            "sort --compress=/tmp/w.sh .git/config",
        ] {
            for result in [spec_shell(unlocked, command), check_shell(&root, &s, command)] {
                let what = format!("{command:?} (unlocked: {unlocked})");
                assert_denied(result.clone(), &[MSG_GIT_DIR, ADR_0043], &what);
                let message = result.unwrap_err();
                assert!(!message.contains(MSG_SELF_UNLOCK), "{what}: {message}");
            }
        }
    });
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0043 (review5 A2): `sort` without an output file and without a
// compress program stays a pure read; `--check` is no abbreviation of
// `--compress-program`. Locked and unlocked, startup files and `.git`.
#[test]
fn a2_sort_without_a_compress_program_stays_a_read() {
    let root = repo("a2_sort_read");
    both_sessions(|unlocked| {
        let s = session(unlocked, &unused_home());
        for command in [
            // The spec's examples.
            "sort -u ~/.zshrc",
            "sort --check ~/.zshrc",
            // `-c` and `--check=quiet` are `--check` as well.
            "sort -c ~/.zprofile",
            "sort --check=quiet ~/.zshenv",
            "sort -u .git/info/exclude",
        ] {
            let what = format!("{command:?} (unlocked: {unlocked})");
            assert_eq!(spec_shell(unlocked, command), Ok(()), "{what}");
            assert_eq!(check_shell(&root, &s, command), Ok(()), "{what}");
        }
    });
    fs::remove_dir_all(&root).unwrap();
}
