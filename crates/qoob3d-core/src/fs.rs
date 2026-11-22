//! File system helper shared by everything that writes project files.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes the temporary files of concurrent writers in one process.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Writes via a temporary file in the same folder and a rename, so a reader
/// (or a crash) never sees half a file. The temporary name starts with a dot
/// and ends in `.tmp`, a leftover is therefore never taken for a project file.
///
/// # Errors
/// The temporary file cannot be written or renamed.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        std::fs::remove_file(&temp).ok();
    })
}
