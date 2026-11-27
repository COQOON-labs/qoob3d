//! `import_mesh(path)` (docs/requirements/agent.md §6.5, AG-SC-23; ADR-0034):
//! STL and 3MF files in the project folder become polyhedral solids. Every
//! expected value is derived by hand next to the assertion; the test files
//! are written here byte by byte (STL) or with `export::zip_store` (3MF),
//! plus the Bambu Studio fixture of `qoob3d-core/tests/fixtures/` (sizes and
//! volumes in its README).
//!
//! Tolerances: `VOL` = 1e-6 relative, the volume rule of AG-SC-23 (the
//! meshes are exact polyhedra, a planar B-rep integrates them exactly up
//! to rounding). `POS` = 1e-6 mm for bounding boxes, as in `tests/script.rs`
//! (AG-HD-11: tight boxes); the vertices are exact decimals and a planar
//! face's bbox is its vertices.

use std::path::{Path, PathBuf};

use qoob3d_core::export::zip_store;
use qoob3d_kernel::script::{RunOptions, RunOutput, ScriptError, Shape, run};

const VOL: f64 = 1e-6;
const POS: f64 = 1e-6;

const FIXTURE: &[u8] = include_bytes!("../../qoob3d-core/tests/fixtures/bambu_two_objects.3mf");

type Tri = [[f64; 3]; 3];

// ===========================================================================
// Project folder and runs
// ===========================================================================

/// Fresh project folder `<tmp>/script-import/<test>/project` with `lib/`.
fn project(test: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("script-import").join(test);
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("project");
    std::fs::create_dir_all(dir.join("lib")).expect("create project");
    dir
}

fn put(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create folder");
    std::fs::write(path, bytes).expect("write test file");
}

fn run_in(dir: &Path, source: &str) -> Result<RunOutput, ScriptError> {
    let options = RunOptions { project_dir: Some(dir.to_path_buf()), ..RunOptions::default() };
    run(source, &options)
}

fn run_ok(dir: &Path, source: &str) -> RunOutput {
    run_in(dir, source).unwrap_or_else(|e| panic!("script failed: {e}\n--- script ---\n{source}"))
}

fn run_err(dir: &Path, source: &str) -> ScriptError {
    match run_in(dir, source) {
        Ok(out) => panic!("script must fail, printed {:?}\n{source}", out.printed),
        Err(e) => e,
    }
}

