use std::path::Path;

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

// ADR-0039, critic finding K-13: `.hxx` is the C++ header extension this
// repository actually uses (`crates/qoob3d-kernel/cpp/qoob3d_occt.hxx`, the
// header of the cxx wrappers, ADR-0014). It is production code like the
// `.cpp` next to it; as `Other` it escapes role separation, the commit
// policy and the critic review.
#[test]
fn cxx_header_hxx_is_code() {
    for path in [
        "crates/qoob3d-kernel/cpp/qoob3d_occt.hxx",
        "crates/qoob3d-kernel/cpp/other.hxx",
        "xtask/src/native/helper.hxx",
    ] {
        assert_eq!(kind(path), Kind::Code, "{path}");
    }
    // The extension does not make a test-tree file code, and outside the
    // source trees it stays `Other` (same rules as for `.cpp`). The vendored
    // forks in `third_party/` are a source tree since critic finding K-13 on
    // da69604 (see `vendored_third_party_sources_are_code`); this assertion
    // used to expect `third_party/occt/include/gp_Pnt.hxx` to be `Other`.
    assert_eq!(kind("crates/qoob3d-kernel/tests/support/fake.hxx"), Kind::Test);
    assert_eq!(kind("third_party/occt/include/gp_Pnt.hxx"), Kind::Code);
    assert_eq!(kind("docs/include/gp_Pnt.hxx"), Kind::Other);
}

// ADR-0039, ADR-0014, ADR-0047, critic finding K-13 on da69604: the vendored
// forks in `third_party/` are compiled into the product (`opencascade-sys`
// with its build script, cxx bridges and C++ headers, `opencascade` with its
// Rust API). Their code files (same extension list as below `crates/`) are
// production code, otherwise role separation, the mixed-commit check and the
// critic record (ADR-0041) skip them. Paths are real ones from
// `git ls-files third_party`.
#[test]
fn vendored_third_party_sources_are_code() {
    for path in [
        "third_party/opencascade-sys/build.rs",
        "third_party/opencascade-sys/src/lib.rs",
        "third_party/opencascade-sys/src/b_rep_algo_api.rs",
        "third_party/opencascade-sys/include/b_rep_algo_api.hxx",
        "third_party/opencascade-sys/include/bindings_common.hxx",
        "third_party/opencascade/src/lib.rs",
        "third_party/opencascade/src/primitives/solid.rs",
        "third_party/opencascade/src/primitives/boolean_shape.rs",
    ] {
        assert_eq!(kind(path), Kind::Code, "{path}");
    }
}

// ADR-0039, K-13: docs, licenses and manifests of the vendored forks stay
// `Other`, exactly like `crates/*/Cargo.toml` and `crates/*/README.md`.
// Real paths from `git ls-files third_party`.
#[test]
fn vendored_third_party_docs_and_manifests_are_other() {
    for path in [
        "third_party/opencascade-sys/Cargo.toml",
        "third_party/opencascade-sys/LICENSE",
        "third_party/opencascade-sys/README.md",
        "third_party/opencascade-sys/OCCT/CMakeLists.txt",
        "third_party/opencascade/Cargo.toml",
        "third_party/opencascade/LICENSE",
        "third_party/opencascade/README.md",
    ] {
        assert_eq!(kind(path), Kind::Other, "{path}");
    }
}

// ADR-0039, K-13: `git ls-files third_party` has no `tests/` directory and
// no `tests.rs` today (upstream tests are inline in the source files). If a
// fork gains one, the existing rule for source trees applies: it is a test.
#[test]
fn vendored_third_party_test_files_are_tests() {
    for path in [
        "third_party/opencascade/tests/boolean.rs",
        "third_party/opencascade/src/primitives/tests.rs",
        "third_party/opencascade-sys/tests/include/fake.hxx",
    ] {
        assert_eq!(kind(path), Kind::Test, "{path}");
    }
}

