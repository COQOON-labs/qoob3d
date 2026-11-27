//! The script kernel after the Damen 5009 project (docs/requirements/agent.md
//! §6.6, AG-SC-25…34): robust booleans on freeform bodies, repair, and the
//! operations a printed kit from an imported model needed. Every expected
//! value is derived by hand next to the assertion.
//!
//! The freeform body of these tests is a smooth loft (B-spline faces), the
//! closest synthetic relative of an imported hull. The real hull is checked
//! by the opt-in acceptance test of the app (`tests/acceptance.rs`).
//!
//! Tolerances: `VOL` = 1e-6 relative (exact or adaptively integrated
//! B-reps), `POS` = 1e-6 mm for boxes and points, as in `tests/script.rs`.

use std::path::{Path, PathBuf};

use qoob3d_kernel::script::{RunOptions, RunOutput, ScriptError, Shape, export_step, run};

const VOL: f64 = 1e-6;
const POS: f64 = 1e-6;

fn run_in(dir: Option<&Path>, source: &str) -> Result<RunOutput, ScriptError> {
    let options = RunOptions { project_dir: dir.map(Path::to_path_buf), ..RunOptions::default() };
    run(source, &options)
}

fn run_ok(source: &str) -> RunOutput {
    run_in(None, source).unwrap_or_else(|e| panic!("script failed: {e}\n--- script ---\n{source}"))
}

fn run_err(source: &str) -> ScriptError {
    match run_in(None, source) {
        Ok(out) => panic!("script must fail, printed {:?}\n{source}", out.printed),
        Err(e) => e,
    }
}

fn shape<'a>(out: &'a RunOutput, name: &str) -> &'a Shape {
    &out.shapes.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no shape {name}")).1
}

fn printed(out: &RunOutput, line: usize) -> f64 {
    let text = out.printed.get(line).unwrap_or_else(|| panic!("no line {line}: {:?}", out.printed));
    text.trim().parse().unwrap_or_else(|e| panic!("line {line} {text:?}: {e}"))
}

fn assert_rel(actual: f64, expected: f64, tol: f64, what: &str) {
    let rel = (actual - expected).abs() / expected.abs();
    assert!(rel < tol, "{what}: got {actual}, expected {expected} (rel {rel:e} >= {tol:e})");
}

fn assert_bbox(s: &Shape, min: [f64; 3], max: [f64; 3], what: &str) {
    let (a, b) = s.bbox();
    let close = (0..3).all(|k| (a[k] - min[k]).abs() < POS && (b[k] - max[k]).abs() < POS);
    assert!(close, "bbox of {what}: got {a:?}..{b:?}, expected {min:?}..{max:?}");
}

/// Fresh project folder `<tmp>/script-robust/<test>`.
fn project(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("script-robust").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).expect("create project");
    dir
}

/// A smooth hull-like loft: rounded sections of changing size at
/// z = 0, 20, 40, 60 (B-spline faces).
const HULL: &str = "let hull = loft([rounded_rect(40, 20, 6).at_z(0), \
     rounded_rect(50, 26, 8).at_z(20), rounded_rect(46, 24, 7).at_z(40), \
     rounded_rect(36, 18, 5).at_z(60)], #{ smooth: true });";

// ===========================================================================
// AG-SC-03: a caught geometry error reads as its message
// ===========================================================================

#[test]
fn ag_sc_03_a_caught_geometry_error_reads_as_its_message() {
    // Finding 3.3: `catch (e)` printed "qoob3d_kernel::script::api::GeometryFault".
    let out = run_ok(
        "try { let x = box(1, 1, 1) - box(1, 1, 1).move(10, 0, 0); } \
         catch (e) { print(`${e}`); print(e); }",
    );
    for line in &out.printed {
        assert!(line.starts_with("cut failed: removed no material"), "{:?}", out.printed);
    }
    assert_eq!(out.printed.len(), 2);
}

