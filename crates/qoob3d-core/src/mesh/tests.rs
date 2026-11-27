use super::*;

#[test]
fn merging_offsets_indices_faces_and_edges() {
    let mut a = ShapeMesh::default();
    a.faces.positions = vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    a.faces.normals = vec![[0.0, 0.0, 1.0]; 3];
    a.faces.indices = vec![0, 1, 2];
    a.faces.face_ids = vec![0];
    a.edges = Polylines { points: vec![[0.0; 3], [1.0, 0.0, 0.0]], starts: vec![0, 2] };
    let m = ShapeMesh::merge([&a, &a]);
    assert_eq!(m.faces.indices, [0, 1, 2, 3, 4, 5]);
    assert_eq!(m.faces.face_ids, [0, 1]);
    assert_eq!(m.edges.starts, [0, 2, 4]);
    assert_eq!(m.edges.len(), 2);
    assert_eq!(ShapeMesh::merge([]).edges.starts, [0]);
}

#[test]
fn bounds_of_empty_mesh_is_none() {
    assert_eq!(TriMesh::default().bounds(), None);
}

#[test]
fn bounds_cover_all_positions() {
    let mesh =
        TriMesh { positions: vec![[0.0, -1.0, 2.0], [3.0, 4.0, -5.0]], ..Default::default() };
    let b = mesh.bounds().unwrap();
    assert_eq!(b.min, [0.0, -1.0, -5.0]);
    assert_eq!(b.max, [3.0, 4.0, 2.0]);
    assert_eq!(b.center(), [1.5, 1.5, -1.5]);
}

#[test]
fn polylines_iterate_lines_and_segments() {
    let lines = Polylines {
        points: vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [5.0; 3], [6.0; 3]],
        starts: vec![0, 3, 5],
    };
    assert_eq!(lines.len(), 2);
    assert_eq!(lines.iter().map(<[_]>::len).collect::<Vec<_>>(), [3, 2]);
    assert_eq!(lines.segments().count(), 3);
    assert!(Polylines::default().is_empty());
    assert_eq!(Polylines::default().segments().count(), 0);
}

