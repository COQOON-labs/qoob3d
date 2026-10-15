use super::*;
use qoob3d_core::document::{OriginPlane, PlaneRef};
use std::f64::consts::PI;

fn rect(s: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<EntityId> {
    let p = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|q| s.add_point(q));
    (0..4).map(|i| s.add_line(p[i], p[(i + 1) % 4])).collect()
}

fn sketch() -> Sketch {
    Sketch::new(PlaneRef::Origin(OriginPlane::XY))
}

#[test]
fn rectangle_is_one_profile() {
    let mut s = sketch();
    let lines = rect(&mut s, 0.0, 0.0, 40.0, 30.0);
    let p = sketch_profiles(&s).unwrap();
    assert_eq!(p.len(), 1);
    assert!((p[0].area_mm2 - 1200.0).abs() < 1e-6, "{}", p[0].area_mm2);
    assert_eq!(p[0].key, lines);
    assert!(p[0].contains([20.0, 15.0]) && !p[0].contains([50.0, 15.0]));
}

#[test]
fn circle_inside_rectangle_gives_ring_and_disc() {
    let mut s = sketch();
    let lines = rect(&mut s, 0.0, 0.0, 40.0, 30.0);
    let c = s.add_point([20.0, 15.0]);
    let circle = s.add_circle(c, 7.0);
    let p = sketch_profiles(&s).unwrap();
    assert_eq!(p.len(), 2, "{:?}", p.iter().map(|x| &x.key).collect::<Vec<_>>());
    let disc = p.iter().find(|x| x.key == vec![circle]).expect("disc");
    let ring = p.iter().find(|x| x.key.len() == 5).expect("ring");
    assert!((disc.area_mm2 - PI * 49.0).abs() < 1e-6);
    assert!((ring.area_mm2 - (1200.0 - PI * 49.0)).abs() < 1e-6);
    assert!(lines.iter().all(|l| ring.key.contains(l)));
    assert!(!ring.contains([20.0, 15.0]), "the hole is not part of the ring");
}

#[test]
fn overlapping_shapes_split_into_regions() {
    let mut s = sketch();
    rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    let p = sketch_profiles(&s).unwrap();
    assert_eq!(p.len(), 3, "two L-shapes and the overlap");
    let total: f64 = p.iter().map(|x| x.area_mm2).sum();
    assert!((total - (400.0 + 400.0 - 100.0)).abs() < 1e-6);
}

#[test]
fn open_contours_and_construction_make_no_profile() {
    let mut s = sketch();
    let a = s.add_point([0.0, 0.0]);
    let b = s.add_point([10.0, 0.0]);
    let c = s.add_point([10.0, 10.0]);
    s.add_line(a, b);
    s.add_line(b, c);
    assert!(sketch_profiles(&s).unwrap().is_empty(), "open polyline");

    let mut s = sketch();
    for id in rect(&mut s, 0.0, 0.0, 10.0, 10.0) {
        s.curve_mut(id).unwrap().construction = true;
    }
    assert!(sketch_profiles(&s).unwrap().is_empty(), "construction rectangle");
    assert!(sketch_profiles(&sketch()).unwrap().is_empty(), "empty sketch");
}
