use super::*;
use qoob3d_core::document::{FeatureKind, OriginPlane, PlaneRef};

fn add_sketch(p: &mut Project) -> FeatureId {
    let mut id = FeatureId(0);
    p.edit("sketch-create", |d| {
        id = d.add_feature(
            "Sketch1".into(),
            FeatureKind::Sketch(Sketch::new(PlaneRef::Origin(OriginPlane::XY))),
        );
    });
    id
}

#[test]
fn dirty_tracking_follows_undo() {
    let mut p = Project::default();
    assert!(!p.is_dirty());
    add_sketch(&mut p);
    assert!(p.is_dirty());
    assert!(p.undo());
    assert!(!p.is_dirty(), "back at the saved snapshot");
    assert!(p.redo());
    assert!(p.is_dirty());
}

#[test]
fn save_and_open_round_trip() {
    let dir = std::env::temp_dir().join(format!("qoob3d-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("part.qbd");
    let mut p = Project::default();
    let id = add_sketch(&mut p);
    p.save_to(&path).unwrap();
    assert!(!p.is_dirty());
    let q = Project::open(&path).unwrap();
    assert_eq!(**q.doc(), **p.doc());
    assert!(q.doc().sketch(id).is_some());
    assert!(!path.with_extension("qbd.tmp").exists(), "temp file renamed");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn preview_becomes_one_undo_step() {
    let mut p = Project::default();
    let id = add_sketch(&mut p);
    let mut s = p.doc().sketch(id).unwrap().clone();
    for x in 0..5 {
        s.add_point([f64::from(x), 0.0]);
        p.preview_sketch(id, s.clone());
    }
    p.commit_preview("edit-move");
    assert_eq!(p.doc().sketch(id).unwrap().points.len(), 5);
    assert!(p.undo());
    assert!(p.doc().sketch(id).unwrap().points.is_empty());
}