#[test]
fn ag_sc_02_ordinary_nested_expressions_in_functions_run_in_every_build() {
    // Finding 3.4: "Expression exceeds maximum complexity" for this line
    // inside a function in a debug build (Rhai's default depth differs).
    let out = run_ok(
        "fn pin(p) { cylinder(1, 2).move(p[0], p[1], p[2]) }\n\
         fn tools(list, f, zw) {\n\
             let tools = [];\n\
             for p in list { tools.push(pin([p[0] * f, p[1] * f, zw])); }\n\
             tools\n\
         }\n\
         print(tools([[1, 2], [3, 4]], 0.5, 7).len());",
    );
    assert_eq!(out.printed, ["2"]);
}

#[test]
fn ag_sc_03_lines_printed_before_an_error_are_kept() {
    // Finding 3.3: a failing run returned none of its printed lines.
    let failure = qoob3d_kernel::script::run_keeping_prints(
        "print(\"before\");\nlet x = box(1, 1, 1) - box(1, 1, 1).move(10, 0, 0);",
        &RunOptions::default(),
    )
    .expect_err("the cut must fail");
    assert_eq!(failure.printed, ["before"]);
    assert!(matches!(failure.error, ScriptError::Geometry { line: 2, .. }), "{:?}", failure.error);
}

// ===========================================================================
// AG-SC-25: volumes and booleans on freeform bodies
// ===========================================================================

#[test]
fn ag_sc_25_the_volume_of_a_freeform_body_is_exact() {
    // A smooth loft through three equal circles is a cylinder with B-spline
    // faces: V = π · 10² · 40.
    let out = run_ok(
        "let c = loft([circle(20).at_z(0), circle(20).at_z(20), circle(20).at_z(40)], \
         #{ smooth: true });\nshow(c, \"c\");",
    );
    assert_rel(shape(&out, "c").volume(), std::f64::consts::PI * 100.0 * 40.0, VOL, "cylinder");
}

#[test]
fn ag_sc_25_cuts_of_a_freeform_body_add_up_and_are_valid() {
    // Slabs between and exactly at the section planes of the loft (where
    // an exact cut of the real hull lost half of its result): common and
    // cut of each slab add up to the body, and both are valid.
    let out = run_ok(&format!(
        "{HULL}\nshow(hull, \"hull\");\n\
         for z in [[-1, 20], [20, 40], [10, 50], [40, 61]] {{\n\
             let slab = box(100, 100, z[1] - z[0]).move(0, 0, z[0]);\n\
             let inside = hull & slab;\n\
             let outside = hull - slab;\n\
             print(inside.volume() + outside.volume());\n\
             print(inside.is_valid() && outside.is_valid());\n\
         }}"
    ));
    let whole = shape(&out, "hull").volume();
    for k in 0..4 {
        assert_rel(printed(&out, 2 * k), whole, 1e-5, &format!("slab {k}: inside + outside"));
        assert_eq!(out.printed[2 * k + 1], "true", "slab {k} valid");
    }
}

#[test]
fn ag_sc_25_a_union_keeps_the_extent_of_every_input() {
    let out = run_ok(&format!(
        "{HULL}\nlet deck = box(30, 14, 3).move(0, 0, 59);\nshow(union([hull, deck]), \"u\");"
    ));
    let (_, max) = shape(&out, "u").bbox();
    assert!((max[2] - 62.0).abs() < POS, "the deck reaches z = 62: {max:?}");
}

