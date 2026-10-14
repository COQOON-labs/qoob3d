//! Solid features end to end in the kernel (ADR-0031): every reference
//! model builds, is a valid closed solid and has the analytic volume.

use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Duration;

use qoob3d_core::document::{Document, examples};
use qoob3d_core::generate;
use qoob3d_kernel::{Kernel, RebuildResult};

fn build(doc: Document) -> RebuildResult {
    let mut kernel = Kernel::spawn(|| {});
    let result = kernel.rebuild_blocking(Arc::new(doc), Duration::from_secs(120)).unwrap();
    assert!(result.errors.is_empty(), "feature errors: {:?}", result.errors);
    for body in &result.bodies {
        assert!(body.valid, "body {} is not a valid solid", body.number);
        let report = body.mesh.faces.analyze();
        assert!(report.is_watertight(), "body {}: {report:?}", body.number);
        // The view mesh (0.05 mm, 0.35 rad) is close to the exact volume;
        // thin wires (coil, Ø2) lose a few percent to chords.
        let rel = (report.volume_mm3 - body.volume_mm3).abs() / body.volume_mm3;
        assert!(rel < 0.05, "mesh {} vs exact {}", report.volume_mm3, body.volume_mm3);
    }
    result
}

fn assert_volume(result: &RebuildResult, expected: f64, rel_tol: f64) {
    let total: f64 = result.bodies.iter().map(|b| b.volume_mm3).sum();
    let rel = (total - expected).abs() / expected;
    assert!(rel < rel_tol, "volume {total:.3} mm³, expected {expected:.3} (±{rel_tol})");
}

#[test]
fn cuboid_from_rectangle_and_extrude() {
    let r = build(examples::cuboid(40.0, 25.0, 10.0));
    assert_eq!(r.bodies.len(), 1);
    assert_volume(&r, 10_000.0, 1e-9);
    let b = r.bodies[0].mesh.faces.bounds().unwrap();
    assert_eq!((b.min, b.max), ([0.0, 0.0, 0.0], [40.0, 25.0, 10.0]));
}

#[test]
fn cylinder_from_circle_and_extrude() {
    let r = build(examples::cylinder(10.0, 30.0));
    assert_volume(&r, PI * 100.0 * 30.0, 1e-9);
}

#[test]
fn sphere_from_half_disc_and_revolve() {
    let r = build(examples::sphere(15.0));
    assert_eq!(r.bodies.len(), 1);
    assert_volume(&r, 4.0 / 3.0 * PI * 15f64.powi(3), 1e-6);
}

#[test]
fn pyramid_from_square_lofted_to_a_point() {
    let r = build(examples::pyramid(30.0, 25.0));
    assert_volume(&r, 30.0 * 30.0 * 25.0 / 3.0, 1e-6);
    let b = r.bodies[0].mesh.faces.bounds().unwrap();
    assert!((b.max[2] - 25.0).abs() < 1e-4, "apex height {b:?}");
}

#[test]
fn spur_gear_matches_its_outline() {
    let doc = examples::gear();
    let qoob3d_core::document::FeatureKind::SpurGear(g) = &doc.features[0].kind else {
        unreachable!()
    };
    let outline = generate::gear_outline(g).area();
    let expected = (outline - PI * 16.0) * g.thickness;
    let r = build(doc.clone());
    // Splines vs. dense polygon: well below 1 %.
    assert_volume(&r, expected, 0.01);
    let b = r.bodies[0].mesh.faces.bounds().unwrap();
    let tip = generate::gear_tip_diameter(g) as f32;
    assert!((b.max[0] - b.min[0] - tip).abs() < 0.05, "tip diameter {b:?}");
}

#[test]
fn coil_and_spiral_are_swept_wires() {
    for kind in [qoob3d_core::document::CoilKind::Helix, qoob3d_core::document::CoilKind::Spiral] {
        let doc = examples::coil(kind);
        let qoob3d_core::document::FeatureKind::Coil(c) = &doc.features[0].kind else {
            unreachable!()
        };
        let expected = PI * (0.5 * c.section).powi(2) * generate::coil_length(c);
        let r = build(doc.clone());
        assert_eq!(r.bodies.len(), 1, "{kind:?}");
        assert_volume(&r, expected, 0.01);
    }
}

