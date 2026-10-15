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

// ADR-0051: each hook that runs xtask sources `lib.sh` before its first
// `cargo xtask` line; `lib.sh` builds xtask and, on macOS only, removes
// `com.apple.quarantine` from the built binary, after the build.
// The binary path is kept in a variable; inside the Darwin block an
// `if [ -f "$var" ]` (or `test -f`) precedes the `xattr` call, and the branch
// where the file is missing writes a message to stderr (`>&2`) instead of
// running xattr blindly, whose failures are discarded (critic finding).
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
    assert_lib_sh_strips_quarantine(&lib);
}

/// Why `lib.sh` must check that the binary exists; part of the failure messages.
const MISSING: &str = "if the xtask binary is missing, lib.sh must say so on stderr \
     (`>&2`, e.g. `hooks: … not found, quarantine not removed`) instead of calling \
     xattr blindly and discarding its failure";

/// Panics unless `lib` (the text of `.githooks/lib.sh`) builds xtask and then,
/// inside a Darwin check, removes the quarantine from the binary held in a
/// variable, after an `if [ -f "$var" ]` whose missing branch writes to stderr.
fn assert_lib_sh_strips_quarantine(lib: &str) {
    let code = shell_code(lib);
    let build = code
        .iter()
        .position(|l| l.starts_with("cargo build") && l.contains("--package xtask"))
        .unwrap_or_else(|| {
            panic!(
                "lib.sh must build xtask with `cargo build --package xtask` ({QUARANTINE_REASON})"
            )
        });
    let binary = "\"${CARGO_TARGET_DIR:-target}/debug/xtask\"";
    let (assignment, var) = code
        .iter()
        .enumerate()
        .find_map(|(i, l)| {
            let name = assigned_name(l)?;
            (l.split_once('=')?.1 == binary).then_some((i, name))
        })
        .unwrap_or_else(|| {
            panic!(
                "lib.sh must store the binary path {binary} in a variable, so that it can \
                 check the file exists before calling xattr ({QUARANTINE_REASON})"
            )
        });
    let uses =
        |l: &str| l.contains(&format!("\"${var}\"")) || l.contains(&format!("\"${{{var}}}\""));
    let strip = code
        .iter()
        .position(|l| l.starts_with("xattr -d com.apple.quarantine") && uses(l))
        .unwrap_or_else(|| {
            panic!(
                "lib.sh must remove com.apple.quarantine from \"${var}\" ({binary}) with \
                 `xattr -d` ({QUARANTINE_REASON})"
            )
        });
    assert!(
        assignment < strip,
        "lib.sh must assign {var} before the xattr call ({QUARANTINE_REASON})"
    );
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
    let (_, darwin_end) = if_branches(&code, guard);
    let darwin_end = darwin_end.unwrap_or_else(|| panic!("lib.sh: the Darwin check has no `fi`"));
    assert!(
        strip < darwin_end,
        "the xattr call in lib.sh must sit inside the Darwin check ({QUARANTINE_REASON})"
    );

    // Inside the Darwin block, before the xattr call: `if [ -f "$var" ]` or
    // `if test -f "$var"` (possibly negated with `!`).
    let tests_file = |l: &str| {
        l.starts_with("if ")
            && (l.contains("[ ") || l.contains("test "))
            && [format!("-f \"${var}\""), format!("-f \"${{{var}}}\"")]
                .iter()
                .any(|t| l.contains(t.as_str()))
    };
    let Some(offset) = code[guard + 1..strip].iter().position(|l| tests_file(l)) else {
        panic!(
            "inside the Darwin check, lib.sh must test `[ -f \"${var}\" ]` before the \
             xattr call ({MISSING})"
        )
    };
    let check = guard + 1 + offset;
    let negated = code[check].contains("! -f") || code[check].contains("! test");
    let (otherwise, check_end) = if_branches(&code, check);
    let check_end = check_end.unwrap_or_else(|| panic!("lib.sh: the file check has no `fi`"));
    assert!(check_end < darwin_end, "the file check must sit inside the Darwin check");
    let otherwise = otherwise
        .unwrap_or_else(|| panic!("the file check in lib.sh has no `else` branch ({MISSING})"));
    let (exists, missing) = if negated {
        (otherwise..check_end, check..otherwise)
    } else {
        (check..otherwise, otherwise..check_end)
    };
    assert!(
        exists.contains(&strip) && strip != exists.start,
        "the xattr call must sit in the branch where {var} exists ({MISSING})"
    );
    assert!(
        code[missing.start + 1..missing.end].iter().any(|l| l.contains(">&2")),
        "the branch where {var} is missing writes nothing to stderr ({MISSING})"
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for hook in [".githooks/commit-msg", ".githooks/pre-push"] {
        let text = fs::read_to_string(root.join(hook)).unwrap();
        let code = shell_code(&text);
        assert_eq!(
            code.first().copied(),
            Some("set -euo pipefail"),
            "{hook}: the first executed line must be `set -euo pipefail` ({REASON})"
        );
        let source = code
            .iter()
            .position(|l| {
                (l.starts_with("source ") || l.starts_with(". ")) && l.contains("/lib.sh")
            })
            .unwrap_or_else(|| panic!("{hook} does not source lib.sh ({REASON})"));
        assert!(
            source > 0,
            "{hook} must set `set -euo pipefail` before sourcing lib.sh ({REASON})"
        );
    }
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
    assert_build_xtask_strips_quarantine(&function_source(&own, "build_xtask"));
}

/// Panics unless `build_xtask` (the source of `fn build_xtask`) builds with
/// `build_xtask_command(` and then removes the quarantine from
/// `xtask_binary(…)` in an `if cfg!(target_os = "macos")` block, not in a
/// `#[cfg(target_os …)]` item.
fn assert_build_xtask_strips_quarantine(build_xtask: &str) {
    let run = build_xtask.find("build_xtask_command(").unwrap_or_else(|| {
        panic!("build_xtask must build with `build_xtask_command(` ({QUARANTINE_REASON})")
    });
    // A `#[cfg(target_os = …)]` item compiles the strip, and with it the only
    // non-test use of `xtask_binary`, away on Linux: clippy `-D warnings`
    // then rejects `xtask_binary` as dead code there (critic finding).
    if let Some(at) = build_xtask.find("#[cfg(target_os") {
        panic!(
            "build_xtask must not gate the quarantine strip with a `#[cfg(target_os …)]` item: \
             the helper must be used on every platform, otherwise it is dead code on Linux and \
             clippy fails there; use `if cfg!(target_os = \"macos\") {{ … }}` \
             ({QUARANTINE_REASON}): {}",
            attributed_item(build_xtask, at)
        );
    }
    let cfg = build_xtask.find("if cfg!(target_os = \"macos\")").unwrap_or_else(|| {
        panic!(
            "build_xtask must strip the quarantine only on macOS, in an \
             `if cfg!(target_os = \"macos\")` block ({QUARANTINE_REASON})"
        )
    });
    assert!(
        run < cfg,
        "build_xtask must strip the quarantine after the build ({QUARANTINE_REASON})"
    );
    let strip = if_block(build_xtask, cfg);
    assert!(
        strip.contains("com.apple.quarantine") && strip.contains("xtask_binary("),
        "the macOS-only block of build_xtask must remove com.apple.quarantine from \
         xtask_binary(…) ({QUARANTINE_REASON}): {strip}"
    );
}

/// The first `{ … }` block in `code` after byte `at`, braces included, up to
/// the `}` that closes it (the body of an `if` that starts at `at`).
fn if_block(code: &str, at: usize) -> &str {
    let Some(open) = code[at..].find('{').map(|i| at + i) else {
        return &code[at..];
    };
    let mut depth = 0_i32;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &code[open..=open + i];
                }
            }
            _ => {}
        }
    }
    &code[open..]
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