/// Imports `path` on line 1, shows it as `s` and prints faces, edges and
/// solids (lines 0, 1, 2 of the printed output).
fn import(dir: &Path, path: &str) -> (Shape, [i64; 3]) {
    let source = format!(
        "let s = import_mesh(\"{path}\");\nshow(s, \"s\");\n\
         print(faces(s).len());\nprint(all_edges(s).len());\nprint(s.solids());"
    );
    let out = run_ok(dir, &source);
    let n = |i: usize| -> i64 {
        let line = out.printed.get(i).unwrap_or_else(|| panic!("no line {i}: {:?}", out.printed));
        line.trim().parse().unwrap_or_else(|e| panic!("line {i} {line:?}: {e}"))
    };
    let shape = out.shapes.first().expect("shown shape").1.clone();
    (shape, [n(0), n(1), n(2)])
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

/// A valid solid of `volume` whose display mesh is watertight (the FDM
/// gate of the cache) with the same volume to the export tolerance.
fn assert_solid(s: &Shape, solids: usize, volume: f64, what: &str) {
    assert!(s.is_valid(), "{what} is not a valid B-rep");
    assert_eq!(s.solid_count(), solids, "{what}: solids");
    assert_rel(s.volume(), volume, VOL, &format!("volume of {what}"));
    let mesh = s.mesh(0.01).unwrap_or_else(|e| panic!("{what}: mesh failed: {e:?}"));
    let report = mesh.faces.analyze();
    assert!(report.is_watertight(), "{what}: mesh not watertight: {report:?}");
    // Planar faces mesh exactly; f32 positions (< 200 mm) round to < 1e-5 mm.
    assert_rel(report.volume_mm3, volume, 1e-5, &format!("mesh volume of {what}"));
}

// ===========================================================================
// Mesh oracles
// ===========================================================================

/// Box corners (index x + 2y + 4z) and their quads counter-clockwise seen
/// from outside, each normal checked with the cross product of the first
/// two edges: z=min [0,2,3,1] → (0,1,0)×(1,1,0) = (0,0,−1), …
const BOX_QUADS: [[usize; 4]; 6] =
    [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];

fn box_corners(min: [f64; 3], size: [f64; 3]) -> Vec<[f64; 3]> {
    (0..8u32)
        .map(|i| {
            let bit = |k: u32| f64::from((i >> k) & 1);
            [min[0] + bit(0) * size[0], min[1] + bit(1) * size[1], min[2] + bit(2) * size[2]]
        })
        .collect()
}

/// 12 triangles, two per quad.
fn box_soup(min: [f64; 3], size: [f64; 3]) -> Vec<Tri> {
    let c = box_corners(min, size);
    BOX_QUADS
        .iter()
        .flat_map(|q| [[c[q[0]], c[q[1]], c[q[2]]], [c[q[0]], c[q[2]], c[q[3]]]])
        .collect()
}

/// 24 triangles: every face split into four around its centre, so a face
/// is more than one pair of triangles.
fn split_box_soup(min: [f64; 3], size: [f64; 3]) -> Vec<Tri> {
    let c = box_corners(min, size);
    BOX_QUADS
        .iter()
        .flat_map(|q| {
            let m = [0, 1, 2].map(|k| q.iter().map(|&i| c[i][k]).sum::<f64>() / 4.0);
            (0..4).map(|i| [c[q[i]], c[q[(i + 1) % 4]], m]).collect::<Vec<_>>()
        })
        .collect()
}

/// Prism over a counter-clockwise outline (seen from +z), z from 0 to `h`.
/// Caps are fans from the first point (valid for the L outline below:
/// every fan triangle lies inside, see `l_prism_is_2760`); sides are quads
/// Pi → Pi+1 with outward normals ((30,0,0)×(30,0,15) = (0,−450,0) for the
/// first edge).
fn prism_soup(outline: &[[f64; 2]], h: f64) -> Vec<Tri> {
    let p = |i: usize, z: f64| [outline[i][0], outline[i][1], z];
    let n = outline.len();
    let mut tris = Vec::new();
    for i in 1..n - 1 {
        tris.push([p(0, h), p(i, h), p(i + 1, h)]);
        tris.push([p(0, 0.0), p(i + 1, 0.0), p(i, 0.0)]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        tris.push([p(i, 0.0), p(j, 0.0), p(j, h)]);
        tris.push([p(i, 0.0), p(j, h), p(i, h)]);
    }
    tris
}

/// The L profile of the fixture README: area 30·4 + 4·16 = 184 mm².
const L_OUTLINE: [[f64; 2]; 6] =
    [[0.0, 0.0], [30.0, 0.0], [30.0, 4.0], [4.0, 4.0], [4.0, 20.0], [0.0, 20.0]];

fn volume(tris: &[Tri]) -> f64 {
    tris.iter()
        .map(|[a, b, c]| {
            let x =
                [b[1] * c[2] - b[2] * c[1], b[2] * c[0] - b[0] * c[2], b[0] * c[1] - b[1] * c[0]];
            (a[0] * x[0] + a[1] * x[1] + a[2] * x[2]) / 6.0
        })
        .sum()
}

/// Binary STL: 80-byte header, u32 count, per triangle a zero normal, three
/// f32 corners and a u16 attribute.
fn binary_stl(tris: &[Tri]) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    out[..11].copy_from_slice(b"QOOB3D test");
    out.extend_from_slice(&u32::try_from(tris.len()).expect("count").to_le_bytes());
    for t in tris {
        out.extend_from_slice(&[0u8; 12]);
        for c in t.iter().flatten() {
            // STL stores f32; the coordinates here are exact in f32.
            out.extend_from_slice(&(*c as f32).to_le_bytes());
        }
        out.extend_from_slice(&[0u8; 2]);
    }
    out
}

fn ascii_stl(tris: &[Tri]) -> String {
    let mut s = String::from("solid test\n");
    for t in tris {
        s.push_str("facet normal 0 0 0\nouter loop\n");
        for v in t {
            s.push_str(&format!("vertex {} {} {}\n", v[0], v[1], v[2]));
        }
        s.push_str("endloop\nendfacet\n");
    }
    s.push_str("endsolid test\n");
    s
}

/// A stored 3MF with one mesh object placed by one build item.
fn three_mf(positions: &[[f64; 3]], tris: &[[usize; 3]], transform: &str, attrs: &str) -> Vec<u8> {
    let mut m = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\" {attrs}>\n\
         <resources><object id=\"1\" type=\"model\"><mesh><vertices>\n"
    );
    for p in positions {
        m.push_str(&format!("<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n", p[0], p[1], p[2]));
    }
    m.push_str("</vertices><triangles>\n");
    for t in tris {
        m.push_str(&format!("<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n", t[0], t[1], t[2]));
    }
    m.push_str(&format!(
        "</triangles></mesh></object></resources>\n<build><item objectid=\"1\" \
         transform=\"{transform}\"/></build>\n</model>\n"
    ));
    let types = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
        r#"<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
        "</Types>"
    );
    let rels = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>"#,
        "</Relationships>"
    );
    zip_store(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("3D/3dmodel.model", m.as_bytes()),
    ])
}

