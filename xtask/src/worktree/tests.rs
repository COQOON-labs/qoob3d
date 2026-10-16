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

/// The text of `rel` in the repository.
fn repo_file(rel: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// ADR-0048: the hook checks the tip of each pushed ref (`$local_sha`) with
// `check-rev` inside the loop over the pushed refs and never runs the plain
// `check` in the working tree (comment lines do not run). ADR-0051: the hook
// runs the xtask binary through the variable `lib.sh` assigns its path to,
// not through `cargo xtask`.
#[test]
fn pre_push_checks_the_tip_of_each_pushed_ref_not_the_working_tree() {
    let hook = repo_file(".githooks/pre-push");
    let lib = repo_file(".githooks/lib.sh");
    let (_, var) = binary_assignment(&shell_code(&lib))
        .expect("lib.sh must assign the xtask binary path to a variable");
    let runs = [format!("\"${var}\""), format!("\"${{{var}}}\""), "cargo xtask".to_owned()];
    let wanted = runs.each_ref().map(|run| format!("{run} check-rev \"$local_sha\""));
    let lines: Vec<&str> = hook.lines().map(str::trim).collect();
    let start = lines.iter().position(|l| l.starts_with("while read")).expect("loop start");
    let end = start + lines[start..].iter().position(|l| *l == "done").expect("loop end");
    // Only the forms through the variable count; `cargo xtask` re-links (ADR-0051).
    assert!(
        lines[start..end].iter().any(|l| wanted[..2].iter().any(|c| l.starts_with(c.as_str()))),
        "pre-push must run `{}` inside the loop over the pushed refs",
        wanted[0]
    );
    for line in lines.iter().filter(|l| !l.starts_with('#')) {
        let bare = runs.iter().any(|run| {
            line.match_indices(&format!("{run} check"))
                .any(|(i, m)| !line[i + m.len()..].starts_with("-rev"))
        });
        assert!(!bare, "pre-push runs the plain `check` in the working tree: {line}");
    }
}

// ---------------------------------------------------------------------------
// ADR-0052 fix round 1 F2.2: the hook passes the review gate the whole
// branch (`<local_sha> --not refs/remotes/<remote>/main`) when that ref
// exists, otherwise the range it used before; commit policy and secret scan
// keep the pushed range. The hook runs for real, with nothing built and no
// network: `cargo` and `gitleaks` are exported bash functions that log their
// arguments (a function wins over the hook's PATH), and the xtask binary the
// hook runs (ADR-0051: `"$xtask_binary"` = `$CARGO_TARGET_DIR/debug/xtask`
// from lib.sh) is a script in a scratch `CARGO_TARGET_DIR` that logs
// `xtask <args>`.
// ---------------------------------------------------------------------------

const ZERO: &str = "0000000000000000000000000000000000000000";

/// Runs the real pre-push hook in `repo` for a push to `remote` with one
/// stdin line `refs/heads/feat <local> refs/heads/feat <remote_sha>`;
/// returns the logged `cargo …`, `gitleaks …` and `xtask …` calls.
fn run_pre_push(repo: &Path, remote: &str, local: &str, remote_sha: &str) -> Vec<String> {
    pre_push(repo, remote, local, remote_sha).0
}

/// [`run_pre_push`], also returning the hook's stderr. Panics unless the hook
/// succeeds and reaches its last step (`xtask check-rev <local>`), so a hook
/// that stops early (e.g. under a quarantining client, ADR-0051) fails the
/// test instead of passing it vacuously.
fn pre_push(repo: &Path, remote: &str, local: &str, remote_sha: &str) -> (Vec<String>, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let hook = root.join(".githooks/pre-push");
    let log = repo.join("stub.log");
    // The xtask binary lib.sh points the hook to; it must exist, otherwise
    // `build_xtask` reports it missing and the hook cannot run it.
    let target = repo.join("stub-target");
    fs::create_dir_all(target.join("debug")).unwrap();
    write_stub(&target.join("debug"), "xtask", "printf 'xtask %s\\n' \"$*\" >> \"$STUB_LOG\"\n");
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
        .env("CARGO_TARGET_DIR", &target)
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
    assert!(
        out.status.success(),
        "pre-push failed ({}); if it reports a quarantining client, this test runs under a \
         client that quarantines new files and cannot check the review range (ADR-0051): \
         {stderr}",
        out.status
    );
    let calls: Vec<String> =
        fs::read_to_string(&log).unwrap_or_default().lines().map(str::to_owned).collect();
    let last = format!("xtask check-rev {local}");
    assert!(
        calls.contains(&last),
        "pre-push did not reach `{last}` through the xtask binary: {calls:?}\n{stderr}"
    );
    (calls, stderr)
}

/// The commits the rev-list arguments after `prefix` in the logged call
/// select, as a set; `prefix` is e.g. `xtask review <tip>`.
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
    let review = format!("xtask review {b2} ");
    assert_eq!(selected(&repo, &calls, &review), set(&[&b1, &b2]), "{calls:?}");
    assert_eq!(selected(&repo, &calls, "xtask commits "), set(&[&b2]), "{calls:?}");
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
    let review = format!("xtask review {b2} ");
    assert_eq!(selected(&repo, &calls, &review), set(&[&b1, &b2]), "{calls:?}");
    assert_eq!(selected(&repo, &calls, "xtask commits "), set(&[&b2]), "{calls:?}");
    fs::remove_dir_all(&repo).unwrap();
}

