//! R1 acceptance (docs/requirements/agent.md §7): the three M0 probe parts
//! (crane column, stair, railing; `docs/concept/m0-report.md`), written as
//! Rhai scripts in `tests/scripts/*.rhai`, reproduce their analytic target
//! volumes to < 1e-6 relative, and build within the AG-SC-15 time budget.
//!
//! The formulas below are the analytic target volumes of the M0 probe: sums
//! of cylinders, prisms, Pappus rings and pipe lengths, each derived at its
//! function. They use only the part dimensions, never the kernel. M0 measured
//! the same parts in OCCT with a deviation < 1e-12
//! (`docs/concept/m0-report.md` §1), so 1e-6 is a wide margin that still
//! catches any missing or extra feature (the smallest feature, one pin
//! chamfer ring, is ~0.5 mm³ = 2e-4 of the crane column).

use std::f64::consts::PI;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use qoob3d_kernel::script::{RunOptions, RunOutput, run};

/// R1 acceptance tolerance (agent.md §7).
const REL_TOL: f64 = 1e-6;

fn script_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scripts").join(name)
}

/// Runs a script file and returns the output and the wall time of `run`.
fn run_file(name: &str) -> (RunOutput, Duration) {
    let path = script_path(name);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let start = Instant::now();
    let out =
        run(&source, &RunOptions::default()).unwrap_or_else(|e| panic!("{name} failed: {e:?}"));
    (out, start.elapsed())
}

fn assert_rel(actual: f64, expected: f64, tol: f64, what: &str) {
    let rel = (actual - expected).abs() / expected.abs();
    assert!(rel < tol, "{what}: {actual} mm³, expected {expected} mm³ (rel {rel:e} >= {tol:e})");
}

/// AG-SC-15: < 1 s per part in release builds. Debug builds of the script
/// layer are much slower than release (OCCT itself is built optimised in
/// `target/OCCT`), so a debug run only has to stay below a generous bound
/// that still catches a hang or an accidental per-element boolean loop.
fn assert_build_time(name: &str, elapsed: Duration) {
    let bound =
        if cfg!(debug_assertions) { Duration::from_secs(10) } else { Duration::from_secs(1) };
    assert!(elapsed < bound, "{name} took {elapsed:?}, bound {bound:?} (AG-SC-15)");
}

// ---------------------------------------------------------------------------
// Crane column
// ---------------------------------------------------------------------------

/// Pappus: a right isosceles triangle with legs `c` in the corner of a turned
/// edge at radius `r` has area c²/2 and its centroid at radius r − c/3, so the
/// chamfer ring removes 2π (r − c/3) c²/2 (centroid radius times area).
fn chamfer_ring(r: f64, c: f64) -> f64 {
    2.0 * PI * (r - c / 3.0) * c * c / 2.0
}

/// Target volume of the crane column: column Ø13 × 20 with two 0.5 chamfer
/// rings, pin Ø4 × 2.45 with one 0.3 chamfer ring, minus the 4.4 × 5.4 × 4.2
/// pocket (the pocket corners lie at radius 3.48 < 6.0, inside the chamfer).
fn crane_column_volume() -> f64 {
    let r = 13.0 / 2.0;
    let rz = 4.0 / 2.0;
    let mut v = PI * r * r * 20.0 - 2.0 * chamfer_ring(r, 0.5);
    v += PI * rz * rz * 2.45 - chamfer_ring(rz, 0.3);
    v -= 4.4 * 5.4 * 4.2;
    v
}

#[test]
fn crane_column_reference_volume_matches_the_python_port() {
    // Guards the formula itself: the value the M0 probe computed for the
    // same dimensions (docs/concept/m0-report.md).
    assert!((crane_column_volume() - 2575.155811208424).abs() < 1e-9);
}

// R1 acceptance, AG-SC-04/05/09/13/15
#[test]
fn crane_column_script_reproduces_the_analytic_volume() {
    let (out, elapsed) = run_file("crane_column.rhai");
    assert_eq!(out.shapes.len(), 1);
    let (name, shape) = &out.shapes[0];
    assert_eq!(name, "crane_column");
    assert!(shape.is_valid());
    assert_eq!(shape.solid_count(), 1, "pin and column are one body");
    assert_rel(shape.volume(), crane_column_volume(), REL_TOL, "crane column");
    // Ø13 column from z = 0 to 20, pin down to z = −2.45.
    let (min, max) = shape.bbox();
    for (a, e) in min.iter().zip([-6.5, -6.5, -2.45]).chain(max.iter().zip([6.5, 6.5, 20.0])) {
        assert!((a - e).abs() < 1e-6, "bbox {min:?} {max:?}");
    }
    assert_build_time("crane column", elapsed);
}

// R1 acceptance, AG-SC-07 (revolve of a polygon profile)
#[test]
fn crane_column_as_revolved_profile_reproduces_the_analytic_volume() {
    let (out, elapsed) = run_file("crane_column_revolve.rhai");
    assert_eq!(out.shapes.len(), 1);
    let shape = &out.shapes[0].1;
    assert!(shape.is_valid());
    assert_eq!(shape.solid_count(), 1);
    assert_rel(shape.volume(), crane_column_volume(), REL_TOL, "revolved crane column");
    assert_build_time("revolved crane column", elapsed);
}

// ---------------------------------------------------------------------------
// Stair
// ---------------------------------------------------------------------------

