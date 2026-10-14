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
            // `>` anywhere, also with a read-only program and to another file.
            "cat ~/.zshrc > /tmp/copy",
            "grep PATH ~/.zshrc 2>/dev/null",
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

// ADR-0043 (addendum T1 + F1): `.git/config` and `.git/hooks` are
// hook-environment fragments in shell commands: pure reads pass, anything
// else is denied, locked and unlocked.
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
            assert_denied(check_shell(&root, &s, command), &[MSG_SELF_UNLOCK], &what);
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
