#[test]
fn all_kinds_covers_every_feature_kind() {
    let doc = super::all_kinds();
    doc.validate().unwrap();
    for kind in super::FeatureKind::KIND_IDS {
        assert!(doc.count_of(kind) > 0, "{kind} missing");
    }
}

#[test]
fn every_example_is_valid_and_round_trips() {
    for (name, doc) in super::all() {
        doc.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = super::Document::from_json(&doc.to_json()).unwrap();
        assert_eq!(back, doc, "{name}");
    }
}
