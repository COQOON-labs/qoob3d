//! Mesh import readers `qoob3d_core::import` (docs/requirements/agent.md §6.5,
//! AG-SC-23; ADR-0034): STL binary and ASCII, 3MF core plus the production
//! extension, nested components, transforms, units, limits against hostile
//! files. Every expected value is derived by hand next to the assertion:
//! volumes by formula, positions from the transforms of the file.
//!
//! The oracles here are independent of the reader: test files are written
//! byte by byte (STL), with `export::zip_store` or our own deflate writer
//! (3MF), or are the third-party fixture `tests/fixtures/bambu_two_objects.3mf`
//! written by Bambu Studio (sizes and volumes in `tests/fixtures/README.md`).
//! Volumes and bounding boxes of the imported meshes are computed again here
//! (divergence theorem, connected components by shared indices), not only
//! through `ImportedMesh::signed_volume`.
//!
//! Tolerances:
//! - `VOL` = 1e-6 relative: the volume rule of AG-SC-23. The meshes here are
//!   exact polyhedra, so the triangle sum differs from the formula only by
//!   f64 rounding (< 1e-12 relative even 120 mm from the origin).
//! - `POS` = 1e-6 mm absolute: positions are f64 and parsed from decimals
//!   with at most 5 places; transforms add rounding of ~1e-14 mm. Welding
//!   (1e-4 mm) must not move a vertex that has no neighbour within 1e-4 mm.
//!   A wrong transform convention is off by millimetres.

use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use qoob3d_core::export::{crc32, zip_store};
use qoob3d_core::import::{ImportError, ImportedMesh, MAX_TRIANGLES, read_3mf, read_stl};

const VOL: f64 = 1e-6;
const POS: f64 = 1e-6;

const FIXTURE: &[u8] = include_bytes!("fixtures/bambu_two_objects.3mf");

const CORE_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
const PROD_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";
const MATERIAL_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/material/2015/02";
const BEAM_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/beamlattice/2017/02";
const MODEL_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

type Tri = [[f64; 3]; 3];

// ===========================================================================
// Geometry oracles
// ===========================================================================

/// The 8 corners of an axis-aligned box, index = x + 2y + 4z (bit set =
/// max side), and its 12 triangles, counter-clockwise seen from outside.
/// Quads: z=min [0,2,3,1] (normal −z: (0,1,0)×(1,1,0) = (0,0,−1)), z=max
/// [4,5,7,6], y=min [0,1,5,4], y=max [2,6,7,3], x=min [0,4,6,2], x=max
/// [1,3,7,5]; each normal checked with the cross product of its first two
/// edges. `box_volume_is_the_formula` confirms the orientation.
fn box_mesh(min: [f64; 3], size: [f64; 3]) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let corners = (0..8u32)
        .map(|i| {
            let bit = |k: u32| f64::from((i >> k) & 1);
            [min[0] + bit(0) * size[0], min[1] + bit(1) * size[1], min[2] + bit(2) * size[2]]
        })
        .collect();
    let quads =
        [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let tris = quads.iter().flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]).collect();
    (corners, tris)
}

/// Unit tetrahedron at `o`: corners o, o+x, o+y, o+z. Faces outward:
/// base (0,2,1) normal −z, (0,1,3) normal −y, (0,3,2) normal −x, (1,2,3)
/// normal +(1,1,1). Volume 1/6.
fn tetra_mesh(o: [f64; 3]) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let p = vec![o, [o[0] + 1.0, o[1], o[2]], [o[0], o[1] + 1.0, o[2]], [o[0], o[1], o[2] + 1.0]];
    (p, vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]])
}

fn soup(positions: &[[f64; 3]], tris: &[[u32; 3]]) -> Vec<Tri> {
    tris.iter().map(|t| t.map(|i| positions[i as usize])).collect()
}

fn box_soup(min: [f64; 3], size: [f64; 3]) -> Vec<Tri> {
    let (p, t) = box_mesh(min, size);
    soup(&p, &t)
}

/// Signed volume by the divergence theorem: Σ a·(b×c) / 6.
fn volume(tris: &[Tri]) -> f64 {
    tris.iter()
        .map(|[a, b, c]| {
            let cross =
                [b[1] * c[2] - b[2] * c[1], b[2] * c[0] - b[0] * c[2], b[0] * c[1] - b[1] * c[0]];
            (a[0] * cross[0] + a[1] * cross[1] + a[2] * cross[2]) / 6.0
        })
        .sum()
}

fn bounds<'a>(points: impl IntoIterator<Item = &'a [f64; 3]>) -> ([f64; 3], [f64; 3]) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

fn assert_rel(actual: f64, expected: f64, tol: f64, what: &str) {
    let rel = (actual - expected).abs() / expected.abs();
    assert!(rel < tol, "{what}: got {actual}, expected {expected} (rel {rel:e} >= {tol:e})");
}

fn assert_point(actual: [f64; 3], expected: [f64; 3], tol: f64, what: &str) {
    let close = (0..3).all(|k| (actual[k] - expected[k]).abs() < tol);
    assert!(close, "{what}: got {actual:?}, expected {expected:?} (tolerance {tol})");
}

fn assert_bounds(actual: ([f64; 3], [f64; 3]), min: [f64; 3], max: [f64; 3], what: &str) {
    assert_point(actual.0, min, POS, &format!("{what}: min"));
    assert_point(actual.1, max, POS, &format!("{what}: max"));
}

// ===========================================================================
// Looking at an ImportedMesh from outside
// ===========================================================================

/// Structural invariants every result must have: indices in range, no
/// triangle with two equal indices (collapsed triangles are dropped), at
/// most `MAX_TRIANGLES` triangles.
fn assert_well_formed(m: &ImportedMesh) {
    assert!(m.triangles.len() <= MAX_TRIANGLES, "{} triangles", m.triangles.len());
    for t in &m.triangles {
        assert!(t.iter().all(|&i| (i as usize) < m.positions.len()), "index out of range: {t:?}");
        assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2], "collapsed triangle kept: {t:?}");
    }
}

fn mesh_soup(m: &ImportedMesh) -> Vec<Tri> {
    soup(&m.positions, &m.triangles)
}

/// Number of distinct vertex indices the triangles use: the vertex count
/// after welding, whatever unreferenced positions the reader keeps.
fn referenced(m: &ImportedMesh) -> usize {
    let mut used: Vec<u32> = m.triangles.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    used.len()
}

