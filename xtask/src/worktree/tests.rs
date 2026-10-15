//! ADR-0048: the pre-push hook checks the pushed commit in a temporary
//! detached worktree, not the working tree.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

const FILE: &str = "file.txt";

/// `git` in `dir` with a fixed identity, ignoring any git environment of the
/// caller (a hook sets `GIT_DIR` and friends) and the user's signing setup.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=worktree-test", "-c", "user.email=worktree-test@localhost"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A throwaway repository in the temp dir with `FILE` committed as each of
/// `versions` in turn; returns the root and the commit ids in order.
fn throwaway_repo(test: &str, versions: &[&str]) -> (PathBuf, Vec<String>) {
    let root = std::env::temp_dir().join(format!("qoob3d-worktree-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    let mut commits = Vec::new();
    for version in versions {
        fs::write(root.join(FILE), version).unwrap();
        git(&root, &["add", FILE]);
        git(&root, &["commit", "-q", "--no-verify", "-m", version]);
        commits.push(git(&root, &["rev-parse", "HEAD"]));
    }
    (root, commits)
}

/// The paths `git worktree list` reports for the repository at `root`.
fn listed_worktrees(root: &Path) -> Vec<String> {
    git(root, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(str::to_owned)
        .collect()
}

/// After `with_worktree`: the temporary worktree directory is gone and git
/// lists only the main worktree.
fn assert_removed(root: &Path, worktree: &Path) {
    assert!(!worktree.exists(), "worktree {} still exists", worktree.display());
    let listed = listed_worktrees(root);
    assert_eq!(listed.len(), 1, "git still lists a temporary worktree: {listed:?}");
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap()
}

// ADR-0048: `run` sees the committed state of `rev`, not the dirty working
// tree, in a detached worktree in the system temp dir outside the repository;
// its value is returned, and the worktree is removed afterwards. The working
// tree itself is left alone.
#[test]
fn with_worktree_checks_out_the_committed_state_not_the_working_tree() {
    let (root, commits) = throwaway_repo("committed", &["committed"]);
    fs::write(root.join(FILE), "dirty").unwrap();
    let mut seen = None;
    let result = with_worktree(&root, &commits[0], |path| {
        seen = Some(path.to_path_buf());
        let head = git(path, &["rev-parse", "HEAD"]);
        let branch = git(path, &["rev-parse", "--abbrev-ref", "HEAD"]);
        (fs::read_to_string(path.join(FILE)).unwrap(), head, branch, canonical(path))
    });
    let (content, head, branch, inside) = result.unwrap();
    let worktree = seen.unwrap();
    assert_eq!(content, "committed", "run must see the commit, not the working tree");
    assert_eq!(head, commits[0], "the worktree must be at the given rev");
    assert_eq!(branch, "HEAD", "the worktree must be detached (no branch)");
    assert!(!inside.starts_with(canonical(&root)), "{} is inside the repo", inside.display());
    assert!(
        inside.starts_with(canonical(&std::env::temp_dir())),
        "{} is not in the system temp dir",
        inside.display()
    );
    assert_removed(&root, &worktree);
    assert_eq!(fs::read_to_string(root.join(FILE)).unwrap(), "dirty", "working tree changed");
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0048: the worktree is of the given rev, also when it is not HEAD (the
// hook passes the pushed `local_sha`, which need not be checked out).
#[test]
fn with_worktree_checks_out_an_older_rev() {
    let (root, commits) = throwaway_repo("older", &["first", "second"]);
    let content =
        with_worktree(&root, &commits[0], |path| fs::read_to_string(path.join(FILE)).unwrap());
    assert_eq!(content, Ok("first".to_owned()));
    assert_eq!(listed_worktrees(&root).len(), 1);
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0048: a panic in `run` still removes the worktree. Whether the panic is
// propagated or turned into `Err` is left open; it must not look like success.
#[test]
fn with_worktree_removes_the_worktree_when_run_panics() {
    let (root, _) = throwaway_repo("panics", &["committed"]);
    let seen = RefCell::new(None);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        with_worktree(&root, "HEAD", |path| -> u8 {
            *seen.borrow_mut() = Some(path.to_path_buf());
            panic!("run fails inside the worktree");
        })
    }));
    assert!(!matches!(outcome, Ok(Ok(_))), "a panicking run must not report success");
    let worktree = seen.into_inner().expect("run was called");
    assert_removed(&root, &worktree);
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0048: an unknown rev is an error that names it; `run` is not called and
// no worktree is left behind.
#[test]
fn with_worktree_rejects_an_unknown_rev_and_leaves_nothing_behind() {
    let (root, _) = throwaway_repo("unknown", &["committed"]);
    let rev = "no-such-rev-4711";
    let mut called = false;
    let result = with_worktree(&root, rev, |_| called = true);
    let message = result.expect_err("an unknown rev must fail");
    assert!(message.contains(rev), "{message:?} does not name {rev:?}");
    assert!(!called, "run must not be called for an unknown rev");
    assert_eq!(listed_worktrees(&root).len(), 1, "a worktree was left behind");
    fs::remove_dir_all(&root).unwrap();
}

// ADR-0048: the shared cargo target dir: unset or empty `CARGO_TARGET_DIR`
// → `<root>/target`, absolute as is, relative to the root.
#[test]
fn target_dir_follows_cargo_target_dir() {
    let root = Path::new("/work/qoob3d");
    assert_eq!(target_dir(root, None), PathBuf::from("/work/qoob3d/target"));
    assert_eq!(target_dir(root, Some("")), PathBuf::from("/work/qoob3d/target"));
    assert_eq!(target_dir(root, Some("/shared/target")), PathBuf::from("/shared/target"));
    assert_eq!(target_dir(root, Some("build/t")), PathBuf::from("/work/qoob3d/build/t"));
}

// ADR-0048 (Decision, second bullet): all worktree checks share one persistent
// directory `check-rev/` inside the checkout's cargo target dir, never the
// target dir itself. Expected paths by hand: `target_dir` of the same cases
// (see `target_dir_follows_cargo_target_dir`) plus `/check-rev`. The checkout's
// own target dir must differ, otherwise the worktree build replaces the
// checkout's artifacts (e.g. `target/debug/xtask`).
#[test]
fn check_rev_target_dir_is_check_rev_inside_the_target_dir_never_the_target_dir() {
    let root = Path::new("/work/qoob3d");
    let cases: [(Option<&str>, &str); 4] = [
        (None, "/work/qoob3d/target/check-rev"),
        (Some(""), "/work/qoob3d/target/check-rev"),
        (Some("/shared/target"), "/shared/target/check-rev"),
        (Some("build/t"), "/work/qoob3d/build/t/check-rev"),
    ];
    for (cargo_target_dir, expected) in cases {
        let dir = check_rev_target_dir(root, cargo_target_dir);
        assert_eq!(dir, PathBuf::from(expected), "CARGO_TARGET_DIR = {cargo_target_dir:?}");
        let own = target_dir(root, cargo_target_dir);
        assert_ne!(dir, own, "check-rev must not use the checkout's target dir {}", own.display());
        assert_eq!(dir.parent(), Some(own.as_path()), "check-rev must be directly inside {own:?}");
    }
}

// ADR-0048: the check runs `cargo xtask check` in the worktree with the shared
// target dir, so OCCT is not rebuilt.
#[test]
fn check_command_runs_the_check_in_the_worktree_with_the_shared_target_dir() {
    let worktree = Path::new("/tmp/qoob3d-wt");
    let target = Path::new("/work/qoob3d/target");
    let command = check_command(worktree, target);
    let program = Path::new(command.get_program());
    assert_eq!(program.file_name(), Some(OsStr::new("cargo")), "program {program:?}");
    let args: Vec<&OsStr> = command.get_args().collect();
    assert_eq!(args, [OsStr::new("xtask"), OsStr::new("check")]);
    assert_eq!(command.get_current_dir(), Some(worktree));
    let target_env = command.get_envs().find(|(key, _)| *key == OsStr::new("CARGO_TARGET_DIR"));
    assert_eq!(target_env, Some((OsStr::new("CARGO_TARGET_DIR"), Some(target.as_os_str()))));
}

// ADR-0048: the hook checks the tip of each pushed ref (`$local_sha`) with
// `check-rev` inside the loop over the pushed refs and never runs
// `cargo xtask check` in the working tree (comment lines do not run).
#[test]
fn pre_push_checks_the_tip_of_each_pushed_ref_not_the_working_tree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let hook = fs::read_to_string(root.join(".githooks/pre-push")).unwrap();
    let call = "cargo xtask check-rev \"$local_sha\"";
    let lines: Vec<&str> = hook.lines().map(str::trim).collect();
    let start = lines.iter().position(|l| l.starts_with("while read")).expect("loop start");
    let end = start + lines[start..].iter().position(|l| *l == "done").expect("loop end");
    assert!(
        lines[start..end].iter().any(|l| l.starts_with(call)),
        "pre-push must run `{call}` inside the loop over the pushed refs"
    );
    for line in lines.iter().filter(|l| !l.starts_with('#')) {
        let bare = line
            .match_indices("cargo xtask check")
            .any(|(i, m)| !line[i + m.len()..].starts_with("-rev"));
        assert!(!bare, "pre-push runs `cargo xtask check` in the working tree: {line}");
    }
}

/// Why the hooks must strip the quarantine; part of every failure message.
const QUARANTINE_REASON: &str = "ADR-0051: GUI git clients such as Sourcetree quarantine the \
     xtask binary a hook builds, and Gatekeeper refuses to start it";

/// The lines of a shell script that run, trimmed (comment and empty lines dropped).
fn shell_code(script: &str) -> Vec<&str> {
    script.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
}

// ADR-0051: each hook that runs xtask sources `lib.sh` before its first
// `cargo xtask` line; `lib.sh` builds xtask and, on macOS only, removes
// `com.apple.quarantine` from the built binary, after the build.
#[test]
fn hooks_build_xtask_and_strip_the_macos_quarantine_before_running_it() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for hook in [".githooks/commit-msg", ".githooks/pre-push"] {
        let text = fs::read_to_string(root.join(hook)).unwrap();
        let code = shell_code(&text);
        let source = code
            .iter()
            .position(|l| {
                (l.starts_with("source ") || l.starts_with(". ")) && l.contains("/lib.sh")
            })
            .unwrap_or_else(|| panic!("{hook} does not source lib.sh ({QUARANTINE_REASON})"));
        let first_xtask = code
            .iter()
            .position(|l| l.contains("cargo xtask"))
            .unwrap_or_else(|| panic!("{hook} has no `cargo xtask` line"));
        assert!(
            source < first_xtask,
            "{hook} must source lib.sh before its first `cargo xtask` line ({QUARANTINE_REASON})"
        );
    }

    let lib = fs::read_to_string(root.join(".githooks/lib.sh")).unwrap();
    let code = shell_code(&lib);
    let build = code
        .iter()
        .position(|l| l.starts_with("cargo build") && l.contains("--package xtask"))
        .unwrap_or_else(|| {
            panic!(
                "lib.sh must build xtask with `cargo build --package xtask` ({QUARANTINE_REASON})"
            )
        });
    let binary = "\"${CARGO_TARGET_DIR:-target}/debug/xtask\"";
    let strip = code
        .iter()
        .position(|l| l.starts_with("xattr -d com.apple.quarantine") && l.contains(binary))
        .unwrap_or_else(|| {
            panic!(
                "lib.sh must remove com.apple.quarantine from {binary} with `xattr -d` \
                 ({QUARANTINE_REASON})"
            )
        });
    assert!(
        build < strip,
        "lib.sh must strip the quarantine after the build ({QUARANTINE_REASON})"
    );
    // The `xattr` call sits in an `if` on `uname` = Darwin that is still open:
    // no `fi` between the guard and the call.
    let guard = code[..strip]
        .iter()
        .rposition(|l| l.starts_with("if ") && l.contains("$(uname)") && l.contains("Darwin"))
        .unwrap_or_else(|| {
            panic!("lib.sh must guard the xattr call with a Darwin check ({QUARANTINE_REASON})")
        });
    assert!(
        !code[guard + 1..strip].contains(&"fi"),
        "the xattr call in lib.sh must sit inside the Darwin check ({QUARANTINE_REASON})"
    );
}

// ADR-0051: before the check, `check-rev` builds the xtask of the worktree
// with `cargo build --quiet --package xtask`, in the worktree, into the shared
// target dir (the one the check uses, so the check reuses this build), and
// without the hook's git environment (as `check_command`, ADR-0048).
#[test]
fn build_xtask_command_builds_xtask_in_the_worktree_with_the_shared_target_dir() {
    let worktree = Path::new("/tmp/qoob3d-wt");
    let target = Path::new("/work/qoob3d/target/check-rev");
    let command = build_xtask_command(worktree, target);
    let program = Path::new(command.get_program());
    assert_eq!(program.file_name(), Some(OsStr::new("cargo")), "program {program:?}");
    let args: Vec<&OsStr> = command.get_args().collect();
    assert_eq!(args, ["build", "--quiet", "--package", "xtask"].map(OsStr::new));
    assert_eq!(command.get_current_dir(), Some(worktree));
    let envs: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
    assert!(
        envs.contains(&(OsStr::new("CARGO_TARGET_DIR"), Some(target.as_os_str()))),
        "CARGO_TARGET_DIR must be {}: {envs:?}",
        target.display()
    );
    // The variables a git hook sets (see `crate::GIT_ENV`), written out here so
    // that shrinking that list does not shrink this test.
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
        assert!(envs.contains(&(OsStr::new(key), None)), "{key} is not removed: {envs:?}");
    }
}

// ADR-0051: the binary whose quarantine is stripped is the one cargo builds
// with the dev profile: `<target_dir>/debug/xtask` (cargo's directory layout).
#[test]
fn xtask_binary_is_debug_xtask_in_the_target_dir() {
    assert_eq!(
        xtask_binary(Path::new("/work/qoob3d/target/check-rev")),
        PathBuf::from("/work/qoob3d/target/check-rev/debug/xtask")
    );
    assert_eq!(xtask_binary(Path::new("rel/t")), PathBuf::from("rel/t/debug/xtask"));
}

/// The source of `fn {name}` in `text`: from its signature to the first line
/// that is a bare `}` (end of a top-level item), without comment lines.
fn function_source(text: &str, name: &str) -> String {
    let signature = format!("fn {name}(");
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.contains(&signature) && !l.trim_start().starts_with("//"))
        .unwrap_or_else(|| panic!("no `{signature}` found"));
    let end = start
        + lines[start..].iter().position(|l| *l == "}").unwrap_or_else(|| panic!("{name}: no end"));
    let code: Vec<&str> =
        lines[start..=end].iter().filter(|l| !l.trim_start().starts_with("//")).copied().collect();
    code.join("\n")
}