// ADR-0039: only the directory `third_party/` itself is a source tree;
// look-alike prefixes and nested `third_party` directories elsewhere are not.
#[test]
fn third_party_prefix_is_exact() {
    for path in [
        "third_partyX/opencascade-sys/build.rs",
        "third-party/opencascade-sys/build.rs",
        "docs/third_party/opencascade-sys/build.rs",
        "third_party.rs",
    ] {
        assert_eq!(kind(path), Kind::Other, "{path}");
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

// ADR-0037, one source of truth for harness paths: `harness_files` takes the
// output of `git diff --name-only` (one path per line) and returns, in order,
// the lines that `is_harness` classifies as harness. Expected values from the
// HARNESS list: root files, directory prefixes, the guard binary; crate
// manifests, docs and `Cargo.lock` are not harness.
#[test]
fn harness_files_keeps_the_harness_lines_in_order() {
    let names = "Cargo.toml\ncrates/a.rs\nxtask/src/x.rs\ndocs/x.md\n.githooks/pre-push\n\
                 Cargo.lock\ntarget/debug/xtask\n";
    assert_eq!(
        harness_files(names),
        ["Cargo.toml", "xtask/src/x.rs", ".githooks/pre-push", "target/debug/xtask"]
    );
    let names = "harness/ratchet.txt\n.github/workflows/ci.yml\ncrates/qoob3d-core/Cargo.toml\n\
                 .claude/settings.json\nrust-toolchain.toml\ntarget/debug/deps/xtask-0123abcd\n\
                 .cargo/config.toml\nREADME.md";
    assert_eq!(
        harness_files(names),
        [
            "harness/ratchet.txt",
            ".github/workflows/ci.yml",
            ".claude/settings.json",
            "rust-toolchain.toml",
            "target/debug/deps/xtask-0123abcd",
            ".cargo/config.toml",
        ]
    );
}

// ADR-0037: no input, no harness line, blank lines → nothing; the last line
// counts without a trailing newline; a path is returned as given, also when
// `is_harness` matched it ignoring case (macOS, ADR-0043).
#[test]
fn harness_files_edge_cases() {
    let none: [&str; 0] = [];
    assert_eq!(harness_files(""), none);
    assert_eq!(harness_files("\n\n"), none);
    assert_eq!(harness_files("crates/a.rs\ndocs/x.md\nCargo.lock\n"), none);
    assert_eq!(harness_files("docs/x.md\nclippy.toml"), ["clippy.toml"]);
    assert_eq!(harness_files("\nCargo.toml\n\n"), ["Cargo.toml"]);
    assert_eq!(
        harness_files("CARGO.TOML\nXtask/src/main.rs\n"),
        ["CARGO.TOML", "Xtask/src/main.rs"]
    );
    // Agrees with `is_harness` line by line.
    let names = "Cargo.toml\nxtask\n.claudeX/foo\ntarget/debug/qoob3d\n.github/CODEOWNERS\n";
    let expected: Vec<&str> = names.lines().filter(|l| is_harness(l)).collect();
    assert_eq!(harness_files(names), expected);
    assert_eq!(expected, ["Cargo.toml", ".github/CODEOWNERS"]);
}

/// The lines of the step `- name: <name>` of a GitHub workflow: from its
/// `- name:` line up to the next line indented no deeper than its `-`.
fn workflow_step<'a>(workflow: &'a str, name: &str) -> Option<Vec<&'a str>> {
    let mut lines = workflow.lines();
    let first = lines.find(|l| l.trim() == format!("- name: {name}"))?;
    let indent = first.len() - first.trim_start().len();
    let rest = lines.take_while(|l| l.trim().is_empty() || l.len() - l.trim_start().len() > indent);
    Some(std::iter::once(first).chain(rest).collect())
}

// ADR-0037, one source of truth for harness paths: the CI step "Flag harness
// changes" asks xtask (`paths::is_harness`) which changed files are harness,
// `cargo xtask harness-changes <base> <head>` with the PR's base and head,
// and carries no hand-written path list (`grep -E '^(…)'`) of its own. It
// still reports into the step summary.
#[test]
fn ci_flags_harness_changes_through_xtask() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace");
    let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("ci.yml");
    let step =
        workflow_step(&workflow, "Flag harness changes").expect("step `Flag harness changes`");
    let text = step.join("\n");
    let call = step
        .iter()
        .find(|l| l.contains("cargo xtask harness-changes"))
        .unwrap_or_else(|| panic!("the step does not run `cargo xtask harness-changes`:\n{text}"));
    assert!(call.contains("$PR_BASE") && call.contains("$PR_HEAD"), "{call}");
    assert!(!text.contains("grep -E"), "the step carries its own path list:\n{text}");
    assert!(text.contains("GITHUB_STEP_SUMMARY"), "{text}");
}

// The step parser of the CI test above: it stops at the next step and at a
// shallower line, keeps blank lines inside the step, and finds nothing for an
// unknown name.
#[test]
fn workflow_step_parser() {
    let workflow = "jobs:\n  a:\n    steps:\n      - name: One\n        run: |\n          x\n\n          y\n      - name: Two\n        run: z\n  b:\n    steps: []\n";
    assert_eq!(
        workflow_step(workflow, "One"),
        Some(vec!["      - name: One", "        run: |", "          x", "", "          y"])
    );
    assert_eq!(workflow_step(workflow, "Two"), Some(vec!["      - name: Two", "        run: z"]));
    assert_eq!(workflow_step(workflow, "Three"), None);
}
