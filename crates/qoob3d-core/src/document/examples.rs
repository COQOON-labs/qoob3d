//! Reference models built the way a user builds them in the UI (sketch →
//! feature). They are the single source for `models/*.qbd` (written by
//! `cargo xtask models --write`), the kernel model tests and the docs
//! (ADR-0033). Each function documents the steps in the UI.

use super::{
    AxisRef, BodyOp, Coil, CoilKind, ConstructionPlane, Document, EntityId, Extrude, FeatureId,
    FeatureKind, Loft, LoftSection, OriginPlane, PlaneRef, ProfileRef, Revolve, Sketch, SpurGear,
};

/// All reference models: (file stem, document).
pub fn all() -> Vec<(&'static str, Document)> {
    vec![
        ("box", cuboid(40.0, 25.0, 10.0)),
        ("cylinder", cylinder(10.0, 30.0)),
        ("sphere", sphere(15.0)),
        ("pyramid", pyramid(30.0, 25.0)),
        ("gear", gear()),
        ("coil", coil(CoilKind::Helix)),
        ("spiral", coil(CoilKind::Spiral)),
        ("plate-with-hole", plate_with_hole()),
    ]
}

fn xy() -> PlaneRef {
    PlaneRef::Origin(OriginPlane::XY)
}

/// Adds a sketch and returns its id.
fn add_sketch(doc: &mut Document, sketch: Sketch) -> FeatureId {
    let n = doc.sketch_count() + 1;
    doc.add_feature(format!("Sketch{n}"), FeatureKind::Sketch(sketch))
}

fn add(doc: &mut Document, name: &str, kind: FeatureKind) -> FeatureId {
    let n = doc.count_of(kind.kind_id()) + 1;
    doc.add_feature(format!("{name}{n}"), kind)
}

/// Rectangle from (x0, y0) to (x1, y1); returns the sorted line ids (= profile key).
pub fn rectangle(s: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<EntityId> {
    let p = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|q| s.add_point(q));
    (0..4).map(|i| s.add_line(p[i], p[(i + 1) % 4])).collect()
}

fn profile(sketch: FeatureId, mut key: Vec<EntityId>) -> ProfileRef {
    key.sort_unstable();
    ProfileRef { sketch, key }
}

/// Quader: Sketch on XY → rectangle (R) w × d from the origin → Extrude h.
pub fn cuboid(w: f64, d: f64, h: f64) -> Document {
    let mut doc = Document::default();
    let mut s = Sketch::new(xy());
    let key = rectangle(&mut s, 0.0, 0.0, w, d);
    let sketch = add_sketch(&mut doc, s);
    let profiles = vec![profile(sketch, key)];
    let kind = FeatureKind::Extrude(Extrude {
        profiles,
        distance: h,
        symmetric: false,
        operation: BodyOp::NewBody,
    });
    add(&mut doc, "Extrude", kind);
    doc
}

/// Zylinder: Sketch on XY → circle (C) radius r at the origin → Extrude h.
pub fn cylinder(r: f64, h: f64) -> Document {
    let mut doc = Document::default();
    let mut s = Sketch::new(xy());
    let c = s.add_point([0.0, 0.0]);
    let circle = s.add_circle(c, r);
    let sketch = add_sketch(&mut doc, s);
    let kind = FeatureKind::Extrude(Extrude {
        profiles: vec![profile(sketch, vec![circle])],
        distance: h,
        symmetric: false,
        operation: BodyOp::NewBody,
    });
    add(&mut doc, "Extrude", kind);
    doc
}

/// Kugel: Sketch on XZ → arc (A) centre origin from (r, 0) to (−r, 0) →
/// line closing the half disc → Revolve 360° around that line.
pub fn sphere(r: f64) -> Document {
    let mut doc = Document::default();
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XZ));
    let c = s.add_point([0.0, 0.0]);
    let a = s.add_point([r, 0.0]);
    let b = s.add_point([-r, 0.0]);
    let arc = s.add_arc(c, a, b);
    let line = s.add_line(b, a);
    let sketch = add_sketch(&mut doc, s);
    let kind = FeatureKind::Revolve(Revolve {
        profiles: vec![profile(sketch, vec![arc, line])],
        axis: AxisRef::SketchLine { sketch, line },
        angle_deg: 360.0,
        operation: BodyOp::NewBody,
    });
    add(&mut doc, "Revolve", kind);
    doc
}

/// Pyramide: Sketch on XY → centred square a → offset plane h above XY →
/// Sketch on it with a point at its origin → Loft square → point.
pub fn pyramid(a: f64, h: f64) -> Document {
    let mut doc = Document::default();
    let mut base = Sketch::new(xy());
    let key = rectangle(&mut base, -0.5 * a, -0.5 * a, 0.5 * a, 0.5 * a);
    let base = add_sketch(&mut doc, base);
    let plane =
        add(&mut doc, "Plane", FeatureKind::Plane(ConstructionPlane { base: xy(), offset: h }));
    let mut tip = Sketch::new(PlaneRef::Construction(plane));
    let apex = tip.add_point([0.0, 0.0]);
    let tip = add_sketch(&mut doc, tip);
    let kind = FeatureKind::Loft(Loft {
        sections: vec![
            LoftSection::Profile(profile(base, key)),
            LoftSection::Point { sketch: tip, point: apex },
        ],
        operation: BodyOp::NewBody,
    });
    add(&mut doc, "Loft", kind);
    doc
}

/// Zahnrad: Create → Spur Gear on XY, module 2, 20 teeth, 20°, 8 mm thick,
/// 8 mm bore.
pub fn gear() -> Document {
    let mut doc = Document::default();
    let kind = FeatureKind::SpurGear(SpurGear {
        plane: xy(),
        module: 2.0,
        teeth: 20,
        pressure_angle_deg: 20.0,
        thickness: 8.0,
        bore: 8.0,
        operation: BodyOp::NewBody,
    });
    add(&mut doc, "Gear", kind);
    doc
}

