//! Script API helpers (docs/requirements/agent.md §6.4,
//! AG-SC-17…22, ADR-0053): profile helpers, screw hole tools, selection by
//! position, `expect_eq`, mass, sections and the first print analysis.
//!
//! Every expected value is derived by hand from the dimensions in the
//! script; the derivation stands next to the assertion.
//!
//! Tolerances: as in `tests/script.rs`. Volumes, areas and lengths of
//! planar, cylindrical, conical and spherical B-reps are integrated by OCCT
//! to far better than 1e-9, so `VOL` = 1e-6 relative is a wide margin that
//! still separates every effect tested here (the smallest, the corner
//! rounding of a profile, is 2.7e-2 relative). Bounding boxes are checked to
//! `POS` = 1e-6 mm (AG-HD-11: tight). AG-SC-22 itself allows 1 % on curved
//! faces (`CURVED`); planar faces must be exact (`VOL`).
//!
//! Readings where the requirement leaves room (each is repeated at its test):
//! - `offset` of a profile with a hole grows or shrinks the **region**: a
//!   positive offset makes holes smaller.
//! - `counterbore(d, head_d, head_h, l)`: `l` is the total depth from z = 0,
//!   like `countersink`; the head is the top `head_h` of it.
//! - `.near(p, r)` keeps an element whose smallest distance to `p` is ≤ `r`.
//! - `section(s, "xz", o)` cuts at the world coordinate y = `o`, `"yz"` at
//!   x = `o`, `"xy"` at z = `o`.
//! - `overhangs(s, limit)`: the angle of a face is measured **from the
//!   horizontal** (0° = flat underside, 90° = vertical wall); a downward
//!   facing face counts when its angle is smaller than the limit.

mod common;
#[path = "common/solid.rs"]
mod solid;

use std::f64::consts::{FRAC_1_SQRT_2, PI, SQRT_2};

use common::{VOL, assert_bbox, assert_rel, printed, run_err, run_ok, shape, src};
use qoob3d_kernel::script::{Expectation, RunOutput, ScriptError};
use serde_json::{Value, json};
use solid::{assert_size, assert_volume, expect_geometry};

/// AG-SC-22: "within 1 % on curved faces".
const CURVED: f64 = 0.01;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Printed line `i` is the number `expected` to `VOL` relative.
fn assert_printed(out: &RunOutput, i: usize, expected: f64, what: &str) {
    assert_rel(printed(out, i), expected, VOL, what);
}

/// Printed line `i` is exactly zero (an empty sum, no face counted).
fn assert_printed_zero(out: &RunOutput, i: usize, what: &str) {
    let v = printed(out, i);
    assert!(v.abs() < 1e-9, "{what}: got {v}, expected 0");
}

/// Printed line `i` is the count `expected`.
fn assert_count(out: &RunOutput, i: usize, expected: i64, what: &str) {
    let line =
        out.printed.get(i).unwrap_or_else(|| panic!("no printed line {i}: {:?}", out.printed));
    assert_eq!(line.trim().parse::<i64>().ok(), Some(expected), "{what}: printed {line:?}");
}

const NAN: &str = "let nan = parse_float(\"NaN\");";