// F2.2: the main ref of the pushed-to remote counts (`upstream`, not a fixed
// `origin`); without `<remote>/main` the hook falls back to the old range.
#[test]
fn pre_push_uses_the_main_of_the_pushed_remote() {
    let (repo, m0, b1, b2) = pushed_branch("pre-push-remote");
    git(&repo, &["update-ref", "refs/remotes/origin/main", &m0]);
    git(&repo, &["update-ref", "refs/remotes/upstream/feat", &b1]);
    let review = format!("xtask review {b2} ");
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

/// Why the hooks must strip the quarantine; part of every failure message.
const QUARANTINE_REASON: &str = "ADR-0051: GUI git clients such as Sourcetree quarantine the \
     xtask binary a hook builds, and Gatekeeper refuses to start it";

/// The lines of a shell script that run, trimmed (comment and empty lines dropped).
fn shell_code(script: &str) -> Vec<&str> {
    script.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
}

/// The name assigned on a shell line of the form `[local |readonly |export ]NAME=…`.
fn assigned_name(line: &str) -> Option<&str> {
    let line = ["local ", "readonly ", "export "]
        .iter()
        .find_map(|p| line.strip_prefix(p))
        .unwrap_or(line);
    let (name, _) = line.split_once('=')?;
    let valid = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    valid.then_some(name)
}

/// The positions of the `else` (if any) and the `fi` of the shell `if` that
/// starts at `code[start]`, counting nested `if … fi` blocks.
fn if_branches(code: &[&str], start: usize) -> (Option<usize>, Option<usize>) {
    let mut depth = 0_usize;
    let mut otherwise = None;
    for (i, line) in code.iter().enumerate().skip(start + 1) {
        if line.starts_with("if ") {
            depth += 1;
        } else if *line == "fi" || line.starts_with("fi ") || line.starts_with("fi;") {
            if depth == 0 {
                return (otherwise, Some(i));
            }
            depth -= 1;
        } else if depth == 0 && (*line == "else" || line.starts_with("else ")) {
            otherwise = Some(i);
        }
    }
    (otherwise, None)
}

/// The name a shell line defines a function with: `name() {`, `name () {`,
/// `function name {` or `function name() {`.
fn defined_function(line: &str) -> Option<&str> {
    let (keyword, rest) = match line.strip_prefix("function ") {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    let rest = rest.strip_suffix('{')?.trim_end();
    let name = match rest.strip_suffix("()") {
        Some(name) => name.trim_end(),
        None if keyword => rest,
        None => return None,
    };
    let valid = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then_some(name)
}

/// The shell functions defined in `code` (see `shell_code`): name, line of the
/// definition and line of the closing `}` (the first line that is only `}`).
fn shell_functions<'a>(code: &[&'a str]) -> Vec<(&'a str, usize, usize)> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < code.len() {
        let Some(name) = defined_function(code[i]) else {
            i += 1;
            continue;
        };
        let end = i + code[i..]
            .iter()
            .position(|l| *l == "}")
            .unwrap_or_else(|| panic!("shell function {name} has no closing `}}` line"));
        found.push((name, i, end));
        i = end + 1;
    }
    found
}

/// The function whose body holds line `at` of the code, `None` at the top level.
fn enclosing_function<'a>(functions: &[(&'a str, usize, usize)], at: usize) -> Option<&'a str> {
    functions.iter().find(|(_, start, end)| *start < at && at < *end).map(|(name, ..)| *name)
}

/// Whether a shell line names `word` as a whole word (a call of a function).
fn calls(line: &str, word: &str) -> bool {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| w == word)
}

/// Whether a shell line sources `lib.sh`.
fn sources_lib_sh(line: &str) -> bool {
    (line.starts_with("source ") || line.starts_with(". ")) && line.contains("/lib.sh")
}

/// Whether a shell line uses the variable `var` quoted: `"$var"` or `"${var}"`.
fn uses_var(line: &str, var: &str) -> bool {
    line.contains(&format!("\"${var}\"")) || line.contains(&format!("\"${{{var}}}\""))
}

/// The xtask binary as cargo builds it, relative to the repository root.
const BINARY: &str = "\"${CARGO_TARGET_DIR:-target}/debug/xtask\"";

/// The line and name of the variable that `lib.sh` (as `shell_code`) assigns
/// the xtask binary path to.
fn binary_assignment<'a>(lib: &[&'a str]) -> Option<(usize, &'a str)> {
    lib.iter().enumerate().find_map(|(i, l)| {
        let name = assigned_name(l)?;
        (l.split_once('=')?.1 == BINARY).then_some((i, name))
    })
}

