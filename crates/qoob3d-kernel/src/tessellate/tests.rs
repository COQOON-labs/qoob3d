use crate::script::{RunOptions, Shape, run};

/// The shape a one-line script shows as "s".
fn shape(expr: &str) -> Shape {
    let out = run(&format!("show({expr}, \"s\");"), &RunOptions::default()).unwrap();
    out.shapes.into_iter().next().unwrap().1
}

/// Cube with edge `size`, centred in x/y, from z = 0 to `size`.
fn cube(size: f64) -> Shape {
    shape(&format!("box({size}, {size}, {size})"))
}

#[test]
fn cube_is_watertight_with_six_faces_and_twelve_edges() {
    let mesh = cube(20.0).mesh(0.05).unwrap();
    assert_eq!(mesh.faces.triangle_count(), 12);

    // Slicer-readiness (ADR-0002): closed, manifold, outward winding.
    let report = mesh.faces.analyze();
    assert!(report.is_watertight(), "{report:?}");
    assert!((report.volume_mm3 - 8000.0).abs() < 1e-3, "{report:?}");

    let mut faces = mesh.faces.face_ids.clone();
    faces.sort_unstable();
    faces.dedup();
    assert_eq!(faces, [0, 1, 2, 3, 4, 5]);

    assert_eq!(mesh.edges.len(), 12);
    assert!(mesh.edges.iter().all(|line| line.len() == 2));
}

#[test]
fn normals_point_outwards_and_agree_with_winding() {
    let mesh = cube(10.0).mesh(0.05).unwrap().faces;
    let c = [0.0, 0.0, 5.0f32];
    for tri in mesh.indices.as_chunks::<3>().0 {
        let [a, b, d] = tri.map(|i| mesh.positions[i as usize]);
        let n = mesh.normals[tri[0] as usize];
        assert!(dot(n, sub(a, c)) > 0.0, "normal {n:?} points inward");
        assert!(dot(cross(sub(b, a), sub(d, a)), n) > 0.0, "winding disagrees with {n:?}");
    }
}

#[test]
fn cylinder_seam_is_not_an_edge() {
    let mesh = shape("cylinder(10, 10)").mesh(0.01).unwrap();
    // Top and bottom circle only – the seam line must not be drawn.
    assert_eq!(mesh.edges.len(), 2, "{:?}", mesh.edges.starts);
    let report = mesh.faces.analyze();
    assert!(report.is_watertight(), "{report:?}");
    let exact = std::f64::consts::PI * 25.0 * 10.0;
    assert!((report.volume_mm3 - exact).abs() / exact < 0.01, "{report:?}");
}

#[test]
fn block_with_hole_is_watertight_with_correct_volume() {
    let part = shape("cut(box(40, 30, 15), move(cylinder(14, 17), 0, 0, -1))");
    let mesh = part.mesh(0.01).unwrap();
    let report = mesh.faces.analyze();
    assert!(report.is_watertight(), "{report:?}");
    let exact = 40.0 * 30.0 * 15.0 - std::f64::consts::PI * 49.0 * 15.0;
    assert!((report.volume_mm3 - exact).abs() / exact < 0.005, "{report:?}");
    // 12 block edges + 2 hole circles.
    assert_eq!(mesh.edges.len(), 14);
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