/// Regular octahedron with vertices ±10 on the axes: 0 = +x, 1 = −x,
/// 2 = +y, 3 = −y, 4 = +z, 5 = −z. Face of octant (sx, sy, sz) is (X, Y, Z),
/// outward ((10,0,0),(0,10,0),(0,0,10) → normal (1,1,1)) when sx·sy·sz > 0,
/// else (X, Z, Y). Volume 8 · 10³/6 = 4000/3; 8 faces, 12 edges.
fn octahedron() -> (Vec<[f64; 3]>, Vec<[usize; 3]>) {
    let p = vec![
        [10.0, 0.0, 0.0],
        [-10.0, 0.0, 0.0],
        [0.0, 10.0, 0.0],
        [0.0, -10.0, 0.0],
        [0.0, 0.0, 10.0],
        [0.0, 0.0, -10.0],
    ];
    let mut t = Vec::new();
    for sx in [1, -1] {
        for sy in [1, -1] {
            for sz in [1, -1] {
                let (x, y, z) =
                    (usize::from(sx < 0), 2 + usize::from(sy < 0), 4 + usize::from(sz < 0));
                t.push(if sx * sy * sz > 0 { [x, y, z] } else { [x, z, y] });
            }
        }
    }
    (p, t)
}

#[test]
fn oracles_are_outward_and_have_the_formula_volume() {
    assert_rel(volume(&box_soup([5.0, -3.0, 2.0], [40.0, 30.0, 10.0])), 12000.0, 1e-12, "box");
    assert_rel(volume(&split_box_soup([0.0; 3], [40.0, 30.0, 10.0])), 12000.0, 1e-12, "split box");
    assert_rel(volume(&prism_soup(&L_OUTLINE, 15.0)), 2760.0, 1e-12, "L prism 184 · 15");
    let (p, t) = octahedron();
    let soup: Vec<Tri> = t.iter().map(|f| f.map(|i| p[i])).collect();
    assert_rel(volume(&soup), 4000.0 / 3.0, 1e-12, "octahedron");
}

// ===========================================================================
// AG-SC-23: STL and 3MF become solids
// ===========================================================================

#[test]
fn ag_sc_23_binary_stl_box_is_one_solid_with_six_faces_and_twelve_edges() {
    // 40 × 30 × 10 from (5, −3, 2): 12 000 mm³; 12 triangles, coplanar
    // pairs merged → 6 faces, 12 edges (the diagonals are no edges).
    let dir = project("binary_box");
    put(&dir, "lib/block.stl", &binary_stl(&box_soup([5.0, -3.0, 2.0], [40.0, 30.0, 10.0])));
    let (s, [faces, edges, solids]) = import(&dir, "lib/block.stl");
    assert_eq!((faces, edges, solids), (6, 12, 1), "faces, edges, solids");
    assert_solid(&s, 1, 12000.0, "block");
    assert_bbox(&s, [5.0, -3.0, 2.0], [45.0, 27.0, 12.0], "block");
}

#[test]
fn ag_sc_23_ascii_stl_box_imports_like_the_binary_one() {
    let dir = project("ascii_box");
    put(
        &dir,
        "lib/block.stl",
        ascii_stl(&box_soup([5.0, -3.0, 2.0], [40.0, 30.0, 10.0])).as_bytes(),
    );
    let (s, counts) = import(&dir, "lib/block.stl");
    assert_eq!(counts, [6, 12, 1], "faces, edges, solids");
    assert_solid(&s, 1, 12000.0, "ASCII block");
    assert_bbox(&s, [5.0, -3.0, 2.0], [45.0, 27.0, 12.0], "ASCII block");
}

