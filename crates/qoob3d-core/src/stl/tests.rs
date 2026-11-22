//! Tests of the binary STL import (WA-LV-03, docs/requirements/project-app.md).
//!
//! Expected values are derived by hand from the geometry built in the test:
//! a cube has 8 corners and 12 edges at which its faces meet at 90°; the
//! two triangles of one square face are coplanar (0°), so the diagonals are
//! no feature edges. Feature edges are edges whose triangle normals differ by
//! more than 30°, plus open edges (used by one triangle only).

use std::collections::HashSet;

use super::read_binary;
use crate::mesh::ShapeMesh;

type Tri = [[f32; 3]; 3];

/// Binary STL: 80-byte header, u32 LE count, then
/// per triangle 12 × f32 LE (normal, 3 vertices) and a u16 attribute.
fn stl(header: &[u8], triangles: &[Tri], normals: Option<&[[f32; 3]]>, attribute: u16) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    out[..header.len()].copy_from_slice(header);
    out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
    for (i, t) in triangles.iter().enumerate() {
        let n = normals.map_or_else(|| normal(t), |ns| ns[i]);
        for c in n.iter().chain(t.iter().flatten()) {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&attribute.to_le_bytes());
    }
    out
}

fn normal(t: &Tri) -> [f32; 3] {
    let a = sub(t[1], t[0]);
    let b = sub(t[2], t[0]);
    let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    n.map(|c| c / len)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Quad (counter-clockwise seen from outside) as two triangles (a,b,c), (a,c,d).
fn quad(q: [[f32; 3]; 4]) -> [Tri; 2] {
    [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]
}

/// Axis-aligned cube from `min` with edge length `s`, 12 outward triangles.
/// Winding checked by hand: e.g. top (0,0,1)→(1,0,1)→(1,1,1) gives
/// (1,0,0)×(1,1,0) = (0,0,1), outward.
fn cube(min: [f32; 3], s: f32) -> Vec<Tri> {
    let p = |x: f32, y: f32, z: f32| [min[0] + x * s, min[1] + y * s, min[2] + z * s];
    let faces = [
        [p(0., 0., 0.), p(0., 1., 0.), p(1., 1., 0.), p(1., 0., 0.)], // -Z
        [p(0., 0., 1.), p(1., 0., 1.), p(1., 1., 1.), p(0., 1., 1.)], // +Z
        [p(0., 0., 0.), p(1., 0., 0.), p(1., 0., 1.), p(0., 0., 1.)], // -Y
        [p(0., 1., 0.), p(0., 1., 1.), p(1., 1., 1.), p(1., 1., 0.)], // +Y
        [p(0., 0., 0.), p(0., 0., 1.), p(0., 1., 1.), p(0., 1., 0.)], // -X
        [p(1., 0., 0.), p(1., 1., 0.), p(1., 1., 1.), p(1., 0., 1.)], // +X
    ];
    faces.into_iter().flat_map(quad).collect()
}

/// Closed right prism over a regular n-gon (radius 10, height 5): caps as
/// fans around the centre, each side a quad. Every corner position is
/// computed once and reused, so equal corners are bit-identical.
fn prism(n: usize) -> Vec<Tri> {
    let ring = |z: f32| -> Vec<[f32; 3]> {
        (0..n)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                [10.0 * a.cos(), 10.0 * a.sin(), z]
            })
            .collect()
    };
    let (lo, hi) = (ring(0.0), ring(5.0));
    let (c_lo, c_hi) = ([0.0, 0.0, 0.0], [0.0, 0.0, 5.0]);
    let mut out = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        out.push([c_hi, hi[i], hi[j]]);
        out.push([c_lo, lo[j], lo[i]]);
        out.extend(quad([lo[i], lo[j], hi[j], hi[i]]));
    }
    out
}

/// Feature edges as a set of unordered segments (bit patterns, so the set
/// is exact). Polylines may be chained or single segments; both count the same.
fn segments(mesh: &ShapeMesh) -> Vec<[[f32; 3]; 2]> {
    mesh.edges.segments().collect()
}

fn unique_segments(mesh: &ShapeMesh) -> HashSet<[[u32; 3]; 2]> {
    segments(mesh)
        .into_iter()
        .map(|[a, b]| {
            let (a, b) = (a.map(f32::to_bits), b.map(f32::to_bits));
            if a <= b { [a, b] } else { [b, a] }
        })
        .collect()
}

// ------------------------------------------------------------------ valid input

