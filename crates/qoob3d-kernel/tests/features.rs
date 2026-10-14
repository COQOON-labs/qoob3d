//! Solid features end to end in the kernel (ADR-0031): every reference
//! model builds, is a valid closed solid and has the analytic volume.

use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Duration;

use qoob3d_core::document::{
    BodyOp, Document, Extrude, FeatureId, FeatureKind, OriginPlane, PlaneRef, ProfileRef, Sketch,
    examples,
};
use qoob3d_core::generate;
use qoob3d_kernel::{Kernel, RebuildResult};

fn build(doc: Document) -> RebuildResult {
    let mut kernel = Kernel::spawn(|| {});
    let result =
        kernel.rebuild_blocking(Arc::new(doc), Duration::from_secs(120)).expect("kernel answers");
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

// ---------------------------------------------------------------------------
// Rebuild cache (ADR-0012): deterministic checks via `RebuildResult::stats`.
// No timing assertions – the counters say what the rebuild had to compute.
// ---------------------------------------------------------------------------

const TIMEOUT: Duration = Duration::from_secs(120);

/// (computed_features, cached_features, analyzed_bodies) of a rebuild.
fn counts(r: &RebuildResult) -> (usize, usize, usize) {
    (r.stats.computed_features, r.stats.cached_features, r.stats.analyzed_bodies)
}

/// Features a rebuild evaluates, counted from the document: every
/// non-suppressed sketch and solid feature. Suppressed features and
/// construction planes count in neither stats field (criterion A).
fn evaluated_features(doc: &Document) -> usize {
    doc.features
        .iter()
        .filter(|f| !f.suppressed && !matches!(f.kind, FeatureKind::Plane(_)))
        .count()
}

fn rebuild(kernel: &mut Kernel, doc: &Document) -> RebuildResult {
    kernel.rebuild_blocking(Arc::new(doc.clone()), TIMEOUT).expect("kernel answers")
}

fn extrude_mut(doc: &mut Document, id: FeatureId) -> &mut Extrude {
    match &mut doc.feature_mut(id).expect("feature exists").kind {
        FeatureKind::Extrude(e) => e,
        other => panic!("feature {id:?} is not an extrude: {other:?}"),
    }
}

/// Ids of the solid features in timeline order.
fn solid_ids(doc: &Document) -> Vec<FeatureId> {
    doc.features.iter().filter(|f| f.kind.is_solid()).map(|f| f.id).collect()
}

/// Body volumes by creation number, as raw bits (bit-identity checks).
fn volume_bits(r: &RebuildResult) -> Vec<(u32, u64)> {
    r.bodies.iter().map(|b| (b.number, b.volume_mm3.to_bits())).collect()
}

/// Two separate bodies with both sketches first, so a change of either
/// extrude has an exact expected count (no sketch after a solid feature):
///   [S1 rectangle 40 × 25 at the origin, S2 circle r = 5 at (100, 0),
///    E1 extrude S1 by 10 (NewBody), E2 extrude S2 by 30 (NewBody)].
/// The bodies do not touch (cuboid x ∈ [0, 40], cylinder x ∈ [95, 105]).
/// Volumes: Body1 = 40 · 25 · 10 = 10 000 mm³, Body2 = π · 5² · 30 = 750π mm³.
fn two_bodies() -> (Document, FeatureId, FeatureId) {
    let xy = PlaneRef::Origin(OriginPlane::XY);
    let mut doc = Document::default();
    let mut s1 = Sketch::new(xy);
    let mut rect = examples::rectangle(&mut s1, 0.0, 0.0, 40.0, 25.0);
    rect.sort_unstable();
    let s1 = doc.add_feature("Sketch1".into(), FeatureKind::Sketch(s1));
    let mut s2 = Sketch::new(xy);
    let c = s2.add_point([100.0, 0.0]);
    let circle = s2.add_circle(c, 5.0);
    let s2 = doc.add_feature("Sketch2".into(), FeatureKind::Sketch(s2));
    let extrude = |sketch, key, distance| {
        FeatureKind::Extrude(Extrude {
            profiles: vec![ProfileRef { sketch, key }],
            distance,
            symmetric: false,
            operation: BodyOp::NewBody,
        })
    };
    let e1 = doc.add_feature("Extrude1".into(), extrude(s1, rect, 10.0));
    let e2 = doc.add_feature("Extrude2".into(), extrude(s2, vec![circle], 30.0));
    doc.validate().expect("valid document");
    (doc, e1, e2)
}

fn body(r: &RebuildResult, number: u32) -> &qoob3d_kernel::BodyResult {
    r.bodies.iter().find(|b| b.number == number).expect("body exists")
}

/// The single body of a document that must produce exactly one.
fn only_body(r: &RebuildResult) -> &qoob3d_kernel::BodyResult {
    assert_eq!(r.bodies.len(), 1, "exactly one body");
    &r.bodies[0]
}

fn assert_rel(actual: f64, expected: f64, rel_tol: f64) {
    let rel = (actual - expected).abs() / expected;
    assert!(rel < rel_tol, "{actual:.6} mm³, expected {expected:.6} (±{rel_tol})");
}

// The exact B-Rep volume of planar/cylindrical solids is accurate to
// rounding; 1e-6 relative matches `join_and_cut` and is far below the
// smallest volume difference the parameter changes below produce (> 1 %).
const VOLUME_TOL: f64 = 1e-6;

#[test]
fn first_rebuild_of_the_gear_computes_everything() {
    // ADR-0012 criterion B. examples::gear() = [Gear1 (SpurGear, NewBody)]:
    // 0 sketches + 1 solid feature = 1 evaluated feature, 1 body.
    let doc = examples::gear();
    assert_eq!(evaluated_features(&doc), 1, "derivation of the expected count");
    let mut kernel = Kernel::spawn(|| {});
    let r = rebuild(&mut kernel, &doc);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.bodies.len(), 1);
    assert_eq!(counts(&r), (evaluated_features(&doc), 0, r.bodies.len()));
}