#[test]
fn ag_sc_23_coplanar_triangles_of_a_finely_split_face_become_one_face() {
    // Every box face as four triangles around its centre: still 6 faces and
    // 12 edges (the 24 inner segments are no edges).
    let dir = project("split_box");
    put(&dir, "lib/split.stl", &binary_stl(&split_box_soup([0.0; 3], [40.0, 30.0, 10.0])));
    let (s, counts) = import(&dir, "lib/split.stl");
    assert_eq!(counts, [6, 12, 1], "faces, edges, solids");
    assert_solid(&s, 1, 12000.0, "split box");
}

#[test]
fn ag_sc_23_l_prism_has_eight_faces_and_eighteen_edges() {
    // L profile (non-convex cap of 4 triangles) × 15: V = 184 · 15 = 2760;
    // faces 6 sides + 2 caps = 8; edges 6 + 6 + 6 = 18.
    let dir = project("l_prism");
    put(&dir, "lib/angle.stl", &binary_stl(&prism_soup(&L_OUTLINE, 15.0)));
    let (s, counts) = import(&dir, "lib/angle.stl");
    assert_eq!(counts, [8, 18, 1], "faces, edges, solids");
    assert_solid(&s, 1, 2760.0, "L prism");
    assert_bbox(&s, [0.0; 3], [30.0, 20.0, 15.0], "L prism");
}

#[test]
fn ag_sc_23_bambu_studio_3mf_imports_as_two_solids() {
    // tests/fixtures/README.md of qoob3d-core: Block 12 000 mm³ (6 faces,
    // 12 edges), Angle 2760 mm³ (8 faces, 18 edges). Positions from the
    // transforms in the file (mesh centred, component + item offset):
    // Block x 80..120, y 95.99998..125.99998, z 0..10;
    // Angle x 87.64437..117.64437, y 74.00003..94.00003, z 0..15.
    let dir = project("bambu");
    put(&dir, "lib/two.3mf", FIXTURE);
    let (s, counts) = import(&dir, "lib/two.3mf");
    assert_eq!(counts, [6 + 8, 12 + 18, 2], "faces, edges, solids");
    assert_solid(&s, 2, 12000.0 + 2760.0, "both objects");
    assert_bbox(&s, [80.0, 74.00003, 0.0], [120.0, 125.99998, 15.0], "both objects");
    // Each object alone, cut out with a box that holds only it: the Block
    // has y ≥ 95.99998, the Angle y ≤ 94.00003.
    // box(60, 40, 30).move(100, 115, 0): x 70..130, y 95..135, z 0..30.
    // box(60, 45, 30).move(100, 72, 0): x 70..130, y 49.5..94.5, z 0..30.
    let out = run_ok(
        &dir,
        "let s = import_mesh(\"lib/two.3mf\");\n\
         show(s & box(60, 40, 30).move(100, 115, 0), \"block\");\n\
         show(s & box(60, 45, 30).move(100, 72, 0), \"angle\");",
    );
    let (block, angle) = (&out.shapes[0].1, &out.shapes[1].1);
    assert_rel(block.volume(), 12000.0, VOL, "Block");
    assert_bbox(block, [80.0, 95.99998, 0.0], [120.0, 125.99998, 10.0], "Block");
    assert_rel(angle.volume(), 2760.0, VOL, "Angle");
    assert_bbox(angle, [87.64437, 74.00003, 0.0], [117.64437, 94.00003, 15.0], "Angle");
}

