//! Script kernel hardening: docs/requirements/agent.md §6.1 AG-HD-02
//! (script size limits of declaration discovery) and AG-HD-06 (`import` with
//! a non-literal path), ADR-0053.
//!
//! AG-HD-02: declaration discovery (`PartMode::Declare`) runs with string
//! 65536 bytes, array 10000 elements, map 1000 entries; exceeding one is
//! `ScriptError::Limit`. Every other run keeps no script size limit (the
//! resident-memory limit of the worker protects those, AG-PJ-05).
//!
//! Test design: the requirement's example doubles a string 40 times
//! (16 B · 2^40 = 16 TiB). A test that runs this in the test process would
//! exhaust the machine as long as the limit is missing, so the kernel tests
//! use the smallest script that crosses each limit (at most a few hundred
//! kilobytes when nothing stops it). The 40-fold doubling runs against the
//! real service under an address-space cap in
//! `crates/qoob3d-app/tests/service.rs`.
//!
//! The declare runs need no OCCT: in `PartMode::Declare` the CAD API is not
//! registered (§2.1), the scripts here use plain Rhai only.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use qoob3d_kernel::script::{PartMode, RunOptions, RunOutput, ScriptError, run};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Options of declaration discovery as far as the kernel is concerned.
fn declare() -> RunOptions {
    RunOptions { parts: PartMode::Declare, ..RunOptions::default() }
}

fn run_ok(source: &str, options: &RunOptions) -> RunOutput {
    run(source, options)
        .unwrap_or_else(|e| panic!("script failed: {e:?}\n--- script ---\n{source}"))
}

/// Runs `source` in declare mode and expects `ScriptError::Limit` on `line`
/// within a second (crossing a size limit costs a handful of operations).
fn assert_declare_limit(source: &str, line: usize) {
    let start = Instant::now();
    let r = run(source, &declare());
    let elapsed = start.elapsed();
    match r {
        Err(ScriptError::Limit { line: at, message, .. }) => {
            assert_eq!(at, line, "position of the limit: {message}\n{source}");
            assert!(!message.is_empty());
        }
        other => panic!("expected Limit in declare mode, got {other:?}\n{source}"),
    }
    assert!(
        elapsed < Duration::from_secs(1),
        "the limit must stop the script quickly: {elapsed:?}"
    );
}

/// Printed line `i` as an integer.
fn printed(out: &RunOutput, i: usize) -> usize {
    out.printed[i].parse().unwrap_or_else(|e| panic!("line {i} {:?}: {e}", out.printed[i]))
}

/// A string of 16 bytes doubled `n` times: 16 · 2^n bytes. A part is
/// declared afterwards, as in a part file.
fn string_doubling(n: u32) -> String {
    format!(
        "let s = \"0123456789abcdef\";\n\
         for i in 0..{n} {{ s += s; }}\n\
         print(s.len());\n\
         part(\"Late\", #{{}}, |deps| 1);\n"
    )
}

/// An array of one element doubled `n` times: 2^n elements.
fn array_doubling(n: u32) -> String {
    format!("let a = [0];\nfor i in 0..{n} {{ a += a; }}\nprint(a.len());\n")
}

/// A map with `n` entries, added one by one.
fn map_filling(n: u32) -> String {
    format!("let m = #{{}};\nfor i in 0..{n} {{ m[`k${{i}}`] = i; }}\nprint(m.len());\n")
}

/// Fresh, deterministic scratch directory for one test.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("script-hardening").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

// ---------------------------------------------------------------------------
// AG-HD-02: size limits of declaration discovery
// ---------------------------------------------------------------------------

#[test]
fn ag_hd_02_declare_mode_stops_a_string_doubling_top_level_with_limit() {
    // 16 B · 2^13 = 131072 B > 65536 B. The loop is on line 2.
    assert_declare_limit(&string_doubling(13), 2);
    // The same with `s = s + s` (a new string instead of an append).
    assert_declare_limit("let s = \"0123456789abcdef\";\nfor i in 0..13 { s = s + s; }\n", 2);
}