#[test]
fn join_and_cut() {
    let r = build(examples::plate_with_hole());
    assert_eq!(r.bodies.len(), 1, "the boss is joined into the plate");
    let expected = 60.0 * 40.0 * 5.0 + PI * 25.0 * 8.0 - PI * 9.0 * 13.0;
    assert_volume(&r, expected, 1e-6);
}

#[test]
fn missing_profile_is_a_feature_error_not_a_guess() {
    let mut doc = examples::cuboid(10.0, 10.0, 10.0);
    // Delete one rectangle side: the profile no longer exists (ADR-0011).
    let sketch = doc.features[0].id;
    let s = doc.sketch_mut(sketch).unwrap();
    let side = s.curves[0].id;
    s.delete(&[side]);
    let mut kernel = Kernel::spawn(|| {});
    let r = kernel.rebuild_blocking(Arc::new(doc), Duration::from_secs(60)).unwrap();
    assert!(r.bodies.is_empty());
    assert_eq!(r.errors.len(), 1);
    assert!(r.errors[0].1.contains("no longer exists"), "{:?}", r.errors);
}

#[test]
fn unchanged_features_come_from_the_cache() {
    let mut kernel = Kernel::spawn(|| {});
    let doc = Arc::new(examples::gear());
    let t = std::time::Instant::now();
    kernel.rebuild_blocking(doc.clone(), Duration::from_secs(60)).unwrap();
    let first = t.elapsed();
    let t = std::time::Instant::now();
    kernel.rebuild_blocking(doc, Duration::from_secs(60)).unwrap();
    let second = t.elapsed();
    assert!(second < first / 2 || second < Duration::from_millis(5), "{first:?} vs {second:?}");
}

#[test]
fn export_meshes_are_finer_and_watertight() {
    let mut kernel = Kernel::spawn(|| {});
    let doc = Arc::new(examples::sphere(15.0));
    let view = kernel.rebuild_blocking(doc.clone(), Duration::from_secs(60)).unwrap();
    let export = kernel
        .export_blocking(
            doc,
            qoob3d_kernel::EXPORT_TESSELLATION_TOLERANCE_MM,
            Duration::from_secs(60),
        )
        .unwrap();
    assert_eq!(export.len(), 1);
    let fine = export[0].mesh.analyze();
    assert!(fine.is_watertight());
    assert!(fine.triangles > view.bodies[0].mesh.faces.analyze().triangles);
    let exact = 4.0 / 3.0 * PI * 15f64.powi(3);
    assert!((fine.volume_mm3 - exact).abs() / exact < 2e-3, "{fine:?}");
}

#[test]
fn export_of_thin_wires_is_accurate() {
    let mut kernel = Kernel::spawn(|| {});
    let doc = examples::coil(qoob3d_core::document::CoilKind::Helix);
    let exact = kernel.rebuild_blocking(Arc::new(doc.clone()), Duration::from_secs(60)).unwrap();
    let export = kernel
        .export_blocking(
            Arc::new(doc),
            qoob3d_kernel::EXPORT_TESSELLATION_TOLERANCE_MM,
            Duration::from_secs(60),
        )
        .unwrap();
    let fine = export[0].mesh.analyze();
    assert!(fine.is_watertight(), "{fine:?}");
    let rel = (fine.volume_mm3 - exact.bodies[0].volume_mm3).abs() / exact.bodies[0].volume_mm3;
    assert!(rel < 0.01, "{rel}");
}

#[test]
fn format_v2_fixture_builds_without_errors() {
    let text = include_str!("../../qoob3d-core/tests/fixtures/format_v2.qbd");
    let doc = Document::from_json(text).unwrap();
    let r = build(doc);
    assert_eq!(r.bodies.len(), 4, "pyramid (with boss), sphere, gear, spiral");
}