#[test]
fn ag_sc_23_rotated_octahedron_keeps_eight_faces_and_its_volume() {
    // Item: 30° about Z, then +(50, 60, 10); 3MF rows x' = x c − y s,
    // y' = x s + y c (m00 = c, m01 = s, m10 = −s, m11 = c). Extremes: the
    // ±x and ±y vertices → x, y within ±10 c = ±8.660254…, z 0..20.
    // No two faces are coplanar: 8 faces, 12 edges, V = 4000/3.
    let dir = project("octahedron");
    let (p, t) = octahedron();
    let (c, s) = (3f64.sqrt() / 2.0, 0.5);
    let transform = format!("{c} {s} 0 {} {c} 0 0 0 1 50 60 10", -s);
    put(&dir, "lib/octa.3mf", &three_mf(&p, &t, &transform, ""));
    let (shape, counts) = import(&dir, "lib/octa.3mf");
    assert_eq!(counts, [8, 12, 1], "faces, edges, solids");
    assert_solid(&shape, 1, 4000.0 / 3.0, "octahedron");
    let r = 10.0 * c;
    assert_bbox(&shape, [50.0 - r, 60.0 - r, 0.0], [50.0 + r, 60.0 + r, 20.0], "octahedron");
}

#[test]
fn ag_sc_23_every_closed_group_of_triangles_is_one_solid() {
    // Two boxes 10³ apart in one STL: 2 solids, 2000 mm³, 12 faces.
    let dir = project("two_boxes");
    let mut tris = box_soup([0.0; 3], [10.0; 3]);
    tris.extend(box_soup([30.0, 0.0, 0.0], [10.0; 3]));
    put(&dir, "lib/two.stl", &binary_stl(&tris));
    let (s, counts) = import(&dir, "lib/two.stl");
    assert_eq!(counts, [12, 24, 2], "faces, edges, solids");
    assert_solid(&s, 2, 2000.0, "two boxes");
}

#[test]
fn ag_sc_23_extensions_are_case_insensitive() {
    let dir = project("case");
    put(&dir, "lib/BLOCK.STL", &binary_stl(&box_soup([0.0; 3], [40.0, 30.0, 10.0])));
    put(&dir, "lib/Two.3Mf", FIXTURE);
    let (s, counts) = import(&dir, "lib/BLOCK.STL");
    assert_eq!(counts, [6, 12, 1]);
    assert_rel(s.volume(), 12000.0, VOL, "BLOCK.STL");
    let (s, counts) = import(&dir, "lib/Two.3Mf");
    assert_eq!(counts[2], 2);
    assert_rel(s.volume(), 14760.0, VOL, "Two.3Mf");
}

#[test]
fn ag_sc_23_imported_solids_take_part_in_booleans() {
    // Box x ±20, y ±15, z 0..10 minus cylinder(4, 30) through it:
    // V = 12 000 − π · 2² · 10 = 12 000 − 40 π.
    let dir = project("boolean");
    put(&dir, "lib/block.stl", &binary_stl(&box_soup([-20.0, -15.0, 0.0], [40.0, 30.0, 10.0])));
    let out =
        run_ok(&dir, "let s = import_mesh(\"lib/block.stl\") - cylinder(4, 30);\nshow(s, \"s\");");
    let s = &out.shapes[0].1;
    assert!(s.is_valid());
    assert_rel(s.volume(), 12000.0 - 40.0 * std::f64::consts::PI, VOL, "drilled import");
}

#[test]
fn ag_sc_23_vertices_closer_than_1e_4_mm_are_welded_into_a_closed_solid() {
    // One of the 36 copies of the corner (40, 30, 10) is 4e-5 mm off: without
    // welding the mesh would have open edges. Moving one vertex of the
    // faces at that corner by δ changes the volume by at most δ · (sum of
    // the three face areas 1200 + 400 + 300) = 0.076 mm³ → 1e-5 relative.
    let dir = project("weld");
    let mut tris = box_soup([0.0; 3], [40.0, 30.0, 10.0]);
    let corner = tris.iter_mut().flatten().find(|v| **v == [40.0, 30.0, 10.0]).expect("corner");
    corner[0] += 4e-5;
    put(&dir, "lib/weld.stl", &binary_stl(&tris));
    let out = run_ok(&dir, "let s = import_mesh(\"lib/weld.stl\");\nshow(s, \"s\");");
    let s = &out.shapes[0].1;
    assert!(s.is_valid(), "welded box is valid");
    assert_eq!(s.solid_count(), 1);
    assert_rel(s.volume(), 12000.0, 1e-5, "welded box");
}

// ===========================================================================
// AG-SC-23: errors
// ===========================================================================

fn runtime_message(e: &ScriptError, want_line: usize) -> String {
    match e {
        ScriptError::Runtime { line, message, .. } => {
            assert_eq!(*line, want_line, "{e}");
            message.clone()
        }
        other => panic!("expected a Runtime error on line {want_line}, got {other:?}"),
    }
}

