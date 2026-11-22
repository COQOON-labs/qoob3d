//! Performance package P1, kernel side (docs/requirements/agent.md §5):
//! AG-PF-01 (batched booleans in parallel mode keep their results) and
//! AG-PF-02 (profile algebra, the 1000-hole plate and its speed-up).
//!
//! Every expected value is derived by hand from the dimensions in the script;
//! the derivation stands next to the assertion.
//!
//! Tolerances: `VOL` = 1e-6 relative is the margin of the R1 tests
//! (`tests/script.rs`) and the one AG-PF-02 names. `SAME` = 1e-9 relative is
//! the bound AG-PF-01 names; prisms and cylinders are exact B-reps whose
//! volume OCCT integrates to rounding (M0 measured < 1e-12), so the analytic
//! value itself is checked to `SAME` there. Bounding boxes: `POS` = 1e-6 mm.
//!
//! Decision of this test file (not named by the requirement, see the report):
//! a profile is placed in its plane with `profile.move(x, y)`, the 2D
//! analogue of `shape.move(x, y, z)` (AG-SC-06). Without it a circle cannot
//! leave the origin and the plate of AG-PF-02 cannot be written. Only the
//! tests that need a moved circle use it; the others offset with `polygon`.
//!
//! All tests of this file take [`serial`]: one of them compares wall clock
//! times, and a neighbour test running OCCT on the other cores would distort
//! the ratio.

use std::f64::consts::PI;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use qoob3d_kernel::script::{RunOptions, RunOutput, ScriptError, Shape, run};

const VOL: f64 = 1e-6;
const SAME: f64 = 1e-9;
const POS: f64 = 1e-6;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static SERIAL: Mutex<()> = Mutex::new(());

/// One test at a time (timing, see the module comment). A failed test must
/// not fail the others through the poisoned lock.
fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn src(lines: &[&str]) -> String {
    lines.join("\n")
}

fn run_ok(source: &str) -> RunOutput {
    run(source, &RunOptions::default())
        .unwrap_or_else(|e| panic!("script failed: {e:?}\n--- script ---\n{source}"))
}

fn run_err(source: &str) -> ScriptError {
    match run(source, &RunOptions::default()) {
        Ok(out) => panic!(
            "script must fail but returned {} shapes, printed {:?}\n{source}",
            out.shapes.len(),
            out.printed
        ),
        Err(e) => e,
    }
}

fn shape<'a>(out: &'a RunOutput, name: &str) -> &'a Shape {
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

fn assert_rel(actual: f64, expected: f64, tol: f64, what: &str) {
    let rel = (actual - expected).abs() / expected.abs();
    assert!(rel < tol, "{what}: got {actual}, expected {expected} (rel {rel:e} >= {tol:e})");
}

/// Valid, `solids` solids, volume to `tol` relative.
fn assert_solid(out: &RunOutput, name: &str, expected: f64, solids: usize, tol: f64) {
    let s = shape(out, name);
    assert!(s.is_valid(), "{name} is not a valid solid");
    assert_eq!(s.solid_count(), solids, "solids of {name}");
    assert_rel(s.volume(), expected, tol, &format!("volume of {name}"));
}

fn assert_bbox(out: &RunOutput, name: &str, min: [f64; 3], max: [f64; 3]) {
    let (a, b) = shape(out, name).bbox();
    for i in 0..3 {
        assert!(
            (a[i] - min[i]).abs() < POS && (b[i] - max[i]).abs() < POS,
            "bbox of {name}: got {a:?}..{b:?}, expected {min:?}..{max:?}"
        );
    }
}

/// Printed line `i` as a number (Rhai prints floats round-trip exact).
fn printed(out: &RunOutput, i: usize) -> f64 {
    let line =
        out.printed.get(i).unwrap_or_else(|| panic!("no printed line {i}: {:?}", out.printed));
    line.trim().parse().unwrap_or_else(|e| panic!("printed line {i} {line:?} is no number: {e}"))
}