#[test]
fn ag_sc_25_fuzzy_is_an_option_of_every_boolean() {
    // Boxes 1e-4 mm apart touch with fuzzy 1e-3: one solid of 2000 mm³.
    let out = run_ok(
        "let a = box(10, 10, 10);\nlet b = box(10, 10, 10).move(10.0001, 0, 0);\n\
         show(union([a, b], #{ fuzzy: 0.001 }), \"u\");\n\
         show(cut(a, box(4, 4, 20).move(5, 0, -5), #{ fuzzy: 0.001 }), \"c\");\n\
         show(common(a, box(4, 4, 20).move(5, 0, -5), #{ fuzzy: 0.001 }), \"m\");",
    );
    assert_eq!(shape(&out, "u").solid_count(), 1);
    assert_rel(shape(&out, "u").volume(), 2000.0, 1e-4, "fused boxes");
    // The tool covers x ∈ [3, 7] ∩ [−5, 5] = [3, 5], y ∈ [−2, 2]: 2 · 4 · 10 = 80.
    assert_rel(shape(&out, "c").volume(), 1000.0 - 80.0, VOL, "cut");
    assert_rel(shape(&out, "m").volume(), 80.0, VOL, "common");
    let e = run_err("let x = union([box(1, 1, 1)], #{ fuzz: 1 });");
    assert!(e.to_string().contains("unknown option \"fuzz\""), "{e}");
}

/// The BREP file of the shape `name` shown by `source`, as bytes.
fn brep_of(source: &str, name: &str, file: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("script-robust").join(file);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create folder");
    shape(&run_ok(source), name).write_brep(&path).expect("write brep");
    std::fs::read(&path).expect("read brep")
}

#[test]
fn ag_sc_25_a_boolean_leaves_its_operands_as_they_were() {
    // Damen 5009 (Quinam): in one run the first `common(prism, hull)` gave
    // the inlay, every later common with the same prism the whole prism.
    // OCCT's booleans add 2D curves to the edges of their arguments and
    // raise tolerances in place unless they run non-destructively, and
    // ShapeFix and SameParameter (repair after booleans, `heal`) change
    // edges in place; the prism shares its edges with the profile and every
    // copy. Oracle: the same script with and without these operations
    // writes the same operands, byte for byte (the BREP holds every curve
    // and tolerance).
    let operands = format!(
        "{HULL}\nlet prism = extrude(polygon([[-6.0, 10.0], [6.0, 10.0], [10.0, 50.0], \
         [-2.0, 50.0]]), 30.0).rotate(\"x\", 90).mirror(\"xz\").move(0, 5, 0);"
    );
    let untouched = format!("{operands}\nshow(prism, \"prism\");\nshow(hull, \"hull\");");
    let used = format!(
        "{operands}\nlet a = common(prism, hull);\nlet b = hull - prism;\n\
         let c = union([hull, prism]);\nlet h = heal(hull, 0.01);\nlet k = heal(prism, 0.01);\n\
         show(prism, \"prism\");\nshow(hull, \"hull\");"
    );
    for name in ["prism", "hull"] {
        let before = brep_of(&untouched, name, &format!("operands-{name}-before.brep"));
        let after = brep_of(&used, name, &format!("operands-{name}-after.brep"));
        assert!(before == after, "{name} changed by common, cut, union or heal");
    }
}

// ===========================================================================
// AG-SC-26: defects and heal
// ===========================================================================

#[test]
fn ag_sc_26_a_valid_shape_has_no_defects_and_heals_into_itself() {
    let out = run_ok(&format!(
        "{HULL}\nprint(defects(hull).len());\nlet h = heal(hull);\nshow(hull, \"hull\");\n\
         show(h, \"h\");\nprint(h.is_valid());"
    ));
    assert_eq!(out.printed, ["0", "true"]);
    assert_rel(shape(&out, "h").volume(), shape(&out, "hull").volume(), 1e-7, "healed volume");
}

#[test]
fn ag_sc_26_heal_needs_a_positive_tolerance() {
    let e = run_err("let h = heal(box(1, 1, 1), 0);");
    assert!(e.to_string().contains("heal: tolerance must be a positive number"), "{e}");
}

// ===========================================================================
// AG-SC-27: outline and fill_voids
// ===========================================================================

