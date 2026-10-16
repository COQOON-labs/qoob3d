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

/// The OCCT boundary, a high-risk review trigger (ADR-0052): the vendored
/// forks, our C++ wrappers and their cxx bridge. Any `build.rs` and C++
/// source counts as well ([`is_occt_boundary`]).
const OCCT_BOUNDARY: &[&str] =
    &["third_party/", "crates/qoob3d-kernel/cpp/", "crates/qoob3d-kernel/src/occt.rs"];
const CPP_EXTENSIONS: &[&str] = &[".cpp", ".hpp", ".hxx", ".h"];

/// The project format with its validation and migrations (ADR-0030) and the
/// exporters (ADR-0034), a high-risk review trigger (ADR-0052).
const FORMAT: &[&str] = &[
    "crates/qoob3d-core/src/document/",
    "crates/qoob3d-core/src/export.rs",
    "crates/qoob3d-core/src/export/",
];

const SOURCE_TREES: &[&str] = &["crates/", "xtask/", "third_party/"];
const CODE_EXTENSIONS: &[&str] = &[".rs", ".wgsl", ".cpp", ".hpp", ".hxx", ".h"];

pub(crate) fn is_harness(path: &str) -> bool {
    listed(HARNESS, path)
}

/// Whether `path` is one of `entries`, where an entry ending in `/` or `-` is
/// a prefix. Ignores case, as macOS file systems do (ADR-0043).
pub(crate) fn listed(entries: &[&str], path: &str) -> bool {
    let path = path.to_lowercase();
    entries
        .iter()
        .map(|e| e.to_lowercase())
        .any(|e| if e.ends_with(['/', '-']) { path.starts_with(&e) } else { path == e })
}

pub(crate) fn is_occt_boundary(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    listed(OCCT_BOUNDARY, path)
        || file == "build.rs"
        || CPP_EXTENSIONS.iter().any(|e| file.ends_with(e))
}

/// Test files under the format prefixes do not count: they cannot break a
/// saved project or an export.
pub(crate) fn is_format(path: &str) -> bool {
    kind(path) != Kind::Test && listed(FORMAT, path)
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