/// Unit-free square polygon `[x0, x1] × [y0, y1]` as script text.
fn quad(x0: f64, y0: f64, x1: f64, y1: f64) -> String {
    format!("polygon([[{x0}, {y0}], [{x1}, {y0}], [{x1}, {y1}], [{x0}, {y1}]])")
}

// ---------------------------------------------------------------------------
// AG-PF-01: batched booleans keep their results
// ---------------------------------------------------------------------------
//
// The parallel mode itself is not observable from outside (no seam to query
// it, see the report). What the requirement demands of the results is: the
// batched operation, the same operation done one by one, a repeated run and
// the analytic value agree to 1e-9 relative.

#[test]
fn ag_pf_01_batched_cut_equals_one_by_one_cut_and_the_analytic_volume() {
    let _serial = serial();
    // Plate 60 × 40 × 5 with 6 × 4 through holes of diameter 4 on a 10 mm
    // grid (x = −25 + 10 i, y = −15 + 10 j; 3 mm of material to the rim).
    // The cylinders (h = 9, from z = −2 to 7) stick out on both sides, so no
    // faces coincide. V = 60·40·5 − 24 · π · 2² · 5 = 12000 − 480π.
    let holes = [
        "let holes = [];",
        "for i in 0..6 { for j in 0..4 {",
        "    holes.push(cylinder(4, 9).move(-25 + 10 * i, -15 + 10 * j, -2));",
        "} }",
    ];
    let source = src(&[
        &src(&holes),
        "show(cut(box(60, 40, 5), holes), \"batched\");",
        "let p = box(60, 40, 5);",
        "for h in holes { p = p - h; }",
        "show(p, \"operators\");",
        "let q = box(60, 40, 5);",
        "for h in holes { q = cut(q, [h]); }",
        "show(q, \"single\");",
    ]);
    let expected = 12_000.0 - 480.0 * PI;
    let first = run_ok(&source);
    for name in ["batched", "operators", "single"] {
        assert_solid(&first, name, expected, 1, SAME);
        assert_bbox(&first, name, [-30.0, -20.0, 0.0], [30.0, 20.0, 5.0]);
    }
    let batched = shape(&first, "batched").volume();
    assert_rel(shape(&first, "operators").volume(), batched, SAME, "one by one (-) vs batched");
    assert_rel(shape(&first, "single").volume(), batched, SAME, "one by one (cut) vs batched");
    // A second run gives the same result: parallel execution must not make
    // the outcome depend on scheduling.
    let again = run_ok(&source);
    assert_rel(shape(&again, "batched").volume(), batched, SAME, "second run vs first run");
}

#[test]
fn ag_pf_01_batched_union_equals_pairwise_union_and_the_analytic_volume() {
    let _serial = serial();
    // 12 cubes of 10 mm shifted by 5 mm along X overlap into one bar from
    // x = −5 to 11·5 + 5 = 60: V = 65 · 10 · 10 = 6500.
    let out = run_ok(&src(&[
        "let parts = [];",
        "for i in 0..12 { parts.push(box(10, 10, 10).move(5 * i, 0, 0)); }",
        "show(union(parts), \"batched\");",
        "let u = parts[0];",
        "for i in 1..12 { u = u + parts[i]; }",
        "show(u, \"pairwise\");",
    ]));
    for name in ["batched", "pairwise"] {
        assert_solid(&out, name, 6500.0, 1, SAME);
        assert_bbox(&out, name, [-5.0, -5.0, 0.0], [60.0, 5.0, 10.0]);
    }
    assert_rel(
        shape(&out, "pairwise").volume(),
        shape(&out, "batched").volume(),
        SAME,
        "pairwise vs batched union",
    );
}

