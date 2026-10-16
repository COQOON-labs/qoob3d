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

// ADR-0051 (finding 2026-10-15, Sourcetree push): `lib.sh` builds xtask and,
// on macOS only, removes `com.apple.quarantine` from the built binary, after
// the build (details in `assert_lib_sh_strips_quarantine`). Each hook sources
// `lib.sh`, has `lib.sh` build xtask, and only then runs the binary directly
// through the variable `lib.sh` assigns its path to; no `cargo xtask` line is
// left, because `cargo run` re-links the binary and re-quarantines it.
// commit-msg: `exec "$var" commits --message "$1"` (the former
// `exec cargo xtask commits --message "$1"`); pre-push runs `commits`,
// `review` and `check-rev` the same way.
#[test]
fn hooks_build_xtask_and_strip_the_macos_quarantine_before_running_it() {
    let lib_text = repo_file(".githooks/lib.sh");
    assert_lib_sh_strips_quarantine(&lib_text);
    let lib = shell_code(&lib_text);
    let (assignment, var) = binary_assignment(&lib).expect("checked above");
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

/// Why `lib.sh` must check that the binary exists; part of the failure messages.
const MISSING: &str = "if the xtask binary is missing, lib.sh must say so on stderr \
     (`>&2`, e.g. `hooks: … not found, quarantine not removed`) instead of calling \
     xattr blindly and discarding its failure";

/// Panics unless `lib` (the text of `.githooks/lib.sh`) builds xtask and then,
/// in the same function (or both at the top level), inside a Darwin check,
/// removes the quarantine from the binary held in a variable, after an
/// `if [ -f "$var" ]` whose missing branch writes to stderr.
fn assert_lib_sh_strips_quarantine(lib: &str) {
    let code = shell_code(lib);
    let build = xtask_build_line(&code).unwrap_or_else(|| {
        panic!("lib.sh must build xtask with `cargo build --package xtask` ({QUARANTINE_REASON})")
    });
    let (assignment, var) = binary_assignment(&code).unwrap_or_else(|| {
        panic!(
            "lib.sh must store the binary path {BINARY} in a variable, so that it can \
             check the file exists before calling xattr ({QUARANTINE_REASON})"
        )
    });
    let strip = code
        .iter()
        .position(|l| l.starts_with("xattr -d com.apple.quarantine") && uses_var(l, var))
        .unwrap_or_else(|| {
            panic!(
                "lib.sh must remove com.apple.quarantine from \"${var}\" ({BINARY}) with \
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
    // A strip at the top level and a build in a function (or the reverse) run
    // at different times: the strip would run when lib.sh is sourced, before
    // the build that re-links the binary.
    let functions = shell_functions(&code);
    let (build_in, strip_in) =
        (enclosing_function(&functions, build), enclosing_function(&functions, strip));
    assert_eq!(
        build_in, strip_in,
        "lib.sh must strip the quarantine in the same function that builds xtask (or both at \
         the top level), so that the strip follows every build ({QUARANTINE_REASON})"
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
    // Build and strip inside one function that the hooks call (ADR-0051:
    // sourcing lib.sh builds nothing, so pre-push can check the client first).
    let lib = format!("{LIB_FUNCTIONS}{BUILD_FUNCTION}");
    assert_eq!(panic_text(|| assert_lib_sh_strips_quarantine(&lib)), None, "{lib}");
}

/// The part of a `lib.sh` that builds nothing when sourced: the setup, the
/// binary path and a quarantine check (the expected shape after ADR-0051's
/// Sourcetree push finding).
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

// Oracle self-check for `assert_lib_sh_strips_quarantine` (ADR-0051): build
// in a function, strip at the top level: the strip runs when lib.sh is
// sourced, before the build re-links the binary; and the reverse.
#[test]
fn lib_sh_quarantine_check_rejects_a_strip_apart_from_the_build() {
    let strip = "if [ \"$(uname)\" = Darwin ]; then\n\
                   if [ -f \"$xtask_binary\" ]; then\n\
                     xattr -d com.apple.quarantine \"$xtask_binary\" 2>/dev/null || true\n\
                   else\n\
                     echo \"hooks: $xtask_binary not found\" >&2\n\
                   fi\n\
                 fi\n";
    let split = [
        format!(
            "{LIB_FUNCTIONS}build_xtask() {{\ncargo build --quiet --package xtask\n}}\n{strip}"
        ),
        format!("{LIB_FUNCTIONS}cargo build --quiet --package xtask\nstrip() {{\n{strip}}}\n"),
    ];
    for lib in split {
        let message = panic_text(|| assert_lib_sh_strips_quarantine(&lib))
            .unwrap_or_else(|| panic!("accepted, but build and strip run apart:\n{lib}"));
        assert!(message.contains("same function"), "{message}\n{lib}");
    }
}

/// Why pre-push must stop under a quarantining client; part of the failure messages.
const STOP_REASON: &str = "ADR-0051 (Sourcetree push, 2026-10-15): cargo re-links \
     target/debug/xtask on every build, and the check of check-rev builds and at once runs \
     test binaries and build scripts; a client that quarantines new files gets them all \
     refused by Gatekeeper, so pre-push must stop first, with a message on stderr";

/// The name of the `lib.sh` function (in `lib`, as `shell_code`) whose body
/// creates a probe file with `mktemp`: the quarantine check.
fn quarantine_check(lib: &[&str]) -> Option<String> {
    shell_functions(lib)
        .into_iter()
        .find(|(_, start, end)| lib[start + 1..*end].iter().any(|l| calls(l, "mktemp")))
        .map(|(name, ..)| name.to_owned())
}

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
    let check = quarantine_check(&lib).unwrap_or_else(|| {
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
    let name = quarantine_check(&lib).unwrap_or_else(|| {
        panic!("lib.sh has no quarantine check (a function that calls `mktemp`) ({STOP_REASON})")
    });
    let (_, start, end) =
        shell_functions(&lib).into_iter().find(|(n, ..)| *n == name).expect("found above");
    let definition = lib[start..=end].join("\n");

    let dir = std::env::temp_dir().join(format!("qoob3d-quarantine-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let stubs = dir.join("bin");
    fs::create_dir_all(&stubs).unwrap();
    let log = dir.join("probes.log");
    let stub = format!(
        "#!/bin/sh\n\
         f=$(PATH=\"$PROBE_REAL_PATH\" mktemp \"$@\") || exit 1\n\
         {treat} \"$f\" 2>/dev/null || true\n\
         printf '%s\\n' \"$f\" >> \"$PROBE_LOG\"\n\
         printf '%s\\n' \"$f\"\n"
    );
    let mktemp = stubs.join("mktemp");
    fs::write(&mktemp, stub).unwrap();
    let mut permissions = fs::metadata(&mktemp).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&mktemp, permissions).unwrap();

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