#[test]
fn ag_sc_27_outline_drops_the_holes_of_a_profile() {
    let out = run_ok("print(outline(rect(20, 20) - circle(10)).area());");
    assert_rel(printed(&out, 0), 400.0, VOL, "outline area");
}

#[test]
fn ag_sc_27_fill_voids_fills_a_closed_cavity() {
    // A 4 mm cube cut out of the middle of a 10 mm cube leaves a void.
    let out = run_ok(
        "let hollow = box(10, 10, 10) - box(4, 4, 4).move(0, 0, 3);\n\
         show(hollow, \"hollow\");\nshow(fill_voids(hollow), \"filled\");",
    );
    assert_rel(shape(&out, "hollow").volume(), 1000.0 - 64.0, VOL, "hollow");
    assert_rel(shape(&out, "filled").volume(), 1000.0, VOL, "filled");
}

// ===========================================================================
// AG-SC-28: on with an offset, split, on_bed
// ===========================================================================

#[test]
fn ag_sc_28_on_places_a_profile_on_a_parallel_plane() {
    // "yz" at x = 7: the normal is +X, the profile spans y, z ∈ [−1, 1].
    let out = run_ok(
        "show(extrude(on(\"yz\", rect(2, 2), 7), 1), \"a\");\n\
         show(extrude(on(\"xz\", rect(2, 2), -3), 1), \"b\");\n\
         show(extrude(on(\"xy\", rect(2, 2), 5), 1), \"c\");",
    );
    assert_bbox(shape(&out, "a"), [7.0, -1.0, -1.0], [8.0, 1.0, 1.0], "yz");
    // "xz": normal −Y, so the extrusion runs from y = −3 to −4.
    assert_bbox(shape(&out, "b"), [-1.0, -4.0, -1.0], [1.0, -3.0, 1.0], "xz");
    assert_bbox(shape(&out, "c"), [-1.0, -1.0, 5.0], [1.0, 1.0, 6.0], "xy");
}

#[test]
fn ag_sc_28_split_returns_both_sides_of_a_plane() {
    let out = run_ok(
        "let parts = split(box(10, 10, 10), \"xy\", 4);\nshow(parts[0], \"below\");\n\
         show(parts[1], \"above\");\nlet side = split(box(10, 10, 10), \"yz\", 1);\n\
         show(side[0], \"left\");",
    );
    assert_rel(shape(&out, "below").volume(), 400.0, VOL, "below");
    assert_rel(shape(&out, "above").volume(), 600.0, VOL, "above");
    assert_bbox(shape(&out, "left"), [-5.0, -5.0, 0.0], [1.0, 5.0, 10.0], "left of x = 1");
    let e = run_err("let p = split(box(10, 10, 10), \"xy\", 12);");
    assert!(e.to_string().contains("split failed: the plane at 12 does not cut the shape"), "{e}");
}

#[test]
fn ag_sc_28_on_bed_stands_a_shape_on_the_bed_centred() {
    let out = run_ok("show(on_bed(sphere(10).move(5, 6, 7)), \"s\");");
    assert_bbox(shape(&out, "s"), [-5.0, -5.0, 0.0], [5.0, 5.0, 10.0], "sphere on the bed");
}

// ===========================================================================
// AG-SC-29: named solids of a STEP file
// ===========================================================================

/// A STEP file with two solids named `A` (10 mm cube) and `B` (2 × 3 × 4).
fn named_step(dir: &Path) {
    let out = run_ok("show(box(10, 10, 10), \"a\");\nshow(box(2, 3, 4).move(20, 0, 0), \"b\");");
    let path = dir.join("lib/two.step");
    export_step(&out.shapes, &path).expect("write STEP");
    let text = std::fs::read_to_string(&path).expect("read STEP");
    let mut names = ["'A'", "'B'"].into_iter();
    let named: Vec<String> = text
        .split_inclusive('\n')
        .map(|line| match line.find("MANIFOLD_SOLID_BREP('") {
            Some(at) => {
                let name = names.next().expect("two solids");
                let rest = &line[at + "MANIFOLD_SOLID_BREP('".len()..];
                let close = rest.find('\'').expect("closing quote");
                format!("{}MANIFOLD_SOLID_BREP({name}{}", &line[..at], &rest[close + 1..])
            }
            None => line.to_owned(),
        })
        .collect();
    assert!(names.next().is_none(), "the writer wrote both solids");
    std::fs::write(&path, named.concat()).expect("write named STEP");
}