/// Target volume of the stair, as the sum of its features:
///
/// - Stringers: an inclined strip with cross-section T × B, cut by z = 0 and
///   z = H. Its horizontal section is T × B / sin(w), so V = T·H·B / sin(w),
///   twice.
/// - Step: 16.8 × 5 × 0.8 minus two through slots 12 × 0.6 × 0.8. The step
///   ends only touch the stringer faces (x = ±8.4), so volumes add.
/// - Eye: disc of radius r minus the circular segment inside the stringer
///   (chord at distance d = r − overlap from the centre: area
///   r² acos(d/r) − d √(r² − d²)) minus the bore, times the height. The
///   bore (x ≥ 10.15) does not reach the stringer (x ≤ 10).
fn stair_volume() -> f64 {
    let w = 52.4_f64.to_radians();
    let stringers = 2.0 * 1.6 * 65.0 * 5.0 / w.sin();
    let inner = 20.0 - 2.0 * 1.6;
    let step = inner * 5.0 * 0.8 - 2.0 * 12.0 * 0.6 * 0.8;
    let r: f64 = 2.2 / 2.0;
    let d = r - 0.5;
    let segment = r * r * (d / r).acos() - d * (r * r - d * d).sqrt();
    let eye = (PI * r * r - segment - PI * (0.9_f64 / 2.0).powi(2)) * 2.0;
    stringers + 11.0 * step + 2.0 * 3.0 * eye
}

#[test]
fn stair_reference_volume_matches_the_python_port() {
    assert!((stair_volume() - 1955.3202693815988).abs() < 1e-9);
}

// R1 acceptance, AG-SC-05 (batched booleans), AG-SC-06, AG-SC-15
#[test]
fn stair_script_reproduces_the_analytic_volume() {
    let (out, elapsed) = run_file("stair.rhai");
    assert_eq!(out.shapes.len(), 1);
    let (name, shape) = &out.shapes[0];
    assert_eq!(name, "stair");
    assert!(shape.is_valid());
    // Steps touch the stringers over their full end face, the eyes overlap
    // them: one connected body.
    assert_eq!(shape.solid_count(), 1);
    assert_rel(shape.volume(), stair_volume(), REL_TOL, "stair");
    // The stringers are cut exactly at z = 0 (nothing else reaches lower:
    // step 1 starts at 5.91 − 0.8). The top is the top face of step 11 at
    // z = 11 · 5.91 = 65.01, 0.01 above the stringer cut at 65 (step k has
    // its top face at k · rise; the analytic volume counts all 11 full
    // steps). Outer width
    // 20 plus the eyes: x = ±(10 + 1.1 − 0.5 + 1.1) = ±11.7.
    let top = 11.0 * 5.91;
    let (min, max) = shape.bbox();
    assert!(min[2].abs() < 1e-6 && (max[2] - top).abs() < 1e-6, "{min:?} {max:?}");
    assert!((min[0] + 11.7).abs() < 1e-6 && (max[0] - 11.7).abs() < 1e-6, "{min:?} {max:?}");
    // AG-SC-15 release bound: < 1 s; script overhead < 5 % of the OCCT time
    // needs a profiling seam (see report), not checked here.
    assert_build_time("stair", elapsed);
}

// ---------------------------------------------------------------------------
// Railing
// ---------------------------------------------------------------------------

/// Target volume of the railing from the centre line lengths of its wires:
/// the volume of a pipe is circle area × centre line length (Pappus), exact
/// for straight and toroidal sections.
///
/// Handrail: two legs from z = −2 to the bend start at 22 − 1.5 (22.5 each),
/// two quarter bends π/2 · 1.5, the top run 96 − 2 · 1.5. Inner posts: 3 ×
/// 24. Rails: 2 × 96 at Ø0.5.
fn railing_volume() -> f64 {
    let length = 4.0 * 24.0;
    let rb = 1.5;
    let legs = 2.0 * (22.0 - rb + 2.0);
    let handrail = legs + 2.0 * (PI / 2.0 * rb) + (length - 2.0 * rb);
    let posts = 3.0 * (22.0 + 2.0);
    let l08 = handrail + posts;
    let l05 = 2.0 * length;
    PI / 4.0 * (0.8 * 0.8 * l08 + 0.5 * 0.5 * l05)
}

#[test]
fn railing_reference_volume_matches_the_python_port() {
    assert!((railing_volume() - 145.625330059956).abs() < 1e-9);
}

// R1 acceptance, AG-SC-08, AG-SC-15
#[test]
fn railing_script_reproduces_the_analytic_volume() {
    let (out, elapsed) = run_file("railing.rhai");
    let names: Vec<&str> = out.shapes.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["handrail", "post_1", "post_2", "post_3", "rail_8.0", "rail_15.0"],
        "every wire is its own body, in call order (AG-SC-01)"
    );
    for (name, shape) in &out.shapes {
        assert!(shape.is_valid(), "{name}");
        assert_eq!(shape.solid_count(), 1, "{name}");
    }
    let total: f64 = out.shapes.iter().map(|(_, s)| s.volume()).sum();
    assert_rel(total, railing_volume(), REL_TOL, "railing");
    // Handrail alone: Ø0.8 along 2·22.5 + 2·(π/2·1.5) + 93.
    let handrail = PI / 4.0 * 0.64 * (45.0 + 1.5 * PI + 93.0);
    assert_rel(out.shapes[0].1.volume(), handrail, REL_TOL, "handrail");
    assert_build_time("railing", elapsed);
}
