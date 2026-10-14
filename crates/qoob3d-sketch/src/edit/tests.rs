use super::*;
use qoob3d_core::document::{OriginPlane, PlaneRef};

fn triangle() -> (Sketch, [EntityId; 3], [EntityId; 3]) {
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let p = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]].map(|q| s.add_point(q));
    let l = [(0, 1), (1, 2), (2, 0)].map(|(a, b)| s.add_line(p[a], p[b]));
    (s, p, l)
}

#[test]
fn dragging_a_point_moves_connected_lines() {
    let (mut s, p, _) = triangle();
    drag(&mut s, &Grab::Point(p[1]), DVec2::new(20.0, 5.0));
    assert_eq!(s.pos(p[1]), [20.0, 5.0]);
    assert_eq!(s.curves_at(p[1]).count(), 2, "both lines still attached");
}

#[test]
fn dragging_a_line_translates_it() {
    let (mut s, p, l) = triangle();
    let g = grab(&s, Hit { id: l[0], kind: HitKind::Curve }, DVec2::new(5.0, 0.0)).unwrap();
    drag(&mut s, &g, DVec2::new(5.0, -3.0));
    assert_eq!(s.pos(p[0]), [0.0, -3.0]);
    assert_eq!(s.pos(p[1]), [10.0, -3.0]);
    assert_eq!(s.pos(p[2]), [0.0, 10.0], "unrelated point stays");
}

#[test]
fn circle_rim_changes_radius() {
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let c = s.add_point([0.0, 0.0]);
    let circle = s.add_circle(c, 5.0);
    drag(&mut s, &Grab::CircleRim(circle), DVec2::new(0.0, 12.0));
    assert!(
        matches!(s.curve(circle).unwrap().kind, CurveKind::Circle { radius, .. } if radius == 12.0)
    );
}

#[test]
fn merging_closes_a_contour() {
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let a = s.add_point([0.0, 0.0]);
    let b = s.add_point([10.0, 0.0]);
    let c = s.add_point([10.0, 10.0]);
    let d = s.add_point([0.1, 0.1]); // almost back at a
    s.add_line(a, b);
    s.add_line(b, c);
    s.add_line(c, d);
    merge_points(&mut s, d, a);
    assert!(s.point(d).is_none());
    assert_eq!(s.curves_at(a).count(), 2, "contour is closed at a");
    // Merging the ends of one line removes the collapsed line.
    merge_points(&mut s, b, a);
    assert_eq!(s.curves.len(), 2);
}

#[test]
fn arcs_keep_their_end_on_the_circle_and_scale_by_the_rim() {
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let c = s.add_point([0.0, 0.0]);
    let a = s.add_point([10.0, 0.0]);
    let b = s.add_point([-10.0, 0.0]);
    let arc = s.add_arc(c, a, b);
    drag(&mut s, &Grab::Point(a), DVec2::new(5.0, 0.0));
    assert_eq!(s.pos(b), [-5.0, 0.0], "end follows the new radius");
    drag(&mut s, &Grab::CircleRim(arc), DVec2::new(0.0, 8.0));
    assert_eq!(s.pos(a), [8.0, 0.0]);
    assert_eq!(s.pos(b), [-8.0, 0.0]);
}

#[test]
fn construction_toggle() {
    let (mut s, _, l) = triangle();
    toggle_construction(&mut s, &[l[0], l[1]]);
    assert!(s.curve(l[0]).unwrap().construction && s.curve(l[1]).unwrap().construction);
    toggle_construction(&mut s, &[l[0], l[1]]);
    assert!(!s.curve(l[0]).unwrap().construction);
    toggle_construction(&mut s, &[l[0], l[2]]);
    assert!(s.curve(l[2]).unwrap().construction, "mixed selection → all construction");
}
