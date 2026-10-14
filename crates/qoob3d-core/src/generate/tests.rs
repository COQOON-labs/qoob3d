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
