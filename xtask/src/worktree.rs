//! The committed state of a revision in a temporary detached `git worktree`,
//! where `check-rev` runs the full check of exactly what is pushed (ADR-0048).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::GIT_ENV;
use crate::git;

/// Calls `run` with a detached worktree of `rev` in the system temp dir and
/// removes the worktree afterwards, also when `run` panics.
pub(crate) fn with_worktree<T>(
    root: &Path,
    rev: &str,
    run: impl FnOnce(&Path) -> T,
) -> Result<T, String> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let commit = format!("{rev}^{{commit}}");
    let sha = git(root, &["rev-parse", "--verify", "--quiet", &commit])
        .map_err(|_| format!("unknown revision `{rev}`"))?;
    let sha = sha.trim();
    let unique = format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let dir = std::env::temp_dir().join(format!("qoob3d-check-{unique}"));
    git(root, &["worktree", "add", "--detach", &dir.to_string_lossy(), sha])?;
    let worktree = Worktree { root, dir };
    Ok(run(&worktree.dir))
}

/// Removes the worktree when dropped, so a panic in `run` leaves none behind.
struct Worktree<'a> {
    root: &'a Path,
    dir: PathBuf,
}

impl Drop for Worktree<'_> {
    fn drop(&mut self) {
        let dir = self.dir.to_string_lossy();
        if let Err(e) = git(self.root, &["worktree", "remove", "--force", &dir]) {
            eprintln!("worktree: {e}");
            let _ = std::fs::remove_dir_all(&self.dir);
            let _ = git(self.root, &["worktree", "prune"]);
        }
    }
}

/// The cargo target directory of the checkout at `root` for the value of
/// `CARGO_TARGET_DIR`: unset or empty → `root/target`, relative to `root`.
pub(crate) fn target_dir(root: &Path, cargo_target_dir: Option<&str>) -> PathBuf {
    // `join` keeps an absolute path as it is.
    root.join(cargo_target_dir.filter(|dir| !dir.is_empty()).unwrap_or("target"))
}

/// The persistent target directory of all `check-rev` worktree checks:
/// `check-rev` inside the checkout's target directory, never that directory
/// itself, whose artifacts a worktree build would replace (ADR-0048).
pub(crate) fn check_rev_target_dir(root: &Path, cargo_target_dir: Option<&str>) -> PathBuf {
    target_dir(root, cargo_target_dir).join("check-rev")
}

/// `cargo xtask check` in `worktree`, building into `target_dir`, which
/// persists across checks so that OCCT is not rebuilt.
pub(crate) fn check_command(worktree: &Path, target_dir: &Path) -> Command {
    cargo(worktree, target_dir, &["xtask", "check"])
}

/// Builds the xtask of `worktree` into `target_dir` before the check runs it.
/// GUI git clients such as Sourcetree quarantine every file their child
/// processes create, and Gatekeeper refuses to start a quarantined xtask, so
/// on macOS the attribute is removed from the built binary (ADR-0051).
pub(crate) fn build_xtask(worktree: &Path, target_dir: &Path) -> Result<(), String> {
    let status = build_xtask_command(worktree, target_dir)
        .status()
        .map_err(|e| format!("could not start cargo in {}: {e}", worktree.display()))?;
    if !status.success() {
        return Err(format!("build of xtask in {} failed ({status})", worktree.display()));
    }
    #[cfg(target_os = "macos")]
    {
        // A non-zero exit means the attribute is missing (no GUI client involved): nothing
        // to do. If xattr cannot be started at all, say so and let the check show whether
        // Gatekeeper blocks the binary.
        if let Err(e) = Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(xtask_binary(target_dir))
            .output()
        {
            eprintln!("check-rev: could not run xattr: {e}");
        }
    }
    Ok(())
}

/// `cargo build` of the xtask package in `worktree`, into `target_dir`.
pub(crate) fn build_xtask_command(worktree: &Path, target_dir: &Path) -> Command {
    cargo(worktree, target_dir, &["build", "--quiet", "--package", "xtask"])
}

/// The xtask binary that cargo builds with the dev profile into `target_dir`.
pub(crate) fn xtask_binary(target_dir: &Path) -> PathBuf {
    target_dir.join("debug").join("xtask")
}

/// `cargo` with `args` in `worktree`, building into `target_dir`, without the
/// hook's [`GIT_ENV`].
fn cargo(worktree: &Path, target_dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("cargo");
    command.args(args).current_dir(worktree).env("CARGO_TARGET_DIR", target_dir);
    for key in GIT_ENV {
        command.env_remove(key);
    }
    command
}

#[cfg(test)]
mod tests;