/// WA-LV-03: a 10 mm cube → 12 triangles, 8 distinct (welded) corners,
/// volume 10³ = 1000 mm³, closed.
#[test]
fn cube_reads_as_closed_mesh_with_eight_corners() {
    let bytes = stl(b"", &cube([5.0, -3.0, 2.0], 10.0), None, 0);
    let mesh = read_binary(&bytes).expect("valid STL");
    assert_eq!(mesh.faces.triangle_count(), 12);
    let corners: HashSet<[u32; 3]> =
        mesh.faces.positions.iter().map(|p| p.map(f32::to_bits)).collect();
    assert_eq!(corners.len(), 8, "a cube has 8 corners");
    let report = mesh.faces.analyze();
    assert!(report.is_watertight(), "{report:?}");
    // Integer coordinates are exact in f32, the volume sum is exact in f64;
    // 1e-6 only absorbs the summation order.
    assert!((report.volume_mm3 - 1000.0).abs() < 1e-6, "volume {}", report.volume_mm3);
    let bounds = mesh.faces.bounds().expect("not empty");
    assert_eq!(bounds.min, [5.0, -3.0, 2.0]);
    assert_eq!(bounds.max, [15.0, 7.0, 12.0]);
}

/// WA-LV-03: welding by exact position. Every STL triangle stores its own
/// corners; only if they are welded do the 12 triangles share edges. Without
/// welding all 36 triangle edges would be open and count as feature edges.
#[test]
fn cube_triangles_are_connected_through_welded_vertices() {
    let mesh = read_binary(&stl(b"", &cube([0.0; 3], 10.0), None, 0)).expect("valid STL");
    // Indexed mesh: every index points into positions.
    assert_eq!(mesh.faces.indices.len(), 36);
    let n = mesh.faces.positions.len() as u32;
    assert!(mesh.faces.indices.iter().all(|&i| i < n));
    assert_eq!(mesh.faces.normals.len(), mesh.faces.positions.len());
    assert_eq!(unique_segments(&mesh).len(), 12);
}

/// WA-LV-03: the 12 feature edges of a cube are exactly its 12 edges of
/// length 10 (two corners differing in one coordinate); no face diagonal.
#[test]
fn cube_has_twelve_feature_edges_along_its_edges() {
    let mesh = read_binary(&stl(b"", &cube([0.0; 3], 10.0), None, 0)).expect("valid STL");
    let segs = segments(&mesh);
    assert_eq!(segs.len(), 12, "segments: {segs:?}");
    assert_eq!(unique_segments(&mesh).len(), 12, "no segment twice");
    for [a, b] in &segs {
        let corner = |p: &[f32; 3]| p.iter().all(|&c| c == 0.0 || c == 10.0);
        assert!(corner(a) && corner(b), "not between corners: {a:?} {b:?}");
        let differing = (0..3).filter(|&i| a[i] != b[i]).count();
        assert_eq!(differing, 1, "diagonal is no feature edge: {a:?} {b:?}");
    }
}

/// WA-LV-03 threshold "more than 30°": an 11-gon prism has side normals
/// 360°/11 = 32.7° apart → the 11 vertical edges are features; plus 2 × 11
/// rim edges at 90° → 33. Cap fans and side diagonals are coplanar (0°).
#[test]
fn edges_steeper_than_thirty_degrees_are_features() {
    let mesh = read_binary(&stl(b"", &prism(11), None, 0)).expect("valid STL");
    assert_eq!(unique_segments(&mesh).len(), 33);
}

/// WA-LV-03: a 13-gon prism has side normals 360°/13 = 27.7° apart, below
/// 30° → only the 2 × 13 = 26 rim edges remain (a smooth cylinder shows no
/// tessellation lines).
#[test]
fn edges_flatter_than_thirty_degrees_are_not_features() {
    let mesh = read_binary(&stl(b"", &prism(13), None, 0)).expect("valid STL");
    assert_eq!(unique_segments(&mesh).len(), 26);
}

/// WA-LV-03 "and open edges": a single triangle has 3 open edges.
#[test]
fn open_edges_of_a_single_triangle_are_features() {
    let tri = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 3.0, 0.0]];
    let mesh = read_binary(&stl(b"", &[tri], None, 0)).expect("valid STL");
    assert_eq!(mesh.faces.triangle_count(), 1);
    assert_eq!(unique_segments(&mesh).len(), 3);
}

/// WA-LV-03: an open square of two coplanar triangles → 4 open border
/// edges are features, the shared diagonal (0°) is not.
#[test]
fn open_square_shows_its_border_but_not_its_diagonal() {
    let sq = quad([[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0], [0.0, 10.0, 0.0]]);
    let mesh = read_binary(&stl(b"", &sq, None, 0)).expect("valid STL");
    let segs = unique_segments(&mesh);
    assert_eq!(segs.len(), 4);
    let diagonal = [[0.0f32, 0.0, 0.0].map(f32::to_bits), [10.0f32, 10.0, 0.0].map(f32::to_bits)];
    assert!(!segs.contains(&diagonal), "diagonal drawn");
}