#[test]
fn ag_pf_01_batched_union_of_curved_bodies_equals_pairwise_union() {
    let _serial = serial();
    // Three spheres of radius R = 8 at x = 0, 10, 20. Neighbours (distance
    // d = 10) overlap in a lens of volume π (4R + d)(2R − d)² / 12
    // = π · 42 · 36 / 12 = 126π (check: d = 0 gives 4/3 π R³); the outer two
    // (distance 20 > 2R) are disjoint.
    // V = 3 · 4/3 π · 512 − 2 · 126π = 2048π − 252π = 1796π.
    let out = run_ok(&src(&[
        "let parts = [];",
        "for i in 0..3 { parts.push(sphere(16).move(10 * i, 0, 0)); }",
        "show(union(parts), \"batched\");",
        "show(parts[0] + parts[1] + parts[2], \"pairwise\");",
    ]));
    // Spheres are integrated numerically: analytic value to VOL, the two
    // routes against each other to the 1e-9 of the requirement.
    for name in ["batched", "pairwise"] {
        assert_solid(&out, name, 1796.0 * PI, 1, VOL);
        assert_bbox(&out, name, [-8.0, -8.0, -8.0], [28.0, 8.0, 8.0]);
    }
    assert_rel(
        shape(&out, "pairwise").volume(),
        shape(&out, "batched").volume(),
        SAME,
        "pairwise vs batched union of spheres",
    );
}

// ---------------------------------------------------------------------------
// AG-PF-02: profile algebra, small analytic cases
// ---------------------------------------------------------------------------

#[test]
fn ag_pf_02_rect_minus_circle_extruded() {
    let _serial = serial();
    let out = run_ok(&src(&[
        "let s = extrude(rect(20, 10) - circle(4), 3);",
        "show(s, \"plate\");",
        "print(s.area());",
    ]));
    // V = (w h − π r²) t = (200 − 4π) · 3.
    assert_solid(&out, "plate", (200.0 - 4.0 * PI) * 3.0, 1, VOL);
    assert_bbox(&out, "plate", [-10.0, -5.0, 0.0], [10.0, 5.0, 3.0]);
    // The hole is a real cylindrical face: area = top + bottom + rim + wall
    // = 2 (200 − 4π) + 2 (20 + 10) · 3 + 2π · 2 · 3 = 580 + 4π.
    assert_rel(printed(&out, 0), 580.0 + 4.0 * PI, VOL, "area of the plate");
}

#[test]
fn ag_pf_02_operators_on_overlapping_squares() {
    let _serial = serial();
    // a = [0, 10]², b = [5, 15]²: overlap [5, 10]² = 25.
    let (a, b) = (quad(0.0, 0.0, 10.0, 10.0), quad(5.0, 5.0, 15.0, 15.0));
    let out = run_ok(&src(&[
        &format!("let a = {a};"),
        &format!("let b = {b};"),
        "show(extrude(a + b, 2), \"union\");",
        "show(extrude(a - b, 2), \"cut\");",
        "show(extrude(a & b, 2), \"common\");",
        "show(extrude(a, 2), \"a\");",
    ]));
    // Areas: union 100 + 100 − 25 = 175, cut 100 − 25 = 75, common 25; t = 2.
    assert_solid(&out, "union", 350.0, 1, VOL);
    assert_bbox(&out, "union", [0.0, 0.0, 0.0], [15.0, 15.0, 2.0]);
    assert_solid(&out, "cut", 150.0, 1, VOL);
    assert_bbox(&out, "cut", [0.0, 0.0, 0.0], [10.0, 10.0, 2.0]);
    assert_solid(&out, "common", 50.0, 1, VOL);
    assert_bbox(&out, "common", [5.0, 5.0, 0.0], [10.0, 10.0, 2.0]);
    // The operands are values: `a` is unchanged by the operations.
    assert_solid(&out, "a", 200.0, 1, VOL);
}

