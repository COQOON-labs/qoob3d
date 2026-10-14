//! Sketch data (ADR-0024): points are entities, curves reference point ids.
//! Coordinates are local to the sketch plane, in mm (`f64`).

use serde::{Deserialize, Serialize};

/// Id of a point or curve, unique within its sketch and never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub u32);

/// The three origin planes (SK-PL-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OriginPlane {
    XY,
    XZ,
    YZ,
}

impl OriginPlane {
    pub const ALL: [OriginPlane; 3] = [OriginPlane::XY, OriginPlane::XZ, OriginPlane::YZ];

    /// Local frame, identical to the ViewCube faces TOP / FRONT / RIGHT, so a
    /// sketch is never mirrored in its standard view (SK-PL-01, ADR-0006).
    pub fn frame(self) -> PlaneFrame {
        let (u, v, n) = match self {
            OriginPlane::XY => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            OriginPlane::XZ => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
            OriginPlane::YZ => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        };
        PlaneFrame { origin: [0.0; 3], u, v, n }
    }
}

/// Where a sketch (or a plane-based feature) lies. Resolve it with
/// `Document::plane_frame`; faces follow in M3c.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlaneRef {
    Origin(OriginPlane),
    /// A `ConstructionPlane` feature (ADR-0031).
    Construction(super::FeatureId),
}

/// Right-handed local frame of a sketch plane: `n = u × v`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneFrame {
    pub origin: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub n: [f64; 3],
}

impl PlaneFrame {
    /// The same plane moved by `d` mm along its normal.
    pub fn offset(&self, d: f64) -> PlaneFrame {
        PlaneFrame { origin: std::array::from_fn(|k| self.origin[k] + self.n[k] * d), ..*self }
    }

    /// Sketch coordinates → world.
    pub fn to_world(&self, p: [f64; 2]) -> [f64; 3] {
        std::array::from_fn(|k| self.origin[k] + self.u[k] * p[0] + self.v[k] * p[1])
    }

    /// Direction in sketch coordinates → world.
    pub fn dir_to_world(&self, d: [f64; 2]) -> [f64; 3] {
        std::array::from_fn(|k| self.u[k] * d[0] + self.v[k] * d[1])
    }

    /// Flat form for FFI: origin, u, v, n.
    pub fn to_array(&self) -> [f64; 12] {
        let mut out = [0.0; 12];
        for (i, v) in [self.origin, self.u, self.v, self.n].iter().enumerate() {
            out[3 * i..3 * i + 3].copy_from_slice(v);
        }
        out
    }

    /// World point → sketch coordinates (orthogonal projection onto the plane).
    pub fn to_local(&self, w: [f64; 3]) -> [f64; 2] {
        let d: [f64; 3] = std::array::from_fn(|k| w[k] - self.origin[k]);
        [dot(d, self.u), dot(d, self.v)]
    }

