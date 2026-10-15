use super::*;
use qoob3d_core::document::{OriginPlane, PlaneRef};

fn sketch() -> (Sketch, EntityId, EntityId) {
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let a = s.add_point([10.0, 10.0]);
    let b = s.add_point([30.0, 10.0]);
    s.add_line(a, b);
    let c = s.add_point([60.0, 0.0]);
    s.add_circle(c, 10.0);
    (s, a, c)
}

fn at(s: &Sketch, x: f64, y: f64) -> Snap {
    snap(s, DVec2::new(x, y), 1.0, None, &SnapSettings::default(), &[])
}

#[test]
fn snaps_to_existing_points_and_reuses_them() {
    let (s, a, c) = sketch();
    let hit = at(&s, 10.4, 9.7);
    assert_eq!(
        (hit.kind, hit.entity, hit.point),
        (SnapKind::Endpoint, Some(a), DVec2::new(10.0, 10.0))
    );
    assert_eq!(at(&s, 60.5, 0.2).entity, Some(c));
    assert_eq!(at(&s, 60.5, 0.2).kind, SnapKind::Center);
    assert_eq!(at(&s, 0.3, -0.4).kind, SnapKind::Origin);
}

#[test]
fn snaps_to_midpoints_quadrants_and_alignment() {
    let (s, ..) = sketch();
    assert_eq!(at(&s, 20.2, 10.3).kind, SnapKind::Midpoint);
    assert_eq!(at(&s, 70.4, 0.3).kind, SnapKind::Quadrant);
    let aligned = at(&s, 10.6, 25.0); // vertically above point a
    assert_eq!(aligned.kind, SnapKind::Aligned);
    assert_eq!(aligned.point.x, 10.0);
    assert_eq!(aligned.guides.len(), 1);
}

#[test]
fn snaps_to_intersections() {
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let p: Vec<_> =
        [[0.0, 0.0], [100.0, 100.0], [0.0, 60.0], [90.0, 0.0]].map(|q| s.add_point(q)).into();
    s.add_line(p[0], p[1]);
    s.add_line(p[2], p[3]);
    let hit = at(&s, 36.4, 35.7);
    assert_eq!(hit.kind, SnapKind::Intersection);
    assert!(hit.point.distance(DVec2::new(36.0, 36.0)) < 1e-9);
}

#[test]
fn disabled_and_excluded() {
    let (s, a, _) = sketch();
    let off = SnapSettings { enabled: false, grid: None };
    assert_eq!(snap(&s, DVec2::new(10.2, 10.0), 1.0, None, &off, &[]).kind, SnapKind::Free);
    let dragging_a = snap(&s, DVec2::new(10.2, 10.1), 1.0, None, &SnapSettings::default(), &[a]);
    assert_ne!(dragging_a.entity, Some(a), "a dragged point never snaps to itself");
}

#[test]
fn grid_snap_is_lowest_priority() {
    let s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let grid = SnapSettings { enabled: true, grid: Some(5.0) };
    let hit = snap(&s, DVec2::new(24.6, 40.3), 1.0, None, &grid, &[]);
    assert_eq!((hit.kind, hit.point), (SnapKind::Grid, DVec2::new(25.0, 40.0)));
}