#[test]
fn ag_pf_02_operators_on_rects_and_circles() {
    let _serial = serial();
    let out = run_ok(&src(&[
        "show(extrude(rect(10, 10) + rect(20, 4), 1), \"cross\");",
        "show(extrude(rect(10, 10) & circle(12), 1), \"rounded\");",
        "show(extrude(circle(20) - rect(4, 4), 5), \"washer\");",
    ]));
    // Cross: 100 + 80 − overlap 10 · 4 = 140.
    assert_solid(&out, "cross", 140.0, 1, VOL);
    assert_bbox(&out, "cross", [-10.0, -5.0, 0.0], [10.0, 5.0, 1.0]);
    // Square [−5, 5]² ∩ disc r = 6: the disc loses four segments beyond the
    // chords at distance 5, each r² acos(5/6) − 5 √(36 − 25):
    // A = 36π − 4 (36 acos(5/6) − 5 √11).
    let segment = 36.0 * (5.0f64 / 6.0).acos() - 5.0 * 11f64.sqrt();
    assert_solid(&out, "rounded", 36.0 * PI - 4.0 * segment, 1, VOL);
    assert_bbox(&out, "rounded", [-5.0, -5.0, 0.0], [5.0, 5.0, 1.0]);
    // Disc r = 10 with a square hole 4 × 4: (100π − 16) · 5.
    assert_solid(&out, "washer", (100.0 * PI - 16.0) * 5.0, 1, VOL);
}

#[test]
fn ag_pf_02_cut_can_split_a_profile_into_two_regions() {
    let _serial = serial();
    // rect(20, 10) minus a strip 4 wide through the middle leaves two
    // regions 8 × 10: the extrusion has two solids, V = 2 · 80 · 1.
    let out = run_ok("show(extrude(rect(20, 10) - rect(4, 20), 1), \"halves\");");
    assert_solid(&out, "halves", 160.0, 2, VOL);
    assert_bbox(&out, "halves", [-10.0, -5.0, 0.0], [10.0, 5.0, 1.0]);
}

#[test]
fn ag_pf_02_profile_cut_with_an_array_of_holes() {
    let _serial = serial();
    // Four square holes 2 × 2 at (±5, ±5) in rect(20, 20): (400 − 16) · 5.
    let holes: Vec<String> = [(-6.0, -6.0), (4.0, -6.0), (-6.0, 4.0), (4.0, 4.0)]
        .iter()
        .map(|(x, y)| quad(*x, *y, x + 2.0, y + 2.0))
        .collect();
    let out = run_ok(&src(&[
        &format!("let holes = [{}];", holes.join(", ")),
        "let s = extrude(profile_cut(rect(20, 20), holes), 5);",
        "show(s, \"plate\");",
        "print(s.area());",
        // The same as a chain of operators.
        "let p = rect(20, 20);",
        "for h in holes { p = p - h; }",
        "show(extrude(p, 5), \"chain\");",
    ]));
    assert_solid(&out, "plate", 1920.0, 1, VOL);
    assert_bbox(&out, "plate", [-10.0, -10.0, 0.0], [10.0, 10.0, 5.0]);
    // Area: 2 (400 − 16) + rim 80 · 5 + four hole walls 8 · 5 = 768 + 400 + 160.
    assert_rel(printed(&out, 0), 1328.0, VOL, "area with four holes");
    assert_solid(&out, "chain", 1920.0, 1, VOL);
}

#[test]
fn ag_pf_02_profile_union_of_an_array() {
    let _serial = serial();
    // Five squares of 10 shifted by 5 along X: one bar [0, 30] × [0, 10],
    // extruded by 10 → 3000 (the 2D twin of ag_sc_05_batched_union_…).
    let squares: Vec<String> =
        (0..5).map(|i| quad(5.0 * f64::from(i), 0.0, 5.0 * f64::from(i) + 10.0, 10.0)).collect();
    let out = run_ok(&src(&[
        &format!("let squares = [{}];", squares.join(", ")),
        "show(extrude(profile_union(squares), 10), \"bar\");",
        "show(extrude(profile_union([rect(4, 4)]), 2), \"one\");",
    ]));
    assert_solid(&out, "bar", 3000.0, 1, VOL);
    assert_bbox(&out, "bar", [0.0, 0.0, 0.0], [30.0, 10.0, 10.0]);
    // A union of one profile is that profile: 4 · 4 · 2.
    assert_solid(&out, "one", 32.0, 1, VOL);
}

