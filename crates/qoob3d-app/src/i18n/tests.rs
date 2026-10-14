use super::*;

#[test]
fn translations_and_formatting() {
    set_language(Language::English);
    assert_eq!(tr("nav-grid"), "Grid");
    assert_eq!(decimal(15.7, 2), "15.70");
    let one = tr_args(
        "status-model",
        &[
            ("name", "X".into()),
            ("triangles", 1.into()),
            ("edges", 2.into()),
            ("tight", "ok".into()),
            ("volume", "1".into()),
        ],
    );
    assert!(one.contains("1 triangle ") && one.contains("2 edges"), "{one}");

    set_language(Language::German);
    assert_eq!(tr("nav-grid"), "Raster");
    assert_eq!(decimal(15.7, 2), "15,70");
    assert_eq!(tr_args("coming-in", &[("milestone", "M3".into())]), "Kommt mit M3");
    assert_eq!(tr("does-not-exist"), "«does-not-exist»");
    set_language(Language::English);
}

#[test]
fn every_language_parses() {
    assert_eq!(bundles().len(), Language::ALL.len());
}