#[test]
fn ag_hd_02_declare_mode_allows_a_string_of_exactly_the_limit() {
    // "exceeding one is Limit": 16 B · 2^12 = 65536 B is the limit itself.
    let out = run_ok(&string_doubling(12), &declare());
    assert_eq!(printed(&out, 0), 65536);
    assert_eq!(out.parts.len(), 1, "the part after the loop is declared");
    assert_eq!(out.parts[0].name, "Late");
}

#[test]
fn ag_hd_02_declare_mode_stops_arrays_above_10000_elements() {
    // 2^14 = 16384 elements > 10000; the doubling loop is on line 2. An
    // array bomb is as cheap as a string bomb: 40 doublings would be 2^40
    // elements.
    assert_declare_limit(&array_doubling(14), 2);
    // One element too many, by the library function that sizes an array.
    assert_declare_limit("let a = [];\na.pad(10001, 0);\nprint(a.len());\n", 2);
    // Grown one by one.
    assert_declare_limit("let a = [];\nfor i in 0..10001 { a.push(i); }\nprint(a.len());\n", 2);
}

#[test]
fn ag_hd_02_declare_mode_allows_an_array_of_exactly_the_limit() {
    let out = run_ok("let a = [];\na.pad(10000, 0);\nprint(a.len());\n", &declare());
    assert_eq!(printed(&out, 0), 10000);
    // 2^13 = 8192 elements by doubling stay below the limit.
    assert_eq!(printed(&run_ok(&array_doubling(13), &declare()), 0), 8192);
}

#[test]
fn ag_hd_02_declare_mode_stops_maps_above_1000_entries() {
    // The 1001st entry crosses the limit; the loop is on line 2.
    assert_declare_limit(&map_filling(1001), 2);
    let out = run_ok(&map_filling(1000), &declare());
    assert_eq!(printed(&out, 0), 1000, "exactly the limit is allowed");
}

#[test]
fn ag_hd_02_a_growth_loop_as_last_statement_is_limited_too() {
    // Nothing reads the value after the loop: the limit must not depend on
    // a later use of the variable (a part file may end with its bomb).
    assert_declare_limit("let s = \"0123456789abcdef\";\nfor i in 0..13 { s += s; }", 2);
    assert_declare_limit("let a = [0];\nfor i in 0..14 { a += a; }", 2);
    assert_declare_limit("let m = #{};\nfor i in 0..1001 { m[`k${i}`] = i; }", 2);
}

#[test]
fn ag_hd_02_other_run_modes_keep_no_script_size_limit() {
    // "Worker runs keep no script size limits": the scripts that declare
    // mode refuses run to their end in a plain run (`PartMode::All`, the
    // default). Sizes: 131072 B, 16384 elements, 1001 entries.
    let plain = RunOptions::default();
    let source = "let s = \"0123456789abcdef\";\nfor i in 0..13 { s += s; }\nprint(s.len());\n";
    assert_eq!(printed(&run_ok(source, &plain), 0), 131_072);
    assert_eq!(printed(&run_ok(&array_doubling(14), &plain), 0), 16384);
    assert_eq!(printed(&run_ok(&map_filling(1001), &plain), 0), 1001);
    // The existing memory-limit tests of the service rely on this: a part
    // closure holds 256 KiB strings and 1200-element arrays of them.
    let hog = "let s = \"0123456789abcdef\";\nfor i in 0..14 { s += s; }\n\
               let a = [];\nfor i in 0..40 { a.push(s + s); }\nprint(a.len());\n";
    assert_eq!(printed(&run_ok(hog, &plain), 0), 40);
}