/// The line of `lib.sh` (as `shell_code`) that builds xtask.
fn xtask_build_line(lib: &[&str]) -> Option<usize> {
    lib.iter().position(|l| l.starts_with("cargo build") && l.contains("--package xtask"))
}

/// The first line of a hook (as `shell_code`) that builds something: a `cargo`
/// call, a call of the `lib.sh` function that builds xtask, or the `source` of
/// `lib.sh` if `lib.sh` builds at its top level.
fn first_build(hook: &[&str], lib: &[&str]) -> Option<usize> {
    let functions = shell_functions(lib);
    let build_function = xtask_build_line(lib).and_then(|at| enclosing_function(&functions, at));
    hook.iter().position(|l| {
        let builds_on_source = build_function.is_none() && sources_lib_sh(l);
        builds_on_source
            || build_function.is_some_and(|f| calls(l, f))
            || l.split_whitespace().any(|w| w == "cargo")
    })
}

/// The first line of a hook (as `shell_code`) that runs xtask: through `"$var"`
/// or through `cargo xtask`.
fn first_run(hook: &[&str], var: &str) -> Option<usize> {
    hook.iter().position(|l| uses_var(l, var) || l.contains("cargo xtask"))
}

/// Why the hooks must not run `cargo xtask`; part of the failure messages.
const RELINK: &str = "ADR-0051: `cargo xtask` (= `cargo run`) re-links target/debug/xtask on \
     every run, even without recompiling, and a client such as Sourcetree quarantines the new \
     link again, so the strip in lib.sh is undone; run the binary through the variable lib.sh \
     assigns it to";

// ADR-0051 (finding 2026-10-15, Sourcetree push): each hook sources `lib.sh`,
// has `lib.sh` build xtask (which strips the quarantine, see the
// `build_xtask_on_*` tests), and only then runs the binary directly through
// the variable `lib.sh` assigns its path to; no `cargo xtask` line is left,
// because `cargo run` re-links the binary and re-quarantines it.
// commit-msg: `exec "$var" commits --message "$1"` (the former
// `exec cargo xtask commits --message "$1"`); pre-push runs `commits`,
// `review` and `check-rev` the same way.
#[test]
fn hooks_build_xtask_before_running_the_binary_directly() {
    let lib_text = repo_file(".githooks/lib.sh");
    let lib = shell_code(&lib_text);
    let (assignment, var) = binary_assignment(&lib).unwrap_or_else(|| {
        panic!("lib.sh must assign the binary path {BINARY} to a variable ({RELINK})")
    });
    assert!(
        !lib[assignment].starts_with("local "),
        "lib.sh must assign {var} globally, the hooks run the binary through it"
    );
    for hook in [".githooks/commit-msg", ".githooks/pre-push"] {
        let text = repo_file(hook);
        let code = shell_code(&text);
        if let Some(line) = code.iter().find(|l| l.contains("cargo xtask")) {
            panic!("{hook} still runs `cargo xtask` ({RELINK}): {line}");
        }
        let source = code
            .iter()
            .position(|l| sources_lib_sh(l))
            .unwrap_or_else(|| panic!("{hook} does not source lib.sh ({QUARANTINE_REASON})"));
        let run = first_run(&code, var).unwrap_or_else(|| {
            panic!("{hook} must run the xtask binary through \"${var}\" ({RELINK})")
        });
        let build = first_build(&code, &lib)
            .unwrap_or_else(|| panic!("{hook} never has lib.sh build xtask ({QUARANTINE_REASON})"));
        assert!(source < run, "{hook} must source lib.sh before it runs \"${var}\"");
        assert!(
            build < run,
            "{hook} must have lib.sh build xtask (and strip its quarantine) before it runs \
             \"${var}\" ({QUARANTINE_REASON})"
        );
    }
    let commit_msg = repo_file(".githooks/commit-msg");
    let exec = [var.to_owned(), format!("{{{var}}}")]
        .map(|v| format!("exec \"${v}\" commits --message \"$1\""));
    assert!(
        shell_code(&commit_msg).iter().any(|l| exec.iter().any(|e| e == l)),
        "commit-msg must run `{}` ({RELINK})",
        exec[0]
    );
    let pre_push = repo_file(".githooks/pre-push");
    for step in ["commits", "review", "check-rev"] {
        let starts = [format!("\"${var}\" {step} "), format!("\"${{{var}}}\" {step} ")];
        assert!(
            shell_code(&pre_push).iter().any(|l| starts.iter().any(|s| l.starts_with(s.as_str()))),
            "pre-push must run `{}…` ({RELINK})",
            starts[0]
        );
    }
}

/// Writes an executable shell script `name` with `body` into `dir`.
fn write_stub(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&path, permissions).unwrap();
}

/// A fresh, empty scratch directory for `test` in the system temp dir.
fn scratch_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qoob3d-hooks-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The definition of the `lib.sh` function (in `lib`, as `shell_code`) whose
/// body satisfies `body`: its name and its source.
fn lib_function(lib: &[&str], body: impl Fn(&str) -> bool) -> Option<(String, String)> {
    shell_functions(lib)
        .into_iter()
        .find(|(_, start, end)| lib[start + 1..*end].iter().any(|l| body(l)))
        .map(|(name, start, end)| (name.to_owned(), lib[start..=end].join("\n")))
}

