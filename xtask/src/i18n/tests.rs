use super::*;

#[test]
fn finds_keys_and_literals() {
    assert_eq!(literals_after(r#"ui.label(tr("nav-grid"))"#, KEY_CALLS), ["nav-grid"]);
    assert_eq!(literals_after(r#"tr_args("a-b", &[])"#, KEY_CALLS), ["a-b"]);
    assert!(literals_after(r#"text.push_str("  x")"#, KEY_CALLS).is_empty());
    assert_eq!(literals_after(r#"ui.button("Fit")"#, WIDGET_CALLS), ["Fit"]);
    assert_eq!(literals_after(r#"ui.checkbox(&mut x, "Grid")"#, WIDGET_CALLS), ["Grid"]);
    assert!(literals_after(r#"ui.checkbox(&mut x, tr("g"))"#, WIDGET_CALLS).is_empty());
}

#[test]
fn recognises_indirect_keys() {
    assert_eq!(all_literals(r#"planned(ui, "file-open", "M3")"#), ["file-open", "M3"]);
    assert!(looks_like_key("file-open"));
    assert!(!looks_like_key("M3") && !looks_like_key("qoob3d-navbar") && !looks_like_key("doc"));
}