fn geometry_message(e: &ScriptError, want_line: usize) -> String {
    match e {
        ScriptError::Geometry { line, function, message, .. } => {
            assert_eq!(*line, want_line, "{e}");
            assert_eq!(function, "import_mesh", "{e}");
            message.clone()
        }
        other => {
            panic!("expected a Geometry error of import_mesh on line {want_line}, got {other:?}")
        }
    }
}

/// `let a = 1;` on line 1, the import on line 2.
fn import_line_2(path: &str) -> String {
    format!("let a = 1;\nlet s = import_mesh(\"{path}\");\nshow(s, \"s\");")
}

#[test]
fn ag_sc_23_an_open_mesh_is_a_geometry_error_naming_the_open_edges() {
    // Box A without its two top triangles: the top square's 4 edges are
    // open (the diagonal is gone with both). Box B without one triangle: 3.
    // Together 7 boundary edges.
    let dir = project("open");
    // Quad 1 (triangles 2 and 3) is z = max.
    let mut a: Vec<Tri> = box_soup([0.0; 3], [10.0; 3])
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !(2..4).contains(i))
        .map(|(_, t)| t)
        .collect();
    let mut b = box_soup([20.0, 0.0, 0.0], [10.0; 3]);
    b.remove(0);
    a.extend(b);
    put(&dir, "lib/open.stl", &binary_stl(&a));
    let message = geometry_message(&run_err(&dir, &import_line_2("lib/open.stl")), 2);
    assert!(message.contains('7'), "names the 7 open edges: {message}");
}

#[test]
fn ag_sc_23_a_non_manifold_mesh_is_a_geometry_error_naming_the_edges() {
    // 4 × 4 checkerboard of 10 mm cubes (cells with i + j even, 8 cubes).
    // At each of the 3 × 3 inner grid points exactly one diagonal pair of
    // cubes is filled, so its vertical edge carries 4 triangles: 9
    // non-manifold edges.
    let dir = project("non_manifold");
    let mut tris = Vec::new();
    for i in 0..4 {
        for j in 0..4 {
            if (i + j) % 2 == 0 {
                tris.extend(box_soup([10.0 * f64::from(i), 10.0 * f64::from(j), 0.0], [10.0; 3]));
            }
        }
    }
    put(&dir, "lib/checker.stl", &binary_stl(&tris));
    let message = geometry_message(&run_err(&dir, &import_line_2("lib/checker.stl")), 2);
    assert!(message.contains('9'), "names the 9 non-manifold edges: {message}");
}

#[test]
fn ag_sc_23_another_extension_is_a_runtime_error_naming_the_path_and_both_extensions() {
    let dir = project("extension");
    let stl = binary_stl(&box_soup([0.0; 3], [1.0; 3]));
    for path in ["lib/block.obj", "lib/block.stl.bak", "lib/block", "lib/block.step"] {
        put(&dir, path, &stl);
        let message = runtime_message(&run_err(&dir, &import_line_2(path)), 2);
        let lower = message.to_lowercase();
        assert!(message.contains(path), "names the path {path}: {message}");
        assert!(lower.contains(".stl") && lower.contains(".3mf"), "names .stl and .3mf: {message}");
    }
}

#[test]
fn ag_sc_23_import_mesh_stays_inside_the_project() {
    // Same confinement as import_step (AG-SC-14).
    let dir = project("confined");
    let stl = binary_stl(&box_soup([0.0; 3], [1.0; 3]));
    put(dir.parent().expect("root"), "outside.stl", &stl);
    let absolute = dir.parent().expect("root").join("outside.stl");
    for path in [
        "../outside.stl".to_owned(),
        "lib/../../outside.stl".to_owned(),
        absolute.display().to_string(),
    ] {
        runtime_message(&run_err(&dir, &import_line_2(&path)), 2);
    }
    let message = runtime_message(&run_err(&dir, &import_line_2("lib/missing.stl")), 2);
    assert!(message.contains("lib/missing.stl"), "names the missing file: {message}");
    // Without a project there is no file access at all.
    let r = run(&import_line_2(&absolute.display().to_string()), &RunOptions::default());
    assert!(r.is_err(), "no project, no import: {r:?}");
}

