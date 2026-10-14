//! Editing operations on existing geometry: dragging, merging points,
//! construction toggle (SK-UI-10, SK-TL-16, SK-ED-01). Without a solver
//! (M3a) a drag simply moves the grabbed geometry; connected curves follow
//! because they reference the same points (ADR-0024).

use glam::DVec2;
use qoob3d_core::document::{CurveKind, EntityId, Sketch};

use crate::geom::v;
use crate::pick::{Hit, HitKind};

/// What a drag grabbed, captured at drag start.
#[derive(Debug, Clone, PartialEq)]
pub enum Grab {
    /// Move one point to the (snapped) cursor.
    Point(EntityId),
    /// Translate the points of a line by the cursor movement.
    Line { points: [EntityId; 2], start: [DVec2; 2], from: DVec2 },
    /// Change a circle's (or arc's) radius to the cursor distance.
    CircleRim(EntityId),
}

pub fn grab(sketch: &Sketch, hit: Hit, cursor: DVec2) -> Option<Grab> {
    match hit.kind {
        HitKind::Point => Some(Grab::Point(hit.id)),
        HitKind::Curve => match sketch.curve(hit.id)?.kind {
            CurveKind::Line { start, end } => Some(Grab::Line {
                points: [start, end],
                start: [v(sketch.pos(start)), v(sketch.pos(end))],
                from: cursor,
            }),
            CurveKind::Circle { .. } | CurveKind::Arc { .. } => Some(Grab::CircleRim(hit.id)),
        },
    }
}

/// Applies a drag to `to` (sketch coordinates, already snapped).
pub fn drag(sketch: &mut Sketch, grab: &Grab, to: DVec2) {
    match grab {
        Grab::Point(id) => {
            set_pos(sketch, *id, to);
            sketch.repair_arcs();
        }
        Grab::Line { points, start, from } => {
            let delta = to - *from;
            for (id, s) in points.iter().zip(start) {
                set_pos(sketch, *id, *s + delta);
            }
        }
        Grab::CircleRim(id) => match sketch.curve(*id).map(|c| c.kind.clone()) {
            Some(CurveKind::Circle { center, .. }) => {
                let r = v(sketch.pos(center)).distance(to);
                if r > 1e-6
                    && let Some(c) = sketch.curve_mut(*id)
                {
                    c.kind = CurveKind::Circle { center, radius: r };
                }
            }
            Some(CurveKind::Arc { center, start, end }) => {
                // Scale both end points radially.
                let c = v(sketch.pos(center));
                let r = c.distance(to);
                if r > 1e-6 {
                    for p in [start, end] {
                        let d = v(sketch.pos(p)) - c;
                        if d.length() > 1e-12 {
                            set_pos(sketch, p, c + d.normalize() * r);
                        }
                    }
                }
            }
            _ => {}
        },
    }
}

fn set_pos(sketch: &mut Sketch, id: EntityId, p: DVec2) {
    if let Some(pt) = sketch.point_mut(id) {
        pt.x = p.x;
        pt.y = p.y;
    }
}

/// Merges point `from` into `into`: curves using `from` now use `into`,
/// curves that would collapse (line from a point to itself) are removed.
/// Used when a dragged point is dropped onto another point.
pub fn merge_points(sketch: &mut Sketch, from: EntityId, into: EntityId) {
    if from == into || sketch.point(into).is_none() {
        return;
    }
    for c in &mut sketch.curves {
        match &mut c.kind {
            CurveKind::Line { start, end } => {
                if *start == from {
                    *start = into;
                }
                if *end == from {
                    *end = into;
                }
            }
            CurveKind::Circle { center, .. } => {
                if *center == from {
                    *center = into;
                }
            }
            CurveKind::Arc { center, start, end } => {
                for p in [center, start, end] {
                    if *p == from {
                        *p = into;
                    }
                }
            }
        }
    }
    // Arcs that collapsed (two equal points) are removed like collapsed lines.
    sketch.curves.retain(|c| {
        !matches!(c.kind, CurveKind::Arc { center, start, end }
            if center == start || center == end || start == end)
    });
    sketch.curves.retain(|c| !matches!(c.kind, CurveKind::Line { start, end } if start == end));
    sketch.points.retain(|p| p.id != from);
}

/// Toggles construction for the selected curves and points: if all are
/// construction already, they become normal geometry, otherwise all
/// become construction (SK-TL-16).
pub fn toggle_construction(sketch: &mut Sketch, ids: &[EntityId]) {
    let curves: Vec<_> = sketch.curves.iter().filter(|c| ids.contains(&c.id)).collect();
    let points: Vec<_> = sketch.points.iter().filter(|p| ids.contains(&p.id)).collect();
    let all = curves.iter().all(|c| c.construction) && points.iter().all(|p| p.construction);
    let target = !all;
    for c in sketch.curves.iter_mut().filter(|c| ids.contains(&c.id)) {
        c.construction = target;
    }
    for p in sketch.points.iter_mut().filter(|p| ids.contains(&p.id)) {
        p.construction = target;
    }
}

#[cfg(test)]
mod tests {
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
}
