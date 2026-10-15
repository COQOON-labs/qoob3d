//! Path classes shared by the commit policy and the Claude Code guard
//! (ADR-0037, ADR-0039). Paths are repository-relative with `/` separators.

/// What a changed file is, for the test/code separation rule (ADR-0039).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Test code and test oracles.
    Test,
    /// Production source code.
    Code,
    /// Docs, config, assets.
    Other,
}

/// The harness itself (ADR-0037): changes need an unlocked session locally
/// and an owner review on GitHub. Entries ending in `/` or `-` are prefixes.
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
    // `cargo xtask` runs this binary, cargo links it from `deps/` (ADR-0037).
    "target/debug/xtask",
    "target/debug/deps/xtask-",
];

/// Format fixtures may be added, never modified or deleted (ADR-0030).
pub(crate) const FIXTURES: &str = "crates/qoob3d-core/tests/fixtures/";

const SOURCE_TREES: &[&str] = &["crates/", "xtask/", "third_party/"];
const CODE_EXTENSIONS: &[&str] = &[".rs", ".wgsl", ".cpp", ".hpp", ".hxx", ".h"];

/// Ignores case: macOS file systems do (ADR-0043).
pub(crate) fn is_harness(path: &str) -> bool {
    let path = path.to_lowercase();
    HARNESS
        .iter()
        .map(|h| h.to_lowercase())
        .any(|h| if h.ends_with(['/', '-']) { path.starts_with(&h) } else { path == h })
}

/// The harness paths among newline-separated file names, in order (CI flags
/// them, ADR-0037).
pub(crate) fn harness_files(names: &str) -> Vec<&str> {
    names.lines().filter(|name| is_harness(name)).collect()
}

/// Source trees are `crates/`, `xtask/` and the vendored forks in
/// `third_party/`, which are compiled into the product (ADR-0014, ADR-0047).
/// Ignores case, as macOS file systems do (ADR-0043): `Crates/X/Tests/A.RS` is
/// a test.
pub(crate) fn kind(path: &str) -> Kind {
    let path = path.to_lowercase();
    let path = path.as_str();
    let in_source_tree = SOURCE_TREES.iter().any(|t| path.starts_with(t));
    let file = path.rsplit('/').next().unwrap_or(path);
    let test_dir = path.split('/').any(|c| c == "tests" || c == "proptest-regressions");
    if (in_source_tree && (test_dir || file == "tests.rs"))
        || (path.starts_with("models/") && file.ends_with(".expect.json"))
    {
        Kind::Test
    } else if in_source_tree && CODE_EXTENSIONS.iter().any(|e| file.ends_with(e)) {
        Kind::Code
    } else {
        Kind::Other
    }
}

#[cfg(test)]
mod tests;