/// What one run of the `lib.sh` build function did.
struct BuildRun {
    /// The `cargo …` and `xattr …` calls of the stubs, in order.
    calls: Vec<String>,
    stderr: String,
    status: std::process::ExitStatus,
    /// Where cargo puts the xtask binary: `$CARGO_TARGET_DIR/debug/xtask`.
    binary: PathBuf,
}

/// Runs the `lib.sh` function that calls `cargo` (the build of xtask) alone
/// under `bash -c` with `set -euo pipefail`, after the top-level variable
/// assignments of lib.sh except `PATH` (so the stubs win). Nothing real is
/// built or changed: stubs on PATH stand in for `cargo` (logs its arguments
/// and, if `creates_binary`, creates `$CARGO_TARGET_DIR/debug/xtask` as cargo
/// does), `uname` (prints `system`) and `xattr` (logs its arguments).
fn run_build_xtask(test: &str, system: &str, creates_binary: bool) -> BuildRun {
    let lib_text = repo_file(".githooks/lib.sh");
    let lib = shell_code(&lib_text);
    let (name, definition) = lib_function(&lib, |l| calls(l, "cargo"))
        .unwrap_or_else(|| panic!("lib.sh has no function that builds xtask with cargo"));
    let functions = shell_functions(&lib);
    let assignments: Vec<&str> = lib
        .iter()
        .enumerate()
        .filter(|(i, l)| {
            enclosing_function(&functions, *i).is_none()
                && assigned_name(l).is_some_and(|n| n != "PATH")
        })
        .map(|(_, l)| *l)
        .collect();

    let dir = scratch_dir(test);
    let stubs = dir.join("bin");
    let work = dir.join("work");
    fs::create_dir_all(&stubs).unwrap();
    fs::create_dir_all(&work).unwrap();
    let target = dir.join("target");
    let log = dir.join("calls.log");
    write_stub(
        &stubs,
        "cargo",
        "printf 'cargo %s\\n' \"$*\" >> \"$STUB_LOG\"\n\
         if [ \"$STUB_BINARY\" = yes ]; then\n\
           mkdir -p \"$CARGO_TARGET_DIR/debug\" && : > \"$CARGO_TARGET_DIR/debug/xtask\"\n\
         fi\n",
    );
    write_stub(&stubs, "uname", "printf '%s\\n' \"$STUB_UNAME\"\n");
    write_stub(&stubs, "xattr", "printf 'xattr %s\\n' \"$*\" >> \"$STUB_LOG\"\n");

    let script = format!("set -euo pipefail\n{}\n{definition}\n{name}\n", assignments.join("\n"));
    let path = format!("{}:{}", stubs.display(), std::env::var("PATH").unwrap_or_default());
    let out = Command::new("bash")
        .args(["-c", &script])
        .current_dir(&work)
        .env("PATH", path)
        .env("CARGO_TARGET_DIR", &target)
        .env("STUB_LOG", &log)
        .env("STUB_UNAME", system)
        .env("STUB_BINARY", if creates_binary { "yes" } else { "no" })
        .output()
        .unwrap();
    let calls = fs::read_to_string(&log).unwrap_or_default().lines().map(str::to_owned).collect();
    fs::remove_dir_all(&dir).unwrap();
    BuildRun {
        calls,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        status: out.status,
        binary: target.join("debug").join("xtask"),
    }
}

/// The position of the `cargo build` of the xtask package among `calls`
/// (`--package xtask`, `--package=xtask` or `-p xtask`).
fn xtask_build_call(calls: &[String]) -> Option<usize> {
    calls.iter().position(|call| {
        let words: Vec<&str> = call.split_whitespace().collect();
        words.get(..2) == Some(&["cargo", "build"])
            && (words.contains(&"--package=xtask")
                || words.windows(2).any(|w| matches!(w, ["--package" | "-p", "xtask"])))
    })
}

/// Whether a logged call removes com.apple.quarantine from `binary`.
fn strips(call: &str, binary: &Path) -> bool {
    let words: Vec<&str> = call.split_whitespace().collect();
    words.first() == Some(&"xattr")
        && words.contains(&"-d")
        && words.contains(&"com.apple.quarantine")
        && words.last().map(Path::new) == Some(binary)
}

// ADR-0051: on macOS the build function of lib.sh builds the xtask package
// and then removes com.apple.quarantine from the built binary
// (`xattr -d com.apple.quarantine <$CARGO_TARGET_DIR/debug/xtask>`), after
// the build that (re-)links it, and succeeds.
#[test]
fn build_xtask_on_darwin_strips_the_quarantine_after_the_build() {
    let run = run_build_xtask("darwin", "Darwin", true);
    let calls = &run.calls;
    assert!(run.status.success(), "build_xtask failed: {} {}", run.status, run.stderr);
    let build = xtask_build_call(calls)
        .unwrap_or_else(|| panic!("build_xtask must run `cargo build --package xtask`: {calls:?}"));
    let strip = calls.iter().position(|c| strips(c, &run.binary)).unwrap_or_else(|| {
        panic!(
            "on Darwin, build_xtask must remove com.apple.quarantine from {} with `xattr -d` \
             ({QUARANTINE_REASON}): {calls:?}",
            run.binary.display()
        )
    });
    assert!(build < strip, "build_xtask must strip the quarantine after the build: {calls:?}");
}