#[test]
fn ag_sc_29_import_step_picks_solids_by_name() {
    let dir = project("named");
    named_step(&dir);
    let out = run_in(
        Some(&dir),
        "print(step_names(\"lib/two.step\"));\nshow(import_step(\"lib/two.step\", \"B\"), \"b\");\n\
         show(import_step(\"lib/two.step\", [\"A\", \"B\"]), \"ab\");\n\
         show(import_step(\"lib/two.step\"), \"all\");",
    )
    .expect("named import");
    assert_eq!(out.printed, ["[\"A\", \"B\"]"]);
    assert_rel(shape(&out, "b").volume(), 24.0, VOL, "B");
    assert_eq!(shape(&out, "ab").solid_count(), 2);
    assert_rel(shape(&out, "all").volume(), 1024.0, VOL, "both");
    assert_eq!(out.files_read, [PathBuf::from("lib/two.step")], "read once, recorded once");
    let e = run_in(Some(&dir), "let x = import_step(\"lib/two.step\", \"C\");").expect_err("no C");
    assert!(e.to_string().contains("has no solid named \"C\"; it has A, B"), "{e}");
}

// ===========================================================================
// AG-SC-30: group, AG-SC-31: offset, AG-SC-32: heights, AG-SC-33: smooth loft
// ===========================================================================

#[test]
fn ag_sc_30_group_keeps_the_shapes_without_a_boolean() {
    // Two cubes overlapping by half: grouped 2 · 1000, fused 1500.
    let out = run_ok(
        "let a = box(10, 10, 10);\nlet b = a.move(5, 0, 0);\n\
         show(group([a, b]), \"g\");\nshow(union([a, b]), \"u\");",
    );
    assert_eq!(shape(&out, "g").solid_count(), 2);
    assert_rel(shape(&out, "g").volume(), 2000.0, VOL, "group");
    assert_rel(shape(&out, "u").volume(), 1500.0, VOL, "union");
}

#[test]
fn ag_sc_31_offset_grows_a_solid_with_round_edges() {
    // A cube a = 10 grown by d = 1: a³ + 6 a² d (faces) + 3 π a d² (12 quarter
    // cylinders) + 4/3 π d³ (8 eighth spheres).
    let out = run_ok(
        "show(offset(box(10, 10, 10), 1), \"o\");\nshow(offset(box(10, 10, 10), -1), \"i\");",
    );
    let pi = std::f64::consts::PI;
    assert_rel(
        shape(&out, "o").volume(),
        1000.0 + 600.0 + 30.0 * pi + 4.0 / 3.0 * pi,
        VOL,
        "grown",
    );
    assert_rel(shape(&out, "i").volume(), 512.0, VOL, "shrunk: 8³");
}

#[test]
fn ag_sc_32_heights_maps_the_top_of_a_shape() {
    // A step: z = 5 for x < 0, z = 8 for x > 0 (x ∈ [−5, 5], y ∈ [−5, 5]).
    let out = run_ok(
        "let s = box(10, 10, 5) + box(5, 10, 3).move(2.5, 0, 5);\n\
         let h = heights(s, [-4, -4], [4, 6], 2);\nprint(h.len());\nprint(h[0].len());\n\
         print(h[0][0]);\nprint(h[0][4]);\nprint(h[4][0]);",
    );
    // y = −4, −2, 0, 2, 4, 6: six rows; x = −4 … 4: five points.
    assert_eq!(out.printed[..2], ["6", "5"]);
    assert_rel(printed(&out, 2), 5.0, VOL, "left");
    assert_rel(printed(&out, 3), 8.0, VOL, "right");
    // The last row lies at y = 6: past the shape, nothing is hit.
    let out = run_ok("print(heights(box(10, 10, 5), [0, 6], [0, 6], 1)[0][0] == ());");
    assert_eq!(out.printed, ["true"]);
}

