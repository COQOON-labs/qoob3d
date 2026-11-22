//! Script kernel R1 (docs/requirements/agent.md §1, ADR-0053): the Rhai CAD
//! API end to end. Every expected value is derived by hand from the
//! dimensions in the script; the derivation stands next to the assertion.
//!
//! Tolerances: volumes of prismatic and cylindrical B-reps are exact in OCCT
//! up to floating point rounding, spheres/tori/cones/fillets are integrated
//! numerically by `BRepGProp` to far better than 1e-9 (M0 measured < 1e-12
//! on the probe parts). `VOL` = 1e-6 relative is therefore a wide margin
//! that still separates every feature tested here (the smallest effect, a
//! 3 mm chamfer pair on a 24 000 mm³ box, is 7.5e-3 relative). Bounding
//! boxes and points are checked to `POS` = 1e-6 mm absolute: an agent
//! measures with them, so they must be tight (no tolerance enlargement).
//!
//! Integer arguments are accepted wherever a number is expected (the task's
//! own example `box(10, 20, 30)`); several scripts use ints on purpose.

use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use qoob3d_kernel::script::{
    RunOptions, RunOutput, ScriptError, Shape, export_3mf, export_step, export_stl, run,
};

const VOL: f64 = 1e-6;
const POS: f64 = 1e-6;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn assert_volume(out: &RunOutput, name: &str, expected: f64) {
    let s = shape(out, name);
    assert!(s.is_valid(), "{name} is not a valid solid");
    assert_rel(s.volume(), expected, VOL, &format!("volume of {name}"));
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

fn assert_size(out: &RunOutput, name: &str, size: [f64; 3]) {
    let (a, b) = shape(out, name).bbox();
    for i in 0..3 {
        assert!(
            (b[i] - a[i] - size[i]).abs() < POS,
            "bbox size of {name}: got {a:?}..{b:?}, expected size {size:?}"
        );
    }
}

/// Printed line `i` as a number (Rhai prints floats round-trip exact).
fn printed(out: &RunOutput, i: usize) -> f64 {
    let line =
        out.printed.get(i).unwrap_or_else(|| panic!("no printed line {i}: {:?}", out.printed));
    line.trim().parse().unwrap_or_else(|e| panic!("printed line {i} {line:?} is no number: {e}"))
}

fn expect_runtime(e: &ScriptError, want_line: usize) {
    match e {
        ScriptError::Runtime { line, message, .. } => {
            assert_eq!(*line, want_line, "{e:?}");
            assert!(!message.is_empty(), "{e:?}");
        }
        other => panic!("expected a Runtime error on line {want_line}, got {other:?}"),
    }
}

/// `function` is the name of the failing script API function (AG-SC-03),
/// or for an operator the operator token; both are accepted.
fn expect_geometry(e: &ScriptError, want_line: usize, functions: &[&str]) {
    match e {
        ScriptError::Geometry { line, function, message, .. } => {
            assert_eq!(*line, want_line, "{e:?}");
            assert!(
                functions.contains(&function.as_str()),
                "function {function:?} not in {functions:?}: {e:?}"
            );
            assert!(!message.is_empty(), "{e:?}");
        }
        other => panic!("expected a Geometry error on line {want_line}, got {other:?}"),
    }
}

/// Fresh, deterministic scratch directory for one test.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("script-tests").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

// ---------------------------------------------------------------------------
// AG-SC-01: run, show, print
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_01_empty_script_has_no_output() {
    let out = run_ok("");
    assert!(out.shapes.is_empty());
    assert!(out.printed.is_empty());
}

#[test]
fn ag_sc_01_shown_shapes_keep_call_order_and_names() {
    let out = run_ok(&src(&[
        "let b = box(10, 20, 30);",
        "show(b, \"second_defined_first_shown\");",
        "show(cylinder(10, 20), \"cyl\");",
        "let unused = sphere(5);",
        "show(b, \"again\");",
    ]));
    let names: Vec<&str> = out.shapes.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["second_defined_first_shown", "cyl", "again"],
        "unshown shapes are not output"
    );
}

#[test]
fn ag_sc_01_print_lines_are_captured_in_order() {
    let out = run_ok(&src(&["print(\"hello\");", "print(1 + 2);", "print(\"a\" + \"b\");"]));
    assert_eq!(out.printed, ["hello", "3", "ab"]);
}

#[test]
fn ag_sc_01_show_of_a_non_shape_is_a_runtime_error() {
    let e = run_err(&src(&["let x = 5;", "show(x, \"x\");"]));
    expect_runtime(&e, 2);
}

// ---------------------------------------------------------------------------
// AG-SC-02: sandbox and limits
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_02_default_operation_limit() {
    let o = RunOptions::default();
    assert_eq!(o.max_operations, 50_000_000);
    assert_eq!(o.project_dir, None);
}

#[test]
fn ag_sc_02_endless_loop_hits_the_operation_limit_quickly() {
    let options = RunOptions { max_operations: 100_000, ..RunOptions::default() };
    let start = Instant::now();
    let r = run("let i = 0;\nloop { i += 1; }", &options);
    let elapsed = start.elapsed();
    match r {
        Err(ScriptError::Limit { line, message, .. }) => {
            assert_eq!(line, 2, "the loop is on line 2");
            assert!(!message.is_empty());
        }
        other => panic!("expected Limit, got {other:?}"),
    }
    // 100 000 Rhai operations take milliseconds even in debug builds.
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
}

#[test]
fn ag_sc_02_endless_recursion_hits_the_call_depth_limit() {
    let r = run("fn f(x) { f(x + 1) }\nf(0);", &RunOptions::default());
    assert!(matches!(r, Err(ScriptError::Limit { .. })), "{r:?}");
}

#[test]
fn ag_sc_02_operation_limit_is_not_hit_by_a_normal_script() {
    // 1000 loop iterations are far below the default of 50 million.
    let out = run_ok("let s = 0;\nfor i in 0..1000 { s += i; }\nprint(s);");
    assert_eq!(out.printed, ["499500"]);
}

#[test]
fn ag_sc_02_no_file_functions_exist() {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let t = target.display();
    for call in [
        format!("open(\"{t}\")"),
        format!("open_file(\"{t}\")"),
        format!("read_string(\"{t}\")"),
        format!("read_file(\"{t}\")"),
        format!("write_file(\"{t}\", \"x\")"),
        "system(\"ls\")".to_string(),
        "http_get(\"http://127.0.0.1\")".to_string(),
    ] {
        let e = run_err(&format!("let x = 1;\nlet y = {call};"));
        expect_runtime(&e, 2);
    }
}