#[test]
fn unchanged_gear_comes_entirely_from_the_cache() {
    // ADR-0012 criterion C (replaces the wall-clock test, criterion F).
    // Second rebuild of the same document: nothing computed, the one
    // feature from B cached, no body analyzed again; results identical.
    let doc = examples::gear();
    let mut kernel = Kernel::spawn(|| {});
    let first = rebuild(&mut kernel, &doc);
    let second = rebuild(&mut kernel, &doc);
    assert_eq!(counts(&second), (0, evaluated_features(&doc), 0));
    assert_eq!(counts(&second), (0, 1, 0));
    assert_eq!(second.bodies.len(), first.bodies.len());
    for (a, b) in first.bodies.iter().zip(&second.bodies) {
        assert_eq!((a.id, a.number), (b.id, b.number));
        // Bit-identical: the cached analysis is reused, not recomputed.
        assert_eq!(a.volume_mm3.to_bits(), b.volume_mm3.to_bits());
        assert_eq!(a.valid, b.valid);
        assert!(b.valid);
    }
    assert_eq!(first.errors, second.errors);
}

#[test]
fn unchanged_multi_feature_document_comes_entirely_from_the_cache() {
    // ADR-0012 criteria B and C on a timeline with a construction plane.
    // examples::plate_with_hole() = [Sketch1, Extrude1 (new), Plane1,
    // Sketch2, Extrude2 (join), Extrude3 (cut)]: 2 sketches + 3 solid
    // features = 5 evaluated, Plane1 counts in neither field. The boss is
    // joined and the hole cut into the plate: 1 body.
    let doc = examples::plate_with_hole();
    assert_eq!(evaluated_features(&doc), 5, "derivation of the expected count");
    let mut kernel = Kernel::spawn(|| {});
    let first = rebuild(&mut kernel, &doc);
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    assert_eq!(first.bodies.len(), 1);
    assert_eq!(counts(&first), (5, 0, 1));
    let second = rebuild(&mut kernel, &doc);
    assert_eq!(counts(&second), (0, 5, 0));
    assert_eq!(volume_bits(&first), volume_bits(&second));
    assert_eq!(
        first.bodies.iter().map(|b| b.valid).collect::<Vec<_>>(),
        second.bodies.iter().map(|b| b.valid).collect::<Vec<_>>()
    );
}

#[test]
fn empty_document_computes_nothing() {
    // ADR-0012 criterion A, degenerate case: no features, no bodies.
    let doc = Document::default();
    let mut kernel = Kernel::spawn(|| {});
    assert_eq!(counts(&rebuild(&mut kernel, &doc)), (0, 0, 0));
    assert_eq!(counts(&rebuild(&mut kernel, &doc)), (0, 0, 0));
}

#[test]
fn suppressed_features_and_planes_count_in_neither_field() {
    // ADR-0012 criterion A. plate_with_hole with Extrude3 (cut) suppressed:
    // Sketch1, Extrude1, Sketch2, Extrude2 = 4 evaluated; Plane1 and the
    // suppressed Extrude3 count in neither field.
    // Volume without the cut: plate 60 · 40 · 5 + boss π · 5² · 8
    // = 12 000 + 200π mm³.
    let mut doc = examples::plate_with_hole();
    let cut = *solid_ids(&doc).last().unwrap();
    doc.feature_mut(cut).unwrap().suppressed = true;
    assert_eq!(evaluated_features(&doc), 4, "derivation of the expected count");
    let mut kernel = Kernel::spawn(|| {});
    let r = rebuild(&mut kernel, &doc);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(counts(&r), (4, 0, 1));
    assert_rel(only_body(&r).volume_mm3, 12_000.0 + 200.0 * PI, VOLUME_TOL);
}