#[test]
fn ag_sc_23_unreadable_files_are_runtime_errors_naming_the_file() {
    let dir = project("unreadable");
    // An archive whose model requires the beam lattice extension.
    let (p, t) = octahedron();
    let beam = "xmlns:b=\"http://schemas.microsoft.com/3dmanufacturing/beamlattice/2017/02\" \
                requiredextensions=\"b\"";
    let cases: [(&str, Vec<u8>); 5] = [
        ("lib/hello.stl", b"hello".to_vec()),
        ("lib/empty.stl", Vec::new()),
        ("lib/text.3mf", b"not a zip".to_vec()),
        ("lib/half.3mf", FIXTURE[..FIXTURE.len() / 2].to_vec()),
        ("lib/beam.3mf", three_mf(&p, &t, "1 0 0 0 1 0 0 0 1 0 0 0", beam)),
    ];
    for (path, bytes) in cases {
        put(&dir, path, &bytes);
        let message = runtime_message(&run_err(&dir, &import_line_2(path)), 2);
        assert!(message.contains(path), "names the file {path}: {message}");
    }
}

#[test]
fn ag_sc_23_more_than_200_000_triangles_is_a_runtime_error_naming_the_count() {
    // 50 000 unit tetrahedra (200 000 triangles) plus one triangle = 200 001.
    let dir = project("too_many");
    let mut tris: Vec<Tri> = (0..50_000u32)
        .flat_map(|i| {
            let o = [3.0 * f64::from(i % 250), 3.0 * f64::from(i / 250), 0.0];
            let p =
                [o, [o[0] + 1.0, o[1], o[2]], [o[0], o[1] + 1.0, o[2]], [o[0], o[1], o[2] + 1.0]];
            [[p[0], p[2], p[1]], [p[0], p[1], p[3]], [p[0], p[3], p[2]], [p[1], p[2], p[3]]]
        })
        .collect();
    tris.push([[0.0, 0.0, 5.0], [1.0, 0.0, 5.0], [0.0, 1.0, 5.0]]);
    put(&dir, "lib/many.stl", &binary_stl(&tris));
    let message = runtime_message(&run_err(&dir, &import_line_2("lib/many.stl")), 2);
    let spelled = ["200001", "200 001", "200,001", "200_001", "200\u{202f}001"];
    assert!(spelled.iter().any(|s| message.contains(s)), "names the count 200001: {message}");
    assert!(message.contains("lib/many.stl"), "names the file: {message}");
}

#[test]
fn ag_sc_23_a_closed_cavity_becomes_an_inner_shell_of_its_solid() {
    // Finding 1.8 of the Damen 5009 project: Fusion wrote a boat with two
    // closed inner surfaces (cavities), and the import refused it.
    // Outer cube 20, inner cube 6 turned inside out: V = 8000 − 216.
    let dir = project("cavity");
    let mut tris = box_soup([0.0, 0.0, 0.0], [20.0, 20.0, 20.0]);
    tris.extend(box_soup([7.0, 7.0, 7.0], [6.0, 6.0, 6.0]).into_iter().map(|[a, b, c]| [a, c, b]));
    put(&dir, "lib/hollow.stl", &binary_stl(&tris));
    let out = run_ok(
        &dir,
        "let s = import_mesh(\"lib/hollow.stl\");\nshow(s, \"s\");\nshow(fill_voids(s), \"f\");",
    );
    let hollow = &out.shapes[0].1;
    assert!(hollow.is_valid(), "the solid with its cavity is valid");
    assert_eq!(hollow.solid_count(), 1);
    assert_rel(hollow.volume(), 8000.0 - 216.0, VOL, "volume without the cavity");
    assert_rel(out.shapes[1].1.volume(), 8000.0, VOL, "filled");
}

#[test]
fn ag_sc_23_an_inside_out_surface_inside_nothing_is_refused() {
    let dir = project("inside-out");
    let tris: Vec<Tri> =
        box_soup([0.0; 3], [5.0; 3]).into_iter().map(|[a, b, c]| [a, c, b]).collect();
    put(&dir, "lib/inverted.stl", &binary_stl(&tris));
    let e = run_err(&dir, "\nlet s = import_mesh(\"lib/inverted.stl\");");
    let message = geometry_message(&e, 2);
    assert!(message.contains("1 of 1 closed surfaces enclose no positive volume"), "{message}");
}
