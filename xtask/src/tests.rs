use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

/// The variables a git hook exports; they point git at another repository,
/// work tree or index than the one at `root`.
const HOOK_GIT_ENV: [&str; 3] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"];

/// Set for the child process of
/// `git_means_the_repository_at_root_even_with_hook_git_variables`: the
/// repository the child's `git` call must mean.
const CHILD_ROOT: &str = "QOOB3D_XTASK_TEST_GIT_CHILD_ROOT";

/// A fresh git repository with one staged file `a.txt`; git runs without
/// global/system config and without any inherited hook variables.
fn repo_with_staged_file(test: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("qoob3d-xtask-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
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
    root
}

// One git helper: `git_command(root, args)` is `git` with exactly `args`, run
// in `root`, with GIT_DIR, GIT_WORK_TREE and GIT_INDEX_FILE removed from its
// environment (hooks export them; xtask always means the repository at
// `root`). A removed variable shows up in `get_envs` as `(key, None)`; no
// other variable is touched.
#[test]
fn git_command_removes_the_hook_git_environment() {
    let root = Path::new("/tmp/qoob3d-some-repo");
    let command = git_command(root, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
    assert_eq!(command.get_program(), "git");
    let args: Vec<&OsStr> = command.get_args().collect();
    assert_eq!(args, ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
    assert_eq!(command.get_current_dir(), Some(root));
    let mut envs: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
    envs.sort();
    let mut expected: Vec<(&OsStr, Option<&OsStr>)> =
        HOOK_GIT_ENV.iter().map(|key| (OsStr::new(key), None)).collect();
    expected.sort();
    assert_eq!(envs, expected);
}

// One git helper: without arguments there are none, and the root is taken as
// given (relative stays relative).
#[test]
fn git_command_takes_args_and_root_as_given() {
    let command = git_command(Path::new("some/dir"), &[]);
    assert_eq!(command.get_args().count(), 0);
    assert_eq!(command.get_current_dir(), Some(Path::new("some/dir")));
    assert_eq!(command.get_program(), "git");
}

// One git helper: `git` returns the untrimmed stdout of a successful call and
// an error naming the command for a failing one.
#[test]
fn git_returns_untrimmed_output_and_errors() {
    let root = repo_with_staged_file("git_output");
    assert_eq!(git(&root, &["ls-files"]), Ok("a.txt\n".to_owned()));
    let error = git(&root, &["rev-parse", "--verify", "no-such-rev"]).unwrap_err();
    assert!(error.starts_with("git rev-parse --verify no-such-rev"), "{error}");
    fs::remove_dir_all(&root).unwrap();
}

// One git helper: `git` means the repository at `root` even when the process
// inherited the variables a hook exports, pointing elsewhere. The test runs
// itself as a child with those variables set (the process environment cannot
// be changed safely in-process); the child checks that `git ls-files` in
// `root` lists the staged `a.txt` (a foreign GIT_INDEX_FILE would list
// nothing, a foreign GIT_DIR fails) and `--show-toplevel` is `root` (a foreign
// GIT_WORK_TREE would be printed instead).
#[test]
fn git_means_the_repository_at_root_even_with_hook_git_variables() {
    let root = repo_with_staged_file("git_hook_env");
    let elsewhere = root.join("elsewhere");
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::git_child_reads_the_repository_at_root", "--nocapture"])
        .env(CHILD_ROOT, &root)
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
    fs::remove_dir_all(&root).unwrap();
}

/// The child half of the test above; does nothing unless started by it.
#[test]
fn git_child_reads_the_repository_at_root() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else { return };
    let root = PathBuf::from(root);
    assert_eq!(git(&root, &["ls-files"]), Ok("a.txt\n".to_owned()));
    let top = git(&root, &["rev-parse", "--show-toplevel"]).unwrap();
    assert_eq!(fs::canonicalize(top.trim()).unwrap(), fs::canonicalize(&root).unwrap());
}

// One git helper: worktree.rs has no git helper of its own and builds no git
// command itself; it calls `crate::git`.
#[test]
fn worktree_uses_the_shared_git_helper() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/worktree.rs")).unwrap();
    assert!(!source.contains("fn git("), "worktree.rs defines its own `git`");
    assert!(!source.contains("Command::new(\"git\")"), "worktree.rs builds its own git command");
    assert!(
        source.contains("crate::git") || source.contains("use crate::{git"),
        "worktree.rs does not use `crate::git`"
    );
}