#[test]
fn suppressing_the_last_feature_reuses_the_steps_before_it() {
    // ADR-0012: suppression is part of the key. After a full rebuild of
    // plate_with_hole, suppressing Extrude3 leaves Sketch1, Extrude1,
    // Sketch2, Extrude2 unchanged → 0 computed, 4 cached. The resulting
    // body (state after Extrude2) was never analyzed in the first rebuild
    // (that showed the state after the cut), so it is analyzed now: 1.
    let doc = examples::plate_with_hole();
    let mut suppressed = doc.clone();
    let cut = *solid_ids(&doc).last().unwrap();
    suppressed.feature_mut(cut).unwrap().suppressed = true;
    let mut kernel = Kernel::spawn(|| {});
    rebuild(&mut kernel, &doc);
    let r = rebuild(&mut kernel, &suppressed);
    assert_eq!(counts(&r), (0, evaluated_features(&suppressed), 1));
    assert_eq!(counts(&r), (0, 4, 1));
    assert_rel(only_body(&r).volume_mm3, 12_000.0 + 200.0 * PI, VOLUME_TOL);
}

#[test]
fn changing_the_last_solid_feature_recomputes_only_it() {
    // ADR-0012 criterion D. plate_with_hole: Extrude3 (cut, symmetric 40)
    // → symmetric 10. Its sketch lies on Plane1 (z = 5), so the Ø6 hole
    // now spans z ∈ [0, 10] instead of [−15, 25]: 10 mm deep instead of
    // through all 13 mm (plate z 0…5, boss z 5…13).
    // Every feature before it is cached: Sketch1, Extrude1, Sketch2,
    // Extrude2 = 4 (Plane1 not counted); Extrude3 computed = 1; the one
    // body it changes is analyzed = 1.
    // Volume = 60 · 40 · 5 + π · 5² · 8 − π · 3² · 10 = 12 000 + 110π mm³
    // (before: 12 000 + 200π − 117π = 12 000 + 83π).
    let doc = examples::plate_with_hole();
    let mut changed = doc.clone();
    let cut = *solid_ids(&doc).last().unwrap();
    extrude_mut(&mut changed, cut).distance = 10.0;
    let mut kernel = Kernel::spawn(|| {});
    let before = rebuild(&mut kernel, &doc);
    assert_rel(only_body(&before).volume_mm3, 12_000.0 + 83.0 * PI, VOLUME_TOL);
    let r = rebuild(&mut kernel, &changed);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(counts(&r), (1, 4, 1));
    assert_eq!(r.bodies.len(), 1);
    assert!(only_body(&r).valid);
    assert_rel(only_body(&r).volume_mm3, 12_000.0 + 110.0 * PI, VOLUME_TOL);
}

#[test]
fn changing_the_last_feature_analyzes_only_the_body_it_changes() {
    // ADR-0012 criteria C/D: an unchanged body keeps its analysis.
    // two_bodies(): Extrude2 30 → 20. Sketch1, Sketch2, Extrude1 cached = 3,
    // Extrude2 computed = 1; only Body2 (the cylinder) is analyzed = 1.
    // Body1 stays 10 000 mm³ (bit-identical), Body2 = π · 5² · 20 = 500π.
    let (doc, _, e2) = two_bodies();
    assert_eq!(evaluated_features(&doc), 4, "derivation of the expected count");
    let mut changed = doc.clone();
    extrude_mut(&mut changed, e2).distance = 20.0;
    let mut kernel = Kernel::spawn(|| {});
    let before = rebuild(&mut kernel, &doc);
    assert!(before.errors.is_empty(), "{:?}", before.errors);
    assert_eq!(counts(&before), (4, 0, 2));
    assert_rel(body(&before, 1).volume_mm3, 10_000.0, VOLUME_TOL);
    assert_rel(body(&before, 2).volume_mm3, 750.0 * PI, VOLUME_TOL);
    let r = rebuild(&mut kernel, &changed);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(counts(&r), (1, 3, 1));
    assert_eq!(r.bodies.len(), 2);
    assert_eq!(body(&r, 1).volume_mm3.to_bits(), body(&before, 1).volume_mm3.to_bits());
    assert_rel(body(&r, 2).volume_mm3, 500.0 * PI, VOLUME_TOL);
}

