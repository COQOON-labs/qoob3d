//! The machine-wide worker budget (WS-PRE-06): at most `max_busy_workers`
//! workers of all project services of the user run jobs at the same time.
//! A slot is an OS file lock on `slot-<i>.lock` (`i` below the budget) in
//! the per-user runtime folder; a service that dies frees its slots because
//! the OS releases its locks. The budget is `budget.json`
//! (`{"max_busy_workers": n}`, written by the app) in the same folder, read
//! at every attempt; without it the budget is one less than the physical
//! cores, at least 1.
//!
//! Declaration discovery takes no slot: it is bounded (AG-HD-02), and
//! builds wait for it.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

/// Overrides the runtime folder (tests, several users of one account).
pub(crate) const RUNTIME_ENV: &str = "QOOB3D_RUNTIME_DIR";
/// The budget the app writes, inside the runtime folder.
pub(crate) const BUDGET_FILE: &str = "budget.json";

/// The per-user runtime folder: [`RUNTIME_ENV`], else
/// `$XDG_RUNTIME_DIR/qoob3d` (Linux), else `qoob3d-<user>` in the temporary
/// folder (on macOS `$TMPDIR`, which is per user).
#[must_use]
pub(crate) fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(RUNTIME_ENV).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("qoob3d");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_owned());
    std::env::temp_dir().join(format!("qoob3d-{user}"))
}

#[derive(Deserialize)]
struct Budget {
    max_busy_workers: usize,
}

/// The physical cores of this machine, if the OS tells them.
fn physical_cores() -> Option<usize> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("sysctl").args(["-n", "hw.physicalcpu"]).output();
        return String::from_utf8_lossy(&out.ok()?.stdout).trim().parse().ok();
    }
    // Linux: distinct (physical id, core id) pairs; ARM kernels list none.
    let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut cores = std::collections::HashSet::new();
    let mut package = "";
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        match key.trim() {
            "physical id" => package = value.trim(),
            "core id" => {
                cores.insert((package.to_owned(), value.trim().to_owned()));
            }
            _ => {}
        }
    }
    (!cores.is_empty()).then_some(cores.len())
}

/// One less than the physical cores (else the logical ones), at least 1.
fn default_budget() -> usize {
    static DEFAULT: OnceLock<usize> = OnceLock::new();
    *DEFAULT.get_or_init(|| {
        let logical = std::thread::available_parallelism().map_or(1, usize::from);
        physical_cores().unwrap_or(logical).saturating_sub(1).max(1)
    })
}

/// A slot held: released when dropped (or when the process dies).
pub(crate) struct Slot(File);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.unlock().ok();
    }
}

pub(crate) struct Slots {
    /// `None` when the runtime folder cannot be made: no budget then.
    dir: Option<PathBuf>,
}

impl Slots {
    /// The slots in [`runtime_dir`], made private to the user.
    pub(crate) fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt as _;
        let dir = runtime_dir();
        let made = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir);
        if let Err(e) = &made {
            tracing::warn!("no worker budget, {} cannot be made: {e}", dir.display());
        }
        Self { dir: made.ok().map(|()| dir) }
    }

    /// The budget now: `budget.json`, else [`default_budget`].
    pub(crate) fn budget(&self) -> usize {
        let file = self.dir.as_deref().map(|d| d.join(BUDGET_FILE));
        let budget = file
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| serde_json::from_str::<Budget>(&text).ok())
            .map(|b| b.max_busy_workers);
        budget.filter(|n| *n > 0).unwrap_or_else(default_budget)
    }

    /// A free slot, if there is one; with no runtime folder always one.
    pub(crate) fn try_take(&self) -> Option<Option<Slot>> {
        let Some(dir) = &self.dir else {
            return Some(None);
        };
        (0..self.budget()).find_map(|i| take(dir, i).map(Some))
    }
}

fn take(dir: &Path, i: usize) -> Option<Slot> {
    let path = dir.join(format!("slot-{i}.lock"));
    let file = File::options().create(true).truncate(false).write(true).open(&path);
    let file = file.map_err(|e| tracing::warn!("{}: {e}", path.display())).ok()?;
    match file.try_lock() {
        Ok(()) => Some(Slot(file)),
        Err(TryLockError::WouldBlock) => None,
        Err(TryLockError::Error(e)) => {
            tracing::warn!("locking {}: {e}", path.display());
            None
        }
    }
}