    /// Intersection of a ray with the plane in sketch coordinates; `None`
    /// if the ray is (nearly) parallel or the hit lies behind the origin.
    pub fn ray_hit(&self, origin: [f64; 3], dir: [f64; 3]) -> Option<[f64; 2]> {
        let denom = dot(dir, self.n);
        if denom.abs() < 1e-9 {
            return None;
        }
        let to_plane: [f64; 3] = std::array::from_fn(|k| self.origin[k] - origin[k]);
        let t = dot(to_plane, self.n) / denom;
        (t >= 0.0).then(|| self.to_local(std::array::from_fn(|k| origin[k] + dir[k] * t)))
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SketchPoint {
    pub id: EntityId,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub construction: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CurveKind {
    Line {
        start: EntityId,
        end: EntityId,
    },
    Circle {
        center: EntityId,
        radius: f64,
    },
    /// Counter-clockwise arc around `center` from `start` to `end`. The
    /// radius is |start − center|; `end` lies on that circle (tools and
    /// `Sketch::repair_arcs` keep it there).
    Arc {
        center: EntityId,
        start: EntityId,
        end: EntityId,
    },
}

/// Resolved arc: centre, radius, start angle and CCW sweep (radians, (0, 2π]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcGeometry {
    pub center: [f64; 2],
    pub radius: f64,
    pub start_angle: f64,
    pub sweep: f64,
}

impl ArcGeometry {
    pub fn from_points(center: [f64; 2], start: [f64; 2], end: [f64; 2]) -> Self {
        let a0 = (start[1] - center[1]).atan2(start[0] - center[0]);
        let a1 = (end[1] - center[1]).atan2(end[0] - center[0]);
        let mut sweep = (a1 - a0).rem_euclid(std::f64::consts::TAU);
        if sweep < 1e-12 {
            sweep = std::f64::consts::TAU;
        }
        Self {
            center,
            radius: (start[0] - center[0]).hypot(start[1] - center[1]),
            start_angle: a0,
            sweep,
        }
    }

    pub fn point_at(&self, angle: f64) -> [f64; 2] {
        [self.center[0] + self.radius * angle.cos(), self.center[1] + self.radius * angle.sin()]
    }

    pub fn end_angle(&self) -> f64 {
        self.start_angle + self.sweep
    }

    pub fn mid(&self) -> [f64; 2] {
        self.point_at(self.start_angle + 0.5 * self.sweep)
    }

    /// Whether the direction `angle` lies within the arc.
    pub fn contains_angle(&self, angle: f64) -> bool {
        (angle - self.start_angle).rem_euclid(std::f64::consts::TAU) <= self.sweep + 1e-12
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SketchCurve {
    pub id: EntityId,
    #[serde(flatten)]
    pub kind: CurveKind,
    /// Construction geometry: shown dashed, never part of a profile.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub construction: bool,
}

impl SketchCurve {
    /// Point ids this curve uses.
    pub fn points(&self) -> Vec<EntityId> {
        match self.kind {
            CurveKind::Line { start, end } => vec![start, end],
            CurveKind::Circle { center, .. } => vec![center],
            CurveKind::Arc { center, start, end } => vec![center, start, end],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    pub plane: PlaneRef,
    #[serde(default)]
    pub points: Vec<SketchPoint>,
    #[serde(default)]
    pub curves: Vec<SketchCurve>,
    /// Next free entity id (ids are never reused, ADR-0024).
    pub next_id: u32,
    /// Shown outside sketch mode (browser eye toggle, SK-VS-10).
    #[serde(default = "default_true")]
    pub visible: bool,
}

fn default_true() -> bool {
    true
}

impl Sketch {
    pub fn new(plane: PlaneRef) -> Self {
        Self { plane, points: Vec::new(), curves: Vec::new(), next_id: 1, visible: true }
    }

    fn take_id(&mut self) -> EntityId {
        let id = EntityId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn point(&self, id: EntityId) -> Option<&SketchPoint> {
        self.points.iter().find(|p| p.id == id)
    }

    pub fn point_mut(&mut self, id: EntityId) -> Option<&mut SketchPoint> {
        self.points.iter_mut().find(|p| p.id == id)
    }

    pub fn curve(&self, id: EntityId) -> Option<&SketchCurve> {
        self.curves.iter().find(|c| c.id == id)
    }

    pub fn curve_mut(&mut self, id: EntityId) -> Option<&mut SketchCurve> {
        self.curves.iter_mut().find(|c| c.id == id)
    }

    /// Position of a point; panics only on a corrupt sketch (validated on load).
    pub fn pos(&self, id: EntityId) -> [f64; 2] {
        let p = self.point(id).expect("curve references an existing point");
        [p.x, p.y]
    }

    pub fn add_point(&mut self, at: [f64; 2]) -> EntityId {
        let id = self.take_id();
        self.points.push(SketchPoint { id, x: at[0], y: at[1], construction: false });
        id
    }

    pub fn add_line(&mut self, start: EntityId, end: EntityId) -> EntityId {
        let id = self.take_id();
        self.curves.push(SketchCurve {
            id,
            kind: CurveKind::Line { start, end },
            construction: false,
        });
        id
    }

    pub fn add_circle(&mut self, center: EntityId, radius: f64) -> EntityId {
        let id = self.take_id();
        self.curves.push(SketchCurve {
            id,
            kind: CurveKind::Circle { center, radius },
            construction: false,
        });
        id
    }

    pub fn add_arc(&mut self, center: EntityId, start: EntityId, end: EntityId) -> EntityId {
        let id = self.take_id();
        self.curves.push(SketchCurve {
            id,
            kind: CurveKind::Arc { center, start, end },
            construction: false,
        });
        id
    }

    /// Geometry of an arc curve.
    pub fn arc(&self, center: EntityId, start: EntityId, end: EntityId) -> ArcGeometry {
        ArcGeometry::from_points(self.pos(center), self.pos(start), self.pos(end))
    }

    /// Moves every arc end point back onto its circle (after a point was
    /// dragged). The start point defines the radius.
    pub fn repair_arcs(&mut self) {
        let arcs: Vec<(EntityId, EntityId, EntityId)> = self
            .curves
            .iter()
            .filter_map(|c| match c.kind {
                CurveKind::Arc { center, start, end } => Some((center, start, end)),
                _ => None,
            })
            .collect();
        for (center, start, end) in arcs {
            let (c, a, e) = (self.pos(center), self.pos(start), self.pos(end));
            let r = (a[0] - c[0]).hypot(a[1] - c[1]);
            let d = (e[0] - c[0]).hypot(e[1] - c[1]);
            if d > 1e-12
                && (d - r).abs() > 1e-9
                && let Some(p) = self.point_mut(end)
            {
                p.x = c[0] + (e[0] - c[0]) * r / d;
                p.y = c[1] + (e[1] - c[1]) * r / d;
            }
        }
    }

    /// Deletes entities: a deleted point takes every curve using it along; a
    /// deleted curve leaves shared points, and points no longer used by any
    /// curve and not explicitly kept are removed (ADR-0024).
    pub fn delete(&mut self, ids: &[EntityId]) {
        let mut dead_points: Vec<EntityId> =
            ids.iter().copied().filter(|id| self.point(*id).is_some()).collect();
        let dead_curves: Vec<EntityId> = self
            .curves
            .iter()
            .filter(|c| ids.contains(&c.id) || c.points().iter().any(|p| dead_points.contains(p)))
            .map(|c| c.id)
            .collect();
        let freed: Vec<EntityId> = self
            .curves
            .iter()
            .filter(|c| dead_curves.contains(&c.id))
            .flat_map(SketchCurve::points)
            .collect();
        self.curves.retain(|c| !dead_curves.contains(&c.id));
        for p in freed {
            if !self.curves.iter().any(|c| c.points().contains(&p)) {
                dead_points.push(p);
            }
        }
        self.points.retain(|p| !dead_points.contains(&p.id));
    }

    /// Curves using a point.
    pub fn curves_at(&self, point: EntityId) -> impl Iterator<Item = &SketchCurve> {
        self.curves.iter().filter(move |c| c.points().contains(&point))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_frames_are_right_handed_and_match_the_viewcube() {
        for plane in OriginPlane::ALL {
            let f = plane.frame();
            let cross = [
                f.u[1] * f.v[2] - f.u[2] * f.v[1],
                f.u[2] * f.v[0] - f.u[0] * f.v[2],
                f.u[0] * f.v[1] - f.u[1] * f.v[0],
            ];
            assert_eq!(cross, f.n, "{plane:?}");
        }
        assert_eq!(OriginPlane::XZ.frame().n, [0.0, -1.0, 0.0], "front plane faces −Y");
    }

    #[test]
    fn local_world_round_trip_and_ray_hit() {
        let f = OriginPlane::XZ.frame();
        let w = f.to_world([3.0, 4.0]);
        assert_eq!(w, [3.0, 0.0, 4.0]);
        assert_eq!(f.to_local(w), [3.0, 4.0]);
        let hit = f.ray_hit([3.0, -50.0, 4.0], [0.0, 1.0, 0.0]).unwrap();
        assert!((hit[0] - 3.0).abs() < 1e-12 && (hit[1] - 4.0).abs() < 1e-12);
        assert!(f.ray_hit([0.0, -50.0, 0.0], [1.0, 0.0, 0.0]).is_none(), "parallel ray");
    }

    #[test]
    fn shared_points_and_deletion() {
        let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
        let a = s.add_point([0.0, 0.0]);
        let b = s.add_point([10.0, 0.0]);
        let c = s.add_point([10.0, 10.0]);
        let ab = s.add_line(a, b);
        let bc = s.add_line(b, c);
        assert_eq!(s.curves_at(b).count(), 2);

        s.delete(&[ab]);
        assert!(s.point(a).is_none(), "orphaned start point removed");
        assert!(s.point(b).is_some(), "shared point kept");

        s.delete(&[c]);
        assert!(s.curve(bc).is_none(), "deleting a point deletes its curves");
        assert!(s.points.is_empty());
        assert_eq!(s.next_id, 6, "ids are never reused");
    }
}
