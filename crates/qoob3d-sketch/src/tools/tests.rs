use super::*;
use crate::snap::SnapKind;
use qoob3d_core::document::{CurveKind, OriginPlane, PlaneRef};

fn sketch() -> Sketch {
    Sketch::new(PlaneRef::Origin(OriginPlane::XY))
}

fn at(x: f64, y: f64) -> Snap {
    Snap::free(DVec2::new(x, y))
}

fn on(id: EntityId, x: f64, y: f64) -> Snap {
    Snap { point: DVec2::new(x, y), kind: SnapKind::Endpoint, entity: Some(id), guides: vec![] }
}

fn click(t: &mut Tool, s: &mut Sketch, snap: &Snap) -> Outcome {
    t.handle(s, ToolEvent::Move(snap));
    t.handle(s, ToolEvent::Click(snap))
}

#[test]
fn polyline_shares_points_and_closes() {
    let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Line));
    click(&mut t, &mut s, &at(0.0, 0.0));
    assert_eq!(click(&mut t, &mut s, &at(10.0, 0.0)), Outcome::Edited);
    click(&mut t, &mut s, &at(10.0, 10.0));
    let first = s.points[0].id;
    // Clicking the first point closes the loop and ends the chain.
    assert_eq!(click(&mut t, &mut s, &on(first, 0.0, 0.0)), Outcome::Edited);
    assert_eq!(s.curves.len(), 3);
    assert_eq!(s.points.len(), 3, "corners are shared points");
    assert!(!t.is_placing(), "closed loop ends the chain");
}

#[test]
fn typed_length_and_angle_lock_the_line() {
    let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Line));
    click(&mut t, &mut s, &at(0.0, 0.0));
    t.handle(&mut s, ToolEvent::Move(&at(3.0, 4.0)));
    assert!((t.fields()[0].value - 5.0).abs() < 1e-9, "live length");
    t.handle(&mut s, ToolEvent::SetField { index: 0, value: 20.0 });
    t.handle(&mut s, ToolEvent::SetField { index: 1, value: 90.0 });
    assert_eq!(t.handle(&mut s, ToolEvent::Commit), Outcome::Edited);
    let end = s.points.last().unwrap();
    assert!(end.x.abs() < 1e-9 && (end.y - 20.0).abs() < 1e-9, "{end:?}");
}

#[test]
fn rectangle_two_point_with_typed_size() {
    let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Rectangle));
    click(&mut t, &mut s, &at(5.0, 5.0));
    t.handle(&mut s, ToolEvent::Move(&at(-1.0, 50.0)));
    t.handle(&mut s, ToolEvent::SetField { index: 0, value: 40.0 });
    t.handle(&mut s, ToolEvent::SetField { index: 1, value: 30.0 });
    assert_eq!(t.handle(&mut s, ToolEvent::Commit), Outcome::Edited);
    assert_eq!((s.points.len(), s.curves.len()), (4, 4));
    let xs: Vec<f64> = s.points.iter().map(|p| p.x).collect();
    assert!(xs.contains(&-35.0), "width extends towards the cursor (left): {xs:?}");
    assert!(!t.is_placing(), "tool is ready for the next rectangle");
}

#[test]
fn center_rectangle_and_circle() {
    let (mut s, mut t) = (sketch(), Tool::new(ToolKind::CenterRectangle));
    click(&mut t, &mut s, &at(0.0, 0.0));
    click(&mut t, &mut s, &at(20.0, 10.0));
    let (xmin, xmax) =
        s.points.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
    assert_eq!((xmin, xmax), (-20.0, 20.0));

    let mut t = Tool::new(ToolKind::Circle);
    click(&mut t, &mut s, &at(100.0, 0.0));
    t.handle(&mut s, ToolEvent::SetField { index: 0, value: 14.0 });
    t.handle(&mut s, ToolEvent::Commit);
    let circle = s.curves.last().unwrap();
    assert!(
        matches!(circle.kind, CurveKind::Circle { radius, .. } if (radius - 7.0).abs() < 1e-12)
    );
}

#[test]
fn construction_flag_and_escape() {
    let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Line));
    t.construction = true;
    click(&mut t, &mut s, &at(0.0, 0.0));
    click(&mut t, &mut s, &at(5.0, 0.0));
    assert!(s.curves[0].construction);
    assert_eq!(t.handle(&mut s, ToolEvent::Escape), Outcome::Nothing, "first Esc ends chain");
    assert_eq!(t.handle(&mut s, ToolEvent::Escape), Outcome::Done, "second Esc closes tool");
}

#[test]
fn degenerate_input_is_ignored() {
    let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Rectangle));
    click(&mut t, &mut s, &at(1.0, 1.0));
    assert_eq!(click(&mut t, &mut s, &at(1.0, 9.0)), Outcome::Nothing, "zero width");
    assert!(s.curves.is_empty());
    let mut t = Tool::new(ToolKind::Line);
    assert_eq!(t.handle(&mut s, ToolEvent::SetField { index: 0, value: -3.0 }), Outcome::Nothing);
    assert!(!t.fields.iter().any(|f| f.locked), "negative length rejected");
}

#[test]
fn point_tool_does_not_duplicate() {
    let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Point));
    assert_eq!(click(&mut t, &mut s, &at(3.0, 3.0)), Outcome::Edited);
    let id = s.points[0].id;
    assert_eq!(click(&mut t, &mut s, &on(id, 3.0, 3.0)), Outcome::Nothing);
    assert_eq!(s.points.len(), 1);
}

#[test]
fn arc_from_centre_start_and_typed_sweep() {
    let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Arc));
    click(&mut t, &mut s, &at(0.0, 0.0));
    assert_eq!(click(&mut t, &mut s, &at(10.0, 0.0)), Outcome::Nothing, "start placed");
    assert!(t.is_placing());
    t.handle(&mut s, ToolEvent::Move(&at(0.0, -10.0)));
    assert_eq!(t.fields().len(), 1, "only the sweep is open now");
    t.handle(&mut s, ToolEvent::SetField { index: 0, value: 180.0 });
    assert_eq!(t.handle(&mut s, ToolEvent::Commit), Outcome::Edited);
    let CurveKind::Arc { center, start, end } = s.curves[0].kind else { panic!() };
    assert_eq!(s.pos(center), [0.0, 0.0]);
    assert_eq!(s.pos(start), [10.0, 0.0]);
    let e = s.pos(end);
    assert!((e[0] + 10.0).abs() < 1e-9 && e[1].abs() < 1e-9, "{e:?}");
    assert!(!t.is_placing());
    assert!((s.arc(center, start, end).sweep - std::f64::consts::PI).abs() < 1e-9);
}

#[test]
fn arc_radius_can_be_typed() {
    let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Arc));
    click(&mut t, &mut s, &at(0.0, 0.0));
    t.handle(&mut s, ToolEvent::Move(&at(3.0, 0.0)));
    t.handle(&mut s, ToolEvent::SetField { index: 0, value: 7.5 });
    t.handle(&mut s, ToolEvent::Commit); // Enter places the start
    t.handle(&mut s, ToolEvent::Move(&at(0.0, 9.0)));
    assert_eq!(t.handle(&mut s, ToolEvent::Commit), Outcome::Edited);
    let CurveKind::Arc { center, start, end } = s.curves[0].kind else { panic!() };
    let g = s.arc(center, start, end);
    assert!((g.radius - 7.5).abs() < 1e-12);
    assert!((g.sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "quarter arc");
}