#[test]
fn ag_sc_33_a_smooth_loft_bulges_where_a_ruled_one_is_straight() {
    // Ruled through circles d 10, 20, 10 (h 20 each): two frustums,
    // V = 2 · π h / 3 (R² + R r + r²) = 2 · π · 20 / 3 · (100 + 50 + 25).
    let out = run_ok(
        "let p = [circle(10).at_z(0), circle(20).at_z(20), circle(10).at_z(40)];\n\
         show(loft(p), \"ruled\");\nshow(loft(p, #{ smooth: true }), \"smooth\");",
    );
    let ruled = 2.0 * std::f64::consts::PI * 20.0 / 3.0 * 175.0;
    assert_rel(shape(&out, "ruled").volume(), ruled, VOL, "ruled");
    let smooth = shape(&out, "smooth").volume();
    assert!(smooth > ruled * 1.01, "the smooth loft bulges: {smooth} vs {ruled}");
    assert!(shape(&out, "smooth").is_valid());
}

// ===========================================================================
// AG-SC-22: the bed face counts whatever its curved neighbours report
// ===========================================================================

#[test]
fn ag_sc_22_the_cut_face_of_a_freeform_body_on_the_bed_is_the_bed_face() {
    // Finding 1.9: a split freeform piece stood on z = 0 but bed_area was 0.
    let out = run_ok(&format!(
        "{HULL}\nlet top = on_bed(split(hull, \"xy\", 30)[1]);\nshow(top, \"top\");\n\
         print(bed_area(top));\nprint(faces(top).bottom().area());\n\
         print(section(hull, \"xy\", 30).area());"
    ));
    // Split and section compute their intersection curves on the B-spline
    // faces separately; the two faces differ by a few µm along the edge.
    let cut = printed(&out, 2);
    assert_rel(printed(&out, 0), cut, 1e-3, "bed area is the cut face");
    assert_rel(printed(&out, 1), cut, 1e-3, "bottom() selects the cut face");
}

// ===========================================================================
// AG-SC-34: mesh_report, AG-PJ-06: the pose option of a part
// ===========================================================================

#[test]
fn ag_sc_34_mesh_report_tells_what_a_slicer_finds() {
    let out = run_ok(
        "let r = mesh_report(box(10, 10, 10));\nprint(r.watertight);\nprint(r.triangles);\n\
         print(r.open_edges);\nprint(r.volume);",
    );
    assert_eq!(out.printed[..3], ["true", "12", "0"]);
    assert_rel(printed(&out, 3), 1000.0, 1e-5, "mesh volume");
}

