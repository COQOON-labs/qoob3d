//! Tests of the mesh transfer format (WA-QM-01..03, docs/requirements/project-app.md §3).
//!
//! Expected bytes are assembled here field by field from the layout table of
//! WA-QM-01 (independent of the writer): magic `QMSH`, u32 version 1, u32 V,
//! T, E, P, then positions f32×3×V, normals f32×3×V, indices u32×3×T, edge
//! points f32×3×E, polyline starts u32×(P+1). All little endian.

use super::{read, write};
use crate::mesh::{Polylines, ShapeMesh, TriMesh};

/// The fields of a qmesh file, freely editable to build invalid files.
#[derive(Clone)]
struct Raw {
    magic: [u8; 4],
    version: u32,
    /// Declared counts V, T, E, P (may lie about the data that follows).
    counts: [u32; 4],
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    edge_points: Vec<[f32; 3]>,
    starts: Vec<u32>,
}

impl Raw {
    /// A consistent file for `mesh` (counts derived from the data).
    fn of(mesh: &ShapeMesh) -> Self {
        Raw {
            magic: *b"QMSH",
            version: 1,
            counts: [
                mesh.faces.positions.len() as u32,
                (mesh.faces.indices.len() / 3) as u32,
                mesh.edges.points.len() as u32,
                (mesh.edges.starts.len() - 1) as u32,
            ],
            positions: mesh.faces.positions.clone(),
            normals: mesh.faces.normals.clone(),
            indices: mesh.faces.indices.clone(),
            edge_points: mesh.edges.points.clone(),
            starts: mesh.edges.starts.clone(),
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = self.magic.to_vec();
        for v in std::iter::once(self.version).chain(self.counts) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let floats = |out: &mut Vec<u8>, v: &[[f32; 3]]| {
            for c in v.iter().flatten() {
                out.extend_from_slice(&c.to_le_bytes());
            }
        };
        floats(&mut out, &self.positions);
        floats(&mut out, &self.normals);
        for i in &self.indices {
            out.extend_from_slice(&i.to_le_bytes());
        }
        floats(&mut out, &self.edge_points);
        for s in &self.starts {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }
}

/// One triangle with one 3-point polyline along two of its sides.
fn tiny() -> ShapeMesh {
    ShapeMesh {
        faces: TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            face_ids: vec![7], // not transferred (WA-QM-02)
        },
        edges: Polylines {
            points: vec![[0.0, 2.0, 0.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            starts: vec![0, 3],
        },
        tints: Vec::new(),
    }
}

/// Two triangles (a quad) and two polylines, with floats that only survive
/// a bit-exact encoding: -0.0, a subnormal, 1/3, f32::MAX, a NaN payload.
fn awkward() -> ShapeMesh {
    let third = 1.0f32 / 3.0;
    let nan = f32::from_bits(0x7fc0_1234);
    let sub = f32::from_bits(1);
    ShapeMesh {
        faces: TriMesh {
            positions: vec![
                [-0.0, sub, third],
                [f32::MAX, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, -third, 1e-30],
            ],
            normals: vec![[0.0, 0.0, 1.0], [0.6, 0.8, 0.0], [nan, 0.0, -0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            face_ids: Vec::new(),
        },
        edges: Polylines {
            points: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [third, 0.5, 0.25], [-1.0, -2.0, -3.0]],
            starts: vec![0, 2, 4],
        },
        tints: Vec::new(),
    }
}

fn bits(v: &[[f32; 3]]) -> Vec<[u32; 3]> {
    v.iter().map(|p| p.map(f32::to_bits)).collect()
}

/// Bit-exact comparison (PartialEq on f32 fails for NaN and ignores -0.0).
fn assert_same(a: &ShapeMesh, b: &ShapeMesh) {
    assert_eq!(bits(&a.faces.positions), bits(&b.faces.positions), "positions");
    assert_eq!(bits(&a.faces.normals), bits(&b.faces.normals), "normals");
    assert_eq!(a.faces.indices, b.faces.indices, "indices");
    assert_eq!(bits(&a.edges.points), bits(&b.edges.points), "edge points");
    assert_eq!(a.edges.starts, b.edges.starts, "starts");
}

// ------------------------------------------------------------------ WA-QM-01 layout

/// WA-QM-01: exact bytes of a tiny mesh. Length by hand: 4 (magic) + 5·4
/// (version, V, T, E, P) + 3·12 positions + 3·12 normals + 3·4 indices +
/// 3·12 edge points + 2·4 starts = 24 + 36 + 36 + 12 + 36 + 8 = 152.
#[test]
fn layout_of_a_tiny_mesh_is_exact() {
    let mesh = tiny();
    let bytes = write(&mesh);
    assert_eq!(bytes.len(), 152);
    assert_eq!(&bytes[..4], b"QMSH");
    assert_eq!(&bytes[4..24], &[1, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0]);
    // Second position (1,0,0) starts after the 24-byte header + 12 bytes.
    assert_eq!(&bytes[36..40], &1.0f32.to_le_bytes());
    // Polyline starts are the last 8 bytes: 0 and the sentinel E = 3.
    assert_eq!(&bytes[144..], &[0, 0, 0, 0, 3, 0, 0, 0]);
    assert_eq!(bytes, Raw::of(&mesh).bytes(), "field order or encoding differs");
}

/// WA-QM-01: the empty mesh is header + the single sentinel start 0 (28 bytes).
#[test]
fn layout_of_the_empty_mesh() {
    let bytes = write(&ShapeMesh::default());
    let mut expected = b"QMSH".to_vec();
    expected.extend_from_slice(&[1, 0, 0, 0]);
    expected.extend_from_slice(&[0; 16]); // V, T, E, P
    expected.extend_from_slice(&[0; 4]); // starts = [0]
    assert_eq!(bytes, expected);
    let back = read(&bytes).expect("empty mesh is valid");
    assert_eq!(back, ShapeMesh::default());
}

/// WA-QM-01: a hand-assembled file reads into exactly that mesh.
#[test]
fn hand_written_file_reads_back() {
    let mesh = awkward();
    let back = read(&Raw::of(&mesh).bytes()).expect("valid file");
    assert_same(&back, &mesh);
}

// ------------------------------------------------------------------ WA-QM-02 round trip

/// WA-QM-02: write → read is bit-identical, also for -0.0, subnormals,
/// f32::MAX and NaN payloads.
#[test]
fn round_trip_is_bit_exact() {
    let mesh = awkward();
    assert_same(&read(&write(&mesh)).expect("own output is valid"), &mesh);
    let mesh = tiny();
    assert_same(&read(&write(&mesh)).expect("own output is valid"), &mesh);
}

/// WA-QM-02: `face_ids` are not transferred and come back empty.
#[test]
fn face_ids_are_not_transferred() {
    let mesh = tiny();
    assert_eq!(mesh.faces.face_ids, [7]);
    let back = read(&write(&mesh)).expect("valid");
    assert!(back.faces.face_ids.is_empty(), "face_ids: {:?}", back.faces.face_ids);
    let mut expected = mesh;
    expected.faces.face_ids.clear();
    assert_eq!(back, expected);
}

/// Indexed means compact: positions are stored once per shared vertex. A
/// 10 mm cube with 8 shared corners, 12 triangles and 12 two-point edge
/// polylines takes 24 + 8·(12 + 12) + 12·12 + 24·12 + 13·4 = 700 bytes.
#[test]
fn cube_size_follows_the_layout() {
    let p = |x: f32, y: f32, z: f32| [x * 10.0, y * 10.0, z * 10.0];
    let positions = vec![
        p(0., 0., 0.),
        p(1., 0., 0.),
        p(1., 1., 0.),
        p(0., 1., 0.),
        p(0., 0., 1.),
        p(1., 0., 1.),
        p(1., 1., 1.),
        p(0., 1., 1.),
    ];
    let indices = vec![
        0, 3, 2, 0, 2, 1, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3,
        1, 2, 6, 1, 6, 5,
    ];
    let pairs = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4)];
    let pairs = pairs.into_iter().chain([(0, 4), (1, 5), (2, 6), (3, 7)]);
    let points: Vec<[f32; 3]> = pairs.flat_map(|(a, b)| [positions[a], positions[b]]).collect();
    let mesh = ShapeMesh {
        faces: TriMesh {
            normals: vec![[0.0, 0.0, 1.0]; 8],
            positions,
            indices,
            face_ids: Vec::new(),
        },
        edges: Polylines { starts: (0..=12).map(|i| i * 2).collect(), points },
        tints: Vec::new(),
    };
    let bytes = write(&mesh);
    assert_eq!(bytes.len(), 700);
    assert_eq!(read(&bytes).expect("valid"), mesh);
}

// ------------------------------------------------------------------ WA-QM-03 rejection

fn valid() -> Raw {
    Raw::of(&awkward())
}

/// WA-QM-03: wrong magic.
#[test]
fn wrong_magic_is_rejected() {
    for magic in [*b"QMSh", *b"STLB", [0; 4], *b"HSMQ"] {
        let raw = Raw { magic, ..valid() };
        assert!(read(&raw.bytes()).is_err(), "magic {magic:?} accepted");
    }
}

/// WA-QM-03: unknown versions (only 1 exists).
#[test]
fn unknown_version_is_rejected() {
    for version in [0, 2, u32::MAX] {
        let raw = Raw { version, ..valid() };
        assert!(read(&raw.bytes()).is_err(), "version {version} accepted");
    }
}

/// WA-QM-03: every proper prefix of a valid file is truncated → error
/// (covers a cut header, cut in each section and a missing last byte).
#[test]
fn every_truncation_is_rejected() {
    let bytes = valid().bytes();
    assert!(read(&bytes).is_ok());
    for len in 0..bytes.len() {
        assert!(read(&bytes[..len]).is_err(), "prefix of {len} of {} bytes accepted", bytes.len());
    }
}

/// WA-QM-03: an index equal to or beyond V (V = 4 here).
#[test]
fn index_out_of_range_is_rejected() {
    for bad in [4, 5, u32::MAX] {
        let mut raw = valid();
        raw.indices[4] = bad;
        assert!(read(&raw.bytes()).is_err(), "index {bad} accepted with V = 4");
    }
    // Boundary: V - 1 is fine.
    let mut raw = valid();
    raw.indices[4] = 3;
    assert!(read(&raw.bytes()).is_ok());
}

/// WA-QM-03: polyline starts that decrease.
#[test]
fn non_monotonic_starts_are_rejected() {
    let mut raw = valid();
    raw.counts[3] = 3;
    raw.starts = vec![0, 3, 1, 4]; // ends at E = 4, but 3 > 1
    assert!(read(&raw.bytes()).is_err());
    let mut raw = valid();
    raw.counts[3] = 3;
    raw.starts = vec![0, 9, 2, 4]; // beyond E in the middle
    assert!(read(&raw.bytes()).is_err());
}

/// WA-QM-03: the sentinel must be E.
#[test]
fn starts_not_ending_at_e_are_rejected() {
    for last in [3, 5, u32::MAX] {
        let mut raw = valid();
        raw.starts = vec![0, 2, last];
        assert!(read(&raw.bytes()).is_err(), "sentinel {last} accepted with E = 4");
    }
}

/// WA-QM-03: declared counts whose data would exceed the input are
/// rejected from the length check alone. With u32::MAX as V, a reader that
/// allocates by the declared count would ask for ~100 GB and abort the test
/// process instead of returning an error.
#[test]
fn absurd_counts_are_rejected_without_allocating() {
    for k in 0..4 {
        let mut raw = valid();
        raw.counts[k] = u32::MAX;
        assert!(read(&raw.bytes()).is_err(), "count {k} = u32::MAX accepted");
        // Only the header, no data at all.
        let mut header = raw.bytes();
        header.truncate(24);
        assert!(read(&header).is_err());
    }
    // All counts maximal: their byte sizes summed must not overflow into
    // something small.
    let raw = Raw { counts: [u32::MAX; 4], ..valid() };
    assert!(read(&raw.bytes()).is_err());
}

/// WA-QM-03: a declared E smaller than the data shifts the following
/// fields; the starts then read float bits and cannot end at E = 3.
#[test]
fn counts_that_do_not_match_the_data_are_rejected() {
    let mut raw = valid();
    raw.counts[2] = 3; // E = 3, but the starts end at 4
    assert!(read(&raw.bytes()).is_err());
}

/// The error explains itself.
#[test]
fn error_has_a_message() {
    let err = read(b"nope").expect_err("too short");
    assert!(!err.to_string().is_empty());
}

/// WA-QM-01: the writer knows the full size from the layout table before it
/// writes, so it allocates once and exactly that size: no reallocation while
/// writing, no spare capacity handed on. `Vec::with_capacity(n)` is
/// documented to give exactly capacity `n`, so `capacity == len` is a
/// deterministic check. Sizes: tiny 152, empty 28 (see the layout tests),
/// awkward 24 + 4·24 + 6·4 + 4·12 + 3·4 = 204.
#[test]
fn write_allocates_exactly_the_layout_size() {
    for (mesh, len) in [(tiny(), 152), (ShapeMesh::default(), 28), (awkward(), 204)] {
        let bytes = write(&mesh);
        assert_eq!(bytes.len(), len);
        assert_eq!(bytes.capacity(), len, "write must allocate exactly the layout size");
    }
}