/// Every call (as line 2, after the definition of `nan`) is a `Runtime`
/// error on its line (AG-SC-04: invalid dimensions fail at the call).
fn assert_runtime_errors(calls: &[&str]) {
    for call in calls {
        match run_err(&format!("{NAN}\nlet x = {call};")) {
            ScriptError::Runtime { line, message, .. } => {
                assert_eq!(line, 2, "{call}");
                // The function must exist and refuse its arguments itself.
                assert!(!message.is_empty() && !message.contains("not found"), "{call}: {message}");
            }
            other => panic!("{call}: expected a Runtime error, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// AG-SC-17: rounded_rect, profile.rotate, offset
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_17_rounded_rect_loses_four_corner_squares_minus_quarter_discs() {
    let out = run_ok("show(extrude(rounded_rect(40, 20, 5), 3), \"p\");");
    // Each corner loses the square r² minus the quarter disc π r²/4; four
    // corners lose (4 − π) r². V = (w h − (4 − π) r²) t
    //   = (800 − (4 − π) · 25) · 3.
    assert_volume(&out, "p", (800.0 - (4.0 - PI) * 25.0) * 3.0);
    // Centred like `rect`; the rounding does not change the extent.
    assert_bbox(&out, "p", [-20.0, -10.0, 0.0], [20.0, 10.0, 3.0]);
    assert_eq!(shape(&out, "p").solid_count(), 1);
}

#[test]
fn ag_sc_17_rounded_rect_with_a_small_radius_is_nearly_the_rectangle() {
    let out = run_ok("show(extrude(rounded_rect(10, 10, 0.1), 1), \"p\");");
    // (100 − (4 − π) · 0.01) · 1: the radius is the third argument, in mm.
    assert_volume(&out, "p", 100.0 - (4.0 - PI) * 0.01);
}

#[test]
fn ag_sc_17_rounded_rect_rejects_invalid_dimensions() {
    assert_runtime_errors(&[
        "rounded_rect(0, 20, 5)",
        "rounded_rect(40, -20, 5)",
        "rounded_rect(40, 20, 0)",
        "rounded_rect(40, 20, -1)",
        "rounded_rect(40, 20, nan)",
        "rounded_rect(nan, 20, 5)",
        // 2 r > h: the corner arcs would overlap.
        "rounded_rect(40, 20, 11)",
    ]);
}

#[test]
fn ag_sc_17_profile_rotate_by_90_degrees_swaps_the_extent() {
    let out = run_ok("show(extrude(rect(10, 20).rotate(90), 2), \"p\");");
    // A rectangle centred on the profile origin, turned by a quarter:
    // x ∈ [−10, 10], y ∈ [−5, 5]. The area stays 200.
    assert_bbox(&out, "p", [-10.0, -5.0, 0.0], [10.0, 5.0, 2.0]);
    assert_volume(&out, "p", 400.0);
}

#[test]
fn ag_sc_17_profile_rotate_by_45_degrees() {
    let out = run_ok("show(extrude(rect(10, 20).rotate(45), 2), \"p\");");
    // Corners (±5, ±10) turned by 45°: the extent in x and in y is
    // w cos 45° + h sin 45° = (w + h) / √2 = 30 / √2.
    let e = 30.0 * FRAC_1_SQRT_2;
    assert_size(&out, "p", [e, e, 2.0]);
    assert_volume(&out, "p", 400.0);
}

#[test]
fn ag_sc_17_profile_rotate_turns_about_the_profile_origin_counter_clockwise() {
    let out = run_ok(&src(&[
        "show(extrude(circle(4).move(10, 0).rotate(90), 1), \"plus\");",
        "show(extrude(circle(4).move(10, 0).rotate(-90), 1), \"minus\");",
        "show(extrude(circle(4).move(10, 0).rotate(360), 1), \"full\");",
    ]));
    // The centre (10, 0) turns about the origin (not about the circle):
    // +90° (right hand rule about +Z, as `s.rotate("z", …)`) → (0, 10),
    // −90° → (0, −10), 360° → back at (10, 0). Radius 2.
    assert_bbox(&out, "plus", [-2.0, 8.0, 0.0], [2.0, 12.0, 1.0]);
    assert_bbox(&out, "minus", [-2.0, -12.0, 0.0], [2.0, -8.0, 1.0]);
    assert_bbox(&out, "full", [8.0, -2.0, 0.0], [12.0, 2.0, 1.0]);
    for name in ["plus", "minus", "full"] {
        assert_volume(&out, name, 4.0 * PI);
    }
}

#[test]
fn ag_sc_17_profile_rotate_rejects_nan() {
    assert_runtime_errors(&["rect(10, 20).rotate(nan)"]);
}

#[test]
fn ag_sc_17_offset_outward_rounds_the_corners() {
    let out = run_ok("show(extrude(offset(rect(20, 10), 2), 1), \"p\");");
    // The region within d of the rectangle: the rectangle w h, four strips
    // along the sides 2 d (w + h), four quarter discs π d².
    // A = 200 + 2 · 2 · 30 + 4 π = 320 + 4 π (sharp corners would give 336).
    assert_volume(&out, "p", 320.0 + 4.0 * PI);
    assert_bbox(&out, "p", [-12.0, -7.0, 0.0], [12.0, 7.0, 1.0]);
}

#[test]
fn ag_sc_17_offset_inward_keeps_the_corners_of_a_rectangle_sharp() {
    let out = run_ok("show(extrude(offset(rect(20, 10), -2), 1), \"p\");");
    // The points further than d from the outline: (w − 2d)(h − 2d) = 16 · 6.
    assert_volume(&out, "p", 96.0);
    assert_bbox(&out, "p", [-8.0, -3.0, 0.0], [8.0, 3.0, 1.0]);
}

#[test]
fn ag_sc_17_offset_of_a_circle_changes_the_radius() {
    let out = run_ok(&src(&[
        "show(extrude(offset(circle(10), 1), 1), \"out\");",
        "show(extrude(offset(circle(10), -1), 1), \"in\");",
        "show(extrude(offset(circle(10).move(20, 0), 1.5), 1), \"moved\");",
    ]));
    // r = 5 → 6 and 4: π · 36 and π · 16. The offset follows a moved
    // profile: r = 6.5 around (20, 0).
    assert_volume(&out, "out", 36.0 * PI);
    assert_volume(&out, "in", 16.0 * PI);
    assert_volume(&out, "moved", 42.25 * PI);
    assert_bbox(&out, "moved", [13.5, -6.5, 0.0], [26.5, 6.5, 1.0]);
}

#[test]
fn ag_sc_17_offset_of_a_profile_with_a_hole_grows_the_region() {
    // Reading: the offset acts on the region (AG-PF-02 profiles may have
    // holes), so "outward" makes the hole smaller and "inward" larger.
    let out = run_ok(&src(&[
        "let ring = circle(20) - circle(12);",
        "show(extrude(offset(ring, 1), 1), \"out\");",
        "show(extrude(offset(ring, -1), 1), \"in\");",
    ]));
    // Radii 10 and 6 → 11 and 5: π (121 − 25); → 9 and 7: π (81 − 49).
    assert_volume(&out, "out", 96.0 * PI);
    assert_volume(&out, "in", 32.0 * PI);
}

#[test]
fn ag_sc_17_negative_offset_that_removes_everything_is_a_geometry_error() {
    // Height 10: an inward offset of 5 leaves a line, 6 leaves nothing.
    for d in ["-5", "-6", "-100"] {
        let e = run_err(&format!("let r = rect(20, 10);\nlet p = offset(r, {d});"));
        expect_geometry(&e, 2, &["offset"]);
    }
    let e = run_err("let p = offset(circle(10), -5);");
    expect_geometry(&e, 1, &["offset"]);
}

#[test]
fn ag_sc_17_offset_rejects_nan() {
    assert_runtime_errors(&["offset(rect(20, 10), nan)"]);
}

// ---------------------------------------------------------------------------
// AG-SC-18: countersink, counterbore
// ---------------------------------------------------------------------------

/// Volume of the tool `countersink(d, head_d, angle, l)`.
///
/// The cone has the full opening angle `angle`, so its wall makes
/// `angle`/2 with the axis. It narrows from the radius R = head_d/2 at
/// z = 0 to the radius r = d/2 of the clearance cylinder over the depth
/// k = (R − r) / tan(angle/2). The frustum has the volume
/// π k (R² + R r + r²) / 3; the cylinder π r² l overlaps it in π r² k
/// (for k ≤ l). Union: π r² l + π k (R² + R r − 2 r²) / 3.
fn countersink_volume(d: f64, head_d: f64, angle_deg: f64, l: f64) -> f64 {
    let (r, big) = (d / 2.0, head_d / 2.0);
    let k = (big - r) / (angle_deg / 2.0).to_radians().tan();
    assert!(k <= l, "the formula needs a cone that is shorter than the hole");
    PI * r * r * l + PI * k * (big * big + big * r - 2.0 * r * r) / 3.0
}

#[test]
fn countersink_formula_matches_hand_computed_cases() {
    // 90°: k = R − r = 2; V = 40 π + 2 π (16 + 8 − 8) / 3 = 40 π + 32 π / 3.
    let v90 = countersink_volume(4.0, 8.0, 90.0, 10.0);
    assert!((v90 - (40.0 * PI + 32.0 * PI / 3.0)).abs() < 1e-9);
    // 60°: k = 2 / tan 30° = 2 √3; V = 40 π + 2 √3 · 16 π / 3.
    let v60 = countersink_volume(4.0, 8.0, 60.0, 10.0);
    assert!((v60 - (40.0 * PI + 32.0 * 3.0_f64.sqrt() * PI / 3.0)).abs() < 1e-9);
}

#[test]
fn ag_sc_18_countersink_is_a_clearance_cylinder_with_a_cone_at_the_top() {
    let out = run_ok(&src(&[
        "show(countersink(4, 8, 90, 10), \"cs90\");",
        "show(countersink(4, 8, 60, 10), \"cs60\");",
        "show(countersink(3.4, 6.5, 90, 12), \"m3\");",
    ]));
    assert_volume(&out, "cs90", countersink_volume(4.0, 8.0, 90.0, 10.0));
    // 60° is the full angle: a half angle of 60° would give k = 2/√3.
    assert_volume(&out, "cs60", countersink_volume(4.0, 8.0, 60.0, 10.0));
    assert_volume(&out, "m3", countersink_volume(3.4, 6.5, 90.0, 12.0));
    // Downward from z = 0 like `pin` and `hole`; the widest place is the
    // head diameter at z = 0.
    assert_bbox(&out, "cs90", [-4.0, -4.0, -10.0], [4.0, 4.0, 0.0]);
    assert_bbox(&out, "m3", [-3.25, -3.25, -12.0], [3.25, 3.25, 0.0]);
    for name in ["cs90", "cs60", "m3"] {
        assert_eq!(shape(&out, name).solid_count(), 1, "{name} is one tool body");
    }
}

#[test]
fn ag_sc_18_countersink_cut_from_a_plate_leaves_the_analytic_remainder() {
    let out = run_ok(&src(&[
        "let plate = box(20, 20, 5);",
        "show(plate - countersink(4, 8, 90, 10).move(0, 0, 5), \"plate\");",
    ]));
    // The tool starts at the top face z = 5 and leaves the plate at the
    // bottom (l = 10 > 5). Inside the plate: the cylinder r = 2 over the
    // thickness 5 plus the cone ring (k = 2 ≤ 5):
    // removed = π · 4 · 5 + 2 π (16 + 8 − 8) / 3 = 20 π + 32 π / 3.
    assert_volume(&out, "plate", 2000.0 - 20.0 * PI - 32.0 * PI / 3.0);
    assert_bbox(&out, "plate", [-10.0, -10.0, 0.0], [10.0, 10.0, 5.0]);
    assert_eq!(shape(&out, "plate").solid_count(), 1);
}

#[test]
fn ag_sc_18_countersink_rejects_invalid_dimensions() {
    assert_runtime_errors(&[
        "countersink(0, 8, 90, 10)",
        "countersink(4, 8, 90, 0)",
        "countersink(4, 8, 90, -10)",
        "countersink(4, 8, 0, 10)",
        "countersink(4, 8, -90, 10)",
        // A cone of 180° is a plane.
        "countersink(4, 8, 180, 10)",
        "countersink(4, 8, nan, 10)",
        "countersink(nan, 8, 90, 10)",
        // The head must be wider than the hole.
        "countersink(8, 8, 90, 10)",
        "countersink(8, 4, 90, 10)",
    ]);
}

#[test]
fn ag_sc_18_counterbore_is_a_clearance_cylinder_with_a_wide_head() {
    // Reading: `l` is the whole depth from z = 0 (as for `countersink`),
    // the head bore is the top `head_h` of it.
    let out = run_ok(&src(&[
        "show(counterbore(4, 8, 3, 10), \"cb\");",
        "show(box(20, 20, 6) - counterbore(4, 8, 3, 10).move(0, 0, 6), \"plate\");",
    ]));
    // Cylinder r = 2 over 10 plus the ring between r = 2 and R = 4 over 3:
    // V = 40 π + π (16 − 4) · 3 = 76 π.
    assert_volume(&out, "cb", 76.0 * PI);
    assert_bbox(&out, "cb", [-4.0, -4.0, -10.0], [4.0, 4.0, 0.0]);
    assert_eq!(shape(&out, "cb").solid_count(), 1);
    // Inside a plate of 6: π · 4 · 6 + π · 12 · 3 = 60 π removed.
    assert_volume(&out, "plate", 2400.0 - 60.0 * PI);
}

#[test]
fn ag_sc_18_counterbore_rejects_invalid_dimensions() {
    assert_runtime_errors(&[
        "counterbore(0, 8, 3, 10)",
        "counterbore(4, 8, 0, 10)",
        "counterbore(4, 8, -3, 10)",
        "counterbore(4, 8, 3, 0)",
        "counterbore(4, 8, 3, nan)",
        "counterbore(8, 8, 3, 10)",
        "counterbore(8, 4, 3, 10)",
    ]);
}

// ---------------------------------------------------------------------------
// AG-SC-19: selection by position, sums
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_19_length_and_area_sum_the_selection_of_a_box() {
    let out = run_ok(&src(&[
        "let b = box(20, 30, 40);",
        "print(edges(b).length());",
        "print(edges(b).parallel(\"z\").length());",
        "print(edges(b).top().length());",
        "print(faces(b).area());",
        "print(faces(b).top().area());",
        "print(faces(b).len());",
        "print(faces(b).top().len());",
    ]));
    // 4 edges per direction: 4 (20 + 30 + 40) = 360; vertical 4 · 40;
    // the top loop 2 (20 + 30).
    assert_printed(&out, 0, 360.0, "all edges");
    assert_printed(&out, 1, 160.0, "vertical edges");
    assert_printed(&out, 2, 100.0, "top edges");
    // 2 (20·30 + 20·40 + 30·40) = 5200; the top face 20 · 30.
    assert_printed(&out, 3, 5200.0, "all faces");
    assert_printed(&out, 4, 600.0, "top face");
    assert_count(&out, 5, 6, "faces of a box");
    assert_count(&out, 6, 1, "top faces of a box");
}

#[test]
fn ag_sc_19_length_and_area_sum_the_selection_of_a_cylinder() {
    let out = run_ok(&src(&[
        "let c = cylinder(10, 20);",
        "print(edges(c).length());",
        "print(edges(c).top().length());",
        "print(faces(c).area());",
        "print(faces(c).top().area());",
        "print(faces(c).len());",
        "print(faces(c).area() - c.area());",
    ]));
    // r = 5, h = 20. The seam is no edge (AG-SC-09): two circles 2 π r.
    assert_printed(&out, 0, 20.0 * PI, "both circles");
    assert_printed(&out, 1, 10.0 * PI, "top circle");
    // Two discs π r² and the wall 2 π r h: 50 π + 200 π.
    assert_printed(&out, 2, 250.0 * PI, "all faces");
    assert_printed(&out, 3, 25.0 * PI, "top disc");
    assert_count(&out, 4, 3, "faces of a cylinder");
    // The sum over all faces is the surface area of the shape.
    assert!(printed(&out, 5).abs() < 1e-6, "faces(c).area() != c.area(): {}", printed(&out, 5));
}

#[test]
fn ag_sc_19_in_box_keeps_edges_that_lie_wholly_inside() {
    // Box x ∈ [−10, 10], y ∈ [−15, 15], z ∈ [0, 40]. All selection boxes
    // keep 1 mm distance from the geometry, so no boundary case decides.
    let out = run_ok(&src(&[
        "let b = box(20, 30, 40);",
        "let top = edges(b).in_box([-11, -16, 39], [11, 16, 41]);",
        "print(top.len()); print(top.length());",
        "let side = edges(b).in_box([9, -16, -1], [11, 16, 41]);",
        "print(side.len()); print(side.length());",
        "let one = edges(b).in_box([9, 14, -1], [11, 16, 41]);",
        "print(one.len()); print(one.length());",
        "let cut_off = edges(b).in_box([9, 14, -1], [11, 16, 20]);",
        "print(cut_off.len()); print(cut_off.length());",
        "let all = edges(b).in_box([-100, -100, -100], [100, 100, 100]);",
        "print(all.len());",
        "let chained = edges(b).parallel(\"z\").in_box([-11, -16, -1], [0, 16, 41]);",
        "print(chained.len()); print(chained.length());",
    ]));
    // The slab around z = 40 holds the four top edges: 2 (20 + 30).
    assert_count(&out, 0, 4, "top edges");
    assert_printed(&out, 1, 100.0, "top edges");
    // The slab around x = 10 holds the loop of that face: 2 (30 + 40).
    assert_count(&out, 2, 4, "edges of the face x = 10");
    assert_printed(&out, 3, 140.0, "edges of the face x = 10");
    // The column around (10, 15) holds one vertical edge of length 40; the
    // six edges that only start in it are not wholly inside.
    assert_count(&out, 4, 1, "the vertical edge at (10, 15)");
    assert_printed(&out, 5, 40.0, "the vertical edge at (10, 15)");
    // The same column cut at z = 20 contains only half of that edge.
    assert_count(&out, 6, 0, "an edge that is only partly inside");
    assert_printed_zero(&out, 7, "length of an empty selection");
    assert_count(&out, 8, 12, "everything");
    // Filters chain: the vertical edges with x ≤ 0 are the two at x = −10.
    assert_count(&out, 9, 2, "vertical edges at x = -10");
    assert_printed(&out, 10, 80.0, "vertical edges at x = -10");
}

#[test]
fn ag_sc_19_in_box_keeps_faces_that_lie_wholly_inside() {
    let out = run_ok(&src(&[
        "let b = box(20, 30, 40);",
        "let top = faces(b).in_box([-11, -16, 39], [11, 16, 41]);",
        "print(top.len()); print(top.area());",
        "let side = faces(b).in_box([9, -16, -1], [11, 16, 41]);",
        "print(side.len()); print(side.area());",
        "let none = faces(b).in_box([9, -16, -1], [11, 16, 20]);",
        "print(none.len()); print(none.area());",
        "let c = cylinder(10, 20);",
        "let disc = faces(c).in_box([-6, -6, 19], [6, 6, 21]);",
        "print(disc.len()); print(disc.area());",
        "print(faces(c).in_box([-6, -6, -1], [6, 6, 21]).len());",
        "print(edges(c).in_box([-6, -6, 19], [6, 6, 21]).length());",
        // The wall of the cylinder reaches x = ±5: not inside x ∈ [−4, 6].
        "print(faces(c).in_box([-4, -6, -1], [6, 6, 21]).len());",
    ]));
    assert_count(&out, 0, 1, "top face");
    assert_printed(&out, 1, 600.0, "top face 20 x 30");
    assert_count(&out, 2, 1, "face x = 10");
    assert_printed(&out, 3, 1200.0, "face x = 10, 30 x 40");
    assert_count(&out, 4, 0, "a face that is only partly inside");
    assert_printed_zero(&out, 5, "area of an empty selection");
    // Cylinder r = 5: the top disc π · 25 and its circle 2 π · 5.
    assert_count(&out, 6, 1, "top disc");
    assert_printed(&out, 7, 25.0 * PI, "top disc");
    assert_count(&out, 8, 3, "all faces of the cylinder");
    assert_printed(&out, 9, 10.0 * PI, "top circle");
    assert_count(&out, 10, 0, "no face of the cylinder fits x in [-4, 6]");
}

#[test]
fn ag_sc_19_near_keeps_elements_within_the_distance_of_a_point() {
    // Reading: an element is near when its smallest distance to the point
    // is at most r (the point need not be the element's centre).
    let out = run_ok(&src(&[
        "let b = box(20, 30, 40);",
        "let mid = edges(b).near([10, 15, 20], 0.5);",
        "print(mid.len()); print(mid.length());",
        "let corner = edges(b).near([10, 15, 40], 0.5);",
        "print(corner.len()); print(corner.length());",
        "print(edges(b).near([0, 0, 20], 0.5).len());",
        "print(edges(b).near([0, 0, 20], 1000).len());",
        "let top = faces(b).near([3, 4, 40], 0.5);",
        "print(top.len()); print(top.area());",
        "let three = faces(b).near([10, 15, 40], 0.5);",
        "print(three.len()); print(three.area());",
        "print(faces(b).near([0, 0, 20], 0.5).len());",
        "print(edges(b).top().near([10, 15, 40], 0.5).length());",
    ]));
    // The middle of the vertical edge at (10, 15): only that edge (the next
    // edges are 20 mm away at its ends).
    assert_count(&out, 0, 1, "edge through (10, 15, 20)");
    assert_printed(&out, 1, 40.0, "edge through (10, 15, 20)");
    // A corner touches three edges: 20 + 30 + 40.
    assert_count(&out, 2, 3, "edges at the corner");
    assert_printed(&out, 3, 90.0, "edges at the corner");
    // The centre of the box is 10 mm from the nearest edge: none, and all
    // twelve within 1000 mm.
    assert_count(&out, 4, 0, "no edge near the centre");
    assert_count(&out, 5, 12, "all edges within 1000 mm");
    // A point on the top face away from its centre: that face, 20 · 30.
    assert_count(&out, 6, 1, "top face");
    assert_printed(&out, 7, 600.0, "top face");
    // A corner touches three faces: 600 + 800 + 1200.
    assert_count(&out, 8, 3, "faces at the corner");
    assert_printed(&out, 9, 2600.0, "faces at the corner");
    assert_count(&out, 10, 0, "no face near the centre");
    // Chained after `.top()`: the two top edges of the corner, 20 + 30.
    assert_printed(&out, 11, 50.0, "top edges at the corner");
}

#[test]
fn ag_sc_19_fillet_of_only_the_selected_edge() {
    let out = run_ok(&src(&[
        "let b = box(20, 30, 40);",
        "show(fillet(b, edges(b).in_box([9, 14, -1], [11, 16, 41]), 3), \"vertical\");",
        "show(fillet(b, edges(b).near([10, 0, 40], 0.5), 3), \"top_y\");",
        "show(fillet(b, edges(b).near([0, 15, 40], 0.5), 3), \"top_x\");",
    ]));
    // One filleted edge loses the corner square minus the quarter disc,
    // r² (1 − π/4), along its length: V = abc − (1 − π/4) r² L with r = 3.
    let lost = (1.0 - PI / 4.0) * 9.0;
    // The vertical edge at (10, 15): L = 40.
    assert_volume(&out, "vertical", 24_000.0 - lost * 40.0);
    // The top edge along y at x = 10 (its middle is (10, 0, 40)): L = 30.
    assert_volume(&out, "top_y", 24_000.0 - lost * 30.0);
    // The top edge along x at y = 15: L = 20.
    assert_volume(&out, "top_x", 24_000.0 - lost * 20.0);
    for name in ["vertical", "top_y", "top_x"] {
        assert_bbox(&out, name, [-10.0, -15.0, 0.0], [10.0, 15.0, 40.0]);
    }
}

#[test]
fn ag_sc_19_selection_by_position_rejects_invalid_arguments() {
    assert_runtime_errors(&[
        "edges(box(1, 1, 1)).near([0, 0, 0], -1)",
        "edges(box(1, 1, 1)).near([0, 0, 0], nan)",
        "faces(box(1, 1, 1)).near([0, 0], 1)",
        "faces(box(1, 1, 1)).near([0, 0, nan], 1)",
        "edges(box(1, 1, 1)).in_box([0, 0], [1, 1, 1])",
        "edges(box(1, 1, 1)).in_box([0, 0, 0], [1, 1, nan])",
    ]);
}

// ---------------------------------------------------------------------------
// AG-SC-20: expect_eq
// ---------------------------------------------------------------------------

/// The expectations of the only check of a script, which must have run
/// without a script error.
fn expectations(source: &str) -> Vec<Expectation> {
    let out = run_ok(source);
    assert_eq!(out.checks.len(), 1, "one check: {:?}", out.checks);
    let check = &out.checks[0];
    assert!(check.ran, "the check ran");
    assert!(check.error.is_none(), "no script error: {:?}", check.error);
    check.expectations.clone()
}

fn assert_expectation(e: &Expectation, text: &str, passed: bool, actual: &Value, expected: &Value) {
    assert_eq!(e.text, text);
    assert_eq!(e.passed, passed, "{text}: {e:?}");
    assert_eq!(&e.actual, actual, "{text}: actual");
    assert_eq!(&e.expected, expected, "{text}: expected");
    // Like `expect` and `expect_between`: no tolerance.
    assert_eq!(e.tol, Value::Null, "{text}: tol");
}

#[test]
fn ag_sc_20_expect_eq_records_actual_and_expected() {
    let list = expectations(&src(&[
        "check(\"eq\", || {",
        "    expect_eq(3, 3, \"ints\");",
        "    expect_eq(3, 4, \"ints differ\");",
        "    expect_eq(0.5, 0.5, \"floats\");",
        "    expect_eq(\"PLA\", \"PLA\", \"strings\");",
        "    expect_eq(\"PLA\", \"PETG\", \"strings differ\");",
        "    expect_eq(true, true, \"bools\");",
        "    expect_eq(true, false, \"bools differ\");",
        "    expect_eq(box(10, 10, 10).solids(), 1, \"measured int\");",
        "    expect_eq(faces(box(10, 10, 10)).len(), 5, \"measured int differs\");",
        "});",
    ]));
    // A failed expectation does not stop the check (AG-CK-02): all nine.
    assert_eq!(list.len(), 9, "{list:?}");
    assert_expectation(&list[0], "ints", true, &json!(3), &json!(3));
    assert_expectation(&list[1], "ints differ", false, &json!(3), &json!(4));
    assert_expectation(&list[2], "floats", true, &json!(0.5), &json!(0.5));
    assert_expectation(&list[3], "strings", true, &json!("PLA"), &json!("PLA"));
    assert_expectation(&list[4], "strings differ", false, &json!("PLA"), &json!("PETG"));
    assert_expectation(&list[5], "bools", true, &json!(true), &json!(true));
    assert_expectation(&list[6], "bools differ", false, &json!(true), &json!(false));
    assert_expectation(&list[7], "measured int", true, &json!(1), &json!(1));
    // A box has six faces.
    assert_expectation(&list[8], "measured int differs", false, &json!(6), &json!(5));
}

#[test]
fn ag_sc_20_expect_eq_compares_floats_exactly() {
    let list = expectations(&src(&[
        "check(\"eq\", || {",
        "    expect_eq(0.1 + 0.2, 0.3, \"no tolerance\");",
        "    expect_eq(0.25 + 0.5, 0.75, \"exact in binary\");",
        "    let nan = parse_float(\"NaN\");",
        "    expect_eq(nan, nan, \"NaN\");",
        "});",
    ]));
    assert_eq!(list.len(), 3, "{list:?}");
    // 0.1 + 0.2 is 0.30000000000000004 in IEEE 754 doubles: not 0.3, and
    // both values are recorded with all their digits.
    let sum = 0.1_f64 + 0.2_f64;
    assert!(sum > 0.3, "the premise of this test");
    assert_expectation(&list[0], "no tolerance", false, &json!(sum), &json!(0.3));
    // 0.25, 0.5 and 0.75 are exact binary fractions.
    assert_expectation(&list[1], "exact in binary", true, &json!(0.75), &json!(0.75));
    // NaN never passes (as for the other expectations, AG-CK-02).
    assert!(!list[2].passed, "{:?}", list[2]);
}

#[test]
fn ag_sc_20_a_failed_expect_eq_fails_the_check() {
    let out = run_ok(&src(&[
        "check(\"good\", || { expect_eq(2, 2, \"two\"); });",
        "check(\"bad\", || { expect_eq(\"a\", \"b\", \"letters\"); });",
    ]));
    assert_eq!(out.checks.len(), 2);
    assert!(out.checks[0].passed(), "{:?}", out.checks[0]);
    assert!(!out.checks[1].passed(), "{:?}", out.checks[1]);
    assert!(out.checks[1].error.is_none(), "a failed expectation is no script error");
}

#[test]
fn ag_sc_20_expect_eq_outside_a_check_is_a_runtime_error() {
    // Like `expect`, `expect_near` and `expect_between` (AG-CK-02).
    match run_err("let a = 1;\nexpect_eq(a, 1, \"outside\");") {
        ScriptError::Runtime { line, message, .. } => {
            assert_eq!(line, 2);
            assert!(message.contains("expect_eq") && !message.contains("not found"), "{message}");
        }
        other => panic!("expected a Runtime error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// AG-SC-21: mass, section
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_21_mass_is_volume_times_density_in_grams() {
    let out = run_ok(&src(&[
        "print(box(10, 20, 50).mass(1.24));",
        "print(sphere(20).mass(7.85));",
        "print(box(10, 10, 10).mass(1));",
    ]));
    // 10 · 20 · 50 mm³ = 10 000 mm³ = 10 cm³; PLA 1.24 g/cm³ → 12.4 g.
    assert_printed(&out, 0, 12.4, "mass of 10 cm3 PLA");
    // Sphere r = 10 mm = 1 cm: 4/3 π cm³ of steel, 7.85 g/cm³.
    assert_printed(&out, 1, 4.0 / 3.0 * PI * 7.85, "mass of a steel ball");
    // 1 cm³ of water (an int density is accepted like every number): 1 g.
    assert_printed(&out, 2, 1.0, "mass of 1 cm3 water");
}

#[test]
fn ag_sc_21_mass_rejects_invalid_densities() {
    assert_runtime_errors(&[
        "box(10, 10, 10).mass(0)",
        "box(10, 10, 10).mass(-1.24)",
        "box(10, 10, 10).mass(nan)",
    ]);
}

#[test]
fn ag_sc_21_section_xy_of_a_box_with_a_hole_and_of_a_sphere() {
    let out = run_ok(&src(&[
        "let b = box(20, 20, 10) - cylinder(4, 10);",
        "print(section(b, \"xy\", 5).area());",
        "print(section(sphere(10), \"xy\", 3).area());",
        "print(section(sphere(10), \"xy\", -3).area());",
        "print(section(sphere(10), \"xy\", 0).area());",
        "print(section(cylinder(10, 20), \"xy\", 7.5).area());",
    ]));
    // Square 20 · 20 minus the bore r = 2: 400 − 4 π.
    assert_printed(&out, 0, 400.0 - 4.0 * PI, "box with a hole");
    // A sphere of radius R cut at the height h is a disc of radius
    // √(R² − h²): area π (R² − h²) = π (25 − 9), the same below the centre.
    assert_printed(&out, 1, 16.0 * PI, "sphere at h = 3");
    assert_printed(&out, 2, 16.0 * PI, "sphere at h = -3");
    assert_printed(&out, 3, 25.0 * PI, "sphere at its centre");
    assert_printed(&out, 4, 25.0 * PI, "cylinder");
}

#[test]
fn ag_sc_21_section_xz_and_yz_cut_at_world_coordinates() {
    // Reading: the offset is the world coordinate along the axis that the
    // plane name leaves out ("xz" → y, "yz" → x), not a distance along the
    // sketch normal of `on` (which is −Y for "xz").
    let out = run_ok(&src(&[
        // x ∈ [−5, 5], y ∈ [5, 25], z ∈ [0, 30]
        "let b = box(10, 20, 30).move(0, 15, 0);",
        "print(section(b, \"xz\", 10).area());",
        "print(section(b, \"yz\", 2).area());",
        "print(section(b, \"xy\", 29).area());",
        // x ∈ [20, 30]
        "print(section(box(10, 20, 30).move(25, 0, 0), \"yz\", 22).area());",
        "print(section(cone(10, 0, 8), \"yz\", 0).area());",
        "print(section(cone(10, 0, 8), \"xz\", 0).area());",
    ]));
    // y = 10 lies inside y ∈ [5, 25] (y = −10 does not): 10 · 30.
    assert_printed(&out, 0, 300.0, "xz section at y = 10");
    assert_printed(&out, 1, 600.0, "yz section at x = 2: 20 x 30");
    assert_printed(&out, 2, 200.0, "xy section at z = 29: 10 x 20");
    assert_printed(&out, 3, 600.0, "yz section at x = 22");
    // A cone with base diameter 10 and height 8 cut through its axis is a
    // triangle: 10 · 8 / 2.
    assert_printed(&out, 4, 40.0, "cone through the axis (yz)");
    assert_printed(&out, 5, 40.0, "cone through the axis (xz)");
}

#[test]
fn ag_sc_21_section_is_a_profile() {
    let out = run_ok(&src(&[
        "let b = box(20, 20, 10) - cylinder(4, 10);",
        "show(extrude(section(b, \"xy\", 5), 2), \"slice\");",
        "print(rect(10, 20).area());",
        "print((circle(20) - circle(12)).area());",
    ]));
    // "Returns the cut profile": it extrudes like any profile with a hole.
    assert_volume(&out, "slice", (400.0 - 4.0 * PI) * 2.0);
    assert_size(&out, "slice", [20.0, 20.0, 2.0]);
    // `.area()` is a function of profiles: 10 · 20 and π (10² − 6²).
    assert_printed(&out, 0, 200.0, "area of a rectangle");
    assert_printed(&out, 1, 64.0 * PI, "area of a ring");
}

#[test]
fn ag_sc_21_section_of_two_bodies_sums_the_regions() {
    let out = run_ok(&src(&[
        "let two = union([box(10, 10, 10), box(4, 4, 10).move(20, 0, 0)]);",
        "print(section(two, \"xy\", 5).area());",
    ]));
    // Two disjoint squares: 100 + 16.
    assert_printed(&out, 0, 116.0, "two regions");
}

#[test]
fn ag_sc_21_section_that_misses_the_shape_is_a_geometry_error() {
    // An empty profile is an error everywhere in the profile API (AG-PF-02).
    let e = run_err("let b = box(10, 10, 10);\nlet p = section(b, \"xy\", 20);");
    expect_geometry(&e, 2, &["section"]);
    let e = run_err("let b = box(10, 20, 30).move(0, 15, 0);\nlet p = section(b, \"xz\", -10);");
    expect_geometry(&e, 2, &["section"]);
}

#[test]
fn ag_sc_21_section_rejects_invalid_arguments() {
    assert_runtime_errors(&[
        "section(box(10, 10, 10), \"ab\", 5)",
        "section(box(10, 10, 10), \"z\", 5)",
        "section(box(10, 10, 10), \"xy\", nan)",
    ]);
}

// ---------------------------------------------------------------------------
// AG-SC-22: overhangs, bed_area
// ---------------------------------------------------------------------------
//
// Reading of "faces whose downward normal is steeper than the limit": the
// angle of a face is measured from the horizontal, as slicers state their
// support threshold. A flat underside has 0°, a vertical wall 90°. A face
// counts when its outward normal points downward and its angle from the
// horizontal is smaller than `max_angle_deg`; equivalently, when the angle
// between its outward normal and −Z is smaller than `max_angle_deg`. With
// the FDM rule of 45° a 30° underside needs support and a 60° one prints.
// Faces in the plane z = min of the shape lie on the bed and never count.
// No test puts a face exactly on the limit.

/// A prism along Y (length 10) whose cross-section in XZ is a trapezoid:
/// 10 wide on the bed, 5 high, both flanks leaning outward with the angle
/// `deg` from the horizontal (run = 5 / tan(deg) per flank).
fn trapezoid_prism(deg: f64) -> String {
    let run = 5.0 / deg.to_radians().tan();
    let x = 5.0 + run;
    format!(
        "let s = extrude(on(\"xz\", polygon([[-5, 0], [5, 0], [{x}, 5], [-{x}, 5]])), 10);\n\
         show(s, \"s\");"
    )
}

#[test]
fn ag_sc_22_a_box_has_no_overhang_and_stands_on_its_bottom_face() {
    let out = run_ok(&src(&[
        "let b = box(20, 30, 10);",
        "print(overhangs(b, 45));",
        "print(bed_area(b));",
        "let lifted = b.move(5, 5, 7);",
        "print(overhangs(lifted, 45));",
        "print(bed_area(lifted));",
        "print(overhangs(b, 89));",
    ]));
    // The only downward face is the bottom, and it lies on the bed.
    assert_printed_zero(&out, 0, "overhang of a box");
    assert_printed(&out, 1, 600.0, "bed area of a box: w d");
    // The bed plane is z = min of the shape, wherever the shape is.
    assert_printed_zero(&out, 2, "overhang of a lifted box");
    assert_printed(&out, 3, 600.0, "bed area of a lifted box");
    // Vertical walls (90° from the horizontal) never count.
    assert_printed_zero(&out, 4, "walls at a limit of 89 degrees");
}

#[test]
fn ag_sc_22_an_upright_t_overhangs_with_its_two_undersides() {
    let out = run_ok(&src(&[
        "let t = union([box(10, 10, 20), box(30, 10, 5).move(0, 0, 20)]);",
        "print(overhangs(t, 45));",
        "print(bed_area(t));",
        "let flipped = t.rotate(\"x\", 180);",
        "print(overhangs(flipped, 45));",
        "print(bed_area(flipped));",
    ]));
    // Stem 10 × 10 × 20, bar 30 × 10 × 5 on top. The bar sticks out
    // (30 − 10) / 2 = 10 on both sides: two undersides 10 · 10 at z = 20.
    assert_printed(&out, 0, 200.0, "undersides of the bar");
    // It stands on the foot of the stem: 10 · 10.
    assert_printed(&out, 1, 100.0, "bed area of the T");
    // Upside down the bar lies on the bed (30 · 10) and nothing faces down
    // above it.
    assert_printed_zero(&out, 2, "overhang of the flipped T");
    assert_printed(&out, 3, 300.0, "bed area of the flipped T");
}

#[test]
fn ag_sc_22_an_underside_at_30_degrees_from_the_horizontal_counts() {
    let out = run_ok(&format!(
        "{}\nprint(overhangs(s, 45));\nprint(overhangs(s, 20));\nprint(bed_area(s));",
        trapezoid_prism(30.0)
    ));
    // Each flank rises 5 at 30°: its width along the slope is
    // 5 / sin 30° = 10, its length 10: 100 mm². Two flanks: 200.
    assert_printed(&out, 0, 200.0, "30 degree flanks at the limit 45");
    // With a limit of 20° the same flanks print without support. This fixes
    // the direction of the limit: a larger limit counts more faces.
    assert_printed_zero(&out, 1, "30 degree flanks at the limit 20");
    // On the bed: the 10 wide bottom, 10 long.
    assert_printed(&out, 2, 100.0, "bed area of the prism");
    // The part itself: trapezoid (10 + 10 + 2 · 5√3) / 2 · 5, length 10.
    assert_volume(&out, "s", (10.0 + 5.0 * 3.0_f64.sqrt()) * 5.0 * 10.0);
}

#[test]
fn ag_sc_22_an_underside_at_60_degrees_from_the_horizontal_does_not_count() {
    let out = run_ok(&format!(
        "{}\nprint(overhangs(s, 45));\nprint(overhangs(s, 70));\nprint(bed_area(s));",
        trapezoid_prism(60.0)
    ));
    assert_printed_zero(&out, 0, "60 degree flanks at the limit 45");
    // Slope width 5 / sin 60° = 10 / √3, length 10, two flanks: 200 / √3.
    assert_printed(&out, 1, 200.0 / 3.0_f64.sqrt(), "60 degree flanks at the limit 70");
    assert_printed(&out, 2, 100.0, "bed area of the prism");
}

#[test]
fn ag_sc_22_a_chamfer_at_the_bottom_reduces_the_bed_area() {
    let out = run_ok(&src(&[
        "let b = box(20, 30, 10);",
        "let c = chamfer(b, edges(b).bottom(), 2);",
        "print(bed_area(c));",
        "print(overhangs(c, 50));",
        "print(overhangs(c, 40));",
    ]));
    // The chamfer takes 2 mm from every side of the bottom face:
    // (20 − 4)(30 − 4) = 16 · 26.
    assert_printed(&out, 0, 416.0, "bed area of the chamfered box");
    // The four chamfer faces stand at 45°. Each is a trapezoid with the
    // parallel sides s and s − 2c and the slant height c √2: area
    // c √2 (s − c). Sum for s = 20, 30, 20, 30 and c = 2:
    // 2 √2 · 2 · (18 + 28) = 184 √2. They count at a limit of 50° …
    assert_printed(&out, 1, 184.0 * SQRT_2, "chamfer faces at the limit 50");
    // … and not at 40°.
    assert_printed_zero(&out, 2, "chamfer faces at the limit 40");
}

#[test]
fn ag_sc_22_bed_area_of_a_cylinder_and_a_ring() {
    let out = run_ok(&src(&[
        "print(bed_area(cylinder(10, 20)));",
        "print(bed_area(cylinder(20, 5) - cylinder(12, 5)));",
        "print(overhangs(cylinder(10, 20), 45));",
    ]));
    // Planar faces are exact, whatever their outline: π · 25 and
    // π (10² − 6²).
    assert_printed(&out, 0, 25.0 * PI, "disc on the bed");
    assert_printed(&out, 1, 64.0 * PI, "ring on the bed");
    assert_printed_zero(&out, 2, "an upright cylinder has no overhang");
}

#[test]
fn ag_sc_22_overhang_of_curved_faces_is_within_one_percent() {
    let out = run_ok(&src(&[
        "print(overhangs(sphere(20), 45));",
        "let lying = cylinder(10, 20).rotate(\"x\", 90);",
        "print(overhangs(lying, 45));",
        "print(overhangs(lying, 30));",
    ]));
    // Sphere R = 10: the normals within 45° of −Z cover the cap with the
    // polar angle 45°, area 2 π R² (1 − cos 45°). The contact with the bed
    // is a point.
    let cap = 2.0 * PI * 100.0 * (1.0 - FRAC_1_SQRT_2);
    assert_rel(printed(&out, 0), cap, CURVED, "cap of the sphere");
    // Lying cylinder r = 5, L = 20: the wall within ±45° of straight down is
    // an arc of 90°: (π/2) r L = 50 π. The end faces are vertical, the
    // contact with the bed is a line. Within ±30°: (π/3) r L.
    assert_rel(printed(&out, 1), 50.0 * PI, CURVED, "lower strip of a lying cylinder, 45");
    assert_rel(printed(&out, 2), 100.0 * PI / 3.0, CURVED, "lower strip of a lying cylinder, 30");
}

#[test]
fn ag_sc_22_print_analysis_rejects_invalid_arguments() {
    assert_runtime_errors(&["overhangs(box(10, 10, 10), -45)", "overhangs(box(10, 10, 10), nan)"]);
}
