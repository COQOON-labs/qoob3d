use super::*;

#[test]
fn defaults_are_english_and_default_printer() {
    let s = Settings::default();
    assert_eq!(s.language, Language::English);
    assert!(qoob3d_core::PrinterProfile::by_id(&s.printer_id).is_some());
}