#[test]
fn ag_pf_02_profiles_with_holes_are_operands_too() {
    let _serial = serial();
    // "Holes allowed": a profile that already has a hole takes part in
    // further operations. ring = rect(30, 20) − circle(12): 600 − 36π.
    let out = run_ok(&src(&[
        "let ring = rect(30, 20) - circle(12);",
        &format!("show(extrude(ring - {}, 2), \"second_hole\");", quad(10.0, 0.0, 12.0, 2.0)),
        &format!("show(extrude(ring & {}, 2), \"right_half\");", quad(0.0, -10.0, 15.0, 10.0)),
        "show(extrude(ring + circle(4), 2), \"island\");",
        "show(extrude(ring + circle(12), 2), \"filled\");",
    ]));
    // A second hole 2 × 2 away from the first: (600 − 36π − 4) · 2.
    assert_solid(&out, "second_hole", (596.0 - 36.0 * PI) * 2.0, 1, VOL);
    // The right half x ∈ [0, 15]: half of everything, (300 − 18π) · 2.
    assert_solid(&out, "right_half", (300.0 - 18.0 * PI) * 2.0, 1, VOL);
    assert_bbox(&out, "right_half", [0.0, -10.0, 0.0], [15.0, 10.0, 2.0]);
    // A disc r = 2 inside the hole r = 6 is an island: a second solid,
    // (600 − 36π + 4π) · 2.
    assert_solid(&out, "island", (600.0 - 32.0 * PI) * 2.0, 2, VOL);
    // The disc that was cut out closes the hole again: 600 · 2.
    assert_solid(&out, "filled", 1200.0, 1, VOL);
}

#[test]
fn ag_pf_02_revolve_of_a_profile_with_a_hole() {
    let _serial = serial();
    // Half section x = radius 4..10, y = height 0..10 (AG-SC-07) with a
    // window radius 6..8, height 2..8. Tube π (10² − 4²) · 10 = 840π minus
    // the ring channel π (8² − 6²) · 6 = 168π → 672π; a quarter turn 168π.
    let out = run_ok(&src(&[
        &format!("let p = {} - {};", quad(4.0, 0.0, 10.0, 10.0), quad(6.0, 2.0, 8.0, 8.0)),
        "show(revolve(p, 360), \"tube\");",
        "show(revolve(p, 90), \"quarter\");",
    ]));
    assert_solid(&out, "tube", 672.0 * PI, 1, VOL);
    assert_bbox(&out, "tube", [-10.0, -10.0, 0.0], [10.0, 10.0, 10.0]);
    assert_solid(&out, "quarter", 168.0 * PI, 1, VOL);
}

#[test]
fn ag_pf_02_combined_profiles_keep_plane_and_height() {
    let _serial = serial();
    let out = run_ok(&src(&[
        "let p = rect(10, 20) - circle(4);",
        "show(extrude(on(\"xz\", p), 5), \"xz\");",
        "show(extrude(p.at_z(7), 2), \"raised\");",
    ]));
    // Plane frames as ag_sc_07_profiles_on_origin_planes: XZ has u = +X,
    // v = +Z, normal −Y. V = (200 − 4π) · 5.
    assert_solid(&out, "xz", (200.0 - 4.0 * PI) * 5.0, 1, VOL);
    assert_bbox(&out, "xz", [-5.0, -5.0, -10.0], [5.0, 0.0, 10.0]);
    assert_solid(&out, "raised", (200.0 - 4.0 * PI) * 2.0, 1, VOL);
    assert_bbox(&out, "raised", [-5.0, -10.0, 7.0], [5.0, 10.0, 9.0]);
}

