use super::*;
use qoob3d_core::document::{OriginPlane, PlaneRef};

#[test]
fn points_win_over_curves() {
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let a = s.add_point([0.0, 0.0]);
    let b = s.add_point([10.0, 0.0]);
    let l = s.add_line(a, b);
    assert_eq!(pick(&s, DVec2::new(0.3, 0.3), 1.0), Some(Hit { id: a, kind: HitKind::Point }));
    assert_eq!(pick(&s, DVec2::new(5.0, 0.5), 1.0), Some(Hit { id: l, kind: HitKind::Curve }));
    assert_eq!(pick(&s, DVec2::new(5.0, 5.0), 1.0), None);
}

#[test]
fn window_versus_crossing_selection() {
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let a = s.add_point([0.0, 0.0]);
    let b = s.add_point([10.0, 0.0]);
    let l = s.add_line(a, b);
    let c = s.add_point([50.0, 50.0]);
    let circle = s.add_circle(c, 5.0);
    // Box covering only the left half of the line.
    let window = in_box(&s, DVec2::new(-1.0, -1.0), DVec2::new(5.0, 1.0), false);
    assert_eq!(window, vec![a]);
    let crossing = in_box(&s, DVec2::new(5.0, 1.0), DVec2::new(-1.0, -1.0), true);
    assert!(crossing.contains(&l) && crossing.contains(&a));
    // Box through the circle's rim.
    let rim = in_box(&s, DVec2::new(54.0, 40.0), DVec2::new(60.0, 60.0), true);
    assert!(rim.contains(&circle));
    assert!(!in_box(&s, DVec2::new(54.0, 40.0), DVec2::new(60.0, 60.0), false).contains(&circle));
}
