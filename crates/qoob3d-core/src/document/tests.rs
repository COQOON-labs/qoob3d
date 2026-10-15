use super::*;

fn sample() -> Document {
    let mut doc = Document::default();
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let a = s.add_point([0.0, 0.0]);
    let b = s.add_point([20.0, 0.0]);
    s.add_line(a, b);
    let c = s.add_point([5.0, 5.0]);
    s.add_circle(c, 3.0);
    doc.add_feature("Sketch1".into(), FeatureKind::Sketch(s));
    doc
}

#[test]
fn json_round_trip() {
    let doc = sample();
    let json = doc.to_json();
    assert!(json.contains("\"format\": \"qoob3d\""));
    assert_eq!(Document::from_json(&json).unwrap(), doc);
}

#[test]
fn version_1_fixture_still_loads() {
    // Never change this fixture; add one per new format version (ADR-0030).
    let text = include_str!("../../tests/fixtures/format_v1.qbd");
    let doc = Document::from_json(text).unwrap();
    let s = doc.sketch(FeatureId(1)).unwrap();
    assert_eq!(s.curves.len(), 5, "rectangle + circle");
    assert_eq!(s.plane, PlaneRef::Origin(OriginPlane::XY));
}

#[test]
fn version_2_fixture_still_loads_with_every_kind() {
    // Never change this fixture (ADR-0030); it was written once from
    // `examples::all_kinds()`.
    let doc = Document::from_json(include_str!("../../tests/fixtures/format_v2.qbd")).unwrap();
    for kind in FeatureKind::KIND_IDS {
        assert!(doc.count_of(kind) > 0, "{kind}");
    }
    let arcs = doc.sketches().flat_map(|(_, s)| &s.curves);
    assert!(arcs.into_iter().any(|c| matches!(c.kind, CurveKind::Arc { .. })));
    assert!(doc.planes().count() == 1);
}

#[test]
fn rejects_foreign_newer_and_broken_files() {
    let wrong = r#"{"format":"stl","format_version":1,"document":{}}"#;
    assert!(matches!(Document::from_json(wrong), Err(LoadError::WrongFormat(_))));
    let newer = r#"{"format":"qoob3d","format_version":99,"document":{}}"#;
    assert_eq!(Document::from_json(newer), Err(LoadError::TooNew(99)));
    assert!(matches!(Document::from_json("{"), Err(LoadError::Json(_))));
}

#[test]
fn rejects_invalid_geometry() {
    let mut doc = sample();
    if let Some(s) = doc.sketch_mut(FeatureId(1)) {
        s.points[0].x = f64::NAN;
    }
    // NaN is not even valid JSON; check the validator directly.
    assert!(doc.validate().unwrap_err().contains("invalid coordinates"));

    let mut doc = sample();
    if let Some(s) = doc.sketch_mut(FeatureId(1)) {
        s.curves.push(SketchCurve {
            id: EntityId(99),
            kind: CurveKind::Line { start: EntityId(1), end: EntityId(42) },
            construction: false,
        });
        s.next_id = 100;
    }
    let json = doc.to_json();
    assert!(
        matches!(Document::from_json(&json), Err(LoadError::Invalid(m)) if m.contains("missing point"))
    );
}