#[test]
fn ag_pj_06_the_pose_option_declares_how_a_part_is_printed() {
    use qoob3d_core::project::PrintPose;
    use qoob3d_kernel::script::PartMode;
    let declare = |source: &str| {
        let options = RunOptions { parts: PartMode::Declare, ..RunOptions::default() };
        run(source, &options)
    };
    let out = declare(
        "part(\"A\", #{ pose: #{ rotate: [\"x\", 180], count: 2 } }, |deps| box(1, 1, 1));\n\
         part(\"B\", #{ pose: #{ rotate: [[\"z\", 90], [[0, 1, 0], 45]] } }, |deps| box(1, 1, 1));\n\
         part(\"C\", #{}, |deps| box(1, 1, 1));",
    )
    .expect("declarations");
    assert_eq!(
        out.parts[0].print,
        Some(PrintPose { rotate: vec![([1.0, 0.0, 0.0], 180.0)], count: 2 })
    );
    let b = out.parts[1].print.as_ref().expect("B has a pose");
    assert_eq!(b.rotate, [([0.0, 0.0, 1.0], 90.0), ([0.0, 1.0, 0.0], 45.0)]);
    assert_eq!(b.count, 1);
    assert_eq!(out.parts[2].print, None);
    for (bad, message) in [
        ("#{ pose: 1 }", "pose must be a map"),
        ("#{ pose: #{ count: 0 } }", "count must be a whole number >= 1"),
        ("#{ pose: #{ rotate: [\"q\", 1] } }", "axis must be"),
        ("#{ pose: #{ spin: 1 } }", "unknown key \"spin\""),
    ] {
        let e = declare(&format!("part(\"A\", {bad}, |deps| box(1, 1, 1));")).expect_err(bad);
        assert!(e.to_string().contains(message), "{bad}: {e}");
    }
}

#[test]
fn ag_pj_07_color_and_print_with_declare_the_filament_and_the_object_of_a_part() {
    use qoob3d_kernel::script::PartMode;
    let declare = |source: &str| {
        let options = RunOptions { parts: PartMode::Declare, ..RunOptions::default() };
        run(source, &options)
    };
    let out = declare(
        "part(\"House\", #{ color: \"#f4f4f4\" }, |deps| box(1, 1, 1));\n\
         part(\"Windows\", #{ color: \"#202020\", print_with: \"House\" }, |deps| box(1, 1, 1));\n\
         part(\"Plain\", #{}, |deps| box(1, 1, 1));\n\
         part(\"Named\", #{ color: \"Orange\" }, |deps| box(1, 1, 1));",
    )
    .expect("declarations");
    assert_eq!(out.parts[0].color.as_deref(), Some("#F4F4F4"), "stored in upper case");
    assert_eq!(out.parts[0].print_with, None);
    assert_eq!(out.parts[1].color.as_deref(), Some("#202020"));
    assert_eq!(out.parts[1].print_with.as_deref(), Some("House"));
    assert_eq!((out.parts[2].color.clone(), out.parts[2].print_with.clone()), (None, None));
    assert_eq!(out.parts[3].color.as_deref(), Some("#FFA500"), "a basic colour name");
    for (bad, message) in [
        ("#{ color: \"RAL 9016\" }", "color must be \"#RRGGBB\""),
        ("#{ color: 255 }", "color must be \"#RRGGBB\""),
        ("#{ color: \"#12345\" }", "or one of black, silver"),
        ("#{ print_with: 1 }", "print_with must be the name of another part"),
        ("#{ print_with: \"A\" }", "print_with must be the name of another part"),
    ] {
        let e = declare(&format!("part(\"A\", {bad}, |deps| box(1, 1, 1));")).expect_err(bad);
        assert!(e.to_string().contains(message), "{bad}: {e}");
    }
}

#[test]
fn ag_sc_22_a_face_a_few_nanometres_above_the_lowest_point_still_lies_on_the_bed() {
    // Finding 1.9: the cut face of the bow pieces lay 20 nm above the lowest
    // point of their freeform faces, and bed_area was 0. Here a sphere dips
    // 20 nm below the bottom of a box: the bottom is the bed face, minus
    // the cap's footprint π (2 r h) ≈ 6e-4 mm².
    let out = run_ok(
        "let s = box(20, 20, 10) + sphere(10).move(0, 0, 4.99998);\nprint(bed_area(s));\n\
         print(faces(s).bottom().area());\nprint(edges(s).bottom().len());",
    );
    assert_rel(printed(&out, 0), 400.0, 1e-5, "bed area");
    assert_rel(printed(&out, 1), 400.0, 1e-5, "bottom() faces");
    assert!(printed(&out, 2) >= 4.0, "bottom() edges: {:?}", out.printed);
}
