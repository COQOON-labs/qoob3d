use super::*;
use occt_glam::{DVec3, dvec3};

fn cube(size: f64) -> Shape {
    Shape::box_from_corners(DVec3::ZERO, dvec3(size, size, size))
}

#[test]
fn cube_is_watertight_with_six_faces_and_twelve_edges() {
    let mesh = tessellate(&cube(20.0), 0.05).unwrap();
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
    let mesh = tessellate(&cube(10.0), 0.05).unwrap().faces;
    let c = [5.0f32; 3];
    for tri in mesh.indices.as_chunks::<3>().0 {
        let [a, b, d] = tri.map(|i| mesh.positions[i as usize]);
        let n = mesh.normals[tri[0] as usize];
        assert!(dot(n, sub(a, c)) > 0.0, "normal {n:?} points inward");
        assert!(dot(cross(sub(b, a), sub(d, a)), n) > 0.0, "winding disagrees with {n:?}");
    }
}

#[test]
fn cylinder_seam_is_not_an_edge() {
    let cylinder = Shape::cylinder(DVec3::ZERO, 5.0, DVec3::Z, 10.0);
    let mesh = tessellate(&cylinder, 0.01).unwrap();
    // Top and bottom circle only – the seam line must not be drawn.
    assert_eq!(mesh.edges.len(), 2, "{:?}", mesh.edges.starts);
    let report = mesh.faces.analyze();
    assert!(report.is_watertight(), "{report:?}");
    let exact = std::f64::consts::PI * 25.0 * 10.0;
    assert!((report.volume_mm3 - exact).abs() / exact < 0.01, "{report:?}");
}

#[test]
fn demo_part_has_hole_and_correct_volume() {
    let mesh = tessellate(&demo_part(), 0.01).unwrap();
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