/// The item that the attribute starting at byte `at` of `code` applies to: up
/// to its `;` or the `}` that closes its first block, at nesting depth zero.
fn attributed_item(code: &str, at: usize) -> &str {
    let after = code[at..].find(']').map_or(code.len(), |i| at + i + 1);
    let mut depth = 0_i32;
    for (i, c) in code[after..].char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' => depth -= 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &code[after..=after + i];
                }
            }
            ';' if depth == 0 => return &code[after..=after + i],
            _ => {}
        }
    }
    &code[after..]
}

// ADR-0051: `check_rev` builds the worktree's xtask with
// `worktree::build_xtask(` (which strips the quarantine) before it builds or
// starts the check command; otherwise `cargo xtask check` in the worktree
// starts a quarantined binary that Gatekeeper refuses. `build_xtask` runs
// `build_xtask_command(` and then, on macOS only, removes
// `com.apple.quarantine` from `xtask_binary(…)`.
#[test]
fn check_rev_builds_xtask_and_strips_the_macos_quarantine_before_the_check() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let main = fs::read_to_string(src.join("main.rs")).unwrap();
    let check_rev = function_source(&main, "check_rev");
    let build = check_rev.find("worktree::build_xtask(").unwrap_or_else(|| {
        panic!("check_rev must call `worktree::build_xtask(` ({QUARANTINE_REASON})")
    });
    let check = check_rev
        .find("worktree::check_command(")
        .expect("check_rev must run `worktree::check_command(`");
    assert!(
        build < check,
        "check_rev must build xtask before the check command ({QUARANTINE_REASON})"
    );
    for start in [".status()", ".spawn()", ".output()"] {
        if let Some(at) = check_rev.find(start) {
            assert!(
                build < at,
                "check_rev runs `{start}` before `worktree::build_xtask(` ({QUARANTINE_REASON})"
            );
        }
    }

    let own = fs::read_to_string(src.join("worktree.rs")).unwrap();
    let build_xtask = function_source(&own, "build_xtask");
    let run = build_xtask.find("build_xtask_command(").unwrap_or_else(|| {
        panic!("build_xtask must build with `build_xtask_command(` ({QUARANTINE_REASON})")
    });
    let cfg = build_xtask.find("#[cfg(target_os = \"macos\")]").unwrap_or_else(|| {
        panic!("build_xtask must strip the quarantine only on macOS ({QUARANTINE_REASON})")
    });
    assert!(
        run < cfg,
        "build_xtask must strip the quarantine after the build ({QUARANTINE_REASON})"
    );
    let strip = attributed_item(&build_xtask, cfg);
    assert!(
        strip.contains("com.apple.quarantine") && strip.contains("xtask_binary("),
        "the macOS-only part of build_xtask must remove com.apple.quarantine from \
         xtask_binary(…) ({QUARANTINE_REASON}): {strip}"
    );
}
