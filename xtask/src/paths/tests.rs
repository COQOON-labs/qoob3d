use super::*;

// ADR-0039: test files are `tests/**`, `**/tests.rs`, `proptest-regressions/`
// below `crates/` or `xtask/`, plus `models/*.expect.json`.
#[test]
fn test_files_are_tests() {
    for path in [
        "crates/qoob3d-core/tests/document.rs",
        "crates/qoob3d-core/tests/fixtures/v1.qbd",
        "crates/qoob3d-app/tests/modeling.rs",
        "crates/qoob3d-sketch/src/edit/tests.rs",
        "crates/qoob3d-core/src/tests/helpers.rs",
        "crates/qoob3d-core/proptest-regressions/expr.txt",
        "xtask/src/docs/tests.rs",
        "xtask/tests/cli.rs",
        "models/gear.expect.json",
    ] {
        assert_eq!(kind(path), Kind::Test, "{path}");
    }
}

// ADR-0039: source files below `crates/` or `xtask/` with a code extension
// (.rs, .wgsl, .cpp, .hpp, .h) that are not tests are production code.
#[test]
fn source_files_are_code() {
    for path in [
        "crates/qoob3d-kernel/build.rs",
        "crates/qoob3d-kernel/cpp/qoob3d_occt.cpp",
        "crates/qoob3d-kernel/cpp/qoob3d_occt.hpp",
        "crates/qoob3d-kernel/cpp/qoob3d_occt.h",
        "crates/qoob3d-render/src/shaders/mesh.wgsl",
        "crates/qoob3d-app/src/app.rs",
        "xtask/src/main.rs",
    ] {
        assert_eq!(kind(path), Kind::Code, "{path}");
    }
}

// ADR-0039: only a whole path component `tests` or the file name `tests.rs`
// marks a test; names that merely contain "tests" are code.
#[test]
fn tests_must_be_a_whole_path_component() {
    for path in [
        "crates/qoob3d-core/src/contests.rs",
        "crates/qoob3d-core/src/mytests.rs",
        "crates/qoob3d-core/src/attests/foo.rs",
        "crates/qoob3d-core/src/tests_util.rs",
    ] {
        assert_eq!(kind(path), Kind::Code, "{path}");
    }
}

// ADR-0039: docs, config, assets and anything outside `crates/`/`xtask/`
// (except `models/*.expect.json`) are neither tests nor code.
#[test]
fn everything_else_is_other() {
    for path in [
        "docs/architecture.md",
        "docs/adr/0039-tests-and-code-in-separate-contexts.md",
        "crates/qoob3d-core/Cargo.toml",
        "crates/qoob3d-app/locales/en/main.ftl",
        "crates/qoob3d-core/README.md",
        "models/gear.qbd",
        "README.md",
        "Cargo.toml",
        "src/main.rs",
        "tests/outside.rs",
    ] {
        assert_eq!(kind(path), Kind::Other, "{path}");
    }
}

// ADR-0037: the harness is everything under the listed directories, the
// guard binary and exactly the listed root files.
#[test]
fn harness_paths() {
    for path in [
        ".claude/settings.json",
        ".claude/hooks/guard.sh",
        ".claude/agents/test-author.md",
        ".githooks/pre-push",
        ".github/workflows/ci.yml",
        ".cargo/config.toml",
        "xtask/src/main.rs",
        "xtask/Cargo.toml",
        "harness/rules.md",
        "target/debug/xtask",
        "Cargo.toml",
        "clippy.toml",
        "deny.toml",
        "rustfmt.toml",
        "rust-toolchain.toml",
    ] {
        assert!(is_harness(path), "{path} should be harness");
    }
}

// ADR-0037: root config names only count at the root; similar prefixes are
// not harness directories.
#[test]
fn non_harness_paths() {
    for path in [
        "crates/qoob3d-core/Cargo.toml",
        "crates/qoob3d-app/src/app.rs",
        "docs/harness.md",
        ".claudeX/foo",
        ".githooksX/pre-push",
        "xtaskX/src/main.rs",
        "harnessX/rules.md",
        "target/debug/qoob3d",
        "README.md",
        "Cargo.lock",
    ] {
        assert!(!is_harness(path), "{path} should not be harness");
    }
}

// ADR-0030/ADR-0039: the fixture directory constant.
#[test]
fn fixture_directory() {
    assert_eq!(FIXTURES, "crates/qoob3d-core/tests/fixtures/");
}

// ADR-0037 (review G3): cargo links the xtask binary also as
// `target/debug/deps/xtask-<hash>`; every path with that prefix is harness,
// and `target/debug/xtask` stays harness. Only at the root, only with the
// hyphen.
#[test]
fn xtask_deps_binary_is_harness() {
    for path in [
        "target/debug/deps/xtask-0123abcd",
        "target/debug/deps/xtask-0123abcd.d",
        "target/debug/deps/xtask-",
        "target/debug/xtask",
    ] {
        assert!(is_harness(path), "{path} should be harness");
    }
    for path in [
        "target/debug/deps/xtaskish-0123abcd",
        "target/debug/deps/qoob3d_app-0123abcd",
        "target/debug/deps/libxtask-0123abcd.rlib",
        "crates/x/target/debug/deps/xtask-0123abcd",
        "target/debug/deps",
    ] {
        assert!(!is_harness(path), "{path} should not be harness");
    }
}