#[test]
fn ag_sc_02_import_of_files_outside_the_project_fails() {
    let root = scratch("import_outside");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(root.join("outside.rhai"), "export const X = 42;\n").unwrap();
    let absolute = root.join("outside");
    let options = RunOptions { project_dir: Some(project.clone()), ..RunOptions::default() };
    for source in [
        "import \"../outside\" as o;\nprint(o::X);".to_string(),
        format!("import \"{}\" as o;\nprint(o::X);", absolute.display()),
    ] {
        let r = run(&source, &options);
        assert!(r.is_err(), "{source}: must not read files outside the project, got {r:?}");
    }
    // Without a project there is nothing to import at all.
    let r = run(
        &format!("import \"{}\" as o;\nprint(o::X);", absolute.display()),
        &RunOptions::default(),
    );
    assert!(r.is_err(), "{r:?}");
}

// ---------------------------------------------------------------------------
// AG-SC-03: error positions
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_03_syntax_error_names_the_line() {
    let e = run_err(&src(&["let a = box(1, 2, 3);", "let b = ;", "show(a, \"a\");"]));
    match e {
        ScriptError::Syntax { line, column, message } => {
            assert_eq!(line, 2);
            assert!(column >= 1);
            assert!(!message.is_empty());
        }
        other => panic!("expected Syntax, got {other:?}"),
    }
}

#[test]
fn ag_sc_03_runtime_error_names_line_and_column_of_the_call() {
    // Column of `box` in "let b = box(0, 10, 10);" is 9 (1-based).
    let e = run_err(&src(&["let a = box(10, 10, 10);", "let b = box(0, 10, 10);"]));
    match e {
        ScriptError::Runtime { line, column, .. } => {
            assert_eq!((line, column), (2, 9), "position of `box`");
        }
        other => panic!("expected Runtime, got {other:?}"),
    }
}

