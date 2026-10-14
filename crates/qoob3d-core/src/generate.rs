//! Pure geometry of the generated features (ADR-0032): the involute outline
//! of a spur gear and the path of a coil. The kernel turns these into
//! solids; keeping the math here makes it testable without OpenCascade.

use std::f64::consts::{PI, TAU};

use crate::document::{Coil, CoilKind, SpurGear};

/// How the kernel connects the points of a piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceKind {
    /// Straight segments between consecutive points.
    Lines,
    /// One smooth interpolating spline through all points.
    Spline,
}

/// A closed planar outline made of pieces; the last point of each piece is
/// the first point of the next, and the last piece ends at the very first point.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    pub pieces: Vec<(PieceKind, Vec<[f64; 2]>)>,
}

impl Outline {
    /// Dense polygon approximation (for area checks and previews).
    pub fn polygon(&self) -> Vec<[f64; 2]> {
        let mut out = Vec::new();
        for (_, pts) in &self.pieces {
            out.extend_from_slice(&pts[..pts.len() - 1]);
        }
        out
    }

    /// Area by the shoelace formula of [`Outline::polygon`] (mm²).
    pub fn area(&self) -> f64 {
        let p = self.polygon();
        let n = p.len();
        0.5 * (0..n)
            .map(|i| {
                let (a, b) = (p[i], p[(i + 1) % n]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
    }
}

/// Points per involute flank and per root / tip arc.
const FLANK_POINTS: usize = 12;
const ARC_POINTS: usize = 5;

fn involute(angle: f64) -> f64 {
    angle.tan() - angle
}

fn polar(r: f64, a: f64) -> [f64; 2] {
    [r * a.cos(), r * a.sin()]
}

/// Standard involute spur gear (addendum m, dedendum 1.25 m), centred on
/// the origin, first tooth on +x. Counter-clockwise outline.
pub fn gear_outline(g: &SpurGear) -> Outline {
    let z = f64::from(g.teeth);
    let alpha = g.pressure_angle_deg.to_radians();
    let rp = 0.5 * g.module * z;
    let rb = rp * alpha.cos();
    let ra = rp + g.module;
    let rf = (rp - 1.25 * g.module).max(0.1 * rp);
    // Half tooth angle at radius r (r ≥ rb).
    let half = |r: f64| PI / (2.0 * z) + involute(alpha) - involute((rb / r).min(1.0).acos());
    let flank_start = rf.max(rb);
    let pitch_angle = TAU / z;
    // Root arc must leave room between teeth.
    let root_half = half(flank_start).min(0.45 * pitch_angle);

    let mut pieces = Vec::new();
    for i in 0..g.teeth {
        let c = f64::from(i) * pitch_angle;
        // Lower flank (angle c − half), from root to tip.
        if rf < rb {
            pieces
                .push((PieceKind::Lines, vec![polar(rf, c - root_half), polar(rb, c - root_half)]));
        }
        let flank: Vec<f64> = (0..=FLANK_POINTS)
            .map(|k| flank_start + (ra - flank_start) * k as f64 / FLANK_POINTS as f64)
            .collect();
        let lower: Vec<[f64; 2]> = flank
            .iter()
            .map(|&r| polar(r, c - if r <= flank_start { root_half } else { half(r) }))
            .collect();
        pieces.push((PieceKind::Spline, lower));
        // Tip arc.
        let tip = half(ra);
        let tip_arc = (0..=ARC_POINTS)
            .map(|k| polar(ra, c - tip + 2.0 * tip * k as f64 / ARC_POINTS as f64))
            .collect();
        pieces.push((PieceKind::Spline, tip_arc));
        // Upper flank, from tip to root.
        let upper: Vec<[f64; 2]> = flank
            .iter()
            .rev()
            .map(|&r| polar(r, c + if r <= flank_start { root_half } else { half(r) }))
            .collect();
        pieces.push((PieceKind::Spline, upper));
        if rf < rb {
            pieces
                .push((PieceKind::Lines, vec![polar(rb, c + root_half), polar(rf, c + root_half)]));
        }
        // Root arc to the next tooth.
        let (a0, a1) = (c + root_half, c + pitch_angle - root_half);
        let root =
            (0..=ARC_POINTS).map(|k| polar(rf, a0 + (a1 - a0) * k as f64 / ARC_POINTS as f64));
        pieces.push((PieceKind::Spline, root.collect()));
    }
    // Chain exactly: rounding differs between pieces computed separately.
    for i in 1..pieces.len() {
        let prev = *pieces[i - 1].1.last().expect("pieces have points");
        pieces[i].1[0] = prev;
    }
    let first = pieces[0].1[0];
    if let Some(last) = pieces.last_mut().and_then(|(_, p)| p.last_mut()) {
        *last = first;
    }
    Outline { pieces }
}

/// Tip (outside) diameter of a gear.
pub fn gear_tip_diameter(g: &SpurGear) -> f64 {
    g.module * (f64::from(g.teeth) + 2.0)
}

/// Samples per revolution of a coil path.
const COIL_SAMPLES_PER_TURN: f64 = 36.0;

/// Path of the coil's wire centre in plane coordinates `[u, v, n]` (mm).
pub fn coil_path(c: &Coil) -> Vec<[f64; 3]> {
    let r0 = 0.5 * c.diameter;
    let n = (c.revolutions * COIL_SAMPLES_PER_TURN).ceil().max(8.0) as usize;
    (0..=n)
        .map(|i| {
            let turns = c.revolutions * i as f64 / n as f64;
            let a = TAU * turns;
            match c.kind {
                CoilKind::Helix => [r0 * a.cos(), r0 * a.sin(), c.pitch * turns],
                CoilKind::Spiral => {
                    let r = r0 + c.pitch * turns;
                    [r * a.cos(), r * a.sin(), 0.0]
                }
            }
        })
        .collect()
}

/// Exact length of the coil path (mm), for volume checks: V ≈ A · L.
pub fn coil_length(c: &Coil) -> f64 {
    match c.kind {
        CoilKind::Helix => c.revolutions * ((PI * c.diameter).powi(2) + c.pitch.powi(2)).sqrt(),
        CoilKind::Spiral => {
            // Archimedean spiral r = r0 + bθ: integrate numerically.
            let b = c.pitch / TAU;
            let r0 = 0.5 * c.diameter;
            let total = TAU * c.revolutions;
            let steps = 4096;
            let h = total / steps as f64;
            (0..steps)
                .map(|i| {
                    let t = (i as f64 + 0.5) * h;
                    let r = r0 + b * t;
                    (r * r + b * b).sqrt() * h
                })
                .sum()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{BodyOp, OriginPlane, PlaneRef};
    use proptest::prelude::*;

    fn gear(module: f64, teeth: u32, pa: f64) -> SpurGear {
        SpurGear {
            plane: PlaneRef::Origin(OriginPlane::XY),
            module,
            teeth,
            pressure_angle_deg: pa,
            thickness: 5.0,
            bore: 0.0,
            operation: BodyOp::NewBody,
        }
    }

    fn check_outline(o: &Outline) {
        // Pieces chain end to start and close.
        for w in o.pieces.windows(2) {
            assert_eq!(w[0].1.last(), w[1].1.first(), "pieces must connect");
        }
        assert_eq!(o.pieces.last().unwrap().1.last(), o.pieces[0].1.first(), "closed");
        assert!(o.pieces.iter().all(|(_, p)| p.len() >= 2));
    }

    #[test]
    fn gear_outline_is_closed_ccw_and_within_tip_and_root() {
        let g = gear(2.0, 20, 20.0);
        let o = gear_outline(&g);
        check_outline(&o);
        assert!(o.area() > 0.0, "counter-clockwise");
        let (ra, rf) = (0.5 * gear_tip_diameter(&g), 0.5 * g.root_diameter());
        for p in o.polygon() {
            let r = p[0].hypot(p[1]);
            assert!(r <= ra + 1e-9 && r >= rf - 1e-9, "{r}");
        }
        // Area between root and tip circle.
        assert!(o.area() > PI * rf * rf && o.area() < PI * ra * ra);
    }

    #[test]
    fn tooth_thickness_at_pitch_circle_is_half_the_pitch() {
        let g = gear(3.0, 24, 20.0);
        let rp = 0.5 * g.module * 24.0;
        // Count crossings of the pitch circle: two per tooth.
        let p = gear_outline(&g).polygon();
        let crossings = (0..p.len())
            .filter(|&i| {
                let (a, b) = (p[i], p[(i + 1) % p.len()]);
                (a[0].hypot(a[1]) - rp).signum() != (b[0].hypot(b[1]) - rp).signum()
            })
            .count();
        assert_eq!(crossings, 48);
    }

    proptest! {
        #[test]
        fn any_valid_gear_has_a_closed_outline(
            module in 0.3f64..10.0, teeth in 6u32..120, pa in 14.5f64..30.0
        ) {
            let o = gear_outline(&gear(module, teeth, pa));
            check_outline(&o);
            prop_assert!(o.area() > 0.0);
        }
    }

    #[test]
    fn coil_paths() {
        let mut c = Coil {
            plane: PlaneRef::Origin(OriginPlane::XY),
            kind: CoilKind::Helix,
            diameter: 20.0,
            revolutions: 5.0,
            pitch: 4.0,
            section: 2.0,
            operation: BodyOp::NewBody,
        };
        let p = coil_path(&c);
        assert_eq!(p[0], [10.0, 0.0, 0.0]);
        let end = p.last().unwrap();
        assert!((end[2] - 20.0).abs() < 1e-9 && (end[0] - 10.0).abs() < 1e-9);
        let poly: f64 = p.windows(2).map(|w| dist(w[0], w[1])).sum();
        assert!((poly - coil_length(&c)).abs() / poly < 3e-3, "chord error of 10° steps");

        c.kind = CoilKind::Spiral;
        let p = coil_path(&c);
        assert!(p.iter().all(|q| q[2] == 0.0), "flat");
        assert!((p.last().unwrap()[0] - 30.0).abs() < 1e-9, "grows by pitch per turn");
        // The polyline is a little shorter (chord error, 10° steps).
        let poly: f64 = p.windows(2).map(|w| dist(w[0], w[1])).sum();
        assert!((poly - coil_length(&c)).abs() / poly < 3e-3);
    }

    fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }
}
