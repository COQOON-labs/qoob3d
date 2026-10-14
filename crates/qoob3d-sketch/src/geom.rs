//! 2D geometry helpers in sketch coordinates (mm).

use glam::DVec2;
use qoob3d_core::document::{CurveKind, Sketch, SketchCurve};

pub fn v(p: [f64; 2]) -> DVec2 {
    DVec2::new(p[0], p[1])
}

/// Closest point on segment `a`–`b` to `p`.
pub fn closest_on_segment(p: DVec2, a: DVec2, b: DVec2) -> DVec2 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-24 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
}

/// Closest point on a circle to `p` (any point if `p` is the centre).
pub fn closest_on_circle(p: DVec2, c: DVec2, r: f64) -> DVec2 {
    let d = p - c;
    if d.length_squared() < 1e-24 { c + DVec2::new(r, 0.0) } else { c + d.normalize() * r }
}

/// A curve resolved to coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Segment(DVec2, DVec2),
    Circle(DVec2, f64),
    /// Centre, radius, start angle, CCW sweep (radians).
    Arc(DVec2, f64, f64, f64),
}

pub fn shape(sketch: &Sketch, curve: &SketchCurve) -> Shape {
    match curve.kind {
        CurveKind::Line { start, end } => Shape::Segment(v(sketch.pos(start)), v(sketch.pos(end))),
        CurveKind::Circle { center, radius } => Shape::Circle(v(sketch.pos(center)), radius),
        CurveKind::Arc { center, start, end } => {
            let g = sketch.arc(center, start, end);
            Shape::Arc(v(g.center), g.radius, g.start_angle, g.sweep)
        }
    }
}

impl Shape {
    pub fn closest(&self, p: DVec2) -> DVec2 {
        match *self {
            Shape::Segment(a, b) => closest_on_segment(p, a, b),
            Shape::Circle(c, r) => closest_on_circle(p, c, r),
            Shape::Arc(c, r, a0, sweep) => {
                let q = closest_on_circle(p, c, r);
                if arc_contains(a0, sweep, angle(c, q)) {
                    return q;
                }
                let (s, e) = arc_ends(c, r, a0, sweep);
                if s.distance(p) <= e.distance(p) { s } else { e }
            }
        }
    }

    /// The circle a circle or arc lies on.
    pub fn circle(&self) -> Option<(DVec2, f64)> {
        match *self {
            Shape::Circle(c, r) | Shape::Arc(c, r, ..) => Some((c, r)),
            Shape::Segment(..) => None,
        }
    }

    /// Whether a point on the shape's circle lies on the shape itself.
    fn keeps(&self, p: DVec2) -> bool {
        match *self {
            Shape::Arc(c, _, a0, sweep) => arc_contains(a0, sweep, angle(c, p)),
            _ => true,
        }
    }
}

fn angle(c: DVec2, p: DVec2) -> f64 {
    (p.y - c.y).atan2(p.x - c.x)
}

fn arc_contains(a0: f64, sweep: f64, a: f64) -> bool {
    (a - a0).rem_euclid(std::f64::consts::TAU) <= sweep + 1e-9
}

/// Start and end point of an arc.
pub fn arc_ends(c: DVec2, r: f64, a0: f64, sweep: f64) -> (DVec2, DVec2) {
    let at = |a: f64| c + DVec2::new(a.cos(), a.sin()) * r;
    (at(a0), at(a0 + sweep))
}

/// Polyline approximation of an arc for drawing.
pub fn arc_polyline(c: DVec2, r: f64, a0: f64, sweep: f64, segments: usize) -> Vec<DVec2> {
    let n = segments.max(4);
    (0..=n)
        .map(|i| {
            let a = a0 + sweep * i as f64 / n as f64;
            c + DVec2::new(a.cos(), a.sin()) * r
        })
        .collect()
}

impl Shape {
    pub fn distance(&self, p: DVec2) -> f64 {
        self.closest(p).distance(p)
    }
}

/// Intersection points of two shapes (segments are bounded).
pub fn intersections(a: Shape, b: Shape) -> Vec<DVec2> {
    match (a, b) {
        (Shape::Segment(p1, p2), Shape::Segment(q1, q2)) => {
            segment_segment(p1, p2, q1, q2).into_iter().collect()
        }
        (Shape::Segment(p1, p2), Shape::Circle(c, r))
        | (Shape::Circle(c, r), Shape::Segment(p1, p2)) => segment_circle(p1, p2, c, r),
        (Shape::Circle(c1, r1), Shape::Circle(c2, r2)) => circle_circle(c1, r1, c2, r2),
        // Arcs: intersect their full circles, keep points on both shapes.
        (a, b) => {
            let full = |s: Shape| match s {
                Shape::Arc(c, r, ..) => Shape::Circle(c, r),
                other => other,
            };
            intersections(full(a), full(b))
                .into_iter()
                .filter(|p| a.keeps(*p) && b.keeps(*p))
                .collect()
        }
    }
}

fn cross(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn segment_segment(p1: DVec2, p2: DVec2, q1: DVec2, q2: DVec2) -> Option<DVec2> {
    let r = p2 - p1;
    let s = q2 - q1;
    let denom = cross(r, s);
    if denom.abs() < 1e-12 {
        return None; // parallel or collinear
    }
    let t = cross(q1 - p1, s) / denom;
    let u = cross(q1 - p1, r) / denom;
    let eps = 1e-9;
    ((-eps..=1.0 + eps).contains(&t) && (-eps..=1.0 + eps).contains(&u)).then(|| p1 + r * t)
}

fn segment_circle(p1: DVec2, p2: DVec2, c: DVec2, r: f64) -> Vec<DVec2> {
    let d = p2 - p1;
    let f = p1 - c;
    let a = d.dot(d);
    if a < 1e-24 {
        return Vec::new();
    }
    let b = 2.0 * f.dot(d);
    let disc = b * b - 4.0 * a * (f.dot(f) - r * r);
    if disc < 0.0 {
        return Vec::new();
    }
    let sq = disc.sqrt();
    let mut out = Vec::new();
    for t in [(-b - sq) / (2.0 * a), (-b + sq) / (2.0 * a)] {
        if (-1e-9..=1.0 + 1e-9).contains(&t) {
            let p = p1 + d * t;
            if out.iter().all(|q: &DVec2| q.distance(p) > 1e-9) {
                out.push(p);
            }
        }
    }
    out
}

fn circle_circle(c1: DVec2, r1: f64, c2: DVec2, r2: f64) -> Vec<DVec2> {
    let d = c1.distance(c2);
    if d < 1e-12 || d > r1 + r2 || d < (r1 - r2).abs() {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();
    let dir = (c2 - c1) / d;
    let mid = c1 + dir * a;
    let perp = DVec2::new(-dir.y, dir.x);
    if h < 1e-12 { vec![mid] } else { vec![mid + perp * h, mid - perp * h] }
}

/// Polyline approximation of a circle for drawing (closed: first = last).
pub fn circle_polyline(c: DVec2, r: f64, segments: usize) -> Vec<DVec2> {
    let n = segments.max(8);
    (0..=n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            c + DVec2::new(a.cos(), a.sin()) * r
        })
        .collect()
}

#[cfg(test)]
mod tests;
