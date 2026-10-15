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

/// The production part of an xtask source file: everything before the
/// out-of-line `#[cfg(test)] mod tests;` (`xtask hygiene` allows no other
/// test code in it).
fn production_part(source: &str) -> &str {
    source.split("#[cfg(test)]\nmod tests;").next().unwrap_or_default()
}

/// `source` without whitespace, so `Command::new( "git" )` and
/// `fn git (` read like their formatted forms.
fn compact(source: &str) -> String {
    source.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `source` spawns git itself: a `Command::new` whose program is the string
/// literal `"git"`, also by path (`"/usr/bin/git"`).
fn spawns_git(source: &str) -> bool {
    let source = compact(source);
    source.match_indices("Command::new(\"").any(|(i, call)| {
        source[i + call.len()..]
            .split_once('"')
            .is_some_and(|(program, _)| program.rsplit('/').next() == Some("git"))
    })
}

/// The `.rs` files below `dir`, relative to `src` with `/`, recursively.
fn rust_files(src: &Path, dir: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(src, &path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let rel = path.strip_prefix(src).unwrap().to_string_lossy().replace('\\', "/");
            files.push(rel);
        }
    }
}

// One git helper (critic K-13 on bd0fade, ADR-0048): `git` / `git_command` in
// main.rs are the only place that spawns git, so every git call drops the
// variables a hook exports. Every other module under xtask/src, also nested
// ones, has a production part (before `#[cfg(test)] mod tests;`; `tests.rs`
// files are test code) without a `Command::new("git")` (also by path) and
// without a `fn git(` of its own. The walk must reach the known modules, and
// the detector must see main.rs's own spawn, or the test proves nothing. One
// message per offender: `<file> builds its own git command` /
// `<file> defines its own `git``.
#[test]
fn no_module_but_main_spawns_git_itself() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &src, &mut files);
    for known in ["main.rs", "selftest.rs", "worktree.rs", "printers.rs", "guard/tests.rs"] {
        assert!(files.iter().any(|f| f == known), "{known} was not scanned: {files:?}");
    }
    let main = fs::read_to_string(src.join("main.rs")).unwrap();
    assert!(spawns_git(production_part(&main)), "the detector misses main.rs's `git_command`");
    let mut offenders = Vec::new();
    for file in files.iter().filter(|f| *f != "main.rs" && f.rsplit('/').next() != Some("tests.rs"))
    {
        let source = fs::read_to_string(src.join(file)).unwrap();
        let production = production_part(&source);
        if spawns_git(production) {
            offenders.push(format!("{file} builds its own git command"));
        }
        if compact(production).contains("fngit(") {
            offenders.push(format!("{file} defines its own `git`"));
        }
    }
    assert!(
        offenders.is_empty(),
        "{}; use `crate::git` / `crate::git_command`",
        offenders.join("; ")
    );
}

/// The text up to the next `"` after `prefix`, on the first line of `text`
/// that starts with `prefix` (leading whitespace ignored); `None` if no line
/// does.
fn quoted_after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.trim_start().strip_prefix(prefix))
        .and_then(|rest| rest.split('"').next())
}

/// The `version` of the `[[package]]` named `name` in a Cargo.lock text.
fn lock_version<'a>(lock: &'a str, name: &str) -> Option<&'a str> {
    let header = format!("name = \"{name}\"\n");
    lock.split("[[package]]")
        .find(|block| block.contains(&header))
        .and_then(|block| quoted_after(block, "version = \""))
}

// OCCT version pins agree (ADR-0050): the fork's build script skips the OCCT
// build only when the install holds `BUILTIN_OCCT_VERSION`, while occt-sys
// (pinned exactly in the fork's Cargo.toml, resolved in Cargo.lock) builds and
// installs its own version. If the strings differ, the installed version never
// matches the wanted one and OCCT is rebuilt (about 20 minutes) on every
// build, silently. Expected: one and the same string in all three files, read
// as text.
#[test]
fn occt_version_pins_agree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let read =
        |rel: &str| fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    let build = read("third_party/opencascade-sys/build.rs");
    let manifest = read("third_party/opencascade-sys/Cargo.toml");
    let lock = read("Cargo.lock");
    let constant = quoted_after(&build, "const BUILTIN_OCCT_VERSION: &str = \"");
    let pin = quoted_after(&manifest, "occt-sys = { version = \"=");
    let locked = lock_version(&lock, "occt-sys");
    assert!(
        constant.is_some() && constant == pin && pin == locked,
        "OCCT version pins disagree: BUILTIN_OCCT_VERSION in \
         third_party/opencascade-sys/build.rs = {constant:?}, occt-sys pin in \
         third_party/opencascade-sys/Cargo.toml = {pin:?}, occt-sys in Cargo.lock = \
         {locked:?}; a mismatch silently rebuilds OCCT on every build (ADR-0050)"
    );
}
