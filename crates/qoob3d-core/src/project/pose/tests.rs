use super::*;

/// A tetrahedron with its apex up: base z = 2, apex (1, 1, 7).
fn tetra() -> TriMesh {
    TriMesh {
        positions: vec![[0.0, 0.0, 2.0], [4.0, 0.0, 2.0], [0.0, 4.0, 2.0], [1.0, 1.0, 7.0]],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        indices: vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 2, 0, 3],
        face_ids: vec![0, 1, 2, 3],
    }
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5)
}

#[test]
fn the_default_pose_stands_the_part_on_the_bed_centred() {
    let posed = PrintPose::default().apply(&tetra());
    let b = posed.bounds().expect("bounds");
    // x, y ∈ [0, 4] → [−2, 2]; z ∈ [2, 7] → [0, 5].
    assert!(close(b.min, [-2.0, -2.0, 0.0]) && close(b.max, [2.0, 2.0, 5.0]), "{b:?}");
    assert_eq!(posed.indices, tetra().indices, "the winding is kept");
}

#[test]
fn upside_down_puts_the_apex_on_the_bed() {
    // 180° about x: (x, y, z) → (x, −y, −z); the apex (1, 1, 7) is lowest.
    let pose = PrintPose { rotate: vec![([1.0, 0.0, 0.0], 180.0)], count: 1 };
    let posed = pose.apply(&tetra());
    let b = posed.bounds().expect("bounds");
    assert!((b.max[2] - b.min[2] - 5.0).abs() < 1e-5 && b.min[2].abs() < 1e-5, "{b:?}");
    // Apex: x = 1 − 2 (centre of [0, 4]) = −1, y = −1 + 2 = 1 (y ∈ [−4, 0]), z = 0.
    assert!(close(posed.positions[3], [-1.0, 1.0, 0.0]), "{:?}", posed.positions[3]);
    // Normals turn with the part: +Z becomes −Z.
    assert!(close(posed.normals[0], [0.0, 0.0, -1.0]), "{:?}", posed.normals[0]);
    // The volume (orientation) survives a proper rotation.
    let before = tetra().analyze().volume_mm3;
    assert!((posed.analyze().volume_mm3 - before).abs() < 1e-4, "{before}");
}

#[test]
fn rotations_apply_in_order() {
    // 90° about z then 90° about x maps the x axis to y, then to z.
    let pose =
        PrintPose { rotate: vec![([0.0, 0.0, 1.0], 90.0), ([1.0, 0.0, 0.0], 90.0)], count: 1 };
    let line = TriMesh {
        positions: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 0.0, 0.0]],
        ..TriMesh::default()
    };
    let posed = pose.apply(&line);
    let b = posed.bounds().expect("bounds");
    assert!(close(b.min, [0.0, 0.0, 0.0]) && close(b.max, [0.0, 0.0, 10.0]), "{b:?}");
}

#[test]
fn arrange_fills_rows_up_to_the_bed_width() {
    // Bed 100 wide, gap 5: 40 + 5 + 40 = 85 fits, a third 40 starts a new
    // row behind the deeper of the two (30) plus the gap.
    let centres = arrange(&[[40.0, 20.0], [40.0, 30.0], [40.0, 10.0], [120.0, 5.0]], 100.0);
    assert_eq!(centres[0], [20.0, 10.0]);
    assert_eq!(centres[1], [65.0, 15.0]);
    assert_eq!(centres[2], [20.0, 40.0]);
    // Too wide for the bed: alone in its row.
    assert_eq!(centres[3], [60.0, 52.5]);
}

#[test]
fn meshes_of_one_object_are_posed_together() {
    // A second tetrahedron 10 mm above the first: upside down it is below,
    // so it stands on the bed and the first one hangs 10 mm higher.
    let mut upper = tetra();
    for p in &mut upper.positions {
        p[2] += 10.0;
    }
    let pose = PrintPose { rotate: vec![([1.0, 0.0, 0.0], 180.0)], count: 1 };
    let posed = pose.apply_all(&[&tetra(), &upper]);
    let (lower_box, upper_box) = (posed[0].bounds().expect("box"), posed[1].bounds().expect("box"));
    assert!((upper_box.min[2]).abs() < 1e-5, "the lowest mesh stands on the bed: {upper_box:?}");
    assert!((lower_box.min[2] - 10.0).abs() < 1e-5, "the offset is kept: {lower_box:?}");
    assert!((lower_box.min[0] - upper_box.min[0]).abs() < 1e-5, "same shift in x");
}