#[test]
fn ag_hd_02_declare_mode_does_not_limit_what_part_closures_would_allocate() {
    // Discovery records the closure and never runs it (§2.1), so what the
    // closure would allocate in the worker is none of its business.
    let source = "part(\"Big\", #{}, |deps| {\n    \
                  let s = \"0123456789abcdef\";\n    for i in 0..40 { s += s; }\n    s\n});\n";
    let out = run_ok(source, &declare());
    assert_eq!(out.parts.len(), 1);
    assert_eq!(out.parts[0].name, "Big");
}

// ---------------------------------------------------------------------------
// AG-HD-06: import with a non-literal path
// ---------------------------------------------------------------------------

/// A project with the module `lib/x.rhai` (`X = 42`).
fn project_with_module(test: &str) -> RunOptions {
    let project = scratch(test);
    std::fs::create_dir_all(project.join("lib")).expect("create lib/");
    std::fs::write(project.join("lib/x.rhai"), "export const X = 42;\n").expect("write module");
    RunOptions { project_dir: Some(project), ..RunOptions::default() }
}

/// Line and column of a script error of any kind.
fn position(e: &ScriptError) -> (usize, usize) {
    let (ScriptError::Syntax { line, column, .. }
    | ScriptError::Runtime { line, column, .. }
    | ScriptError::Geometry { line, column, .. }
    | ScriptError::Limit { line, column, .. }) = e;
    (*line, *column)
}

#[test]
fn ag_hd_06_literal_import_still_works() {
    // The premise of the next test: the module exists and loads.
    let options = project_with_module("import_literal");
    let out = run_ok("import \"lib/x\" as x;\nprint(x::X);\n", &options);
    assert_eq!(out.printed, ["42"]);
}

#[test]
fn ag_hd_06_import_with_a_non_literal_path_is_an_error_at_its_position() {
    // "`import` with a non-literal path is refused with a script error at
    // its position": the scan that feeds the cache key (AG-PJ-04) reads
    // literal paths only, so a computed path would load a module the key
    // does not know. The module exists, so only this rule can refuse it.
    //
    // Position: line of the import statement. The column is that of the
    // statement (1-based column of `import`) or of its path expression,
    // 7 characters further (`import ` is 7 long); both name the place.
    let options = project_with_module("import_variable");
    for (source, line, import_column) in [
        ("let m = \"lib/x\";\nimport m as x;\nprint(x::X);\n", 2, 1),
        // A computed path.
        ("let d = \"lib\";\n\n  import d + \"/x\" as x;\nprint(x::X);\n", 3, 3),
        // A path from a function.
        ("fn name() { \"lib/x\" }\nimport name() as x;\n", 2, 1),
        // Inside a block, as a part closure has it.
        (
            "let f = |d| {\n    let m = \"lib/x\";\n    import m as x;\n    x::X\n};\nf.call(1);\n",
            3,
            5,
        ),
    ] {
        let e = match run(source, &options) {
            Ok(out) => {
                panic!("a non-literal import must fail, printed {:?}\n{source}", out.printed)
            }
            Err(e) => e,
        };
        let (at, column) = position(&e);
        assert_eq!(at, line, "{e:?}\n{source}");
        assert!(
            column == import_column || column == import_column + 7,
            "column {column}, expected {import_column} or {}: {e:?}\n{source}",
            import_column + 7
        );
    }
}

#[test]
fn ag_hd_06_non_literal_import_is_refused_in_declare_mode_too() {
    // Discovery evaluates the top level of a part file: the same rule.
    let mut options = project_with_module("import_variable_declare");
    options.parts = PartMode::Declare;
    let source = "let m = \"lib/x\";\nimport m as x;\npart(\"P\", #{}, |deps| x::X);\n";
    let e = run(source, &options).expect_err("a non-literal import must fail");
    assert_eq!(position(&e).0, 2, "{e:?}");
}

// ---------------------------------------------------------------------------
// ADR-0055: the size measurement of declare mode costs linear time
// ---------------------------------------------------------------------------

/// Loop iterations of [`counting`] that cost about 900 000 operations in
/// declare mode (4 per iteration there; 250 000 iterations reach the
/// 1 000 000 of AG-HD-02). `discovery_counting_premise` checks the bounds
/// that do not depend on how declare mode counts.
const LONG_TOP_LEVEL: u32 = 225_000;