// ADR-0051: elsewhere there is no quarantine attribute; the build function
// builds xtask and never runs xattr (a stub would log it even on Linux).
#[test]
fn build_xtask_on_linux_never_runs_xattr() {
    let run = run_build_xtask("linux", "Linux", true);
    let calls = &run.calls;
    assert!(run.status.success(), "build_xtask failed: {} {}", run.status, run.stderr);
    assert!(xtask_build_call(calls).is_some(), "build_xtask must build xtask: {calls:?}");
    assert!(
        !calls.iter().any(|c| c.starts_with("xattr")),
        "build_xtask must run xattr only on Darwin: {calls:?}"
    );
}

// ADR-0051 (critic finding): if the build leaves no binary on macOS, the
// build function says so on stderr instead of calling xattr blindly and
// discarding its failure.
#[test]
fn build_xtask_on_darwin_reports_a_missing_binary_on_stderr() {
    let run = run_build_xtask("missing", "Darwin", false);
    let calls = &run.calls;
    assert!(
        !calls.iter().any(|c| c.starts_with("xattr")),
        "build_xtask must not call xattr on a missing binary: {calls:?}"
    );
    assert!(
        !run.stderr.trim().is_empty(),
        "build_xtask writes nothing to stderr when {} is missing ({QUARANTINE_REASON})",
        run.binary.display()
    );
}

