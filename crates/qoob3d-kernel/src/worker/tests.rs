use super::*;

#[test]
fn rebuild_returns_profiles_per_sketch() {
    use qoob3d_core::document::{FeatureKind, OriginPlane, PlaneRef, Sketch};
    let mut doc = Document::default();
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XZ));
    let c = s.add_point([0.0, 10.0]);
    s.add_circle(c, 5.0);
    let feature = doc.add_feature("Sketch1".into(), FeatureKind::Sketch(s));

    let (tx, rx) = mpsc::channel();
    let mut kernel = Kernel::spawn(move || {
        let _ = tx.send(());
    });
    let id = kernel.submit(Request::Rebuild { document: Arc::new(doc) }).unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(30)).expect("no notification");
    let Some(Response::Rebuilt { id: got, result }) = kernel.try_recv() else {
        panic!("no rebuild response");
    };
    let sketches = result.sketches;
    assert_eq!(got, id);
    assert_eq!(sketches.len(), 1);
    assert_eq!(sketches[0].feature, feature);
    assert_eq!(sketches[0].profiles.as_ref().unwrap().len(), 1);
}

#[test]
fn worker_answers_demo_cube_request() {
    let (tx, rx) = mpsc::channel();
    let mut kernel = Kernel::spawn(move || {
        let _ = tx.send(());
    });
    let id = kernel.submit(Request::DemoPart).unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(30)).expect("no notification");
    let Some(Response::Mesh { id: got, result }) = kernel.try_recv() else {
        panic!("no response");
    };
    assert_eq!(got, id);
    assert!(result.unwrap().faces.analyze().is_watertight());
}