/// A top level of `n` cheap loop iterations; the loop is on line 2.
fn counting(n: u32) -> String {
    format!("let x = 0;\nfor i in 0..{n} {{ x += 1; }}\n")
}

/// Debug-build bound of ADR-0055 for a top level of about a million
/// operations. Plain evaluation of that many operations takes well under a
/// second in a debug build; the quadratic staged measurement (a measurement
/// every 4096 operations) needed 3.7 s for this loop and more than the 20 s
/// start-up budget of the service for a file that runs into the limit.
const LINEAR_BOUND: Duration = Duration::from_secs(3);

#[test]
fn discovery_counting_premise() {
    // The premise of the tests below: the counting loop is a long top
    // level (more than 600 000 operations even by the cheaper count of a
    // plain run, measured with its operation limit) and it fits into the
    // 1 000 000 operations of declaration discovery (it runs to its end).
    let source = counting(LONG_TOP_LEVEL);
    let short = RunOptions { max_operations: 600_000, ..RunOptions::default() };
    let r = run(&source, &short);
    assert!(matches!(r, Err(ScriptError::Limit { .. })), "fewer than 600 000 operations: {r:?}");
    run_ok(&source, &declare());
}

#[test]
fn discovery_a_long_top_level_without_violation_finishes_in_linear_time() {
    // ADR-0055: "The measuring stages double (4096, 8192, …), so the total
    // cost is linear in the operations of the file." About 900 000
    // operations and no size violation: the part after the loop is
    // declared, within 3 s.
    let source = format!("{}part(\"Late\", #{{}}, |deps| 1);\n", counting(LONG_TOP_LEVEL));
    let start = Instant::now();
    let out = run_ok(&source, &declare());
    let elapsed = start.elapsed();
    assert_eq!(out.parts.len(), 1, "the part after the loop is declared");
    assert!(elapsed < LINEAR_BOUND, "declare mode took {elapsed:?} for about 900 000 operations");
}

#[test]
fn discovery_a_violation_after_a_long_top_level_is_reported_in_linear_time() {
    // The same loop, then a string that crosses 65536 bytes on line 4
    // (16 B * 2^13, a few dozen operations more). The loop alone runs to
    // its end (premise), so line 4 can only be the size limit; it must be
    // reported within 3 s.
    let source = format!(
        "{}let s = \"0123456789abcdef\";\nfor i in 0..13 {{ s += s; }}\nprint(s.len());\n",
        counting(LONG_TOP_LEVEL)
    );
    let start = Instant::now();
    let r = run(&source, &declare());
    let elapsed = start.elapsed();
    match r {
        Err(ScriptError::Limit { line, message, .. }) => {
            assert_eq!(line, 4, "position of the limit: {message}");
        }
        other => panic!("expected Limit in declare mode, got {other:?}"),
    }
    assert!(elapsed < LINEAR_BOUND, "the size limit was reported after {elapsed:?}");
}

#[test]
fn discovery_a_violation_early_in_a_long_top_level_is_still_found() {
    // Doubling stages must not lose a violation: the array passes 10 000
    // elements after 10 001 of 100 000 iterations and keeps its 20 000
    // elements to the end of the loop on line 2. The same loop without the
    // growth runs to its end, so the `Limit` is the size limit.
    let looping = |body: &str| {
        format!(
            "let a = [];\nfor i in 0..100000 {{ if i < 20000 {{ {body} }} }}\nprint(a.len());\n"
        )
    };
    run_ok(&looping("a = [i];"), &declare());
    let start = Instant::now();
    let r = run(&looping("a += [i];"), &declare());
    let elapsed = start.elapsed();
    assert!(matches!(r, Err(ScriptError::Limit { line: 2, .. })), "{r:?}");
    assert!(elapsed < LINEAR_BOUND, "the size limit was reported after {elapsed:?}");
}
