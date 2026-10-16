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

// ---------------------------------------------------------------------------
// ADR-0052 fix round 1 F2.2: the hook passes the review gate the whole
// branch (`<local_sha> --not refs/remotes/<remote>/main`) when that ref
// exists, otherwise the range it used before; commit policy and secret scan
// keep the pushed range. The hook runs for real with `cargo` and `gitleaks`
// replaced by exported bash functions that log their arguments (a function
// wins over the hook's PATH, so no toolchain or network is involved).
// ---------------------------------------------------------------------------

const ZERO: &str = "0000000000000000000000000000000000000000";

/// Runs the real pre-push hook in `repo` for a push to `remote` with one
/// stdin line `refs/heads/feat <local> refs/heads/feat <remote_sha>`;
/// returns the logged `cargo …` and `gitleaks …` calls.
fn run_pre_push(repo: &Path, remote: &str, local: &str, remote_sha: &str) -> Vec<String> {
    pre_push(repo, remote, local, remote_sha).0
}

/// [`run_pre_push`], also returning the hook's stderr.
fn pre_push(repo: &Path, remote: &str, local: &str, remote_sha: &str) -> (Vec<String>, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let hook = root.join(".githooks/pre-push");
    let log = repo.join("stub.log");
    let stubs = r#"cargo() { printf 'cargo %s\n' "$*" >> "$STUB_LOG"; }
gitleaks() { printf 'gitleaks %s\n' "$*" >> "$STUB_LOG"; }
export -f cargo gitleaks
exec bash "$0" "$@""#;
    let mut child = Command::new("bash")
        .args(["-c", stubs])
        .arg(&hook)
        .args([remote, "git@example.com:x.git"])
        .current_dir(repo)
        .env("STUB_LOG", &log)
        .env_remove("QOOB3D_SKIP_REVIEW")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let line = format!("refs/heads/feat {local} refs/heads/feat {remote_sha}\n");
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), line.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "pre-push failed: {stderr}");
    let calls = fs::read_to_string(&log).unwrap_or_default().lines().map(str::to_owned).collect();
    (calls, stderr)
}

/// The commits the rev-list arguments after `prefix` in the logged call
/// select, as a set; `prefix` is e.g. `cargo xtask review <tip>`.
fn selected(repo: &Path, calls: &[String], prefix: &str) -> std::collections::BTreeSet<String> {
    let call = calls
        .iter()
        .find_map(|c| c.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no call `{prefix} …` in {calls:?}"));
    let mut args = vec!["rev-list"];
    args.extend(call.split_whitespace());
    git(repo, &args).lines().map(str::to_owned).collect()
}

fn set(commits: &[&String]) -> std::collections::BTreeSet<String> {
    commits.iter().map(|c| (*c).clone()).collect()
}

/// main = m0, branch feat = b1 (already pushed) → b2 (new).
fn pushed_branch(test: &str) -> (PathBuf, String, String, String) {
    let (repo, commits) = throwaway_repo(test, &["m0", "b1", "b2"]);
    let [m0, b1, b2] = <[String; 3]>::try_from(commits).unwrap();
    (repo, m0, b1, b2)
}

// F2.2: pushing b2 onto the remote's b1 with `origin/main` = m0 reviews the
// whole branch {b1, b2}; the commit policy and gitleaks keep b1..b2.
#[test]
fn pre_push_reviews_the_whole_branch_against_the_remote_main() {
    let (repo, m0, b1, b2) = pushed_branch("pre-push-branch");
    git(&repo, &["update-ref", "refs/remotes/origin/main", &m0]);
    git(&repo, &["update-ref", "refs/remotes/origin/feat", &b1]);
    let calls = run_pre_push(&repo, "origin", &b2, &b1);
    let review = format!("cargo xtask review {b2} ");
    assert_eq!(selected(&repo, &calls, &review), set(&[&b1, &b2]), "{calls:?}");
    assert_eq!(selected(&repo, &calls, "cargo xtask commits "), set(&[&b2]), "{calls:?}");
    assert!(
        calls.iter().any(|c| c.starts_with("gitleaks ") && c.contains(&format!("{b1}..{b2}"))),
        "{calls:?}"
    );
    fs::remove_dir_all(&repo).unwrap();
}

// F2.2: a new branch (remote sha zero) is reviewed against `<remote>/main`
// too, even if another remote ref already holds b1 (the old range
// `--not --remotes=origin` would review only b2); the commit policy keeps
// the old range.
#[test]
fn pre_push_reviews_a_new_branch_against_the_remote_main() {
    let (repo, m0, b1, b2) = pushed_branch("pre-push-new-branch");
    git(&repo, &["update-ref", "refs/remotes/origin/main", &m0]);
    git(&repo, &["update-ref", "refs/remotes/origin/backup", &b1]);
    let calls = run_pre_push(&repo, "origin", &b2, ZERO);
    let review = format!("cargo xtask review {b2} ");
    assert_eq!(selected(&repo, &calls, &review), set(&[&b1, &b2]), "{calls:?}");
    assert_eq!(selected(&repo, &calls, "cargo xtask commits "), set(&[&b2]), "{calls:?}");
    fs::remove_dir_all(&repo).unwrap();
}

// F2.2: the main ref of the pushed-to remote counts (`upstream`, not a fixed
// `origin`); without `<remote>/main` the hook falls back to the old range.
#[test]
fn pre_push_uses_the_main_of_the_pushed_remote() {
    let (repo, m0, b1, b2) = pushed_branch("pre-push-remote");
    git(&repo, &["update-ref", "refs/remotes/origin/main", &m0]);
    git(&repo, &["update-ref", "refs/remotes/upstream/feat", &b1]);
    let review = format!("cargo xtask review {b2} ");
    let calls = run_pre_push(&repo, "upstream", &b2, &b1);
    assert_eq!(selected(&repo, &calls, &review), set(&[&b2]), "fallback: {calls:?}");

    git(&repo, &["update-ref", "refs/remotes/upstream/main", &m0]);
    fs::remove_file(repo.join("stub.log")).unwrap();
    let calls = run_pre_push(&repo, "upstream", &b2, &b1);
    assert_eq!(selected(&repo, &calls, &review), set(&[&b1, &b2]), "upstream/main: {calls:?}");
    fs::remove_dir_all(&repo).unwrap();
}

// ADR-0052 fix round 2 (fallback visible): without `<remote>/main` the hook
// warns on stderr naming the missing ref; with it, there is no such warning.
#[test]
fn pre_push_warns_when_it_falls_back() {
    let (repo, m0, b1, b2) = pushed_branch("pre-push-warning");
    git(&repo, &["update-ref", "refs/remotes/upstream/feat", &b1]);
    let (_, stderr) = pre_push(&repo, "upstream", &b2, &b1);
    assert!(stderr.contains("refs/remotes/upstream/main"), "no fallback warning: {stderr}");

    git(&repo, &["update-ref", "refs/remotes/upstream/main", &m0]);
    fs::remove_file(repo.join("stub.log")).unwrap();
    let (_, stderr) = pre_push(&repo, "upstream", &b2, &b1);
    assert!(!stderr.contains("refs/remotes/upstream/main"), "warning without fallback: {stderr}");
    fs::remove_dir_all(&repo).unwrap();
}