/// Cube `[0, s]^3` built like kernel output: 4 separate vertices per face.
fn split_face_cube(s: f32) -> TriMesh {
    let faces: [[[f32; 3]; 4]; 6] = [
        [[0., 0., s], [s, 0., s], [s, s, s], [0., s, s]],
        [[0., 0., 0.], [0., s, 0.], [s, s, 0.], [s, 0., 0.]],
        [[s, 0., 0.], [s, s, 0.], [s, s, s], [s, 0., s]],
        [[0., 0., 0.], [0., 0., s], [0., s, s], [0., s, 0.]],
        [[0., s, 0.], [0., s, s], [s, s, s], [s, s, 0.]],
        [[0., 0., 0.], [s, 0., 0.], [s, 0., s], [0., 0., s]],
    ];
    let mut mesh = TriMesh::default();
    for quad in faces {
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(quad);
        mesh.normals.extend([[0.0; 3]; 4]);
        mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh
}

#[test]
fn closed_cube_is_watertight_with_exact_volume() {
    let r = split_face_cube(20.0).analyze();
    assert!(r.is_watertight(), "{r:?}");
    assert_eq!(r.triangles, 12);
    assert!((r.volume_mm3 - 8000.0).abs() < 1e-6, "{r:?}");
}

#[test]
fn missing_triangle_is_reported_as_open() {
    let mut mesh = split_face_cube(10.0);
    mesh.indices.truncate(mesh.indices.len() - 3);
    let r = mesh.analyze();
    assert!(!r.is_watertight());
    assert_eq!(r.open_edges, 3);
}

#[test]
fn flipped_triangle_is_reported_as_inconsistent() {
    let mut mesh = split_face_cube(10.0);
    mesh.indices.swap(0, 1);
    let r = mesh.analyze();
    assert!(!r.is_watertight());
    assert!(r.inconsistent_edges > 0, "{r:?}");
}

#[test]
fn inside_out_cube_has_negative_volume() {
    let mut mesh = split_face_cube(10.0);
    for tri in mesh.indices.as_chunks_mut::<3>().0 {
        tri.swap(1, 2);
    }
    let r = mesh.analyze();
    assert!(r.is_watertight(), "consistently inverted is still closed: {r:?}");
    assert!((r.volume_mm3 + 1000.0).abs() < 1e-6);
}

#[test]
fn raycast_finds_closest_face() {
    let mesh = split_face_cube(10.0);
    let hit = mesh.raycast([5.0, 5.0, 30.0], [0.0, 0.0, -1.0]).unwrap();
    assert!((hit.t - 20.0).abs() < 1e-5, "{hit:?}");
    assert!((hit.point[2] - 10.0).abs() < 1e-5);
    // From inside, the ray still reports the face it exits through.
    let inside = mesh.raycast([5.0, 5.0, 5.0], [1.0, 0.0, 0.0]).unwrap();
    assert!((inside.point[0] - 10.0).abs() < 1e-5);
    assert!(mesh.raycast([50.0, 5.0, 30.0], [0.0, 0.0, -1.0]).is_none());
    assert!(mesh.raycast([5.0, 5.0, 30.0], [0.0, 0.0, 1.0]).is_none(), "behind origin");
}

#[test]
fn empty_mesh_is_not_watertight() {
    assert!(!TriMesh::default().analyze().is_watertight());
}

proptest::proptest! {
    /// Translating and scaling never changes topology; volume scales with s³.
    #[test]
    fn analysis_is_invariant_under_translation_and_scale(
        s in 0.5f32..200.0,
        t in proptest::array::uniform3(-500.0f32..500.0),
    ) {
        let mut mesh = split_face_cube(s);
        for p in &mut mesh.positions {
            for k in 0..3 { p[k] += t[k]; }
        }
        let r = mesh.analyze();
        proptest::prop_assert!(r.is_watertight(), "{:?}", r);
        let expected = f64::from(s).powi(3);
        proptest::prop_assert!((r.volume_mm3 - expected).abs() / expected < 1e-3, "{:?}", r);
    }
}

/// The cube with one corner of its front face `offset` beside the same
/// corner of the neighbouring faces: what the kernel produced where two
/// faces sampled a shared edge slightly differently (a sliver).
fn cube_with_sliver(s: f32, offset: f32) -> TriMesh {
    let mut mesh = split_face_cube(s);
    // Front face (y = 0): [0,0,0] [s,0,0] [s,0,s] [0,0,s]; its corner (s, 0, s).
    mesh.positions[5 * 4 + 2][0] += offset;
    mesh
}

#[test]
fn a_sliver_along_a_shared_edge_is_closed_in_the_export() {
    // 0.7 µm beside the edge: the raw mesh is open (more than the weld
    // tolerance), the exported mesh must be closed (finding 1.5 of the
    // Damen 5009 project: one such sliver made a 3MF non-manifold).
    let mesh = cube_with_sliver(20.0, 0.0007);
    assert!(!mesh.analyze().is_watertight(), "{:?}", mesh.analyze());
    let r = mesh.export_report();
    assert!(r.is_watertight(), "{r:?}");
    assert!((r.volume_mm3 - 8000.0).abs() < 1e-2, "{r:?}");
}

#[test]
fn a_real_gap_stays_open_in_the_export() {
    // 0.1 mm is a hole, not a sliver.
    let r = cube_with_sliver(20.0, 0.1).export_report();
    assert!(!r.is_watertight(), "{r:?}");
}

#[test]
fn vertices_on_both_sides_of_a_weld_cell_border_are_welded() {
    // 0.5 · WELD_TOLERANCE rounds up, a hair less rounds down: the two
    // copies of one corner land in different cells of the quantised weld.
    let mut mesh = split_face_cube(10.0);
    let half = (WELD_TOLERANCE_MM / 2.0) as f32;
    for p in &mut mesh.positions {
        for c in p.iter_mut() {
            *c += half;
        }
    }
    // One copy of the corner (0, 0, 0) of the bottom face moves by 2 nm.
    let i = mesh.positions.iter().position(|p| p[0] < 1.0 && p[1] < 1.0 && p[2] < 1.0).unwrap();
    mesh.positions[i][0] -= 2e-6;
    let r = mesh.export_report();
    assert!(r.is_watertight(), "{r:?}");
}

#[test]
fn degenerate_triangles_the_export_drops_do_not_count_against_it() {
    // Finding 1.6: the build called parts "not watertight" whose exported
    // files were closed, because the analysis counted triangles the export
    // drops anyway.
    let mut mesh = split_face_cube(10.0);
    let base = mesh.positions.len() as u32;
    mesh.positions.extend([[0.0, 0.0, 10.0], [0.0, 0.0, 10.0], [10.0, 0.0, 10.0]]);
    mesh.normals.extend([[0.0; 3]; 3]);
    mesh.indices.extend([base, base + 1, base + 2]);
    assert!(!mesh.analyze().is_watertight());
    let r = mesh.export_report();
    assert!(r.is_watertight(), "{r:?}");
    assert_eq!(r.triangles, 12);
}