/// WA-LV-03 reads binary STL; a header that happens to start with "solid"
/// (allowed, written by many exporters) must still be read as binary.
/// The attribute word is ignored.
#[test]
fn header_starting_with_solid_and_attribute_bytes_are_ignored() {
    let bytes = stl(b"solid exported part", &cube([0.0; 3], 2.0), None, 0xBEEF);
    let mesh = read_binary(&bytes).expect("binary STL despite 'solid' header");
    assert_eq!(mesh.faces.triangle_count(), 12);
    assert_eq!(unique_segments(&mesh).len(), 12);
}

/// WA-LV-03: normals are computed from the winding. Writers often leave the
/// stored normal zero; the mesh must still carry unit normals pointing out
/// of the solid (counter-clockwise seen from outside).
/// Tolerance 1e-4: f32 normalisation error is ~1e-7, any wrong normal is off
/// by O(1).
#[test]
fn normals_are_unit_and_outward_even_if_the_file_stores_zero() {
    let tris = cube([0.0; 3], 10.0);
    let zeros = vec![[0.0f32; 3]; tris.len()];
    let mesh = read_binary(&stl(b"", &tris, Some(&zeros), 0)).expect("valid STL");
    let f = &mesh.faces;
    for t in f.indices.as_chunks::<3>().0 {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| f.positions[i as usize]);
        let centroid: [f32; 3] = std::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.0);
        for &i in t {
            let n = f.normals[i as usize];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "normal {n:?} not unit");
            // Outward: points away from the cube centre (5,5,5).
            let out: f32 = (0..3).map(|k| n[k] * (centroid[k] - 5.0)).sum();
            assert!(out > 0.0, "normal {n:?} points inward at {centroid:?}");
        }
    }
}

/// A well-formed file with zero triangles is not malformed: empty mesh.
#[test]
fn zero_triangles_give_an_empty_mesh() {
    let mesh = read_binary(&stl(b"", &[], None, 0)).expect("84 bytes, count 0 is valid");
    assert!(mesh.faces.is_empty());
    assert!(mesh.edges.is_empty());
}

// ------------------------------------------------------------------ malformed input

/// WA-LV-03: shorter than the declared count → error, never a panic.
/// 12 triangles declared, the last one cut in half (84 + 11·50 + 25 bytes).
#[test]
fn truncated_triangle_data_is_an_error() {
    let mut bytes = stl(b"", &cube([0.0; 3], 10.0), None, 0);
    assert_eq!(bytes.len(), 84 + 12 * 50);
    bytes.truncate(84 + 11 * 50 + 25);
    assert!(read_binary(&bytes).is_err());
    // Missing exactly the last attribute word (1 byte short) is still short.
    let mut bytes = stl(b"", &cube([0.0; 3], 10.0), None, 0);
    bytes.pop();
    assert!(read_binary(&bytes).is_err());
}

/// WA-LV-03: no room for header + count.
#[test]
fn input_shorter_than_header_and_count_is_an_error() {
    assert!(read_binary(&[]).is_err());
    assert!(read_binary(&[0u8; 80]).is_err());
    assert!(read_binary(&[0u8; 83]).is_err());
}

/// WA-LV-03 "never allocates for an absurd declared count": a huge declared
/// count with no data must be rejected from the
/// length check, not by trying to allocate 4·10⁹ triangles (which would
/// abort the process).
#[test]
fn huge_declared_count_is_an_error_without_allocating() {
    let mut bytes = vec![0u8; 80];
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 50]);
    assert!(read_binary(&bytes).is_err());
}

/// The error explains itself (shown to the user / on stderr).
#[test]
fn error_has_a_message() {
    let err = read_binary(&[0u8; 10]).expect_err("too short");
    assert!(!err.to_string().is_empty());
}

/// Never a panic: degenerate (zero-area, collinear) triangles and NaN
/// coordinates may be accepted or rejected, but must not crash the reader.
#[test]
fn degenerate_and_nan_triangles_do_not_panic() {
    let point = [[1.0, 1.0, 1.0]; 3];
    let line = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let nan = [[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let inf = [[f32::INFINITY, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let zeros = [[0.0f32; 3]; 4];
    let _ = read_binary(&stl(b"", &[point, line, nan, inf], Some(&zeros), 0));
    let mut tris = cube([0.0; 3], 1.0);
    tris.push(point);
    let _ = read_binary(&stl(b"", &tris, None, 0));
}
