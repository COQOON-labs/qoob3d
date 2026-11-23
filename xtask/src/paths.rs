//! Path classes of the harness (ADR-0037). Paths are repository-relative with
//! `/` separators.

/// The harness itself (ADR-0037): changes need an unlocked session locally
/// and an owner review on GitHub. Entries ending in `/` are prefixes. The
/// write protection `.claude/hooks/guard.sh` carries the same list.
pub(crate) const HARNESS: &[&str] = &[
    ".claude/",
    ".githooks/",
    ".github/",
    ".cargo/",
    "xtask/",
    "harness/",
    "Cargo.toml",
    "clippy.toml",
    "deny.toml",
    "rustfmt.toml",
    "rust-toolchain.toml",
];

/// Ignores case, as macOS file systems do.
pub(crate) fn is_harness(path: &str) -> bool {
    let path = path.to_lowercase();
    HARNESS
        .iter()
        .map(|h| h.to_lowercase())
        .any(|h| if h.ends_with('/') { path.starts_with(&h) } else { path == h })
}

/// The harness paths among newline-separated file names, in order (CI flags
/// them, ADR-0037).
pub(crate) fn harness_files(names: &str) -> Vec<&str> {
    names.lines().filter(|name| is_harness(name)).collect()
}

/// Test code: integration tests (a `tests` directory) and unit test modules
/// (`tests.rs`). Everything else under `crates/` and `xtask/` is production
/// code and must not hold tests (`hygiene`).
pub(crate) fn is_test(path: &str) -> bool {
    let mut components = path.rsplit('/');
    components.next() == Some("tests.rs") || components.any(|c| c == "tests")
}

#[cfg(test)]
mod tests;