#[test]
fn changing_the_first_solid_feature_recomputes_it_and_every_later_one() {
    // ADR-0012 criterion D, hash chain: key_i = hash(key_{i-1}, param_i).
    // two_bodies(): Extrude1 10 → 12. The sketches precede it and are
    // cached = 2; Extrude1 and Extrude2 (its key depends on Extrude1's)
    // are computed = 2. Body1 changes and is analyzed again; whether Body2
    // (geometrically unchanged, but from a recomputed step) is analyzed is
    // not specified, so 1 ≤ analyzed ≤ 2.
    // Body1 = 40 · 25 · 12 = 12 000 mm³, Body2 = 750π mm³ (unchanged).
    let (doc, e1, _) = two_bodies();
    let mut changed = doc.clone();
    extrude_mut(&mut changed, e1).distance = 12.0;
    let mut kernel = Kernel::spawn(|| {});
    rebuild(&mut kernel, &doc);
    let r = rebuild(&mut kernel, &changed);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let (computed, cached, analyzed) = counts(&r);
    assert_eq!((computed, cached), (2, 2));
    assert!((1..=2).contains(&analyzed), "analyzed {analyzed}");
    assert_rel(body(&r, 1).volume_mm3, 12_000.0, VOLUME_TOL);
    assert_rel(body(&r, 2).volume_mm3, 750.0 * PI, VOLUME_TOL);
}

#[test]
fn changing_the_first_solid_feature_of_a_join_cut_chain() {
    // ADR-0012 criterion D on plate_with_hole: Extrude1 (plate) 5 → 6.
    // Extrude1, Extrude2, Extrude3 must be computed (3). Sketch1 precedes
    // Extrude1 and is cached. Sketch2 comes after Extrude1; the spec does not
    // say whether a sketch after a changed feature is recomputed, so it may
    // land in either field: computed ∈ {3, 4}, cached = 5 − computed.
    // Volume: plate 60 · 40 · 6 + boss above it π · 5² · (13 − 6) − hole
    // π · 3² · 13 = 14 400 + 175π − 117π = 14 400 + 58π mm³.
    let doc = examples::plate_with_hole();
    let mut changed = doc.clone();
    let plate = solid_ids(&doc)[0];
    extrude_mut(&mut changed, plate).distance = 6.0;
    let mut kernel = Kernel::spawn(|| {});
    rebuild(&mut kernel, &doc);
    let r = rebuild(&mut kernel, &changed);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let (computed, cached, analyzed) = counts(&r);
    assert!((3..=4).contains(&computed), "computed {computed}");
    assert_eq!(computed + cached, evaluated_features(&changed));
    assert_eq!(analyzed, 1, "the single body changed");
    assert_rel(only_body(&r).volume_mm3, 14_400.0 + 58.0 * PI, VOLUME_TOL);
}

#[test]
fn undo_to_the_previous_document_computes_nothing() {
    // ADR-0012 criterion E: rebuild A, then B (one parameter changed), then
    // A again. The kernel keeps this and the previous rebuild, so the third
    // rebuild takes all 5 evaluated features of A from the cache.
    // A = plate_with_hole, B = A with Extrude3 symmetric 40 → 10.
    let a = examples::plate_with_hole();
    let mut b = a.clone();
    let cut = *solid_ids(&a).last().unwrap();
    extrude_mut(&mut b, cut).distance = 10.0;
    let mut kernel = Kernel::spawn(|| {});
    let first = rebuild(&mut kernel, &a);
    let second = rebuild(&mut kernel, &b);
    assert_eq!(counts(&second).0, 1, "B changes one feature");
    let third = rebuild(&mut kernel, &a);
    assert_eq!(third.stats.computed_features, 0);
    assert_eq!(third.stats.cached_features, evaluated_features(&a));
    assert_eq!(volume_bits(&third), volume_bits(&first));
    assert!(third.errors.is_empty(), "{:?}", third.errors);
}

#[test]
fn failed_features_are_cached_too() {
    // ADR-0012 criterion C for a document with a feature error: cuboid
    // with one rectangle side deleted (see
    // `missing_profile_is_a_feature_error_not_a_guess`). Evaluated: Sketch1 +
    // Extrude1 = 2, no body. The second rebuild of the same document takes
    // both from the cache and reports the same error.
    let mut doc = examples::cuboid(10.0, 10.0, 10.0);
    let sketch = doc.features[0].id;
    let s = doc.sketch_mut(sketch).unwrap();
    let side = s.curves[0].id;
    s.delete(&[side]);
    let mut kernel = Kernel::spawn(|| {});
    let first = rebuild(&mut kernel, &doc);
    assert_eq!(first.errors.len(), 1, "{:?}", first.errors);
    assert_eq!(counts(&first), (2, 0, 0));
    let second = rebuild(&mut kernel, &doc);
    assert_eq!(counts(&second), (0, 2, 0));
    assert_eq!(first.errors, second.errors);
    assert!(second.bodies.is_empty());
}
