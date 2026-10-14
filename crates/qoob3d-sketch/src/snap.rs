//! Snapping and inference (SK-SN-01 … SK-SN-05, ADR-0027).
//!
//! Priority: existing points (endpoint, centre, origin) > midpoints and
//! quadrants > intersections > alignment (horizontal/vertical to points,
//! the tool anchor and the axes) > on curve > grid > free. All tolerances are
//! in sketch units; the UI converts its pixel tolerance (SK-SN-03).

use glam::DVec2;
use qoob3d_core::document::{CurveKind, EntityId, Sketch};

use crate::geom::{self, Shape, v};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    /// An existing point that is the end of a line (or a lone point).
    Endpoint,
    /// An existing circle centre.
    Center,
    Origin,
    Midpoint,
    Quadrant,
    Intersection,
    /// Aligned horizontally and/or vertically with a reference.
    Aligned,
    OnCurve,
    Grid,
    Free,
}

/// A dashed helper line shown while aligned (sketch coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub from: DVec2,
    pub to: DVec2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snap {
    pub point: DVec2,
    pub kind: SnapKind,
    /// Existing point to reuse instead of creating a new one (ADR-0024).
    pub entity: Option<EntityId>,
    pub guides: Vec<Guide>,
}

impl Snap {
    pub fn free(p: DVec2) -> Self {
        Self { point: p, kind: SnapKind::Free, entity: None, guides: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapSettings {
    /// Off while Cmd/Ctrl is held or switched off in the palette (SK-SN-05).
    pub enabled: bool,
    /// Grid spacing for grid snapping, `None` = no grid snapping.
    pub grid: Option<f64>,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self { enabled: true, grid: None }
    }
}

/// Finds the best snap for `cursor`. `anchor` is the tool's current base
/// point (for horizontal/vertical locks); `exclude` lists entities that must
/// not be snapped to (e.g. the point being dragged).
pub fn snap(
    sketch: &Sketch,
    cursor: DVec2,
    tol: f64,
    anchor: Option<DVec2>,
    settings: &SnapSettings,
    exclude: &[EntityId],
) -> Snap {
    if !settings.enabled {
        return Snap::free(cursor);
    }
    let near = |p: DVec2| p.distance(cursor) <= tol;
    let best = |cands: Vec<(DVec2, SnapKind, Option<EntityId>)>| {
        cands
            .into_iter()
            .filter(|(p, ..)| near(*p))
            .min_by(|a, b| a.0.distance(cursor).total_cmp(&b.0.distance(cursor)))
    };

    // 1. Existing points and the origin.
    let mut points = Vec::new();
    for p in sketch.points.iter().filter(|p| !exclude.contains(&p.id)) {
        let is_center = sketch.curves_at(p.id).any(|c| match c.kind {
            CurveKind::Circle { center, .. } | CurveKind::Arc { center, .. } => center == p.id,
            CurveKind::Line { .. } => false,
        });
        let kind = if is_center { SnapKind::Center } else { SnapKind::Endpoint };
        points.push((v([p.x, p.y]), kind, Some(p.id)));
    }
    points.push((DVec2::ZERO, SnapKind::Origin, None));
    if let Some((p, kind, entity)) = best(points) {
        // Prefer an existing point at the origin over the bare origin.
        return Snap { point: p, kind, entity, guides: Vec::new() };
    }

    let curves: Vec<_> = sketch
        .curves
        .iter()
        .filter(|c| !exclude.contains(&c.id) && !c.points().iter().any(|p| exclude.contains(p)))
        .map(|c| geom::shape(sketch, c))
        .collect();

    // 2. Midpoints and quadrants.
    let mut derived = Vec::new();
    for s in &curves {
        match *s {
            Shape::Segment(a, b) => derived.push(((a + b) * 0.5, SnapKind::Midpoint, None)),
            Shape::Circle(c, r) => {
                for d in [DVec2::X, DVec2::Y, DVec2::NEG_X, DVec2::NEG_Y] {
                    derived.push((c + d * r, SnapKind::Quadrant, None));
                }
            }
            Shape::Arc(c, r, a0, sweep) => {
                let mid = a0 + 0.5 * sweep;
                derived.push((c + DVec2::new(mid.cos(), mid.sin()) * r, SnapKind::Midpoint, None));
            }
        }
    }
    if let Some((p, kind, entity)) = best(derived) {
        return Snap { point: p, kind, entity, guides: Vec::new() };
    }

    // 3. Intersections of curves that pass near the cursor.
    let close: Vec<Shape> = curves.iter().copied().filter(|s| s.distance(cursor) <= tol).collect();
    let mut crossings = Vec::new();
    for (i, a) in close.iter().enumerate() {
        for b in &close[i + 1..] {
            for p in geom::intersections(*a, *b) {
                crossings.push((p, SnapKind::Intersection, None));
            }
        }
    }
    if let Some((p, kind, entity)) = best(crossings) {
        return Snap { point: p, kind, entity, guides: Vec::new() };
    }

    // 4. Horizontal / vertical alignment with the anchor, points and axes.
    let mut refs: Vec<DVec2> = anchor.into_iter().collect();
    refs.extend(sketch.points.iter().filter(|p| !exclude.contains(&p.id)).map(|p| v([p.x, p.y])));
    refs.push(DVec2::ZERO); // the axes pass through the origin
    let mut x_ref: Option<DVec2> = None; // same x → vertical alignment
    let mut y_ref: Option<DVec2> = None; // same y → horizontal alignment
    for r in &refs {
        if (r.x - cursor.x).abs() <= tol
            && x_ref.is_none_or(|q| (q.x - cursor.x).abs() > (r.x - cursor.x).abs())
        {
            x_ref = Some(*r);
        }
        if (r.y - cursor.y).abs() <= tol
            && y_ref.is_none_or(|q| (q.y - cursor.y).abs() > (r.y - cursor.y).abs())
        {
            y_ref = Some(*r);
        }
    }
    if x_ref.is_some() || y_ref.is_some() {
        let point = DVec2::new(x_ref.map_or(cursor.x, |r| r.x), y_ref.map_or(cursor.y, |r| r.y));
        let guides = x_ref.into_iter().chain(y_ref).map(|r| Guide { from: r, to: point }).collect();
        return Snap { point, kind: SnapKind::Aligned, entity: None, guides };
    }

    // 5. On a curve.
    if let Some(p) = curves
        .iter()
        .map(|s| s.closest(cursor))
        .filter(|p| near(*p))
        .min_by(|a, b| a.distance(cursor).total_cmp(&b.distance(cursor)))
    {
        return Snap { point: p, kind: SnapKind::OnCurve, entity: None, guides: Vec::new() };
    }

    // 6. Grid.
    if let Some(step) = settings.grid.filter(|s| *s > 0.0) {
        let g = (cursor / step).round() * step;
        if near(g) {
            return Snap { point: g, kind: SnapKind::Grid, entity: None, guides: Vec::new() };
        }
    }

    Snap::free(cursor)
}

#[cfg(test)]
mod tests {
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
        let dragging_a =
            snap(&s, DVec2::new(10.2, 10.1), 1.0, None, &SnapSettings::default(), &[a]);
        assert_ne!(dragging_a.entity, Some(a), "a dragged point never snaps to itself");
    }

    #[test]
    fn grid_snap_is_lowest_priority() {
        let s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
        let grid = SnapSettings { enabled: true, grid: Some(5.0) };
        let hit = snap(&s, DVec2::new(24.6, 40.3), 1.0, None, &grid, &[]);
        assert_eq!((hit.kind, hit.point), (SnapKind::Grid, DVec2::new(25.0, 40.0)));
    }
}
