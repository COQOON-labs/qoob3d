//! Hit tests and box selection (SK-UI-08, SK-UI-09).

use glam::DVec2;
use qoob3d_core::document::{EntityId, Sketch};

use crate::geom::{self, Shape, v};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Point,
    Curve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub id: EntityId,
    pub kind: HitKind,
}

/// The entity under `cursor`: points win over curves (SK-UI-09).
pub fn pick(sketch: &Sketch, cursor: DVec2, tol: f64) -> Option<Hit> {
    let point = sketch
        .points
        .iter()
        .map(|p| (p.id, v([p.x, p.y]).distance(cursor)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((id, _)) = point {
        return Some(Hit { id, kind: HitKind::Point });
    }
    sketch
        .curves
        .iter()
        .map(|c| (c.id, geom::shape(sketch, c).distance(cursor)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| Hit { id, kind: HitKind::Curve })
}

/// Box selection between two corners. `crossing` (dragged right→left)
/// also selects what the box only touches; otherwise only what lies
/// completely inside (Fusion semantics).
pub fn in_box(sketch: &Sketch, a: DVec2, b: DVec2, crossing: bool) -> Vec<EntityId> {
    let (lo, hi) = (a.min(b), a.max(b));
    let inside = |p: DVec2| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y;
    let mut out: Vec<EntityId> =
        sketch.points.iter().filter(|p| inside(v([p.x, p.y]))).map(|p| p.id).collect();
    for c in &sketch.curves {
        let shape = geom::shape(sketch, c);
        let selected = match shape {
            Shape::Segment(p, q) => {
                if crossing {
                    inside(p) || inside(q) || segment_hits_box(p, q, lo, hi)
                } else {
                    inside(p) && inside(q)
                }
            }
            Shape::Circle(center, r) => {
                let full = inside(center - DVec2::splat(r)) && inside(center + DVec2::splat(r));
                if crossing { full || circle_hits_box(center, r, lo, hi) } else { full }
            }
            Shape::Arc(c, r, a0, sweep) => {
                let pts = geom::arc_polyline(c, r, a0, sweep, 32);
                if crossing {
                    pts.iter().any(|p| inside(*p))
                        || pts.windows(2).any(|w| segment_hits_box(w[0], w[1], lo, hi))
                } else {
                    pts.iter().all(|p| inside(*p))
                }
            }
        };
        if selected {
            out.push(c.id);
        }
    }
    out
}

fn segment_hits_box(p: DVec2, q: DVec2, lo: DVec2, hi: DVec2) -> bool {
    let corners = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
    (0..4).any(|i| {
        !geom::intersections(Shape::Segment(p, q), Shape::Segment(corners[i], corners[(i + 1) % 4]))
            .is_empty()
    })
}

fn circle_hits_box(c: DVec2, r: f64, lo: DVec2, hi: DVec2) -> bool {
    let corners = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
    (0..4).any(|i| {
        !geom::intersections(Shape::Circle(c, r), Shape::Segment(corners[i], corners[(i + 1) % 4]))
            .is_empty()
    })
}

#[cfg(test)]
mod tests {
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
        assert!(
            !in_box(&s, DVec2::new(54.0, 40.0), DVec2::new(60.0, 60.0), false).contains(&circle)
        );
    }
}
