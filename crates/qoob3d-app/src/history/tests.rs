use super::*;

fn doc(n: u64) -> Arc<Document> {
    Arc::new(Document { next_feature_id: n, ..Default::default() })
}

#[test]
fn undo_redo_and_branching() {
    let mut h = History::new(doc(1));
    h.push(doc(2), "a");
    h.push(doc(3), "b");
    assert_eq!(h.undo().unwrap().next_feature_id, 2);
    assert_eq!(h.redo_label(), Some("b"));
    h.push(doc(4), "c"); // drops "b"
    assert!(h.redo().is_none());
    assert_eq!(h.undo().unwrap().next_feature_id, 2);
    assert_eq!(h.undo().unwrap().next_feature_id, 1);
    assert!(h.undo().is_none());
}

#[test]
fn collapse_makes_one_step() {
    let mut h = History::new(doc(1));
    let m = h.marker();
    for n in 2..6 {
        h.push(doc(n), "edit");
    }
    h.collapse_since(m, "sketch");
    assert_eq!(h.current().next_feature_id, 5);
    assert_eq!(h.undo_label(), Some("sketch"));
    assert_eq!(h.undo().unwrap().next_feature_id, 1);
}