/// The part of `lib.sh` before the Darwin check, as in the repository.
const LIB_HEAD: &str = "set -euo pipefail\n\
    export PATH=\"/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH\"\n\
    cd \"$(git rev-parse --show-toplevel)\"\n\
    # Builds xtask before the hook runs it.\n\
    cargo build --quiet --package xtask\n";

// Oracle self-check for `assert_lib_sh_strips_quarantine` (ADR-0051, critic
// finding): the shapes the fix may take pass. Without this and the next
// test, a matcher that never fires (or always fires) would decide the
// repository test whatever lib.sh contains.
#[test]
fn lib_sh_quarantine_check_accepts_a_guarded_strip() {
    let accepted = [
        // `[ -f ]`, missing branch in `else`
        "if [ \"$(uname)\" = Darwin ]; then\n\
           xtask_binary=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
           if [ -f \"$xtask_binary\" ]; then\n\
             xattr -d com.apple.quarantine \"$xtask_binary\" 2>/dev/null || true\n\
           else\n\
             echo \"hooks: $xtask_binary not found, quarantine not removed\" >&2\n\
           fi\n\
         fi\n",
        // negated `test -f`, braces, variable assigned before the Darwin check
        "binary=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
         if [ \"$(uname)\" = Darwin ]; then\n\
           if ! test -f \"${binary}\"; then\n\
             echo \"hooks: ${binary} not found, quarantine not removed\" >&2\n\
           else\n\
             xattr -d com.apple.quarantine \"${binary}\" 2>/dev/null || true\n\
           fi\n\
         fi\n",
    ];
    for tail in accepted {
        let lib = format!("{LIB_HEAD}{tail}");
        assert_eq!(panic_text(|| assert_lib_sh_strips_quarantine(&lib)), None, "{lib}");
    }
}

// Oracle self-check for `assert_lib_sh_strips_quarantine` (ADR-0051, critic
// finding): each regression fails with the message that names it.
#[test]
fn lib_sh_quarantine_check_rejects_each_regression() {
    let rejected = [
        // the code before the critic finding: xattr runs blindly on a literal path
        (
            "if [ \"$(uname)\" = Darwin ]; then\n\
               xattr -d com.apple.quarantine \"${CARGO_TARGET_DIR:-target}/debug/xtask\" \
               2>/dev/null || true\n\
             fi\n",
            "in a variable",
        ),
        // the sabotage `hooks-keep-quarantine`: no xattr call left
        (
            "if [ \"$(uname)\" = Darwin ]; then\n\
               b=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
               if [ -f \"$b\" ]; then\n\
                 true\n\
               else\n\
                 echo \"hooks: $b not found\" >&2\n\
               fi\n\
             fi\n",
            "lib.sh must remove com.apple.quarantine",
        ),
        // variable, but no file check
        (
            "if [ \"$(uname)\" = Darwin ]; then\n\
               b=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
               xattr -d com.apple.quarantine \"$b\" 2>/dev/null || true\n\
             fi\n",
            "must test `[ -f",
        ),
        // file check without an else branch: a missing binary stays silent
        (
            "if [ \"$(uname)\" = Darwin ]; then\n\
               b=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
               if [ -f \"$b\" ]; then\n\
                 xattr -d com.apple.quarantine \"$b\" 2>/dev/null || true\n\
               fi\n\
             fi\n",
            "no `else` branch",
        ),
        // else branch, but its message goes to stdout
        (
            "if [ \"$(uname)\" = Darwin ]; then\n\
               b=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
               if [ -f \"$b\" ]; then\n\
                 xattr -d com.apple.quarantine \"$b\" 2>/dev/null || true\n\
               else\n\
                 echo \"hooks: $b not found\"\n\
               fi\n\
             fi\n",
            "writes nothing to stderr",
        ),
        // the branches swapped: xattr runs exactly when the file is missing
        (
            "if [ \"$(uname)\" = Darwin ]; then\n\
               b=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
               if [ -f \"$b\" ]; then\n\
                 echo \"hooks: $b found\" >&2\n\
               else\n\
                 xattr -d com.apple.quarantine \"$b\" 2>/dev/null || true\n\
               fi\n\
             fi\n",
            "branch where b exists",
        ),
        // the strip outside the Darwin check
        (
            "b=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
             if [ \"$(uname)\" = Darwin ]; then\n\
               true\n\
             fi\n\
             if [ -f \"$b\" ]; then\n\
               xattr -d com.apple.quarantine \"$b\" 2>/dev/null || true\n\
             else\n\
               echo \"hooks: $b not found\" >&2\n\
             fi\n",
            "inside the Darwin check",
        ),
    ];
    for (tail, expected) in rejected {
        let lib = format!("{LIB_HEAD}{tail}");
        let message = panic_text(|| assert_lib_sh_strips_quarantine(&lib))
            .unwrap_or_else(|| panic!("accepted, but must fail with {expected:?}:\n{lib}"));
        assert!(message.contains(expected), "expected {expected:?}, got {message:?}\n{lib}");
    }
    // The strip before the build.
    let lib = "if [ \"$(uname)\" = Darwin ]; then\n\
                 b=\"${CARGO_TARGET_DIR:-target}/debug/xtask\"\n\
                 if [ -f \"$b\" ]; then\n\
                   xattr -d com.apple.quarantine \"$b\" 2>/dev/null || true\n\
                 else\n\
                   echo \"hooks: $b not found\" >&2\n\
                 fi\n\
               fi\n\
               cargo build --quiet --package xtask\n";
    let message = panic_text(|| assert_lib_sh_strips_quarantine(lib)).expect("strip before build");
    assert!(message.contains("after the build"), "{message}");
}

// Oracle self-check for `assert_build_xtask_strips_quarantine` (ADR-0051,
// critic finding: `#[cfg(target_os = "macos")]` leaves `xtask_binary` dead on
// Linux, where clippy `-D warnings` then fails).
#[test]
fn build_xtask_quarantine_check_requires_a_cfg_macro_block_after_the_build() {
    const SIGNATURE: &str =
        "pub(crate) fn build_xtask(worktree: &Path, target_dir: &Path) -> Result<(), String> {\n";
    const BUILD: &str = "    let status = build_xtask_command(worktree, target_dir)\n\
        .status()\n\
        .map_err(|e| e.to_string())?;\n\
        if !status.success() {\n\
            return Err(status.to_string());\n\
        }\n";
    const STRIP: &str = "{\n\
            if let Err(e) = Command::new(\"xattr\")\n\
                .args([\"-d\", \"com.apple.quarantine\"])\n\
                .arg(xtask_binary(target_dir))\n\
                .output()\n\
            {\n\
                eprintln!(\"check-rev: could not run xattr: {e}\");\n\
            }\n\
        }\n";
    let accepted =
        format!("{SIGNATURE}{BUILD}    if cfg!(target_os = \"macos\") {STRIP}    Ok(())\n}}");
    assert_eq!(panic_text(|| assert_build_xtask_strips_quarantine(&accepted)), None, "{accepted}");

    let rejected = [
        // the code before the critic finding
        (
            format!(
                "{SIGNATURE}{BUILD}    #[cfg(target_os = \"macos\")]\n    {STRIP}    Ok(())\n}}"
            ),
            "dead code on Linux",
        ),
        // not gated at all: xattr would run on Linux too
        (format!("{SIGNATURE}{BUILD}    {STRIP}    Ok(())\n}}"), "only on macOS"),
        // gated on the wrong platform
        (
            format!("{SIGNATURE}{BUILD}    if cfg!(target_os = \"linux\") {STRIP}    Ok(())\n}}"),
            "only on macOS",
        ),
        // the macOS block no longer strips; the strip runs on every platform
        (
            format!(
                "{SIGNATURE}{BUILD}    if cfg!(target_os = \"macos\") {{\n        let _ = 0;\n    }}\n    \
                 {STRIP}    Ok(())\n}}"
            ),
            "must remove com.apple.quarantine",
        ),
        // the strip before the build
        (
            format!("{SIGNATURE}    if cfg!(target_os = \"macos\") {STRIP}{BUILD}    Ok(())\n}}"),
            "after the build",
        ),
    ];
    for (source, expected) in rejected {
        let message = panic_text(|| assert_build_xtask_strips_quarantine(&source))
            .unwrap_or_else(|| panic!("accepted, but must fail with {expected:?}:\n{source}"));
        assert!(message.contains(expected), "expected {expected:?}, got {message:?}\n{source}");
    }
}