#[test]
fn ag_pf_02_profiles_move_in_their_plane() {
    let _serial = serial();
    // `profile.move(x, y)`: decision of this test file (module comment).
    let out = run_ok(&src(&[
        "let c = circle(2);",
        "show(extrude(c.move(3, 4), 5), \"moved\");",
        "show(extrude(c, 5), \"original\");",
        "show(extrude(rect(10, 10) - circle(2).move(3, 2.5), 1), \"plate\");",
    ]));
    // π · 1² · 5, centre (3, 4); the original stays at the origin.
    assert_solid(&out, "moved", 5.0 * PI, 1, VOL);
    assert_bbox(&out, "moved", [2.0, 3.0, 0.0], [4.0, 5.0, 5.0]);
    assert_bbox(&out, "original", [-1.0, -1.0, 0.0], [1.0, 1.0, 5.0]);
    assert_solid(&out, "plate", 100.0 - PI, 1, VOL);
}

#[test]
fn ag_pf_02_profiles_and_solids_do_not_mix() {
    let _serial = serial();
    for call in [
        "rect(10, 10) - box(1, 1, 1)",
        "box(10, 10, 10) + rect(1, 1)",
        "rect(10, 10) & sphere(4)",
        "profile_cut(rect(10, 10), [box(1, 1, 1)])",
        "profile_cut(box(10, 10, 10), [circle(2)])",
        "profile_union([rect(1, 1), 5])",
        "profile_union([])",
        "extrude(profile_union([]), 1)",
    ] {
        let e = run_err(&format!("let a = 1;\nlet p = {call};"));
        match e {
            ScriptError::Runtime { line, .. } | ScriptError::Geometry { line, .. } => {
                assert_eq!(line, 2, "{call}");
            }
            other => panic!("{call}: expected an error at the call, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// AG-PF-02: the plate with a grid of holes
// ---------------------------------------------------------------------------

/// Plate with `nx` × `ny` holes of diameter 2 on a 4 mm grid, thickness 3.
/// The outermost hole centres are 4 mm from the rim (3 mm of material), so
/// the plate measures `4 (nx − 1) + 8` by `4 (ny − 1) + 8`: 40 × 25 holes
/// give the 164 × 104 of the requirement.
struct Plate {
    nx: u32,
    ny: u32,
}

impl Plate {
    fn width(&self) -> f64 {
        4.0 * f64::from(self.nx - 1) + 8.0
    }

    fn height(&self) -> f64 {
        4.0 * f64::from(self.ny - 1) + 8.0
    }

    fn holes(&self) -> f64 {
        f64::from(self.nx * self.ny)
    }

    /// (w h − n π r²) t with r = 1, t = 3.
    fn volume(&self) -> f64 {
        (self.width() * self.height() - self.holes() * PI) * 3.0
    }

    /// Top and bottom, the rim, and one cylindrical wall 2π r t per hole.
    fn area(&self) -> f64 {
        2.0 * (self.width() * self.height() - self.holes() * PI)
            + 2.0 * (self.width() + self.height()) * 3.0
            + self.holes() * 2.0 * PI * 3.0
    }

    /// First hole centre: half the grid extent left of / below the middle.
    fn origin(&self) -> (f64, f64) {
        (-2.0 * f64::from(self.nx - 1), -2.0 * f64::from(self.ny - 1))
    }

    /// `extrude(profile_cut(rect, circles), 3)` as in AG-PF-02.
    fn profile_route(&self) -> String {
        let (x0, y0) = self.origin();
        src(&[
            "let holes = [];",
            &format!("for i in 0..{} {{ for j in 0..{} {{", self.nx, self.ny),
            &format!("    holes.push(circle(2).move({x0} + 4 * i, {y0} + 4 * j));"),
            "} }",
            &format!(
                "let s = extrude(profile_cut(rect({}, {}), holes), 3);",
                self.width(),
                self.height()
            ),
            "show(s, \"plate\");",
            "print(s.area());",
        ])
    }

    /// The same part as one batched solid cut (AG-SC-05). The cylinders
    /// (h = 5 from z = −1) stick out on both sides.
    fn solid_route(&self) -> String {
        let (x0, y0) = self.origin();
        src(&[
            "let holes = [];",
            &format!("for i in 0..{} {{ for j in 0..{} {{", self.nx, self.ny),
            &format!("    holes.push(cylinder(2, 5).move({x0} + 4 * i, {y0} + 4 * j, -1));"),
            "} }",
            &format!("let s = cut(box({}, {}, 3), holes);", self.width(), self.height()),
            "show(s, \"plate\");",
            "print(s.area());",
        ])
    }

    fn check(&self, out: &RunOutput, what: &str) {
        let s = shape(out, "plate");
        assert!(s.is_valid(), "{what}: not a valid solid");
        assert_eq!(s.solid_count(), 1, "{what}: one solid");
        assert_rel(s.volume(), self.volume(), VOL, &format!("{what}: volume"));
        // The area counts the hole walls: a plate with a missing or a
        // doubled hole cannot pass both volume and area.
        assert_rel(printed(out, 0), self.area(), VOL, &format!("{what}: area"));
        let (w, h) = (self.width() / 2.0, self.height() / 2.0);
        assert_bbox(out, "plate", [-w, -h, 0.0], [w, h, 3.0]);
    }
}

/// Runs `source` once; its output and the wall clock time.
fn timed(source: &str) -> (RunOutput, Duration) {
    let start = Instant::now();
    let out = run_ok(source);
    (out, start.elapsed())
}

#[test]
fn plate_dimensions_match_the_requirement() {
    // 40 × 25 = 1000 holes on a 4 mm grid ↔ the plate 164 × 104 of AG-PF-02;
    // V = (164 · 104 − 1000π) · 3.
    let plate = Plate { nx: 40, ny: 25 };
    assert_eq!((plate.width(), plate.height(), plate.holes()), (164.0, 104.0, 1000.0));
    assert!((plate.volume() - (17_056.0 - 1000.0 * PI) * 3.0).abs() < 1e-9);
    assert_eq!(plate.origin(), (-78.0, -48.0));
}

#[test]
fn ag_pf_02_small_plate_is_the_same_part_by_both_routes() {
    let _serial = serial();
    // 5 × 3 = 15 holes in a plate 24 × 16 × 3: (384 − 15π) · 3.
    let plate = Plate { nx: 5, ny: 3 };
    assert_eq!((plate.width(), plate.height()), (24.0, 16.0));
    plate.check(&run_ok(&plate.solid_route()), "15 holes by solid cut");
    plate.check(&run_ok(&plate.profile_route()), "15 holes by profile");
}

#[test]
fn ag_pf_02_plate_with_1000_holes_has_the_analytic_volume() {
    let _serial = serial();
    let plate = Plate { nx: 40, ny: 25 };
    plate.check(&run_ok(&plate.profile_route()), "1000 holes by profile");
}

#[test]
fn ag_pf_02_profile_route_is_ten_times_faster_than_the_batched_solid_cut() {
    let _serial = serial();
    // The part of the requirement, both routes measured in this test run,
    // one run each. The solid route is the slow one (§5: 12.5 s); it is the
    // price of measuring what the requirement says. A smaller plate would
    // not be the same statement: the solid cut grows faster than linearly
    // with the number of holes (measured 1.0 s for 200, 11.7 s for 1000), so
    // a factor of 10 on 200 holes demands more than the requirement does.
    let plate = Plate { nx: 40, ny: 25 };
    // Warm-up, so that neither route pays for the first use of OCCT.
    let warm_up = Plate { nx: 2, ny: 2 };
    run_ok(&warm_up.solid_route());
    run_ok(&warm_up.profile_route());
    let (by_profile, profile_time) = timed(&plate.profile_route());
    plate.check(&by_profile, "1000 holes by profile");
    let (by_solid, solid_time) = timed(&plate.solid_route());
    plate.check(&by_solid, "1000 holes by solid cut");
    let ratio = solid_time.as_secs_f64() / profile_time.as_secs_f64();
    assert!(
        ratio >= 10.0,
        "profile route {profile_time:?}, batched solid cut {solid_time:?}: only {ratio:.1} times faster"
    );
}
