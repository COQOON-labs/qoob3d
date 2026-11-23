//! Shared helpers of the script kernel tests `tests/script.rs`,
//! `tests/script_helpers.rs` and `tests/script_perf.rs` (ADR-0053): run a
//! script, find a shown shape, compare numbers and bounding boxes.
//!
//! Included with `mod common;`. Every item here is used by each of those
//! test files, so no test binary sees dead code; what only `script.rs` and
//! `script_helpers.rs` share is in `common/solid.rs`.
//!
//! Tolerances: `VOL` = 1e-6 relative for volumes (exact or numerically
//! integrated B-reps, M0 measured < 1e-12), `POS` = 1e-6 mm absolute for
//! bounding boxes and points (AG-HD-11: tight).

use qoob3d_kernel::script::{RunOptions, RunOutput, ScriptError, Shape, run};

pub(crate) const VOL: f64 = 1e-6;
pub(crate) const POS: f64 = 1e-6;

/// The script made of `lines`.
pub(crate) fn src(lines: &[&str]) -> String {
    lines.join("\n")
}

pub(crate) fn run_ok(source: &str) -> RunOutput {
    run(source, &RunOptions::default())
        .unwrap_or_else(|e| panic!("script failed: {e:?}\n--- script ---\n{source}"))
}

pub(crate) fn run_err(source: &str) -> ScriptError {
    match run(source, &RunOptions::default()) {
        Ok(out) => panic!(
            "script must fail but returned {} shapes, printed {:?}\n{source}",
            out.shapes.len(),
            out.printed
        ),
        Err(e) => e,
    }
}

pub(crate) fn shape<'a>(out: &'a RunOutput, name: &str) -> &'a Shape {
    &out.shapes
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| {
            panic!(
                "no shape {name:?}, have {:?}",
                out.shapes.iter().map(|s| &s.0).collect::<Vec<_>>()
            )
        })
        .1
}

pub(crate) fn assert_rel(actual: f64, expected: f64, tol: f64, what: &str) {
    let rel = (actual - expected).abs() / expected.abs();
    assert!(rel < tol, "{what}: got {actual}, expected {expected} (rel {rel:e} >= {tol:e})");
}

pub(crate) fn assert_bbox(out: &RunOutput, name: &str, min: [f64; 3], max: [f64; 3]) {
    let (a, b) = shape(out, name).bbox();
    for i in 0..3 {
        assert!(
            (a[i] - min[i]).abs() < POS && (b[i] - max[i]).abs() < POS,
            "bbox of {name}: got {a:?}..{b:?}, expected {min:?}..{max:?}"
        );
    }
}

/// Printed line `i` as a number (Rhai prints floats round-trip exact).
pub(crate) fn printed(out: &RunOutput, i: usize) -> f64 {
    let line =
        out.printed.get(i).unwrap_or_else(|| panic!("no printed line {i}: {:?}", out.printed));
    line.trim().parse().unwrap_or_else(|e| panic!("printed line {i} {line:?} is no number: {e}"))
}