#[test]
fn ag_sc_03_failed_occt_operation_is_geometry_with_function_name_and_position() {
    // A fillet radius of 20 on a 10 mm cube cannot be built on any edge.
    let e = run_err(&src(&["let b = box(10, 10, 10);", "let f = fillet(b, all_edges(b), 20);"]));
    match &e {
        ScriptError::Geometry { line, column, function, .. } => {
            assert_eq!((*line, *column), (2, 9), "{e:?}");
            assert_eq!(function, "fillet");
        }
        other => panic!("expected Geometry, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// AG-SC-04: solids and placement
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_04_box_is_centered_in_xy_with_bottom_at_zero() {
    let out = run_ok("show(box(10, 20, 30), \"b\");\nshow(box(2.5, 4, 1.5), \"mixed\");");
    // V = 10 · 20 · 30
    assert_volume(&out, "b", 6000.0);
    assert_bbox(&out, "b", [-5.0, -10.0, 0.0], [5.0, 10.0, 30.0]);
    assert_eq!(shape(&out, "b").solid_count(), 1);
    // Mixed int/float arguments: 2.5 · 4 · 1.5 = 15
    assert_volume(&out, "mixed", 15.0);
    assert_bbox(&out, "mixed", [-1.25, -2.0, 0.0], [1.25, 2.0, 1.5]);
}

#[test]
fn ag_sc_04_cylinder_has_axis_z_bottom_at_zero_centered() {
    let out = run_ok("show(cylinder(10, 20), \"c\");");
    // V = π r² h = π · 25 · 20
    assert_volume(&out, "c", PI * 25.0 * 20.0);
    assert_bbox(&out, "c", [-5.0, -5.0, 0.0], [5.0, 5.0, 20.0]);
}

#[test]
fn ag_sc_04_sphere_is_centered_at_the_origin() {
    let out = run_ok("show(sphere(10), \"s\");");
    // V = 4/3 π r³, r = 5
    assert_volume(&out, "s", 4.0 / 3.0 * PI * 125.0);
    assert_bbox(&out, "s", [-5.0; 3], [5.0; 3]);
}

#[test]
fn ag_sc_04_cone_is_a_frustum() {
    let out = run_ok("show(cone(10, 4, 6), \"k\");");
    // Frustum V = π h / 3 (R² + R r + r²) = π · 6/3 · (25 + 10 + 4) = 78 π
    assert_volume(&out, "k", 78.0 * PI);
    // Placement along Z is not specified beyond "like cylinder"; the size is.
    assert_size(&out, "k", [10.0, 10.0, 6.0]);
}

#[test]
fn ag_sc_04_torus_volume() {
    let out = run_ok("show(torus(20, 4), \"t\");");
    // d_ring = centre line diameter (R = 10), d_tube = tube diameter (r = 2):
    // V = 2 π² R r² = 2 π² · 10 · 4 = 80 π²; outer size 2(R + r) = 24, height 4.
    assert_volume(&out, "t", 80.0 * PI * PI);
    assert_size(&out, "t", [24.0, 24.0, 4.0]);
}

#[test]
fn ag_sc_04_invalid_dimensions_are_runtime_errors_at_the_call() {
    let nan = "let nan = parse_float(\"NaN\");";
    for (call, column) in [
        ("box(0, 10, 10)", 9),
        ("box(10, -1, 10)", 9),
        ("box(10, 10, nan)", 9),
        ("cylinder(0, 10)", 9),
        ("cylinder(10, -5)", 9),
        ("sphere(-2)", 9),
        ("sphere(nan)", 9),
        ("cone(-1, 4, 6)", 9),
        ("cone(10, 4, 0)", 9),
        ("torus(0, 4)", 9),
        ("torus(20, nan)", 9),
    ] {
        let source = format!("{nan}\nlet x = {call};");
        match run_err(&source) {
            ScriptError::Runtime { line, column: c, .. } => {
                assert_eq!((line, c), (2, column), "{call}");
            }
            other => panic!("{call}: expected Runtime, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// AG-SC-05: booleans
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_05_operators_union_cut_common() {
    let out = run_ok(&src(&[
        "let a = box(20, 20, 10);",
        "let b = cylinder(10, 30).move(0, 0, -10);",
        "show(a + b, \"union\");",
        "show(a - b, \"cut\");",
        "show(a & b, \"common\");",
    ]));
    // Box 4000, cylinder π·25·30 = 750π, overlap (z 0..10) π·25·10 = 250π.
    assert_volume(&out, "union", 4000.0 + 750.0 * PI - 250.0 * PI);
    assert_volume(&out, "cut", 4000.0 - 250.0 * PI);
    assert_volume(&out, "common", 250.0 * PI);
    assert_bbox(&out, "union", [-10.0, -10.0, -10.0], [10.0, 10.0, 20.0]);
    assert_bbox(&out, "common", [-5.0, -5.0, 0.0], [5.0, 5.0, 10.0]);
}

#[test]
fn ag_sc_05_batched_cut_with_an_array() {
    let out = run_ok(&src(&[
        "let plate = box(40, 40, 5);",
        "let holes = [];",
        "for x in [-10, 10] { for y in [-10, 10] {",
        "    holes.push(cylinder(4, 20).move(x, y, -5));",
        "} }",
        "show(cut(plate, holes), \"plate\");",
    ]));
    // 40·40·5 − 4 · π · 2² · 5
    assert_volume(&out, "plate", 8000.0 - 80.0 * PI);
    assert_eq!(shape(&out, "plate").solid_count(), 1);
}

#[test]
fn ag_sc_05_batched_union_of_disjoint_bodies_is_allowed() {
    let out = run_ok(&src(&[
        "let parts = [box(2, 2, 2), box(2, 2, 2).move(10, 0, 0), sphere(2).move(0, 10, 0)];",
        "show(union(parts), \"u\");",
    ]));
    // 8 + 8 + 4/3 π, three separate solids (AG-SC-12 only forbids shrinking).
    assert_volume(&out, "u", 16.0 + 4.0 / 3.0 * PI);
    assert_eq!(shape(&out, "u").solid_count(), 3);
}

#[test]
fn ag_sc_05_batched_union_of_overlapping_bodies() {
    let out = run_ok(&src(&[
        "let parts = [];",
        "for i in 0..5 { parts.push(box(10, 10, 10).move(i * 5, 0, 0)); }",
        "show(union(parts), \"bar\");",
    ]));
    // Five 10-cubes shifted by 5: one bar from x = −5 to 25, V = 30 · 10 · 10.
    assert_volume(&out, "bar", 3000.0);
    assert_eq!(shape(&out, "bar").solid_count(), 1);
    assert_bbox(&out, "bar", [-5.0, -5.0, 0.0], [25.0, 5.0, 10.0]);
}

#[test]
fn ag_sc_05_union_of_an_empty_array_is_an_error() {
    let r = run("let u = union([]);\nshow(u, \"u\");", &RunOptions::default());
    assert!(r.is_err(), "{r:?}");
}

// ---------------------------------------------------------------------------
// AG-SC-12: plausibility after booleans
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_12_cut_of_a_disjoint_body_is_a_geometry_error() {
    // The cut does not shrink the volume: almost always a placement mistake.
    let e = run_err(&src(&[
        "let a = box(10, 10, 10);",
        "let b = box(10, 10, 10).move(50, 0, 0);",
        "let c = cut(a, [b]);",
    ]));
    expect_geometry(&e, 3, &["cut"]);
}

#[test]
fn ag_sc_12_cut_operator_of_a_disjoint_body_is_a_geometry_error() {
    let e = run_err(&src(&[
        "let a = box(10, 10, 10);",
        "let b = box(10, 10, 10).move(50, 0, 0);",
        "let c = a - b;",
    ]));
    expect_geometry(&e, 3, &["cut", "-"]);
}

#[test]
fn ag_sc_12_cut_that_removes_everything_is_a_geometry_error() {
    // Result volume 0 ≤ 1e-6 mm³.
    let e = run_err(&src(&["let a = box(10, 10, 10);", "let c = a - box(20, 20, 20);"]));
    expect_geometry(&e, 2, &["cut", "-"]);
}

#[test]
fn ag_sc_12_common_of_disjoint_bodies_is_a_geometry_error() {
    let e = run_err(&src(&["let a = box(10, 10, 10);", "let c = a & sphere(4).move(0, 0, 50);"]));
    expect_geometry(&e, 2, &["common", "&"]);
}

// ---------------------------------------------------------------------------
// AG-SC-06: transforms and patterns
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_06_move_returns_a_new_shape() {
    let out = run_ok(&src(&[
        "let a = box(10, 20, 30);",
        "let b = a.move(5, 0, 0);",
        "show(a, \"a\");",
        "show(b, \"b\");",
    ]));
    assert_bbox(&out, "a", [-5.0, -10.0, 0.0], [5.0, 10.0, 30.0]);
    assert_bbox(&out, "b", [0.0, -10.0, 0.0], [10.0, 10.0, 30.0]);
    assert_volume(&out, "b", 6000.0);
}

#[test]
fn ag_sc_06_rotate_about_named_and_vector_axes_in_degrees() {
    let out = run_ok(&src(&[
        "let a = box(10, 20, 30);",
        "show(a.rotate(\"z\", 90), \"z90\");",
        "show(a.rotate([0, 0, 1], 90), \"v90\");",
        "show(a.rotate(\"x\", 90), \"x90\");",
        "show(a.rotate(\"y\", 90), \"y90\");",
        "show(box(10, 10, 10).rotate(\"z\", 45), \"z45\");",
    ]));
    // About Z by +90°: (x, y) → (−y, x); x ∈ [−5, 5], y ∈ [−10, 10] → x ∈ [−10, 10], y ∈ [−5, 5].
    assert_bbox(&out, "z90", [-10.0, -5.0, 0.0], [10.0, 5.0, 30.0]);
    assert_bbox(&out, "v90", [-10.0, -5.0, 0.0], [10.0, 5.0, 30.0]);
    // About X by +90°: (y, z) → (−z, y); y ∈ [−10, 10], z ∈ [0, 30] → y ∈ [−30, 0], z ∈ [−10, 10].
    assert_bbox(&out, "x90", [-5.0, -30.0, -10.0], [5.0, 0.0, 10.0]);
    // About Y by +90°: (z, x) → (−x, z), i.e. x' = z, z' = −x:
    // x ∈ [−5, 5], z ∈ [0, 30] → x' ∈ [0, 30], z' ∈ [−5, 5].
    assert_bbox(&out, "y90", [0.0, -10.0, -5.0], [30.0, 10.0, 5.0]);
    // A 10-cube turned 45° about Z: half diagonal 5√2 in X and Y.
    let h = 5.0 * 2f64.sqrt();
    assert_bbox(&out, "z45", [-h, -h, 0.0], [h, h, 10.0]);
    assert_volume(&out, "z45", 1000.0);
}

#[test]
fn ag_sc_06_mirror_keeps_a_valid_positive_solid() {
    let out = run_ok(&src(&[
        "let a = box(10, 20, 30).move(5, 10, 0);",
        "show(a.mirror(\"yz\"), \"yz\");",
        "show(a.mirror(\"xz\"), \"xz\");",
        "show(a.mirror(\"xy\"), \"xy\");",
    ]));
    // a: x ∈ [0, 10], y ∈ [0, 20], z ∈ [0, 30]; each mirror negates one axis.
    assert_bbox(&out, "yz", [-10.0, 0.0, 0.0], [0.0, 20.0, 30.0]);
    assert_bbox(&out, "xz", [0.0, -20.0, 0.0], [10.0, 0.0, 30.0]);
    assert_bbox(&out, "xy", [0.0, 0.0, -30.0], [10.0, 20.0, 0.0]);
    for n in ["yz", "xz", "xy"] {
        assert_volume(&out, n, 6000.0);
    }
}

#[test]
fn ag_sc_06_scale_about_the_origin() {
    let out = run_ok("show(box(10, 20, 30).move(5, 0, 0).scale(2), \"s\");");
    // Volume × 2³; x ∈ [0, 10] → [0, 20], y ∈ [−10, 10] → [−20, 20], z → [0, 60].
    assert_volume(&out, "s", 48_000.0);
    assert_bbox(&out, "s", [0.0, -20.0, 0.0], [20.0, 20.0, 60.0]);
}

#[test]
fn ag_sc_06_invalid_transforms_fail() {
    for call in
        ["box(1, 1, 1).rotate(\"w\", 90)", "box(1, 1, 1).mirror(\"xx\")", "box(1, 1, 1).scale(0)"]
    {
        let r = run(&format!("let a = 1;\nlet s = {call};"), &RunOptions::default());
        assert!(r.is_err(), "{call}: {r:?}");
    }
}

#[test]
fn ag_sc_06_linear_pattern() {
    let out = run_ok("show(linear(box(2, 2, 2), 4, [5, 0, 1]), \"row\");");
    // Copies at offsets k · (5, 0, 1), k = 0..3, disjoint (gap 3 in x):
    // x ∈ [−1, 15 + 1], z ∈ [0, 3 + 2], V = 4 · 8.
    assert_volume(&out, "row", 32.0);
    assert_eq!(shape(&out, "row").solid_count(), 4);
    assert_bbox(&out, "row", [-1.0, -1.0, 0.0], [16.0, 1.0, 5.0]);
}

#[test]
fn ag_sc_06_polar_pattern_about_z() {
    let out = run_ok("show(polar(box(2, 2, 2).move(10, 0, 0), 4, 360), \"ring\");");
    // 4 copies over a full turn: 0°, 90°, 180°, 270° → the cube [9, 11] lands
    // on ±x and ±y; bbox ±11 in X and Y, V = 4 · 8.
    assert_volume(&out, "ring", 32.0);
    assert_eq!(shape(&out, "ring").solid_count(), 4);
    assert_bbox(&out, "ring", [-11.0, -11.0, 0.0], [11.0, 11.0, 2.0]);
}

// ---------------------------------------------------------------------------
// AG-SC-07: profiles, extrude, revolve, loft, planes
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_07_extrude_rect_circle_polygon() {
    let out = run_ok(&src(&[
        "show(extrude(rect(10, 20), 5), \"rect\");",
        "show(extrude(circle(10), 4), \"circle\");",
        "show(extrude(polygon([[0, 0], [10, 0], [0, 10]]), 3), \"tri\");",
    ]));
    assert_volume(&out, "rect", 1000.0);
    assert_bbox(&out, "rect", [-5.0, -10.0, 0.0], [5.0, 10.0, 5.0]);
    // π r² h = π · 25 · 4
    assert_volume(&out, "circle", 100.0 * PI);
    assert_bbox(&out, "circle", [-5.0, -5.0, 0.0], [5.0, 5.0, 4.0]);
    // Right triangle 10 × 10 / 2 · 3
    assert_volume(&out, "tri", 150.0);
    assert_bbox(&out, "tri", [0.0, 0.0, 0.0], [10.0, 10.0, 3.0]);
}

#[test]
fn ag_sc_07_slot_is_a_stadium_of_overall_length_l() {
    let out = run_ok("show(extrude(slot(20, 6), 2), \"slot\");");
    // Overall length 20, width 6: rectangle (20 − 6) × 6 plus two half
    // circles of radius 3: A = 84 + 9π; V = 2A. Long axis along X, centered.
    assert_volume(&out, "slot", 2.0 * (84.0 + 9.0 * PI));
    assert_bbox(&out, "slot", [-10.0, -3.0, 0.0], [10.0, 3.0, 2.0]);
}

#[test]
fn ag_sc_07_revolved_rectangle_is_a_hollow_cylinder() {
    let out = run_ok(&src(&[
        "let p = polygon([[4, 0], [10, 0], [10, 10], [4, 10]]);",
        "show(revolve(p, 360), \"tube\");",
        "show(revolve(p, 90), \"quarter\");",
    ]));
    // x = radius 4..10, y = height 0..10: V = π (10² − 4²) · 10 = 840π.
    assert_volume(&out, "tube", 840.0 * PI);
    assert_bbox(&out, "tube", [-10.0, -10.0, 0.0], [10.0, 10.0, 10.0]);
    assert_volume(&out, "quarter", 210.0 * PI);
}

#[test]
fn ag_sc_07_loft_of_two_squares_is_a_pyramid_frustum() {
    let out = run_ok(&src(&[
        "show(loft([rect(20, 20).at_z(0), rect(10, 10).at_z(15)]), \"frustum\");",
        "show(loft([circle(10).at_z(0), circle(4).at_z(6)]), \"cone\");",
    ]));
    // V = h/3 (A1 + A2 + √(A1 A2)) = 15/3 (400 + 100 + 200) = 3500
    assert_volume(&out, "frustum", 3500.0);
    assert_bbox(&out, "frustum", [-10.0, -10.0, 0.0], [10.0, 10.0, 15.0]);
    // Circular frustum as in ag_sc_04_cone_is_a_frustum: 78π
    assert_volume(&out, "cone", 78.0 * PI);
}

#[test]
fn ag_sc_07_profiles_on_origin_planes() {
    let out = run_ok(&src(&[
        "show(extrude(on(\"xz\", rect(10, 20)), 5), \"xz\");",
        "show(extrude(on(\"yz\", rect(10, 20)), 5), \"yz\");",
        "show(extrude(on(\"xy\", rect(10, 20)), 5), \"xy\");",
    ]));
    // Plane frames as SK-PL-01 (ADR-0006): XZ u = +X, v = +Z, normal −Y;
    // YZ u = +Y, v = +Z, normal +X. Extrusion follows the normal.
    assert_bbox(&out, "xz", [-5.0, -5.0, -10.0], [5.0, 0.0, 10.0]);
    assert_bbox(&out, "yz", [0.0, -5.0, -10.0], [5.0, 5.0, 10.0]);
    assert_bbox(&out, "xy", [-5.0, -10.0, 0.0], [5.0, 10.0, 5.0]);
    for n in ["xz", "yz", "xy"] {
        assert_volume(&out, n, 1000.0);
    }
}

#[test]
fn ag_sc_07_invalid_profiles_are_runtime_errors() {
    for call in [
        "rect(0, 5)",
        "circle(-1)",
        "slot(4, 6)",
        "polygon([[0, 0], [1, 1]])",
        "extrude(rect(1, 1), 0)",
        "loft([rect(1, 1).at_z(0)])",
    ] {
        let e = run_err(&format!("let a = 1;\nlet p = {call};"));
        expect_runtime(&e, 2);
    }
}

// ---------------------------------------------------------------------------
// AG-SC-08: pipe
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_08_pipe_volume_is_section_times_centre_line_length() {
    let out = run_ok(&src(&[
        "show(pipe([[0, 0, 0], [0, 0, 20], [30, 0, 20]], 2, 5), \"bent\");",
        "show(pipe([[0, 0, 0], [0, 0, 10]], 2, 5), \"straight\");",
    ]));
    // Corner rounded with R = 5: straight parts 20 − 5 and 30 − 5, quarter
    // arc π/2 · 5. Pappus: V = π (d/2)² · L = π · (40 + 2.5π).
    assert_volume(&out, "bent", PI * (40.0 + 2.5 * PI));
    // Start disc at z = 0 (x, y ∈ ±1); arc centre (5, 0, 15), outer reach
    // z = 15 + 5 + 1 = 21; end disc in the plane x = 30.
    assert_bbox(&out, "bent", [-1.0, -1.0, 0.0], [30.0, 1.0, 21.0]);
    assert_volume(&out, "straight", 10.0 * PI);
}

#[test]
fn ag_sc_08_bend_radius_not_above_half_the_diameter_is_a_runtime_error() {
    // bend_radius <= d/2 would self-intersect (M0: OCCT does not notice).
    for r in ["1", "0.5", "0"] {
        let e = run_err(&format!(
            "let pts = [[0, 0, 0], [0, 0, 20], [30, 0, 20]];\nlet p = pipe(pts, 2, {r});"
        ));
        expect_runtime(&e, 2);
    }
    // Just above d/2 is allowed.
    run_ok("show(pipe([[0, 0, 0], [0, 0, 20], [30, 0, 20]], 2, 1.01), \"p\");");
}

#[test]
fn ag_sc_08_pipe_needs_two_points_and_a_positive_diameter() {
    for call in ["pipe([[0, 0, 0]], 2, 5)", "pipe([[0, 0, 0], [0, 0, 10]], 0, 5)"] {
        let e = run_err(&format!("let a = 1;\nlet p = {call};"));
        expect_runtime(&e, 2);
    }
}

// ---------------------------------------------------------------------------
// AG-SC-09: fillet, chamfer, edge selectors
// ---------------------------------------------------------------------------

/// Box a × b × c with all 12 edges rounded by r (sphere octants at the
/// corners) is the parallel body at distance r of the inner box
/// A = a − 2r, B = b − 2r, C = c − 2r. Steiner's formula:
///   V = ABC + 2r (AB + BC + CA) + π r² (A + B + C) + 4/3 π r³
/// Expanded: V = abc − (4 − π) r² (a + b + c) + (16 − 14π/3) r³
/// (check: a = b = c = 2r gives 4/3 π r³, the sphere).
fn rounded_box_volume(a: f64, b: f64, c: f64, r: f64) -> f64 {
    let (aa, bb, cc) = (a - 2.0 * r, b - 2.0 * r, c - 2.0 * r);
    aa * bb * cc
        + 2.0 * r * (aa * bb + bb * cc + cc * aa)
        + PI * r * r * (aa + bb + cc)
        + 4.0 / 3.0 * PI * r.powi(3)
}

#[test]
fn rounded_box_formula_forms_agree() {
    let (a, b, c, r) = (20.0, 30.0, 40.0, 2.0);
    let expanded =
        a * b * c - (4.0 - PI) * r * r * (a + b + c) + (16.0 - 14.0 * PI / 3.0) * r.powi(3);
    assert!((rounded_box_volume(a, b, c, r) - expanded).abs() < 1e-9);
    assert!((rounded_box_volume(2.0, 2.0, 2.0, 1.0) - 4.0 / 3.0 * PI).abs() < 1e-12);
}

#[test]
fn ag_sc_09_fillet_all_edges_of_a_box() {
    let out = run_ok("let b = box(20, 30, 40);\nshow(fillet(b, all_edges(b), 2), \"r\");");
    assert_volume(&out, "r", rounded_box_volume(20.0, 30.0, 40.0, 2.0));
    assert_bbox(&out, "r", [-10.0, -15.0, 0.0], [10.0, 15.0, 40.0]);
}

#[test]
fn ag_sc_09_fillet_vertical_edges() {
    let out =
        run_ok("let b = box(20, 30, 40);\nshow(fillet(b, edges(b).parallel(\"z\"), 3), \"r\");");
    // Each vertical edge loses the corner square minus the quarter disc,
    // r² (1 − π/4), along the height c: V = abc − 4 r² (1 − π/4) c.
    assert_volume(&out, "r", 24_000.0 - (4.0 - PI) * 9.0 * 40.0);
}

#[test]
fn ag_sc_09_fillet_top_circle_of_a_cylinder() {
    let out = run_ok("let c = cylinder(20, 10);\nshow(fillet(c, edges(c).top(), 2), \"r\");");
    // Removed cross-section: corner square r² minus quarter disc, area
    // A = r² (1 − π/4). Its centroid lies ū = r (10 − 3π) / (3 (4 − π)) from
    // the outer face (moments: square r³/2, quarter disc π r³/4 − r³/3).
    // Pappus: removed = 2π (R − ū) A with R = 10, r = 2.
    let r: f64 = 2.0;
    let a = r * r * (1.0 - PI / 4.0);
    let u = r * (10.0 - 3.0 * PI) / (3.0 * (4.0 - PI));
    assert_volume(&out, "r", PI * 100.0 * 10.0 - 2.0 * PI * (10.0 - u) * a);
}

#[test]
fn ag_sc_09_chamfer_two_top_edges() {
    let out = run_ok(
        "let b = box(20, 30, 40);\nshow(chamfer(b, edges(b).parallel(\"x\").top(), 3), \"c\");",
    );
    // Two top edges along X (length 20); each removes a triangular prism
    // d²/2 · l = 4.5 · 20.
    assert_volume(&out, "c", 24_000.0 - 2.0 * 4.5 * 20.0);
}

#[test]
fn ag_sc_09_chamfer_edge_loops_bottom_and_at_z() {
    let out = run_ok(&src(&[
        "let b = box(20, 30, 40);",
        "show(chamfer(b, edges(b).bottom(), 2), \"bottom\");",
        "show(chamfer(b, edges(b).lines().at_z(40, 0.001), 2), \"top\");",
        "show(chamfer(b, edges(b).lines().parallel(\"z\"), 2), \"vertical\");",
    ]));
    // A chamfered loop of four edges: four prisms d²/2 · L (perimeter
    // 2(a + b)) overlap pairwise at each corner in {x' + z' < d, y' + z' < d},
    // volume ∫₀ᵈ (d − z)² dz = d³/3. V = abc − d² (a + b) + 4 d³/3.
    let loop_v = 24_000.0 - 4.0 * 50.0 + 4.0 * 8.0 / 3.0;
    assert_volume(&out, "bottom", loop_v);
    assert_volume(&out, "top", loop_v);
    // Four separate vertical edges: 4 · d²/2 · c.
    assert_volume(&out, "vertical", 24_000.0 - 4.0 * 2.0 * 40.0);
}

#[test]
fn ag_sc_09_chamfer_circles_of_a_cylinder() {
    let out = run_ok("let c = cylinder(20, 10);\nshow(chamfer(c, edges(c).circles(), 1), \"c\");");
    // Two rings, each triangle d²/2 with centroid at R − d/3 (Pappus):
    // V = π R² h − 2 · 2π (R − d/3) d²/2, R = 10, d = 1.
    assert_volume(&out, "c", 1000.0 * PI - 2.0 * 2.0 * PI * (10.0 - 1.0 / 3.0) * 0.5);
}

#[test]
fn ag_sc_09_failing_fillet_names_the_edge() {
    let e = run_err("let b = box(10, 10, 10);\nlet f = fillet(b, all_edges(b), 20);");
    match &e {
        ScriptError::Geometry { function, message, .. } => {
            assert_eq!(function, "fillet");
            assert!(
                message.to_lowercase().contains("edge"),
                "message names the failing edge: {e:?}"
            );
        }
        other => panic!("expected Geometry, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// AG-SC-10 / AG-SC-12: shell
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_10_shell_open_at_the_top() {
    let out = run_ok(&src(&[
        "let b = box(20, 30, 40);",
        "show(shell(b, 2, faces(b).top()), \"cup\");",
        "show(shell(b, 2, faces(b).bottom()), \"hood\");",
    ]));
    // Wall inward, outer size kept: V = abc − (a − 2t)(b − 2t)(c − t)
    // = 24000 − 16 · 26 · 38 = 8192.
    assert_volume(&out, "cup", 8192.0);
    assert_bbox(&out, "cup", [-10.0, -15.0, 0.0], [10.0, 15.0, 40.0]);
    assert_volume(&out, "hood", 8192.0);
}

#[test]
fn ag_sc_10_shell_thicker_than_half_the_box_is_a_geometry_error() {
    // M0: OCCT returned a "valid" shell with volume 0 or unchanged. 6 > 10/2.
    let e = run_err("let b = box(10, 10, 10);\nlet s = shell(b, 6, faces(b).top());");
    expect_geometry(&e, 2, &["shell"]);
}

// ---------------------------------------------------------------------------
// AG-SC-11: measure
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_11_volume_area_center_bbox_solids_in_the_script() {
    let out = run_ok(&src(&[
        "let b = box(10, 20, 30).move(5, 0, 0);",
        "print(b.volume());",
        "print(b.area());",
        "let c = b.center();",
        "print(c[0]); print(c[1]); print(c[2]);",
        "let bb = b.bbox();",
        "print(bb.min[0]); print(bb.max[1]); print(bb.size[2]);",
        "print(b.is_valid());",
        "print((b + box(1, 1, 1).move(50, 0, 0)).solids());",
        "print(cylinder(10, 20).area());",
    ]));
    assert_rel(printed(&out, 0), 6000.0, VOL, "volume");
    // 2 (10·20 + 20·30 + 10·30) = 2200
    assert_rel(printed(&out, 1), 2200.0, VOL, "area");
    // Centre of the moved box
    for (i, e) in [5.0, 0.0, 15.0].into_iter().enumerate() {
        assert!((printed(&out, 2 + i) - e).abs() < POS, "center {:?}", out.printed);
    }
    assert!((printed(&out, 5) - 0.0).abs() < POS, "bbox.min.x {:?}", out.printed);
    assert!((printed(&out, 6) - 10.0).abs() < POS, "bbox.max.y {:?}", out.printed);
    assert!((printed(&out, 7) - 30.0).abs() < POS, "bbox.size.z {:?}", out.printed);
    assert_eq!(out.printed[8], "true");
    assert_eq!(out.printed[9], "2");
    // 2πr² + 2πrh = 2π·25 + 2π·5·20 = 250π
    assert_rel(printed(&out, 10), 250.0 * PI, VOL, "cylinder area");
}

#[test]
fn ag_sc_11_distance_between_two_boxes() {
    let out = run_ok(&src(&[
        "let a = box(10, 10, 10);",
        "print(distance(a, box(10, 10, 10).move(30, 0, 0)));",
        "print(distance(a, box(10, 10, 10).move(30, 40, 0)));",
        "print(distance(a, box(10, 10, 10).move(5, 0, 0)));",
    ]));
    // Faces x = 5 and x = 25: gap 20.
    assert!((printed(&out, 0) - 20.0).abs() < POS, "{:?}", out.printed);
    // Gaps 20 in X and 30 in Y, edge to edge: √(20² + 30²).
    assert!((printed(&out, 1) - 1300f64.sqrt()).abs() < POS, "{:?}", out.printed);
    // Overlapping bodies: 0.
    assert!(printed(&out, 2).abs() < POS, "{:?}", out.printed);
}

#[test]
fn ag_sc_11_ray_hits_are_sorted_by_distance() {
    let out = run_ok(&src(&[
        "let b = box(20, 20, 10) - cylinder(4, 10);",
        "for h in ray(b, [-50, 0, 5], [1, 0, 0]) { print(h[0]); print(h[1]); print(h[2]); }",
        "print(\"--\");",
        "for h in ray(b, [50, 0, 5], [-2, 0, 0]) { print(h[0]); }",
        "print(\"--\");",
        "for h in ray(b, [0, 0, 5], [1, 0, 0]) { print(h[0]); }",
        "print(\"--\");",
        "print(ray(b, [-50, 50, 5], [1, 0, 0]).len());",
    ]));
    // Box faces at x = ±10, hole wall (r = 2) at x = ±2, all at y = 0, z = 5.
    let groups: Vec<&[String]> = out.printed.split(|l| l == "--").collect();
    assert_eq!(groups.len(), 4, "{:?}", out.printed);
    let xs = |g: &[String]| -> Vec<f64> { g.iter().map(|l| l.parse::<f64>().unwrap()).collect() };
    let first = xs(groups[0]);
    assert_eq!(first.len(), 12, "4 hits × 3 coordinates: {first:?}");
    for (k, x) in [-10.0, -2.0, 2.0, 10.0].into_iter().enumerate() {
        let p = &first[3 * k..3 * k + 3];
        assert!(
            (p[0] - x).abs() < POS && p[1].abs() < POS && (p[2] - 5.0).abs() < POS,
            "hit {k}: {p:?}"
        );
    }
    // From the other side, direction not normalised: sorted by distance.
    let back = xs(groups[1]);
    assert_eq!(back.len(), 4, "{back:?}");
    for (a, e) in back.iter().zip([10.0, 2.0, -2.0, -10.0]) {
        assert!((a - e).abs() < POS, "{back:?}");
    }
    // A ray is a half line: from inside the hole only the hits ahead count.
    let ahead = xs(groups[2]);
    assert_eq!(ahead.len(), 2, "{ahead:?}");
    assert!((ahead[0] - 2.0).abs() < POS && (ahead[1] - 10.0).abs() < POS, "{ahead:?}");
    // y = 50 misses the box.
    assert_eq!(groups[3], ["0"]);
}

#[test]
fn ag_sc_11_rust_side_measures_agree_with_the_script() {
    let out =
        run_ok("let s = box(10, 20, 30) + box(10, 20, 30).move(0, 0, 40);\nshow(s, \"two\");");
    let s = shape(&out, "two");
    assert!(s.is_valid());
    assert_eq!(s.solid_count(), 2);
    assert_rel(s.volume(), 12_000.0, VOL, "two boxes");
    assert_bbox(&out, "two", [-5.0, -10.0, 0.0], [5.0, 10.0, 70.0]);
}

// ---------------------------------------------------------------------------
// AG-SC-13: pin and hole
// ---------------------------------------------------------------------------

#[test]
fn ag_sc_13_pin_points_down_from_zero() {
    let out = run_ok("show(pin(4, 2.45), \"pin\");");
    // π r² l = π · 4 · 2.45
    assert_volume(&out, "pin", PI * 4.0 * 2.45);
    assert_bbox(&out, "pin", [-2.0, -2.0, -2.45], [2.0, 2.0, 0.0]);
}

#[test]
fn ag_sc_13_hole_adds_the_clearance() {
    let out = run_ok("show(hole(4, 5, 0.2), \"hole\");");
    // Diameter 4 + 2 · 0.2 = 4.4, depth 5 + 0.2 = 5.2: V = π · 2.2² · 5.2.
    assert_volume(&out, "hole", PI * 2.2 * 2.2 * 5.2);
    assert_size(&out, "hole", [4.4, 4.4, 5.2]);
}

#[test]
fn ag_sc_13_invalid_pin_and_hole() {
    for call in ["pin(0, 2)", "pin(4, -1)", "hole(4, 5, -0.1)", "hole(-4, 5, 0.2)"] {
        let e = run_err(&format!("let a = 1;\nlet p = {call};"));
        expect_runtime(&e, 2);
    }
}

// ---------------------------------------------------------------------------
// AG-SC-14: import and export
// ---------------------------------------------------------------------------

/// Independent binary STL reader: triangles as f32 corner triples.
fn read_stl(bytes: &[u8]) -> Vec<[[f32; 3]; 3]> {
    assert!(bytes.len() >= 84, "STL shorter than its header");
    let n = u32::from_le_bytes(bytes[80..84].try_into().expect("4 bytes")) as usize;
    assert_eq!(bytes.len(), 84 + 50 * n, "binary STL size for {n} triangles");
    let f = |o: usize| f32::from_le_bytes(bytes[o..o + 4].try_into().expect("4 bytes"));
    (0..n)
        .map(|t| {
            let base = 84 + 50 * t + 12;
            [0, 1, 2].map(|v| [0, 1, 2].map(|c| f(base + 12 * v + 4 * c)))
        })
        .collect()
}

/// Signed volume (divergence theorem) and closedness: every directed edge
/// appears exactly once and its reverse exists (after exact vertex welding).
fn mesh_volume_and_closed(tris: &[[[f32; 3]; 3]]) -> (f64, bool) {
    use std::collections::HashMap;
    let key = |p: [f32; 3]| p.map(f32::to_bits);
    let mut edges: HashMap<([u32; 3], [u32; 3]), usize> = HashMap::new();
    let mut volume = 0.0;
    for t in tris {
        let [a, b, c] = t.map(|p| p.map(f64::from));
        volume += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
        for (p, q) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *edges.entry((key(p), key(q))).or_default() += 1;
        }
    }
    let closed = edges.iter().all(|((p, q), n)| *n == 1 && edges.get(&(*q, *p)) == Some(&1));
    (volume, closed)
}

#[test]
fn ag_sc_14_export_stl_is_closed_with_the_exact_box_volume() {
    let dir = scratch("export_stl");
    let out = run_ok("show(box(10, 20, 30) - box(4, 4, 30), \"frame\");");
    let path = dir.join("frame.stl");
    export_stl(&out.shapes, &path).expect("STL export");
    let tris = read_stl(&std::fs::read(&path).unwrap());
    let (v, closed) = mesh_volume_and_closed(&tris);
    assert!(closed, "STL must be watertight");
    // Planar faces mesh exactly: 6000 − 4 · 4 · 30 = 5520 (f32 corners exact).
    assert_rel(v, 5520.0, VOL, "STL volume");
}

/// Independent 3MF check: `unzip -t` validates the ZIP, then the model XML
/// is parsed here (not with our reader) into objects of triangles.
fn read_3mf(path: &Path) -> Vec<(String, Vec<[[f32; 3]; 3]>)> {
    let test =
        std::process::Command::new("unzip").arg("-t").arg(path).output().expect("unzip installed");
    assert!(test.status.success(), "unzip -t: {}", String::from_utf8_lossy(&test.stdout));
    let model = std::process::Command::new("unzip")
        .arg("-p")
        .arg(path)
        .arg("3D/3dmodel.model")
        .output()
        .expect("unzip -p");
    assert!(model.status.success());
    let xml = String::from_utf8(model.stdout).expect("3MF model is UTF-8");
    // `tag` is the text after "<vertex ", "<triangle " or "<object ", so its
    // first attribute has no leading space: prepend one before searching.
    let attr = |tag: &str, name: &str| -> String {
        let tag = format!(" {tag}");
        let pat = format!(" {name}=\"");
        let start = tag.find(&pat).unwrap_or_else(|| panic!("{name} missing in {tag}")) + pat.len();
        tag[start..start + tag[start..].find('"').expect("closing quote")].to_string()
    };
    let mut objects = Vec::new();
    for chunk in xml.split("<object ").skip(1) {
        let head = &chunk[..chunk.find('>').expect("end of object tag")];
        let name = attr(head, "name");
        let verts: Vec<[f32; 3]> = chunk
            .split("<vertex ")
            .skip(1)
            .map(|t| ["x", "y", "z"].map(|a| attr(t, a).parse().expect("vertex coordinate")))
            .collect();
        let tris = chunk
            .split("<triangle ")
            .skip(1)
            .map(|t| {
                ["v1", "v2", "v3"]
                    .map(|a| verts[attr(t, a).parse::<usize>().expect("vertex index")])
            })
            .collect();
        objects.push((name, tris));
    }
    objects
}

#[test]
fn ag_sc_14_export_3mf_has_one_closed_object_per_shape() {
    let dir = scratch("export_3mf");
    let out =
        run_ok("show(box(10, 20, 30), \"block\");\nshow(sphere(30).move(40, 0, 15), \"ball\");");
    let path = dir.join("parts.3mf");
    export_3mf(&out.shapes, &path).expect("3MF export");
    let objects = read_3mf(&path);
    let names: Vec<&str> = objects.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["block", "ball"]);
    let (v, closed) = mesh_volume_and_closed(&objects[0].1);
    assert!(closed);
    // 3MF stores decimal text: allow the printed precision, still 1e-6.
    assert_rel(v, 6000.0, VOL, "3MF block");
    let (v, closed) = mesh_volume_and_closed(&objects[1].1);
    assert!(closed);
    // Chordal tolerance 0.01 mm on r = 15: the inscribed mesh loses < 0.2 %
    // (same bound as features.rs `export_meshes_are_finer_and_watertight`).
    let exact = 4.0 / 3.0 * PI * 15f64.powi(3);
    assert!(v < exact && (exact - v) / exact < 2e-3, "3MF sphere {v} vs {exact}");
}

#[test]
fn ag_sc_14_step_round_trip_keeps_the_exact_volume() {
    let dir = scratch("step_round_trip");
    let out = run_ok(&src(&[
        "show(box(10, 20, 30) - cylinder(4, 30), \"block\");",
        "show(sphere(10).move(30, 0, 0), \"ball\");",
    ]));
    let path = dir.join("part.step");
    export_step(&out.shapes, &path).expect("STEP export");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("ISO-10303-21;"), "STEP header: {}", &text[..text.len().min(40)]);
    let options = RunOptions { project_dir: Some(dir.clone()), ..RunOptions::default() };
    let back = run("let s = import_step(\"part.step\");\nshow(s, \"back\");", &options)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let s = &back.shapes[0].1;
    assert!(s.is_valid());
    assert_eq!(s.solid_count(), 2);
    // 6000 − π · 4 · 30 + 4/3 π · 125. M0: STEP round trip deviates < 1e-12.
    let expected = 6000.0 - 120.0 * PI + 4.0 / 3.0 * PI * 125.0;
    assert_rel(s.volume(), expected, VOL, "STEP round trip");
    let original: f64 = out.shapes.iter().map(|(_, s)| s.volume()).sum();
    assert_rel(s.volume(), original, 1e-9, "STEP round trip vs original");
}

#[test]
fn ag_sc_14_import_step_stays_inside_the_project() {
    let root = scratch("step_outside");
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let out = run_ok("show(box(1, 1, 1), \"b\");");
    export_step(&out.shapes, &root.join("outside.step")).unwrap();
    let options = RunOptions { project_dir: Some(project), ..RunOptions::default() };
    let absolute = root.join("outside.step");
    for source in [
        "let s = import_step(\"../outside.step\");".to_string(),
        format!("let s = import_step(\"{}\");", absolute.display()),
        "let s = import_step(\"missing.step\");".to_string(),
    ] {
        let r = run(&source, &options);
        assert!(r.is_err(), "{source}: {r:?}");
    }
    // No project: no file access at all.
    let r =
        run(&format!("let s = import_step(\"{}\");", absolute.display()), &RunOptions::default());
    assert!(r.is_err(), "{r:?}");
}

// ---------------------------------------------------------------------------
// AG-SC-16: every documented example runs
// ---------------------------------------------------------------------------

fn docs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

/// All ```rhai fenced blocks of a Markdown file, with their start line.
fn rhai_blocks(markdown: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut current: Option<(usize, Vec<&str>)> = None;
    for (i, line) in markdown.lines().enumerate() {
        let trimmed = line.trim_start();
        match &mut current {
            None if trimmed.starts_with("```rhai") => current = Some((i + 1, Vec::new())),
            None => {}
            Some(_) if trimmed.starts_with("```") => {
                let (start, body) = current.take().expect("inside a block");
                blocks.push((start, body.join("\n")));
            }
            Some((_, body)) => body.push(line),
        }
    }
    assert!(current.is_none(), "unterminated ```rhai block");
    blocks
}

/// Script API functions of AG-SC-01…14 that need a doc entry.
const API: &[&str] = &[
    "show",
    "box",
    "cylinder",
    "sphere",
    "cone",
    "torus",
    "union",
    "cut",
    "move",
    "rotate",
    "mirror",
    "scale",
    "linear",
    "polar",
    "rect",
    "circle",
    "polygon",
    "slot",
    "extrude",
    "revolve",
    "loft",
    "at_z",
    "on",
    "pipe",
    "fillet",
    "chamfer",
    "edges",
    "all_edges",
    "parallel",
    "circles",
    "lines",
    "top",
    "bottom",
    "shell",
    "faces",
    "volume",
    "area",
    "bbox",
    "center",
    "distance",
    "ray",
    "is_valid",
    "solids",
    "pin",
    "hole",
    "import_step",
];

#[test]
fn ag_sc_16_every_example_in_the_api_reference_runs() {
    let path = docs_dir().join("script-api.md");
    let doc = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} must exist (AG-SC-16): {e}", path.display()));
    let blocks = rhai_blocks(&doc);
    assert!(!blocks.is_empty(), "no ```rhai examples in {}", path.display());
    // Examples that import files resolve them relative to docs/.
    let options = RunOptions { project_dir: Some(docs_dir()), ..RunOptions::default() };
    let failures: Vec<String> = blocks
        .iter()
        .filter_map(|(line, code)| run(code, &options).err().map(|e| format!("line {line}: {e:?}")))
        .collect();
    assert!(failures.is_empty(), "failing examples in script-api.md:\n{}", failures.join("\n"));
}

#[test]
fn ag_sc_16_every_api_function_is_documented_with_an_example() {
    let path = docs_dir().join("script-api.md");
    let doc = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} must exist (AG-SC-16): {e}", path.display()));
    let examples: String =
        rhai_blocks(&doc).into_iter().map(|(_, c)| c).collect::<Vec<_>>().join("\n");
    let call = |text: &str, f: &str| {
        text.match_indices(&format!("{f}(")).any(|(i, _)| {
            i == 0
                || !text[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    };
    let undocumented: Vec<&str> = API.iter().copied().filter(|f| !call(&doc, f)).collect();
    assert!(undocumented.is_empty(), "no signature in script-api.md for {undocumented:?}");
    let no_example: Vec<&str> = API.iter().copied().filter(|f| !call(&examples, f)).collect();
    assert!(no_example.is_empty(), "no ```rhai example uses {no_example:?}");
}
