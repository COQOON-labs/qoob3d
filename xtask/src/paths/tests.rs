use std::path::Path;

use super::*;

// Integration tests live in a `tests` directory, unit tests in `tests.rs`;
// only a whole path component counts.
#[test]
fn test_files() {
    for path in [
        "crates/qoob3d-core/tests/project.rs",
        "crates/qoob3d-app/tests/common/agent.rs",
        "crates/qoob3d-core/src/mesh/tests.rs",
        "xtask/src/docs/tests.rs",
        "xtask/src/tests.rs",
    ] {
        assert!(is_test(path), "{path}");
    }
    for path in [
        "crates/qoob3d-core/src/contests.rs",
        "crates/qoob3d-core/src/attests/foo.rs",
        "crates/qoob3d-core/src/tests_util.rs",
        "crates/qoob3d-kernel/build.rs",
        "xtask/src/main.rs",
    ] {
        assert!(!is_test(path), "{path}");
    }
}

// The harness is everything under the listed directories and exactly the
// listed root files, ignoring case.
#[test]
fn harness_paths() {
    for path in [
        ".claude/settings.json",
        ".claude/hooks/guard.sh",
        ".githooks/pre-push",
        ".github/workflows/ci.yml",
        ".cargo/config.toml",
        "xtask/src/main.rs",
        "xtask/Cargo.toml",
        "harness/ratchet.txt",
        "Cargo.toml",
        "CARGO.TOML",
        "Xtask/src/main.rs",
        "clippy.toml",
        "deny.toml",
        "rustfmt.toml",
        "rust-toolchain.toml",
    ] {
        assert!(is_harness(path), "{path} should be harness");
    }
}

// Root config names only count at the root; similar prefixes are not harness
// directories.
#[test]
fn non_harness_paths() {
    for path in [
        "crates/qoob3d-core/Cargo.toml",
        "crates/qoob3d-app/src/app.rs",
        "docs/harness.md",
        ".claudeX/foo",
        ".githooksX/pre-push",
        "xtaskX/src/main.rs",
        "xtask",
        "target/debug/xtask",
        "README.md",
        "Cargo.lock",
    ] {
        assert!(!is_harness(path), "{path} should not be harness");
    }
}

// `harness_files` takes the output of `git diff --name-only` and returns the
// harness lines in order, as given.
#[test]
fn harness_files_keeps_the_harness_lines_in_order() {
    let names = "Cargo.toml\ncrates/a.rs\nxtask/src/x.rs\ndocs/x.md\n\n.githooks/pre-push\n\
                 Cargo.lock\nCLIPPY.toml";
    assert_eq!(
        harness_files(names),
        ["Cargo.toml", "xtask/src/x.rs", ".githooks/pre-push", "CLIPPY.toml"]
    );
    assert!(harness_files("").is_empty());
    assert!(harness_files("crates/a.rs\ndocs/x.md\n").is_empty());
}

// The write protection for the edit tools carries the same list as `HARNESS`.
#[test]
fn guard_script_lists_every_harness_path() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace");
    let guard = std::fs::read_to_string(root.join(".claude/hooks/guard.sh")).expect("guard.sh");
    for entry in HARNESS {
        let quoted = format!("\"{}\"", entry.to_lowercase());
        assert!(guard.contains(&quoted), "guard.sh does not list {quoted}");
    }
}