/// Spirale: Create → Coil on XY. Helix: Ø20, 5 turns, pitch 4, wire 2.
/// Spiral: flat, starts at Ø10, 4 turns, grows 4 mm per turn, wire 2.
pub fn coil(kind: CoilKind) -> Document {
    let mut doc = Document::default();
    let coil = match kind {
        CoilKind::Helix => Coil {
            plane: xy(),
            kind,
            diameter: 20.0,
            revolutions: 5.0,
            pitch: 4.0,
            section: 2.0,
            operation: BodyOp::NewBody,
        },
        CoilKind::Spiral => Coil {
            plane: xy(),
            kind,
            diameter: 10.0,
            revolutions: 4.0,
            pitch: 4.0,
            section: 2.0,
            operation: BodyOp::NewBody,
        },
    };
    add(&mut doc, "Coil", FeatureKind::Coil(coil));
    doc
}

/// Join and Cut: 60×40×5 plate, a Ø10 boss joined on top, a Ø6 hole cut
/// through both.
pub fn plate_with_hole() -> Document {
    let mut doc = Document::default();
    let mut s = Sketch::new(xy());
    let key = rectangle(&mut s, -30.0, -20.0, 30.0, 20.0);
    let plate = add_sketch(&mut doc, s);
    add(
        &mut doc,
        "Extrude",
        FeatureKind::Extrude(Extrude {
            profiles: vec![profile(plate, key)],
            distance: 5.0,
            symmetric: false,
            operation: BodyOp::NewBody,
        }),
    );
    let plane =
        add(&mut doc, "Plane", FeatureKind::Plane(ConstructionPlane { base: xy(), offset: 5.0 }));
    let mut s = Sketch::new(PlaneRef::Construction(plane));
    let c = s.add_point([0.0, 0.0]);
    let boss = s.add_circle(c, 5.0);
    let hole = s.add_circle(c, 3.0);
    let top = add_sketch(&mut doc, s);
    add(
        &mut doc,
        "Extrude",
        FeatureKind::Extrude(Extrude {
            // The ring between the two circles plus the inner disc.
            profiles: vec![profile(top, vec![boss, hole]), profile(top, vec![hole])],
            distance: 8.0,
            symmetric: false,
            operation: BodyOp::Join,
        }),
    );
    add(
        &mut doc,
        "Extrude",
        FeatureKind::Extrude(Extrude {
            // Symmetric through plate and boss.
            profiles: vec![profile(top, vec![hole])],
            distance: 40.0,
            symmetric: true,
            operation: BodyOp::Cut,
        }),
    );
    doc
}

/// One document with every feature and curve kind – the source of the
/// format v2 fixture (`tests/fixtures/format_v2.qbd`, never regenerated).
pub fn all_kinds() -> Document {
    // Pyramid: sketch, construction plane, sketch on it, loft.
    let mut doc = pyramid(30.0, 25.0);
    // Sphere next to it: arc + line on XZ, revolve around the line.
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XZ));
    let c = s.add_point([60.0, 15.0]);
    let a = s.add_point([75.0, 15.0]);
    let b = s.add_point([45.0, 15.0]);
    let arc = s.add_arc(c, a, b);
    let line = s.add_line(b, a);
    let sketch = add_sketch(&mut doc, s);
    let kind = FeatureKind::Revolve(Revolve {
        profiles: vec![profile(sketch, vec![arc, line])],
        axis: AxisRef::SketchLine { sketch, line },
        angle_deg: 360.0,
        operation: BodyOp::NewBody,
    });
    add(&mut doc, "Revolve", kind);
    // Boss joined onto the pyramid base, hole cut through it.
    let mut s = Sketch::new(xy());
    let c = s.add_point([0.0, 0.0]);
    let boss = s.add_circle(c, 20.0);
    let sketch = add_sketch(&mut doc, s);
    let extrude = |profiles, distance, operation| {
        FeatureKind::Extrude(Extrude { profiles, distance, symmetric: false, operation })
    };
    add(&mut doc, "Extrude", extrude(vec![profile(sketch, vec![boss])], -3.0, BodyOp::Join));
    let mut s = Sketch::new(xy());
    let c = s.add_point([0.0, 0.0]);
    let hole = s.add_circle(c, 2.0);
    let sketch = add_sketch(&mut doc, s);
    add(&mut doc, "Extrude", extrude(vec![profile(sketch, vec![hole])], -3.0, BodyOp::Cut));
    // Gear and coil on the construction plane.
    let plane = doc.planes().next().map(|(id, _)| id).expect("pyramid has a plane");
    let mut g = gear();
    if let FeatureKind::SpurGear(gear) = &mut g.features[0].kind {
        gear.plane = PlaneRef::Construction(plane);
        add(&mut doc, "Gear", FeatureKind::SpurGear(gear.clone()));
    }
    let c = coil(CoilKind::Spiral);
    if let FeatureKind::Coil(coil) = &c.features[0].kind {
        add(&mut doc, "Coil", FeatureKind::Coil(coil.clone()));
    }
    doc
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_kinds_covers_every_feature_kind() {
        let doc = super::all_kinds();
        doc.validate().unwrap();
        for kind in super::FeatureKind::KIND_IDS {
            assert!(doc.count_of(kind) > 0, "{kind} missing");
        }
    }

    #[test]
    fn every_example_is_valid_and_round_trips() {
        for (name, doc) in super::all() {
            doc.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
            let back = super::Document::from_json(&doc.to_json()).unwrap();
            assert_eq!(back, doc, "{name}");
        }
    }
}