// ADR-0037, ADR-0051: each hook sets `set -euo pipefail` itself as its first
// executed line (after shebang and comments), before it sources `lib.sh`. If
// errexit came only from `lib.sh`, a failing `source` (missing or broken file)
// would leave the hook running without errexit, and a failing check would no
// longer stop the commit or push: the wall would fail open.
#[test]
fn hooks_set_errexit_themselves_before_sourcing_lib_sh() {
    const REASON: &str = "a failing `source` of lib.sh must stop the hook; errexit set only \
         inside lib.sh is lost when sourcing fails, and the wall must not fail open";
    for hook in [".githooks/commit-msg", ".githooks/pre-push"] {
        let text = repo_file(hook);
        let code = shell_code(&text);
        assert_eq!(
            code.first().copied(),
            Some("set -euo pipefail"),
            "{hook}: the first executed line must be `set -euo pipefail` ({REASON})"
        );
        let source = code
            .iter()
            .position(|l| sources_lib_sh(l))
            .unwrap_or_else(|| panic!("{hook} does not source lib.sh ({REASON})"));
        assert!(
            source > 0,
            "{hook} must set `set -euo pipefail` before sourcing lib.sh ({REASON})"
        );
    }
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

// ADR-0051 (finding 2026-10-15, Sourcetree push): `check-rev` does not try to
// strip the quarantine. It cannot help: the check itself re-links xtask and
// builds and immediately runs test binaries and build scripts, none of which
// can be cleared in time; pre-push now stops under a quarantining client
// before check-rev runs. Neither `check_rev` nor the worktree module may call
// `xattr`, name `com.apple.quarantine` or build xtask separately again (dead
// protection that suggests a safety it does not give). Comment lines do not
// count.
#[test]
fn check_rev_does_not_strip_the_quarantine() {
    let main = repo_file("xtask/src/main.rs");
    let check_rev = function_source(&main, "check_rev");
    let worktree = repo_file("xtask/src/worktree.rs");
    let worktree_code: Vec<&str> =
        worktree.lines().filter(|l| !l.trim_start().starts_with("//")).collect();
    let worktree_code = worktree_code.join("\n");
    for (place, code) in
        [("check_rev in main.rs", check_rev.as_str()), ("worktree.rs", &worktree_code)]
    {
        for dead in ["xattr", "com.apple.quarantine", "build_xtask"] {
            assert!(
                !code.contains(dead),
                "{place} still contains `{dead}`: check-rev cannot clear the quarantine of the \
                 binaries its check builds and runs; pre-push stops under a quarantining client \
                 instead (ADR-0051)"
            );
        }
    }
}

/// The panic message of `check`, or `None` if it does not panic.
fn panic_text(check: impl FnOnce()) -> Option<String> {
    let payload = catch_unwind(AssertUnwindSafe(check)).err()?;
    Some(
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default(),
    )
}

/// The part of a `lib.sh` that builds nothing when sourced: the setup, the
/// binary path and a quarantine check (the expected shape after ADR-0051's
/// Sourcetree push finding), for the oracle self-checks of the pre-push
/// matcher.
const LIB_FUNCTIONS: &str = "set -euo pipefail\n\
    export PATH=\"/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH\"\n\
    cd \"$(git rev-parse --show-toplevel)\"\n\
    xtask_binary=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
    quarantining_client() {\n\
      [ \"$(uname)\" = Darwin ] || return 1\n\
      local probe\n\
      probe=\"$(mktemp)\"\n\
      if xattr -p com.apple.quarantine \"$probe\" >/dev/null 2>&1; then\n\
        rm -f \"$probe\"\n\
        return 0\n\
      fi\n\
      rm -f \"$probe\"\n\
      return 1\n\
    }\n";

/// A `build_xtask` function that builds xtask and strips its quarantine.
const BUILD_FUNCTION: &str = "build_xtask() {\n\
      cargo build --quiet --package xtask\n\
      if [ \"$(uname)\" = Darwin ]; then\n\
        if [ -f \"$xtask_binary\" ]; then\n\
          xattr -d com.apple.quarantine \"$xtask_binary\" 2>/dev/null || true\n\
        else\n\
          echo \"hooks: $xtask_binary not found, quarantine not removed\" >&2\n\
        fi\n\
      fi\n\
    }\n";

/// Why pre-push must stop under a quarantining client; part of the failure messages.
const STOP_REASON: &str = "ADR-0051 (Sourcetree push, 2026-10-15): cargo re-links \
     target/debug/xtask on every build, and the check of check-rev builds and at once runs \
     test binaries and build scripts; a client that quarantines new files gets them all \
     refused by Gatekeeper, so pre-push must stop first, with a message on stderr";

/// Whether `text` names an ADR: `ADR-` followed by four digits.
fn names_an_adr(text: &str) -> bool {
    text.match_indices("ADR-").any(|(i, m)| {
        let digits = &text[i + m.len()..];
        digits.len() >= 4 && digits[..4].chars().all(|c| c.is_ascii_digit())
    })
}

/// Whether a shell line exits with a literal non-zero status (`exit 1`).
fn exits_non_zero(line: &str) -> bool {
    line.strip_prefix("exit ")
        .map(|status| status.trim_end_matches(';').trim())
        .and_then(|status| status.parse::<u8>().ok())
        .is_some_and(|status| status != 0)
}

/// Panics unless the pre-push hook `hook` calls the quarantine check of `lib`
/// in an `if` before it builds or runs anything, and the branch for a
/// quarantining client writes a message to stderr that names the cause (the
/// quarantine), the way out (a terminal) and an ADR, and exits non-zero.
fn assert_pre_push_stops_under_quarantine(hook: &str, lib: &str) {
    let lib = shell_code(lib);
    // The quarantine check: the function that creates a probe with `mktemp`.
    let (check, _) = lib_function(&lib, |l| calls(l, "mktemp")).unwrap_or_else(|| {
        panic!(
            "lib.sh must define a shell function that creates a probe file with `mktemp` and \
             tests it for com.apple.quarantine, the quarantine check ({STOP_REASON})"
        )
    });
    let hook = shell_code(hook);
    let at = hook.iter().position(|l| calls(l, &check)).unwrap_or_else(|| {
        panic!("pre-push does not call the quarantine check `{check}` ({STOP_REASON})")
    });
    let line = hook[at];
    assert!(line.starts_with("if "), "pre-push must call `{check}` in an `if`: {line}");
    let negated = line["if ".len()..].trim_start().starts_with('!');
    let (otherwise, end) = if_branches(&hook, at);
    let end = end.unwrap_or_else(|| panic!("pre-push: the quarantine check has no `fi`"));
    let branch = if negated {
        let otherwise = otherwise.unwrap_or_else(|| {
            panic!("pre-push: `if ! {check}` has no `else` branch for a quarantining client")
        });
        &hook[otherwise + 1..end]
    } else {
        &hook[at + 1..otherwise.unwrap_or(end)]
    };
    let text = branch.join("\n");
    assert!(
        branch.iter().any(|l| l.contains(">&2")),
        "pre-push must write its message for a quarantining client to stderr (`>&2`) \
         ({STOP_REASON}): {branch:?}"
    );
    let lower = text.to_lowercase();
    for (word, what) in [("quarantin", "the cause (the quarantine)"), ("terminal", "the way out")] {
        assert!(lower.contains(word), "the pre-push message must name {what}: {branch:?}");
    }
    assert!(names_an_adr(&text), "the pre-push message must name the ADR: {branch:?}");
    assert!(
        branch.iter().any(|l| exits_non_zero(l)),
        "pre-push must exit non-zero (`exit 1`) under a quarantining client ({STOP_REASON}): \
         {branch:?}"
    );
    if let Some(build) = first_build(&hook, &lib) {
        assert!(
            at < build,
            "pre-push must run the quarantine check before it builds anything; a lib.sh that \
             builds when sourced must move the build into a function ({STOP_REASON}): `{}`",
            hook[build]
        );
    }
    let var = binary_assignment(&lib).map_or("xtask_binary", |(_, var)| var);
    if let Some(run) = first_run(&hook, var) {
        assert!(
            at < run,
            "pre-push must run the quarantine check before it runs xtask ({STOP_REASON}): `{}`",
            hook[run]
        );
    }
}

// ADR-0051, user decision (option a) after the Sourcetree push of 2026-10-15:
// commits and merges keep working from a quarantining client, a push from it
// stops before anything is built or run, with a message on stderr that names
// the cause and the way out (push from a terminal), and a non-zero exit.
#[test]
fn pre_push_stops_under_a_quarantining_client_before_building_anything() {
    let hook = repo_file(".githooks/pre-push");
    let lib = repo_file(".githooks/lib.sh");
    assert_pre_push_stops_under_quarantine(&hook, &lib);
}

/// A pre-push hook in the expected shape: check, build, run.
const PRE_PUSH: &str = "set -euo pipefail\n\
    source \"$(dirname \"$0\")/lib.sh\"\n\
    if quarantining_client; then\n\
      echo \"pre-push: this client quarantines new files (com.apple.quarantine), Gatekeeper \
    would refuse the check; push from a terminal (ADR-0051)\" >&2\n\
      exit 1\n\
    fi\n\
    build_xtask\n\
    while read -r local_ref local_sha remote_ref remote_sha; do\n\
      \"$xtask_binary\" commits \"$local_sha\"\n\
      \"$xtask_binary\" check-rev \"$local_sha\"\n\
    done\n";

// Oracle self-check for `assert_pre_push_stops_under_quarantine`: the shapes
// the fix may take pass.
#[test]
fn pre_push_quarantine_oracle_accepts_a_stop_before_the_build() {
    let lib = format!("{LIB_FUNCTIONS}{BUILD_FUNCTION}");
    let negated = PRE_PUSH.replace(
        "if quarantining_client; then\n\
           echo \"pre-push: this client quarantines new files (com.apple.quarantine), Gatekeeper \
         would refuse the check; push from a terminal (ADR-0051)\" >&2\n\
           exit 1\n",
        "if ! quarantining_client; then\n\
           true\n\
         else\n\
           echo \"pre-push: Sourcetree quarantines every new file;\" >&2\n\
           echo \"pre-push: push from a terminal instead (ADR-0051)\" >&2\n\
           exit 2\n",
    );
    assert_ne!(negated, PRE_PUSH, "the replacement must match");
    for hook in [PRE_PUSH.to_owned(), negated] {
        assert_eq!(
            panic_text(|| assert_pre_push_stops_under_quarantine(&hook, &lib)),
            None,
            "{hook}"
        );
    }
}

// Oracle self-check for `assert_pre_push_stops_under_quarantine`: each
// regression fails with the message that names it.
#[test]
fn pre_push_quarantine_oracle_rejects_each_regression() {
    let lib = format!("{LIB_FUNCTIONS}{BUILD_FUNCTION}");
    // Each pattern must occur in PRE_PUSH; the string continuations there drop
    // the indentation, so the patterns have none either.
    let hook = |from: &str, to: &str| {
        let changed = PRE_PUSH.replace(from, to);
        assert_ne!(changed, PRE_PUSH, "{from:?} not found in PRE_PUSH");
        changed
    };
    let rejected = [
        // the sabotage `pre-push-ignores-quarantine`: the call is gone
        (hook("if quarantining_client;", "if false;"), "does not call the quarantine check"),
        // not in an `if`
        (
            hook("if quarantining_client; then", "quarantining_client && exit 1\nif true; then"),
            "in an `if`",
        ),
        // the check after the build
        (
            hook(
                "source \"$(dirname \"$0\")/lib.sh\"\n",
                "source \"$(dirname \"$0\")/lib.sh\"\nbuild_xtask\n",
            ),
            "before it builds",
        ),
        // a direct cargo call before the check
        (
            hook(
                "source \"$(dirname \"$0\")/lib.sh\"\n",
                "source \"$(dirname \"$0\")/lib.sh\"\ncargo build -p xtask\n",
            ),
            "before it builds",
        ),
        // xtask runs before the check
        (
            hook(
                "source \"$(dirname \"$0\")/lib.sh\"\n",
                "source \"$(dirname \"$0\")/lib.sh\"\n\"$xtask_binary\" commits HEAD\n",
            ),
            "before it runs xtask",
        ),
        // the message goes to stdout
        (hook("(ADR-0051)\" >&2", "(ADR-0051)\""), "stderr"),
        // no way out
        (hook("push from a terminal", "push failed"), "the way out"),
        // no cause
        (
            hook("this client quarantines new files (com.apple.quarantine)", "the push cannot run"),
            "the cause",
        ),
        // no ADR
        (hook("(ADR-0051)", "(see docs)"), "the ADR"),
        // the push goes on
        (hook("exit 1\n", "true\n"), "exit non-zero"),
        (hook("exit 1\n", "exit 0\n"), "exit non-zero"),
        (hook("exit 1\n", "exit\n"), "exit non-zero"),
    ];
    for (hook, expected) in rejected {
        let message = panic_text(|| assert_pre_push_stops_under_quarantine(&hook, &lib))
            .unwrap_or_else(|| panic!("accepted, but must fail with {expected:?}:\n{hook}"));
        assert!(message.contains(expected), "expected {expected:?}, got {message:?}\n{hook}");
    }
    // lib.sh builds when sourced: the check cannot come first.
    let builds_on_source = format!("{LIB_FUNCTIONS}cargo build --quiet --package xtask\n");
    let message =
        panic_text(|| assert_pre_push_stops_under_quarantine(PRE_PUSH, &builds_on_source))
            .expect("a lib.sh that builds when sourced must be rejected");
    assert!(message.contains("before it builds"), "{message}");
    // lib.sh without a quarantine check.
    let message = panic_text(|| assert_pre_push_stops_under_quarantine(PRE_PUSH, BUILD_FUNCTION))
        .expect("a lib.sh without a quarantine check must be rejected");
    assert!(message.contains("must define a shell function"), "{message}");
}

/// What the quarantine check of `lib.sh` reports when its probe file is
/// created by a `mktemp` stub that runs `treat` on the new file: the last
/// line of stdout (`quarantining` or `clean`) and the probe files it created.
/// The check runs alone (no `source`, so nothing is built), with errexit and
/// nounset as in lib.sh, and `mktemp` is found through PATH.
fn run_quarantine_check(test: &str, treat: &str) -> (String, Vec<PathBuf>) {
    let lib_text = repo_file(".githooks/lib.sh");
    let lib = shell_code(&lib_text);
    let (name, definition) = lib_function(&lib, |l| calls(l, "mktemp")).unwrap_or_else(|| {
        panic!("lib.sh has no quarantine check (a function that calls `mktemp`) ({STOP_REASON})")
    });

    let dir = scratch_dir(&format!("quarantine-{test}"));
    let stubs = dir.join("bin");
    fs::create_dir_all(&stubs).unwrap();
    let log = dir.join("probes.log");
    write_stub(
        &stubs,
        "mktemp",
        &format!(
            "f=$(PATH=\"$PROBE_REAL_PATH\" mktemp \"$@\") || exit 1\n\
             {treat} \"$f\" 2>/dev/null || true\n\
             printf '%s\\n' \"$f\" >> \"$PROBE_LOG\"\n\
             printf '%s\\n' \"$f\"\n"
        ),
    );

    let real_path = std::env::var("PATH").unwrap_or_default();
    let script = format!(
        "set -euo pipefail\n{definition}\nif {name}; then echo quarantining; else echo clean; fi\n"
    );
    let out = Command::new("bash")
        .args(["-c", &script])
        .env("PATH", format!("{}:{real_path}", stubs.display()))
        .env("PROBE_REAL_PATH", &real_path)
        .env("PROBE_LOG", &log)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let verdict = stdout.lines().last().unwrap_or_default().to_owned();
    assert!(
        out.status.success() && ["quarantining", "clean"].contains(&verdict.as_str()),
        "the quarantine check `{name}` must return true or false, not abort: {} {stdout}{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let probes = fs::read_to_string(&log).unwrap_or_default().lines().map(PathBuf::from).collect();
    fs::remove_dir_all(&dir).unwrap();
    (verdict, probes)
}

/// Panics unless the check created at least one probe through `mktemp` and
/// removed every probe again.
fn assert_probes_removed(probes: &[PathBuf]) {
    assert!(
        !probes.is_empty(),
        "the quarantine check must create its probe file with `mktemp` (found through PATH)"
    );
    for probe in probes {
        let gone = !probe.exists();
        let _ = fs::remove_file(probe);
        let _ = fs::remove_dir_all(probe);
        assert!(gone, "the quarantine check left its probe {} behind", probe.display());
    }
}

// ADR-0051: the quarantine check of lib.sh, run for real. A `mktemp` stub
// stands in for a client such as Sourcetree: it sets com.apple.quarantine on
// every file it creates (`xattr -w`, as macOS does for such a client). On
// macOS the check must report a quarantining client (return success); on
// other systems there is no such attribute and the check is Darwin-only, so it
// reports a clean client. Either way it removes its probe file.
#[test]
fn quarantine_check_detects_a_client_that_quarantines_new_files() {
    let (verdict, probes) = run_quarantine_check(
        "quarantining",
        "xattr -w com.apple.quarantine '0081;00000000;QOOB3D test;'",
    );
    let expected = if cfg!(target_os = "macos") { "quarantining" } else { "clean" };
    assert_eq!(verdict, expected, "probe files: {probes:?} ({STOP_REASON})");
    assert_probes_removed(&probes);
}

// ADR-0051: a client that does not quarantine (a terminal): the `mktemp`
// stub removes any quarantine from the new file, so the result does not
// depend on how the test itself was started. The check returns failure, the
// push goes on, and the probe file is removed.
#[test]
fn quarantine_check_passes_a_client_that_does_not_quarantine() {
    let (verdict, probes) = run_quarantine_check("clean", "xattr -d com.apple.quarantine");
    assert_eq!(verdict, "clean", "probe files: {probes:?}");
    assert_probes_removed(&probes);
}