/// One group of triangles connected through shared vertex indices.
struct Component {
    volume: f64,
    min: [f64; 3],
    max: [f64; 3],
    triangles: usize,
    vertices: Vec<[f64; 3]>,
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Connected components (union-find over indices), sorted by min x, then min y.
fn components(m: &ImportedMesh) -> Vec<Component> {
    let mut parent: Vec<usize> = (0..m.positions.len()).collect();
    for t in &m.triangles {
        for k in 1..3 {
            let (a, b) = (find(&mut parent, t[0] as usize), find(&mut parent, t[k] as usize));
            parent[a] = b;
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<[u32; 3]>> = Default::default();
    for t in &m.triangles {
        groups.entry(find(&mut parent, t[0] as usize)).or_default().push(*t);
    }
    let mut out: Vec<Component> = groups
        .into_values()
        .map(|tris| {
            let mut idx: Vec<u32> = tris.iter().flatten().copied().collect();
            idx.sort_unstable();
            idx.dedup();
            let vertices: Vec<[f64; 3]> = idx.iter().map(|&i| m.positions[i as usize]).collect();
            let (min, max) = bounds(&vertices);
            let volume = volume(&soup(&m.positions, &tris));
            Component { volume, min, max, triangles: tris.len(), vertices }
        })
        .collect();
    out.sort_by(|a, b| a.min[0].total_cmp(&b.min[0]).then(a.min[1].total_cmp(&b.min[1])));
    out
}

fn ok(r: Result<ImportedMesh, ImportError>, what: &str) -> ImportedMesh {
    let m = r.unwrap_or_else(|e| panic!("{what}: import failed: {e}"));
    assert_well_formed(&m);
    m
}

/// The error text of a result that must fail; it must carry a reason.
fn err(r: Result<ImportedMesh, ImportError>, what: &str) -> String {
    match r {
        Ok(m) => panic!("{what}: must fail, got {} triangles", m.triangles.len()),
        Err(e) => {
            let text = e.to_string();
            assert!(!text.trim().is_empty(), "{what}: the error has no text");
            text
        }
    }
}

/// `n` as the reader may spell it: 200001, 200 001, 200,001, 200_001.
fn names_count(text: &str, n: usize) -> bool {
    let plain = n.to_string();
    let grouped = |sep: &str| {
        let digits: Vec<char> = plain.chars().collect();
        let mut s = String::new();
        for (i, c) in digits.iter().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                s.push_str(sep);
            }
            s.push(*c);
        }
        s
    };
    [plain.clone(), grouped(" "), grouped(","), grouped("_"), grouped("\u{202f}")]
        .iter()
        .any(|s| text.contains(s.as_str()))
}

// ===========================================================================
// STL writers (byte layout of the format, independent of the reader)
// ===========================================================================

/// Binary STL: 80-byte header, u32 count, per triangle normal (written as
/// zero: orientation comes from the winding), three corners as f32 LE and a
/// u16 attribute.
fn binary_stl(header: &[u8], tris: &[Tri]) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    out[..header.len()].copy_from_slice(header);
    let count = u32::try_from(tris.len()).expect("count fits u32");
    out.extend_from_slice(&count.to_le_bytes());
    for t in tris {
        out.extend_from_slice(&[0u8; 12]);
        for v in t {
            for c in v {
                // STL stores f32; every coordinate here is exact in f32.
                let f = *c as f32;
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        out.extend_from_slice(&[0u8; 2]);
    }
    out
}

/// ASCII STL as most writers produce it; `sci` writes the numbers in
/// scientific notation (`4e1`).
fn ascii_stl(name: &str, tris: &[Tri], sci: bool) -> String {
    let num = |x: f64| if sci { format!("{x:e}") } else { format!("{x}") };
    let mut s = format!("solid {name}\n");
    for t in tris {
        s.push_str("  facet normal 0 0 0\n    outer loop\n");
        for v in t {
            s.push_str(&format!("      vertex {} {} {}\n", num(v[0]), num(v[1]), num(v[2])));
        }
        s.push_str("    endloop\n  endfacet\n");
    }
    s.push_str(&format!("endsolid {name}\n"));
    s
}

/// 50 000 unit tetrahedra on a 250 × 200 grid with 3 mm pitch: 200 000
/// triangles, all vertices distinct, volume 50 000 / 6.
fn tetra_grid(n: usize) -> Vec<Tri> {
    (0..n)
        .flat_map(|i| {
            let o = [3.0 * (i % 250) as f64, 3.0 * (i / 250) as f64, 0.0];
            let (p, t) = tetra_mesh(o);
            soup(&p, &t)
        })
        .collect()
}

// ===========================================================================
// AG-SC-23: STL
// ===========================================================================

#[test]
fn ag_sc_23_the_triangle_limit_is_200_000() {
    assert_eq!(MAX_TRIANGLES, 200_000);
}

#[test]
fn ag_sc_23_box_volume_is_the_formula() {
    // Oracle self-check: the box helper is outward oriented.
    assert_rel(volume(&box_soup([5.0, -3.0, 2.0], [40.0, 30.0, 10.0])), 12000.0, 1e-12, "box");
    let (p, t) = tetra_mesh([0.0; 3]);
    assert_rel(volume(&soup(&p, &t)), 1.0 / 6.0, 1e-12, "tetra");
}

#[test]
fn ag_sc_23_binary_stl_box_is_welded_to_eight_vertices() {
    // 40 × 30 × 10 from (5, −3, 2): 12 000 mm³, bbox (5, −3, 2)..(45, 27, 12).
    // A binary STL repeats every corner per triangle: 36 corners → 8.
    let bytes = binary_stl(b"QOOB3D test", &box_soup([5.0, -3.0, 2.0], [40.0, 30.0, 10.0]));
    let m = ok(read_stl(&bytes), "binary STL");
    assert_eq!(m.triangles.len(), 12);
    assert_eq!(referenced(&m), 8, "welded corners");
    assert_rel(m.signed_volume(), 12000.0, VOL, "signed_volume");
    assert_rel(volume(&mesh_soup(&m)), 12000.0, VOL, "triangle volume");
    assert_bounds(bounds(&m.positions), [5.0, -3.0, 2.0], [45.0, 27.0, 12.0], "bbox");
}

#[test]
fn ag_sc_23_ascii_stl_box_reads_like_the_binary_one() {
    let tris = box_soup([5.0, -3.0, 2.0], [40.0, 30.0, 10.0]);
    for sci in [false, true] {
        let text = ascii_stl("block", &tris, sci);
        let m = ok(read_stl(text.as_bytes()), &format!("ASCII STL (scientific: {sci})"));
        assert_eq!(m.triangles.len(), 12);
        assert_eq!(referenced(&m), 8);
        assert_rel(m.signed_volume(), 12000.0, VOL, "ASCII signed_volume");
        assert_bounds(bounds(&m.positions), [5.0, -3.0, 2.0], [45.0, 27.0, 12.0], "ASCII bbox");
    }
    // CRLF line endings and tabs are whitespace too.
    let crlf = ascii_stl("block", &tris, false).replace('\n', "\r\n").replace("  ", "\t");
    let m = ok(read_stl(crlf.as_bytes()), "ASCII STL with CRLF and tabs");
    assert_rel(m.signed_volume(), 12000.0, VOL, "CRLF signed_volume");
}

#[test]
fn ag_sc_23_binary_stl_whose_header_starts_with_solid_is_still_binary() {
    // Many CAD tools write "solid …" into the binary header; the content
    // (84 + 50 · 12 = 684 bytes, no "facet") says binary.
    let tris = box_soup([0.0; 3], [40.0, 30.0, 10.0]);
    let bytes = binary_stl(b"solid exported by some CAD tool", &tris);
    assert_eq!(bytes.len(), 684);
    let m = ok(read_stl(&bytes), "binary STL with a 'solid' header");
    assert_eq!(m.triangles.len(), 12);
    assert_rel(m.signed_volume(), 12000.0, VOL, "signed_volume");
}

#[test]
fn ag_sc_23_signed_volume_follows_the_winding_not_the_normals() {
    // All triangles reversed: the same box, inside out → −12 000.
    let tris: Vec<Tri> =
        box_soup([0.0; 3], [40.0, 30.0, 10.0]).into_iter().map(|[a, b, c]| [a, c, b]).collect();
    let m = ok(read_stl(&binary_stl(b"", &tris)), "reversed box");
    assert_rel(m.signed_volume(), -12000.0, VOL, "reversed signed_volume");
}

#[test]
fn ag_sc_23_vertices_closer_than_1e_4_mm_are_welded_farther_ones_not() {
    // One of the 36 corner copies of the corner (40, 30, 10) is moved.
    let moved = |dx: f64| {
        let mut tris = box_soup([0.0; 3], [40.0, 30.0, 10.0]);
        let corner = tris
            .iter_mut()
            .flatten()
            .find(|v| **v == [40.0, 30.0, 10.0])
            .expect("the box has the corner (40, 30, 10)");
        corner[0] += dx;
        tris
    };
    // 4e-5 mm < 1e-4 mm: welded, still 8 vertices.
    let m = ok(read_stl(&binary_stl(b"", &moved(4e-5))), "corner moved by 4e-5");
    assert_eq!(referenced(&m), 8, "a copy 4e-5 mm away is welded");
    assert_eq!(m.triangles.len(), 12);
    // 1e-3 mm (ten times the tolerance): a ninth vertex.
    let m = ok(read_stl(&binary_stl(b"", &moved(1e-3))), "corner moved by 1e-3");
    assert_eq!(referenced(&m), 9, "a copy 1e-3 mm away is a vertex of its own");
    assert_eq!(m.triangles.len(), 12);
}

#[test]
fn ag_sc_23_collapsed_triangles_are_dropped() {
    let mut tris = box_soup([0.0; 3], [40.0, 30.0, 10.0]);
    // Two corners identical.
    tris.push([[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [5.0, 5.0, 5.0]]);
    // Two corners 4e-5 mm apart: identical after welding.
    tris.push([[40.0, 0.0, 0.0], [40.0, 0.0, 4e-5], [5.0, 5.0, 5.0]]);
    // All three corners the same point.
    tris.push([[1.0, 2.0, 3.0]; 3]);
    let m = ok(read_stl(&binary_stl(b"", &tris)), "box with collapsed triangles");
    assert_eq!(m.triangles.len(), 12, "only the 12 box triangles remain");
    assert_rel(m.signed_volume(), 12000.0, VOL, "signed_volume");
    let text = ascii_stl("x", &tris, false);
    assert_eq!(ok(read_stl(text.as_bytes()), "ASCII").triangles.len(), 12);
}

#[test]
fn ag_sc_23_200_000_triangles_are_read_one_more_is_refused_naming_the_count() {
    // 50 000 tetrahedra = 200 000 triangles (10 MB binary STL), V = 50 000 / 6.
    let mut tris = tetra_grid(50_000);
    let m = ok(read_stl(&binary_stl(b"", &tris)), "200 000 triangles");
    assert_eq!(m.triangles.len(), 200_000);
    assert_rel(m.signed_volume(), 50_000.0 / 6.0, VOL, "tetra grid");
    // One more (non-degenerate) triangle: 200 001.
    tris.push([[0.0, 0.0, 5.0], [1.0, 0.0, 5.0], [0.0, 1.0, 5.0]]);
    let text = err(read_stl(&binary_stl(b"", &tris)), "200 001 triangles");
    assert!(names_count(&text, 200_001), "the error names the count 200001: {text}");
}

#[test]
fn ag_sc_23_binary_stl_with_a_count_the_bytes_cannot_hold_is_refused() {
    // Header claims u32::MAX triangles in an 84-byte file: never allocate
    // for the claim (AG-SC-23: no more than the input justifies).
    let mut bytes = vec![0u8; 80];
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    err(read_stl(&bytes), "count u32::MAX in 84 bytes");
    // Count 12, bytes for 11 triangles.
    let full = binary_stl(b"", &box_soup([0.0; 3], [1.0, 1.0, 1.0]));
    err(read_stl(&full[..full.len() - 50]), "truncated binary STL");
}

#[test]
fn ag_sc_23_unreadable_stl_is_an_error_with_a_reason() {
    err(read_stl(b""), "empty file");
    err(read_stl(b"hello"), "five bytes");
    err(read_stl(&[0u8; 83]), "shorter than a binary header");
    // ASCII with a vertex of two numbers, and a facet with only two vertices.
    let tris = box_soup([0.0; 3], [1.0, 1.0, 1.0]);
    let good = ascii_stl("x", &tris, false);
    let short_vertex = good.replacen("vertex 0 0 0", "vertex 0 0", 1);
    assert_ne!(short_vertex, good);
    err(read_stl(short_vertex.as_bytes()), "vertex with two coordinates");
    let bad_number = good.replacen("vertex 0 0 0", "vertex 0 zero 0", 1);
    err(read_stl(bad_number.as_bytes()), "vertex with a word as coordinate");
    let two_vertices = good.replacen("      vertex 0 0 0\n", "", 1);
    err(read_stl(two_vertices.as_bytes()), "facet with two vertices");
    let unterminated = good.replace("endsolid x\n", "").replace("endloop", "");
    err(read_stl(unterminated.as_bytes()), "loops never closed");
}

// ===========================================================================
// 3MF package writers
// ===========================================================================

const CONTENT_TYPES: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    r#"<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>"#,
    "</Types>"
);

fn rels(targets: &[&str]) -> String {
    let mut s = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#
    ));
    for (i, t) in targets.iter().enumerate() {
        s.push_str(&format!(r#"<Relationship Target="{t}" Id="rel{i}" Type="{MODEL_REL}"/>"#));
    }
    s.push_str("</Relationships>");
    s
}

/// A stored 3MF: content types, `_rels/.rels` → `/3D/3dmodel.model` (the
/// first model), and for further models (`3D/Objects/…`) the part
/// relationships of the root model, as the production extension asks.
fn package(root: &str, others: &[(&str, String)]) -> Vec<u8> {
    let targets: Vec<String> = others.iter().map(|(name, _)| format!("/{name}")).collect();
    let target_refs: Vec<&str> = targets.iter().map(String::as_str).collect();
    let root_rels = rels(&["/3D/3dmodel.model"]);
    let model_rels = rels(&target_refs);
    let mut files: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("3D/3dmodel.model", root.as_bytes()),
    ];
    if !others.is_empty() {
        files.push(("3D/_rels/3dmodel.model.rels", model_rels.as_bytes()));
    }
    for (name, text) in others {
        files.push((*name, text.as_bytes()));
    }
    zip_store(&files)
}

/// A model file; `attrs` adds namespaces or `requiredextensions`.
fn model(unit: Option<&str>, attrs: &str, resources: &str, build: &str) -> String {
    let unit = unit.map(|u| format!(r#" unit="{u}""#)).unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model{unit} xml:lang=\"en-US\" \
         xmlns=\"{CORE_NS}\" {attrs}>\n<resources>\n{resources}</resources>\n\
         <build>\n{build}</build>\n</model>\n"
    )
}

fn mesh_object(id: u32, (positions, tris): &(Vec<[f64; 3]>, Vec<[u32; 3]>)) -> String {
    let mut s = format!("<object id=\"{id}\" type=\"model\"><mesh><vertices>\n");
    for p in positions {
        s.push_str(&format!("<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n", p[0], p[1], p[2]));
    }
    s.push_str("</vertices><triangles>\n");
    for t in tris {
        s.push_str(&format!("<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n", t[0], t[1], t[2]));
    }
    s.push_str("</triangles></mesh></object>\n");
    s
}

/// `<object>` with components `(objectid, transform, extra attributes)`.
fn components_object(id: u32, parts: &[(u32, Option<String>, &str)]) -> String {
    let mut s = format!("<object id=\"{id}\" type=\"model\"><components>\n");
    for (target, transform, extra) in parts {
        let t = transform.as_ref().map(|t| format!(" transform=\"{t}\"")).unwrap_or_default();
        s.push_str(&format!("<component objectid=\"{target}\"{t} {extra}/>\n"));
    }
    s.push_str("</components></object>\n");
    s
}

fn item(id: u32, transform: Option<&str>) -> String {
    let t = transform.map(|t| format!(" transform=\"{t}\"")).unwrap_or_default();
    format!("<item objectid=\"{id}\"{t}/>\n")
}

/// 3MF matrix "m00 m01 m02 m10 m11 m12 m20 m21 m22 m30 m31 m32" of a pure
/// translation (row vectors: x' = x + m30, …).
fn translate(t: [f64; 3]) -> String {
    format!("1 0 0 0 1 0 0 0 1 {} {} {}", t[0], t[1], t[2])
}

/// +90° about Z: (x, y) → (−y, x). With x' = x·m00 + y·m10 + z·m20 + m30:
/// m00 = 0, m10 = −1; y' = x·m01 + y·m11 + z·m21 + m31: m01 = 1, m11 = 0.
fn rot90z(t: [f64; 3]) -> String {
    format!("0 1 0 -1 0 0 0 0 1 {} {} {}", t[0], t[1], t[2])
}

// ===========================================================================
// AG-SC-23: 3MF written by Bambu Studio (third-party fixture)
// ===========================================================================

#[test]
fn ag_sc_23_bambu_studio_3mf_imports_both_objects_with_their_volumes_and_sizes() {
    // tests/fixtures/README.md: Block 40 × 30 × 10 = 12 000 mm³, Angle
    // (L-profile prism) 30 × 20 × 15 = 2760 mm³; 12 + 20 triangles, 8 + 12
    // vertices. Deflated entries, production extension (`p:path` into
    // 3D/Objects/object_*.model, requiredextensions="p").
    let m = ok(read_3mf(FIXTURE), "Bambu Studio 3MF");
    assert_eq!(m.triangles.len(), 32, "12 + 20 triangles");
    assert_eq!(referenced(&m), 20, "8 + 12 vertices");
    assert_rel(m.signed_volume(), 12000.0 + 2760.0, VOL, "signed_volume of both");
    // Positions from the transforms in 3D/3dmodel.model:
    // Block: mesh centred (x ±20, y ±15, z ±5), component +(0, 0, 5), item
    //   +(100, 110.99998, 0) → x 80..120, y 95.99998..125.99998, z 0..10.
    // Angle: mesh centred (x ±15, y ±10, z ±7.5), component +(15, 10, 7.5)
    //   undoes the centring, item +(87.64437, 74.00003, 0)
    //   → x 87.64437..117.64437, y 74.00003..94.00003, z 0..15.
    assert_bounds(bounds(&m.positions), [80.0, 74.00003, 0.0], [120.0, 125.99998, 15.0], "all");
    let parts = components(&m);
    assert_eq!(parts.len(), 2, "two separate objects");
    let (block, angle) =
        if parts[0].min[1] > 95.0 { (&parts[0], &parts[1]) } else { (&parts[1], &parts[0]) };

    assert_eq!((block.triangles, block.vertices.len()), (12, 8), "Block mesh");
    assert_rel(block.volume, 12000.0, VOL, "Block volume");
    assert_bounds((block.min, block.max), [80.0, 95.99998, 0.0], [120.0, 125.99998, 10.0], "Block");

    assert_eq!((angle.triangles, angle.vertices.len()), (20, 12), "Angle mesh");
    assert_rel(angle.volume, 2760.0, VOL, "Angle volume");
    let lo = [87.64437, 74.00003, 0.0];
    assert_bounds((angle.min, angle.max), lo, [117.64437, 94.00003, 15.0], "Angle");
    // The L outline of the README, placed at the item offset, at z 0 and 15:
    // every corner is a vertex (a mirrored or transposed transform would not).
    let outline = [[0.0, 0.0], [30.0, 0.0], [30.0, 4.0], [4.0, 4.0], [4.0, 20.0], [0.0, 20.0]];
    for [x, y] in outline {
        for z in [0.0, 15.0] {
            let want = [lo[0] + x, lo[1] + y, z];
            let found = angle.vertices.iter().any(|v| (0..3).all(|k| (v[k] - want[k]).abs() < POS));
            assert!(found, "Angle corner {want:?} missing in {:?}", angle.vertices);
        }
    }
}

// ===========================================================================
// AG-SC-23: 3MF made by hand
// ===========================================================================

#[test]
fn ag_sc_23_core_3mf_without_transforms_or_unit_is_millimetres() {
    // No `unit` attribute: the 3MF default is millimeter.
    let b = box_mesh([1.0, 2.0, 3.0], [40.0, 30.0, 10.0]);
    let bytes = package(&model(None, "", &mesh_object(1, &b), &item(1, None)), &[]);
    let m = ok(read_3mf(&bytes), "core 3MF");
    assert_eq!(m.triangles.len(), 12);
    assert_rel(m.signed_volume(), 12000.0, VOL, "volume");
    assert_bounds(bounds(&m.positions), [1.0, 2.0, 3.0], [41.0, 32.0, 13.0], "bbox");
}

#[test]
fn ag_sc_23_item_transform_rotates_and_translates_in_row_vector_convention() {
    // Box x 0..40, y 0..30, z 0..10; item: +90° about Z, then +(100, 50, 2).
    // x' = −y + 100 ∈ [70, 100], y' = x + 50 ∈ [50, 90], z' = z + 2 ∈ [2, 12].
    // (The transposed, column-vector reading would give x' ∈ [100, 130].)
    let b = box_mesh([0.0; 3], [40.0, 30.0, 10.0]);
    let t = rot90z([100.0, 50.0, 2.0]);
    let bytes =
        package(&model(Some("millimeter"), "", &mesh_object(1, &b), &item(1, Some(&t))), &[]);
    let m = ok(read_3mf(&bytes), "rotated item");
    assert_rel(m.signed_volume(), 12000.0, VOL, "a rotation keeps the volume");
    assert_bounds(bounds(&m.positions), [70.0, 50.0, 2.0], [100.0, 90.0, 12.0], "bbox");
    // The corner (40, 0, 0) → (−0 + 100, 40 + 50, 0 + 2).
    let want = [100.0, 90.0, 2.0];
    assert!(m.positions.iter().any(|p| (0..3).all(|k| (p[k] - want[k]).abs() < POS)), "{want:?}");
}

#[test]
fn ag_sc_23_general_rotation_maps_every_vertex() {
    // Tetrahedron (0,0,0), (10,0,0), (0,10,0), (0,0,10) scaled from the unit
    // one; item: 30° about Z, then +(5, 6, 7). c = cos 30° = √3/2, s = 1/2:
    // m00 = c, m01 = s, m10 = −s, m11 = c (x' = x c − y s, y' = x s + y c).
    let (p, t) = tetra_mesh([0.0; 3]);
    let p: Vec<[f64; 3]> = p.iter().map(|v| v.map(|c| c * 10.0)).collect();
    let (c, s) = (3f64.sqrt() / 2.0, 0.5);
    let transform = format!("{c} {s} 0 {} {c} 0 0 0 1 5 6 7", -s);
    let bytes =
        package(&model(None, "", &mesh_object(1, &(p, t)), &item(1, Some(&transform))), &[]);
    let m = ok(read_3mf(&bytes), "30° tetrahedron");
    // Volume 1000 / 6, unchanged by the rotation.
    assert_rel(m.signed_volume(), 1000.0 / 6.0, VOL, "volume");
    let expected = [
        [5.0, 6.0, 7.0],
        [5.0 + 10.0 * c, 6.0 + 10.0 * s, 7.0],
        [5.0 - 10.0 * s, 6.0 + 10.0 * c, 7.0],
        [5.0, 6.0, 17.0],
    ];
    assert_eq!(referenced(&m), 4);
    for want in expected {
        let found = m.positions.iter().any(|q| (0..3).all(|k| (q[k] - want[k]).abs() < POS));
        assert!(found, "vertex {want:?} missing in {:?}", m.positions);
    }
}

#[test]
fn ag_sc_23_components_nest_to_any_depth_innermost_transform_first() {
    // obj 1: box [0, 2]³. obj 2 → 1 with T1 = +(10, 0, 0). obj 3 → 2 with
    // T2 = 90° about Z. obj 4 → 3 with T3 = +(0, 0, 5), and → 1 directly.
    // obj 5 → 4 (identity). Item → 5 with +(0, 0, 1).
    // Copy A, p·T1·T2·T3·item: x 10..12, y 0..2 → (x' = −y, y' = x)
    //   x −2..0, y 10..12 → z 5..7 → z 6..8. (T2 before T1: x 8..10.)
    // Copy B, p·item: [0, 2]² × [1, 3].
    let resources = [
        mesh_object(1, &box_mesh([0.0; 3], [2.0; 3])),
        components_object(2, &[(1, Some(translate([10.0, 0.0, 0.0])), "")]),
        components_object(3, &[(2, Some(rot90z([0.0; 3])), "")]),
        components_object(4, &[(3, Some(translate([0.0, 0.0, 5.0])), ""), (1, None, "")]),
        components_object(5, &[(4, None, "")]),
    ]
    .concat();
    let t = translate([0.0, 0.0, 1.0]);
    let bytes = package(&model(None, "", &resources, &item(5, Some(&t))), &[]);
    let m = ok(read_3mf(&bytes), "nested components");
    assert_eq!(m.triangles.len(), 24, "two copies of the box");
    assert_rel(m.signed_volume(), 16.0, VOL, "2 · 8 mm³");
    let parts = components(&m);
    assert_eq!(parts.len(), 2);
    assert_bounds((parts[0].min, parts[0].max), [-2.0, 10.0, 6.0], [0.0, 12.0, 8.0], "copy A");
    assert_bounds((parts[1].min, parts[1].max), [0.0, 0.0, 1.0], [2.0, 2.0, 3.0], "copy B");
}

#[test]
fn ag_sc_23_every_build_item_is_imported_unreferenced_resources_are_not() {
    // obj 1: box 10³ at the origin, placed twice (+0 and +(50, 0, 0)).
    // obj 2: box 20³ that no item references.
    let resources = [
        mesh_object(1, &box_mesh([0.0; 3], [10.0; 3])),
        mesh_object(2, &box_mesh([0.0; 3], [20.0; 3])),
    ]
    .concat();
    let build = [item(1, None), item(1, Some(&translate([50.0, 0.0, 0.0])))].concat();
    let m = ok(read_3mf(&package(&model(None, "", &resources, &build), &[])), "two items");
    assert_eq!(m.triangles.len(), 24);
    assert_rel(m.signed_volume(), 2000.0, VOL, "2 · 1000 mm³, without the 8000 of obj 2");
    let parts = components(&m);
    assert_eq!(parts.len(), 2);
    assert_bounds((parts[0].min, parts[0].max), [0.0; 3], [10.0; 3], "first item");
    assert_bounds((parts[1].min, parts[1].max), [50.0, 0.0, 0.0], [60.0, 10.0, 10.0], "second");
}

#[test]
fn ag_sc_23_units_are_converted_to_millimetres() {
    // Box 1 × 2 × 3 (× c) units, item translated by c units in x; k mm per
    // unit. Expected: min (c k, 0, 0), max (2 c k, 2 c k, 3 c k), volume
    // 6 (c k)³. c = 1000 for micron keeps the box at 1 × 2 × 3 mm.
    let units = [
        ("micron", 0.001, 1000.0),
        ("millimeter", 1.0, 1.0),
        ("centimeter", 10.0, 1.0),
        ("inch", 25.4, 1.0),
        ("foot", 304.8, 1.0),
        ("meter", 1000.0, 1.0),
    ];
    for (unit, k, c) in units {
        let b = box_mesh([0.0; 3], [c, 2.0 * c, 3.0 * c]);
        let t = translate([c, 0.0, 0.0]);
        let bytes = package(&model(Some(unit), "", &mesh_object(1, &b), &item(1, Some(&t))), &[]);
        let m = ok(read_3mf(&bytes), unit);
        let l = c * k;
        assert_rel(m.signed_volume(), 6.0 * l * l * l, VOL, &format!("volume in {unit}"));
        let (lo, hi) = bounds(&m.positions);
        // Relative position tolerance: 1e-9 of the size (f64 scaling only).
        let tol = 1e-9 * 3.0 * l;
        assert_point(lo, [l, 0.0, 0.0], tol, &format!("{unit}: min"));
        assert_point(hi, [2.0 * l, 2.0 * l, 3.0 * l], tol, &format!("{unit}: max"));
    }
    let b = box_mesh([0.0; 3], [1.0; 3]);
    let bytes = package(&model(Some("furlong"), "", &mesh_object(1, &b), &item(1, None)), &[]);
    err(read_3mf(&bytes), "unknown unit");
}

#[test]
fn ag_sc_23_production_paths_resolve_object_ids_per_model_file() {
    // Root: obj 1 → p:path a.model obj 2. a.model: obj 2 → obj 1 (no path:
    // the same file), obj 1 = box 10 × 20 × 30 at the origin. Object ids
    // are scoped per model file: root obj 1 is not a.model obj 1. Root
    // component +(0, 0, 5), a.model component +(100, 0, 0), item +(0, 50, 0).
    let attrs = format!(r#"xmlns:p="{PROD_NS}" requiredextensions="p""#);
    let sub_resources = [
        mesh_object(1, &box_mesh([0.0; 3], [10.0, 20.0, 30.0])),
        components_object(2, &[(1, Some(translate([100.0, 0.0, 0.0])), "")]),
    ]
    .concat();
    let sub = model(Some("millimeter"), &attrs, &sub_resources, "");
    let path = r#"p:path="/3D/Objects/a.model""#;
    let root_res = components_object(1, &[(2, Some(translate([0.0, 0.0, 5.0])), path)]);
    let root =
        model(Some("millimeter"), &attrs, &root_res, &item(1, Some(&translate([0.0, 50.0, 0.0]))));
    let bytes = package(&root, &[("3D/Objects/a.model", sub)]);
    let m = ok(read_3mf(&bytes), "production 3MF");
    assert_rel(m.signed_volume(), 6000.0, VOL, "10 · 20 · 30");
    // x 0..10 + 100, y 0..20 + 50, z 0..30 + 5.
    assert_bounds(bounds(&m.positions), [100.0, 50.0, 5.0], [110.0, 70.0, 35.0], "bbox");
}

#[test]
fn ag_sc_23_production_extension_is_known_by_its_namespace_not_its_prefix() {
    let attrs = format!(r#"xmlns:prod="{PROD_NS}" requiredextensions="prod""#);
    let sub = model(None, &attrs, &mesh_object(7, &box_mesh([0.0; 3], [10.0; 3])), "");
    let root_res = components_object(1, &[(7, None, r#"prod:path="/3D/Objects/b.model""#)]);
    let root = model(None, &attrs, &root_res, &item(1, None));
    let m = ok(read_3mf(&package(&root, &[("3D/Objects/b.model", sub)])), "prefix prod");
    assert_rel(m.signed_volume(), 1000.0, VOL, "volume");
}

#[test]
fn ag_sc_23_a_required_extension_other_than_production_is_refused() {
    let b = mesh_object(1, &box_mesh([0.0; 3], [10.0; 3]));
    let cases = [
        format!(r#"xmlns:b="{BEAM_NS}" requiredextensions="b""#),
        format!(r#"xmlns:p="{PROD_NS}" xmlns:m="{MATERIAL_NS}" requiredextensions="p m""#),
        // A prefix that is not declared at all.
        r#"requiredextensions="q""#.to_owned(),
    ];
    for attrs in cases {
        err(read_3mf(&package(&model(None, &attrs, &b, &item(1, None)), &[])), &attrs);
    }
    // Declared but not required: fine (Bambu Studio declares its own namespace).
    let attrs = format!(
        r#"xmlns:m="{MATERIAL_NS}" xmlns:BambuStudio="http://schemas.bambulab.com/package/2021""#
    );
    let m = ok(read_3mf(&package(&model(None, &attrs, &b, &item(1, None)), &[])), "declared only");
    assert_rel(m.signed_volume(), 1000.0, VOL, "volume");
}

#[test]
fn ag_sc_23_well_formed_xml_in_every_spelling_is_read() {
    // XML 1.0 allows single quotes, spaces around `=`, explicit end tags,
    // comments, CRLF, entities and exponents; 3MF numbers allow exponents.
    // Triangle property attributes (pid, p1) are ignored. Box 40 × 30 × 10.
    let text = "<?xml version='1.0' encoding='UTF-8'?>\r\n\
        <!-- written by hand -->\r\n\
        <model unit = 'millimeter' xml:lang='en-US' xmlns='http://schemas.microsoft.com/3dmanufacturing/core/2015/02'>\r\n\
        <metadata name='Title'>A &amp; B &lt;test&gt;</metadata>\r\n\
        <resources>\r\n<object id='1' type='model' name='Bl&#246;ck'>\r\n<mesh>\r\n<vertices>\r\n\
        <vertex x='0' y='0' z='0'/>\r\n<vertex x='4.0E1' y='0' z='0'></vertex>\r\n\
        <vertex x='0' y='3e1' z='0' />\r\n<vertex x='40' y='30.000' z='0'/>\r\n\
        <vertex x='0' y='0' z='1e+1'/>\r\n<vertex x='40' y='0' z='10'/>\r\n\
        <!-- the last two -->\r\n<vertex x='0' y='30' z='10'/>\r\n<vertex x='+40' y='30' z='10'/>\r\n\
        </vertices>\r\n<triangles>\r\n\
        <triangle v1='0' v2='2' v3='3' pid='1' p1='0'/>\r\n<triangle v1='0' v2='3' v3='1'/>\r\n\
        <triangle v1='4' v2='5' v3='7'/>\r\n<triangle v1='4' v2='7' v3='6'/>\r\n\
        <triangle v1='0' v2='1' v3='5'/>\r\n<triangle v1='0' v2='5' v3='4'/>\r\n\
        <triangle v1='2' v2='6' v3='7'/>\r\n<triangle v1='2' v2='7' v3='3'/>\r\n\
        <triangle v1='0' v2='4' v3='6'/>\r\n<triangle v1='0' v2='6' v3='2'/>\r\n\
        <triangle v1='1' v2='3' v3='7'></triangle>\r\n<triangle v1='1' v2='7' v3='5'/>\r\n\
        </triangles>\r\n</mesh>\r\n</object>\r\n</resources>\r\n\
        <build>\r\n<item objectid='1'/>\r\n</build>\r\n</model>\r\n";
    let m = ok(read_3mf(&package(text, &[])), "XML spellings");
    assert_eq!(m.triangles.len(), 12);
    assert_rel(m.signed_volume(), 12000.0, VOL, "volume");
    assert_bounds(bounds(&m.positions), [0.0; 3], [40.0, 30.0, 10.0], "bbox");
}

// ===========================================================================
// AG-SC-23: 3MF errors
// ===========================================================================

fn box_package_with(edit: impl Fn(String) -> String) -> Vec<u8> {
    let b = box_mesh([0.0; 3], [40.0, 30.0, 10.0]);
    let text = model(None, "", &mesh_object(1, &b), &item(1, None));
    let edited = edit(text.clone());
    assert_ne!(edited, text, "the edit must change the model");
    package(&edited, &[])
}

#[test]
fn ag_sc_23_malformed_model_xml_is_an_error_naming_the_element() {
    // Triangle index 8 of 8 vertices (valid are 0..=7).
    let text = err(
        read_3mf(&box_package_with(|t| t.replacen(r#"v3="3""#, r#"v3="8""#, 1))),
        "index out of range",
    );
    assert!(text.contains("triangle"), "names the triangle: {text}");
    let text = err(
        read_3mf(&box_package_with(|t| t.replacen(r#"v1="0""#, r#"v1="-1""#, 1))),
        "negative index",
    );
    assert!(text.contains("triangle"), "names the triangle: {text}");
    let text = err(
        read_3mf(&box_package_with(|t| t.replacen(r#"x="40""#, r#"x="forty""#, 1))),
        "word as coordinate",
    );
    assert!(text.contains("vertex"), "names the vertex: {text}");
    let text = err(
        read_3mf(&box_package_with(|t| t.replacen(r#" z="0"/>"#, "/>", 1))),
        "vertex without z",
    );
    assert!(text.contains("vertex"), "names the vertex: {text}");
    let text = err(
        read_3mf(&box_package_with(|t| {
            t.replace(r#"<item objectid="1"/>"#, r#"<item objectid="99"/>"#)
        })),
        "item of an unknown object",
    );
    assert!(text.contains("99") || text.contains("item"), "names the item: {text}");
    err(read_3mf(&box_package_with(|t| t.replace("</model>", ""))), "unclosed model");
    err(read_3mf(&box_package_with(|t| t.replacen("</vertices>", "", 1))), "unclosed vertices");
    err(
        read_3mf(&box_package_with(|t| t.replacen(r#"x="40""#, r#"x="4"0""#, 1))),
        "broken attribute",
    );
    err(read_3mf(&box_package_with(|t| t[..t.len() / 2].to_owned())), "model cut in half");
}

#[test]
fn ag_sc_23_component_cycles_and_dangling_references_are_errors() {
    let b = mesh_object(9, &box_mesh([0.0; 3], [1.0; 3]));
    let cycle =
        [b.clone(), components_object(1, &[(2, None, "")]), components_object(2, &[(1, None, "")])]
            .concat();
    err(read_3mf(&package(&model(None, "", &cycle, &item(1, None)), &[])), "cycle 1 → 2 → 1");
    let own = [b.clone(), components_object(1, &[(1, None, "")])].concat();
    err(read_3mf(&package(&model(None, "", &own, &item(1, None)), &[])), "1 → 1");
    let dangling = [b, components_object(1, &[(42, None, "")])].concat();
    err(read_3mf(&package(&model(None, "", &dangling, &item(1, None)), &[])), "1 → missing 42");
}

#[test]
fn ag_sc_23_a_missing_production_model_names_the_zip_entry() {
    let attrs = format!(r#"xmlns:p="{PROD_NS}" requiredextensions="p""#);
    let root_res = components_object(1, &[(1, None, r#"p:path="/3D/Objects/missing.model""#)]);
    let root = model(None, &attrs, &root_res, &item(1, None));
    let text = err(read_3mf(&package(&root, &[])), "p:path to a missing entry");
    assert!(text.contains("missing.model"), "names the entry: {text}");
}

#[test]
fn ag_sc_23_broken_zip_is_an_error_with_a_reason() {
    err(read_3mf(b""), "empty file");
    err(read_3mf(b"PK\x03\x04 not really"), "truncated local header");
    err(read_3mf(b"solid x\nendsolid x\n"), "an STL is no 3MF");
    err(read_3mf(&FIXTURE[..FIXTURE.len() / 2]), "fixture cut in half");
    err(read_3mf(&FIXTURE[..FIXTURE.len() - 10]), "end of central directory cut off");
    // An archive without any model.
    let text = err(
        read_3mf(&zip_store(&[("[Content_Types].xml", CONTENT_TYPES.as_bytes())])),
        "no 3D/3dmodel.model",
    );
    assert!(text.contains("3dmodel.model"), "names the missing entry: {text}");
}

#[test]
fn ag_sc_23_a_corrupted_entry_fails_its_crc() {
    // "40" → "90" in one coordinate keeps the XML well-formed, so only the
    // CRC-32 of the ZIP entry can tell (a box 90 wide would import).
    let b = box_mesh([0.0; 3], [40.0, 30.0, 10.0]);
    let mut bytes = package(&model(None, "", &mesh_object(1, &b), &item(1, None)), &[]);
    let at = bytes.windows(6).position(|w| w == br#"x="40""#).expect("a coordinate 40") + 3;
    bytes[at] = b'9';
    err(read_3mf(&bytes), "CRC mismatch");
}

#[test]
fn ag_sc_23_an_unsupported_compression_method_is_an_error() {
    // Method 12 (bzip2) in the local header and the central directory entry
    // of the model; the bytes stay stored.
    let b = box_mesh([0.0; 3], [10.0; 3]);
    let mut bytes = package(&model(None, "", &mesh_object(1, &b), &item(1, None)), &[]);
    let name = b"3D/3dmodel.model";
    let mut patched = 0;
    for i in 0..bytes.len().saturating_sub(4) {
        let local = bytes[i..].starts_with(&[0x50, 0x4b, 0x03, 0x04]);
        let central = bytes[i..].starts_with(&[0x50, 0x4b, 0x01, 0x02]);
        let (method_at, name_at) = if local {
            (8, 30)
        } else if central {
            (10, 46)
        } else {
            continue;
        };
        if bytes.get(i + name_at..i + name_at + name.len()) == Some(name) {
            bytes[i + method_at] = 12;
            patched += 1;
        }
    }
    assert_eq!(patched, 2, "local header and central directory entry");
    err(read_3mf(&bytes), "bzip2 entry");
}

#[test]
fn ag_sc_23_triangle_limit_counts_after_expanding_components() {
    // obj 1: unit tetrahedron (4 triangles). obj 2: `a` copies of obj 1 at
    // x = 3i. obj 3: 200 copies of obj 2 at y = 3j. a = 250: 250 · 200 · 4
    // = 200 000 triangles (accepted, V = 50 000 / 6); a = 251: 200 800.
    let build_with = |a: u32| {
        let row: Vec<(u32, Option<String>, &str)> =
            (0..a).map(|i| (1, Some(translate([3.0 * f64::from(i), 0.0, 0.0])), "")).collect();
        let grid: Vec<(u32, Option<String>, &str)> =
            (0..200).map(|j| (2, Some(translate([0.0, 3.0 * f64::from(j), 0.0])), "")).collect();
        let resources = [
            mesh_object(1, &tetra_mesh([0.0; 3])),
            components_object(2, &row),
            components_object(3, &grid),
        ]
        .concat();
        package(&model(None, "", &resources, &item(3, None)), &[])
    };
    let m = ok(read_3mf(&build_with(250)), "200 000 triangles through components");
    assert_eq!(m.triangles.len(), 200_000);
    assert_rel(m.signed_volume(), 50_000.0 / 6.0, VOL, "volume");
    let text = err(read_3mf(&build_with(251)), "200 800 triangles through components");
    assert!(names_count(&text, 200_800), "the error names the count 200800: {text}");
}

#[test]
fn ag_sc_23_a_tiny_file_that_expands_to_millions_of_triangles_is_refused() {
    // Six levels of ten components over a tetrahedron: 10⁶ · 4 triangles
    // from a few kilobytes. Must be refused by the limit, not built.
    let mut resources = mesh_object(1, &tetra_mesh([0.0; 3]));
    for level in 2..=7u32 {
        let parts: Vec<(u32, Option<String>, &str)> = (0..10)
            .map(|i| (level - 1, Some(translate([0.0, 0.0, 2.0 * f64::from(i)])), ""))
            .collect();
        resources.push_str(&components_object(level, &parts));
    }
    let bytes = package(&model(None, "", &resources, &item(7, None)), &[]);
    assert!(bytes.len() < 10_000, "{} bytes", bytes.len());
    err(read_3mf(&bytes), "4 000 000 triangles");
}

// ===========================================================================
// Deflate and the 512 MiB limit
// ===========================================================================

/// Bit writer for a deflate stream (RFC 1951 §3.1.1): values LSB first,
/// Huffman codes MSB first.
#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn push(&mut self, value: u32, count: u32) {
        self.acc |= u64::from(value) << self.n;
        self.n += count;
        while self.n >= 8 {
            self.out.push((self.acc & 0xff) as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn code(&mut self, code: u32, len: u32) {
        let reversed = (0..len).fold(0, |r, i| r | (((code >> i) & 1) << (len - 1 - i)));
        self.push(reversed, len);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc & 0xff) as u8);
        }
        self.out
    }
}

/// One final fixed-Huffman block (RFC 1951 §3.2.6): `prefix` as literals,
/// then `byte` once and `matches` back-references of length 258 at
/// distance 1. Inflates to `prefix` + `byte` × (1 + 258 · matches).
/// Literals 0–143: 8-bit codes 0x30 + b; 144–255: 9-bit 0x190 + b − 144;
/// length 258 = symbol 285: 8-bit 0xC5; distance 1 = code 0: 5 bits;
/// end of block 256: 7-bit 0.
fn deflate_run(prefix: &[u8], byte: u8, matches: usize) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.push(1, 1); // BFINAL
    bits.push(1, 2); // BTYPE = 01, fixed Huffman
    let literal = |bits: &mut Bits, b: u8| {
        if b < 144 {
            bits.code(0x30 + u32::from(b), 8);
        } else {
            bits.code(0x190 + u32::from(b) - 144, 9);
        }
    };
    for &b in prefix {
        literal(&mut bits, b);
    }
    literal(&mut bits, byte);
    for _ in 0..matches {
        bits.code(0xC5, 8);
        bits.code(0, 5);
    }
    bits.code(0, 7);
    bits.finish()
}

/// GF(2) matrix helpers of zlib's `crc32_combine`: the CRC-32 of A‖B from
/// the CRCs of A and B and the length of B.
fn gf2_times(mat: &[u32; 32], mut vec: u32) -> u32 {
    let mut sum = 0;
    let mut i = 0;
    while vec != 0 {
        if vec & 1 != 0 {
            sum ^= mat[i];
        }
        vec >>= 1;
        i += 1;
    }
    sum
}

fn gf2_square(mat: &[u32; 32]) -> [u32; 32] {
    std::array::from_fn(|n| gf2_times(mat, mat[n]))
}

fn crc32_combine(mut crc1: u32, crc2: u32, mut len2: u64) -> u32 {
    if len2 == 0 {
        return crc1;
    }
    // Operator for one zero bit, then two, then four.
    let mut odd = [0u32; 32];
    odd[0] = 0xEDB8_8320;
    for (n, row) in odd.iter_mut().enumerate().skip(1) {
        *row = 1 << (n - 1);
    }
    let mut even = gf2_square(&odd);
    odd = gf2_square(&even);
    loop {
        even = gf2_square(&odd);
        if len2 & 1 != 0 {
            crc1 = gf2_times(&even, crc1);
        }
        len2 >>= 1;
        if len2 == 0 {
            break;
        }
        odd = gf2_square(&even);
        if len2 & 1 != 0 {
            crc1 = gf2_times(&odd, crc1);
        }
        len2 >>= 1;
        if len2 == 0 {
            break;
        }
    }
    crc1 ^ crc2
}

/// CRC-32 of `byte` repeated `n` times, by doubling with `crc32_combine`.
fn crc32_run(byte: u8, n: u64) -> u32 {
    let (mut result, mut have) = (0u32, 0u64);
    let (mut block, mut block_len) = (crc32(&[byte]), 1u64);
    let mut rest = n;
    while rest > 0 {
        if rest & 1 != 0 {
            result = if have == 0 { block } else { crc32_combine(result, block, block_len) };
            have += block_len;
        }
        block = crc32_combine(block, block, block_len);
        block_len *= 2;
        rest >>= 1;
    }
    result
}

/// One ZIP entry: name, method (0 stored, 8 deflate), the bytes as stored,
/// the CRC-32 and size of the inflated data as declared in the headers.
struct Entry<'a> {
    name: &'a str,
    method: u16,
    data: &'a [u8],
    crc: u32,
    size: u32,
}

impl<'a> Entry<'a> {
    fn stored(name: &'a str, data: &'a [u8]) -> Self {
        let size = u32::try_from(data.len()).expect("small entry");
        Self { name, method: 0, data, crc: crc32(data), size }
    }
}

/// ZIP archive (APPNOTE 4.3.7 local header, 4.3.12 central directory,
/// 4.3.16 end record) with stored or deflated entries.
fn zip(entries: &[Entry<'_>]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for e in entries {
        let offset = u32::try_from(out.len()).expect("small archive");
        let packed = u32::try_from(e.data.len()).expect("small entry");
        let name_len = u16::try_from(e.name.len()).expect("short name");
        le32(&mut out, 0x0403_4b50);
        for x in [20, 0, e.method, 0, 0x21] {
            le16(&mut out, x);
        }
        for x in [e.crc, packed, e.size] {
            le32(&mut out, x);
        }
        le16(&mut out, name_len);
        le16(&mut out, 0);
        out.extend_from_slice(e.name.as_bytes());
        out.extend_from_slice(e.data);
        le32(&mut central, 0x0201_4b50);
        for x in [20, 20, 0, e.method, 0, 0x21] {
            le16(&mut central, x);
        }
        for x in [e.crc, packed, e.size] {
            le32(&mut central, x);
        }
        for x in [name_len, 0, 0, 0, 0] {
            le16(&mut central, x);
        }
        le32(&mut central, 0);
        le32(&mut central, offset);
        central.extend_from_slice(e.name.as_bytes());
    }
    let central_offset = u32::try_from(out.len()).expect("small archive");
    let count = u16::try_from(entries.len()).expect("few entries");
    let central_len = u32::try_from(central.len()).expect("small directory");
    out.extend_from_slice(&central);
    le32(&mut out, 0x0605_4b50);
    for x in [0, 0, count, count] {
        le16(&mut out, x);
    }
    le32(&mut out, central_len);
    le32(&mut out, central_offset);
    le16(&mut out, 0);
    out
}

/// A deflated model entry: `text` followed by 1 + 258 · `matches` spaces
/// (whitespace after the root element is well-formed XML), with the true
/// CRC and size, or a lie about the size.
fn padded_model<'a>(
    name: &'a str,
    text: &str,
    matches: usize,
    stream: &'a mut Vec<u8>,
) -> (Entry<'a>, u64) {
    *stream = deflate_run(text.as_bytes(), b' ', matches);
    let pad = 1 + 258 * u64::try_from(matches).expect("fits");
    let total = u64::try_from(text.len()).expect("fits") + pad;
    let crc = crc32_combine(crc32(text.as_bytes()), crc32_run(b' ', pad), pad);
    let size = u32::try_from(total).expect("below 4 GiB");
    let data: &'a [u8] = stream.as_slice();
    (Entry { name, method: 8, data, crc, size }, total)
}

const MIB: u64 = 1024 * 1024;

#[test]
fn crc32_combine_matches_the_crc_of_the_concatenation() {
    // Oracle self-check against the crate's plain CRC-32.
    let (a, b) = (b"<model>".as_slice(), [b' '; 1000]);
    let whole: Vec<u8> = [a, &b].concat();
    assert_eq!(crc32_combine(crc32(a), crc32(&b), 1000), crc32(&whole));
    for n in [1, 2, 3, 258, 259, 1000] {
        assert_eq!(
            crc32_run(b' ', n),
            crc32(&vec![b' '; usize::try_from(n).expect("small")]),
            "{n}"
        );
    }
}

#[test]
fn ag_sc_23_deflated_entries_with_back_references_import() {
    // Control for the bomb tests below: the same construction with 3
    // matches (775 spaces) and the true size imports the box.
    let b = box_mesh([0.0; 3], [40.0, 30.0, 10.0]);
    let text = model(None, "", &mesh_object(1, &b), &item(1, None));
    let mut stream = Vec::new();
    let (entry, total) = padded_model("3D/3dmodel.model", &text, 3, &mut stream);
    assert_eq!(total, text.len() as u64 + 775);
    let root_rels = rels(&["/3D/3dmodel.model"]);
    let bytes = zip(&[
        Entry::stored("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        Entry::stored("_rels/.rels", root_rels.as_bytes()),
        entry,
    ]);
    let m = ok(read_3mf(&bytes), "deflated model");
    assert_rel(m.signed_volume(), 12000.0, VOL, "volume");
}

#[test]
fn ag_sc_23_a_model_that_inflates_beyond_512_mib_is_refused() {
    // 2 081 000 matches: 1 + 258 · 2 081 000 = 536 898 001 spaces > 512 MiB
    // = 536 870 912 bytes, from ~3.4 MB of deflate. Without the limit this
    // file would import (the true CRC and size are declared).
    let b = box_mesh([0.0; 3], [40.0, 30.0, 10.0]);
    let text = model(None, "", &mesh_object(1, &b), &item(1, None));
    let root_rels = rels(&["/3D/3dmodel.model"]);
    let mut stream = Vec::new();
    let (entry, total) = padded_model("3D/3dmodel.model", &text, 2_081_000, &mut stream);
    assert!(total > 512 * MIB, "{total}");
    let honest = zip(&[
        Entry::stored("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        Entry::stored("_rels/.rels", root_rels.as_bytes()),
        Entry { name: entry.name, method: 8, data: entry.data, crc: entry.crc, size: entry.size },
    ]);
    err(read_3mf(&honest), "512 MiB bomb with its true size");
    // The same stream declaring only the size of the XML text.
    let small = u32::try_from(text.len()).expect("small");
    let liar = zip(&[
        Entry::stored("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        Entry::stored("_rels/.rels", root_rels.as_bytes()),
        Entry { name: entry.name, method: 8, data: entry.data, crc: entry.crc, size: small },
    ]);
    err(read_3mf(&liar), "512 MiB bomb that declares a small size");
}

#[test]
fn ag_sc_23_the_512_mib_limit_is_for_all_entries_together() {
    // Root and production model, each 1 + 258 · 1 219 275 = 314 572 951
    // bytes of padding (300 MiB = 314 572 800): each below the limit,
    // together about 600 MiB.
    let attrs = format!(r#"xmlns:p="{PROD_NS}" requiredextensions="p""#);
    let sub = model(None, &attrs, &mesh_object(1, &box_mesh([0.0; 3], [10.0; 3])), "");
    let root_res = components_object(1, &[(1, None, r#"p:path="/3D/Objects/a.model""#)]);
    let root = model(None, &attrs, &root_res, &item(1, None));
    let (mut s1, mut s2) = (Vec::new(), Vec::new());
    let (e1, t1) = padded_model("3D/3dmodel.model", &root, 1_219_275, &mut s1);
    let (e2, t2) = padded_model("3D/Objects/a.model", &sub, 1_219_275, &mut s2);
    assert!(t1 < 512 * MIB && t2 < 512 * MIB && t1 + t2 > 512 * MIB, "{t1} + {t2}");
    let root_rels = rels(&["/3D/3dmodel.model"]);
    let model_rels = rels(&["/3D/Objects/a.model"]);
    let bytes = zip(&[
        Entry::stored("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        Entry::stored("_rels/.rels", root_rels.as_bytes()),
        Entry::stored("3D/_rels/3dmodel.model.rels", model_rels.as_bytes()),
        e1,
        e2,
    ]);
    err(read_3mf(&bytes), "two entries of 300 MiB");
}

// ===========================================================================
// Hostile bytes never panic (AG-SC-23)
// ===========================================================================

/// Tokens of the STL grammar plus numbers that are hard to parse.
const STL_TOKENS: &[&str] = &[
    "solid", "endsolid", "facet", "normal", "outer", "loop", "endloop", "endfacet", "vertex", "0",
    "1", "-1", "1e400", "nan", "inf", "-0", "3.5", "1e-400", "x", "\n", " ",
];

/// Fragments of 3MF model XML, assembled at random inside a valid package.
const XML_TOKENS: &[&str] = &[
    "<model xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">",
    "</model>",
    "<resources>",
    "</resources>",
    "<object id=\"1\">",
    "<object id=\"2\">",
    "</object>",
    "<mesh>",
    "</mesh>",
    "<vertices>",
    "</vertices>",
    "<vertex x=\"1\" y=\"2\" z=\"3\"/>",
    "<vertex x=\"nan\" y=\"1e999\" z=\"-0\"/>",
    "<triangles>",
    "</triangles>",
    "<triangle v1=\"0\" v2=\"1\" v3=\"2\"/>",
    "<triangle v1=\"4294967295\" v2=\"0\" v3=\"0\"/>",
    "<components>",
    "</components>",
    "<component objectid=\"1\"/>",
    "<component objectid=\"2\" transform=\"1 0 0\"/>",
    "<component objectid=\"1\" p:path=\"/3D/3dmodel.model\"/>",
    "<build>",
    "</build>",
    "<item objectid=\"1\"/>",
    "<item objectid=\"2\" transform=\"0 0 0 0 0 0 0 0 0 1e308 1e308 1e308\"/>",
    "<!--",
    "-->",
    "&amp;",
    "&#0;",
    "<![CDATA[",
    "]]>",
    "<",
    ">",
    "\"",
    "unit=\"inch\"",
];

fn config() -> ProptestConfig {
    // Fixed seed: deterministic runs (the test rules); 256 cases each.
    ProptestConfig {
        cases: 256,
        rng_seed: RngSeed::Fixed(0x0051_b3d0),
        ..ProptestConfig::default()
    }
}

fn check_no_panic(r: Result<ImportedMesh, ImportError>) {
    match r {
        Ok(m) => assert_well_formed(&m),
        Err(e) => assert!(!e.to_string().trim().is_empty(), "error without text"),
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn ag_sc_23_arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        check_no_panic(read_stl(&bytes));
        check_no_panic(read_3mf(&bytes));
    }

    #[test]
    fn ag_sc_23_truncated_or_flipped_fixture_never_panics(
        cut in 0..FIXTURE.len(),
        flips in proptest::collection::vec((0..FIXTURE.len(), any::<u8>()), 0..8),
    ) {
        check_no_panic(read_3mf(&FIXTURE[..cut]));
        let mut bytes = FIXTURE.to_vec();
        for (i, b) in flips {
            bytes[i] = b;
        }
        check_no_panic(read_3mf(&bytes));
    }

    #[test]
    fn ag_sc_23_binary_stl_with_any_count_and_length_never_panics(
        count in any::<u32>(),
        body in proptest::collection::vec(any::<u8>(), 0..1200),
    ) {
        let mut bytes = vec![0u8; 80];
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&body);
        check_no_panic(read_stl(&bytes));
    }

    #[test]
    fn ag_sc_23_ascii_stl_token_soup_never_panics(
        tokens in proptest::collection::vec(0..STL_TOKENS.len(), 0..300),
    ) {
        let text: Vec<&str> = tokens.iter().map(|&i| STL_TOKENS[i]).collect();
        check_no_panic(read_stl(text.join(" ").as_bytes()));
    }

    #[test]
    fn ag_sc_23_model_xml_token_soup_never_panics(
        tokens in proptest::collection::vec(0..XML_TOKENS.len(), 0..200),
    ) {
        let text: String = tokens.iter().map(|&i| XML_TOKENS[i]).collect();
        check_no_panic(read_3mf(&package(&text, &[])));
    }
}
