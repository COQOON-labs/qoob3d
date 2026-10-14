//! Solid features (ADR-0031): everything after the sketches that makes or
//! changes bodies. Parameters only – the kernel derives the geometry.

use serde::{Deserialize, Serialize};

use super::{EntityId, FeatureId, PlaneRef};

/// A closed region of a sketch (ADR-0011): the sorted ids of its boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileRef {
    pub sketch: FeatureId,
    pub key: Vec<EntityId>,
}

/// What a feature does with its result (Fusion: "Operation").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyOp {
    /// A separate new body.
    #[default]
    NewBody,
    /// Fuse with every body the result touches (new body if none).
    Join,
    /// Subtract from every body the result overlaps.
    Cut,
}

impl BodyOp {
    pub const ALL: [BodyOp; 3] = [BodyOp::NewBody, BodyOp::Join, BodyOp::Cut];
}

/// Extrusion of profiles along the sketch normal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extrude {
    pub profiles: Vec<ProfileRef>,
    /// mm along the plane normal; negative = opposite direction.
    pub distance: f64,
    /// Centred on the sketch plane; `distance` is the total length.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub symmetric: bool,
    #[serde(default)]
    pub operation: BodyOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OriginAxis {
    X,
    Y,
    Z,
}

impl OriginAxis {
    pub const ALL: [OriginAxis; 3] = [OriginAxis::X, OriginAxis::Y, OriginAxis::Z];

    pub fn direction(self) -> [f64; 3] {
        match self {
            OriginAxis::X => [1.0, 0.0, 0.0],
            OriginAxis::Y => [0.0, 1.0, 0.0],
            OriginAxis::Z => [0.0, 0.0, 1.0],
        }
    }
}

/// Rotation axis of a revolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AxisRef {
    Origin(OriginAxis),
    /// A line of a sketch (construction lines too).
    SketchLine {
        sketch: FeatureId,
        line: EntityId,
    },
}

/// Rotation of profiles around an axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revolve {
    pub profiles: Vec<ProfileRef>,
    pub axis: AxisRef,
    /// Degrees, (0, 360].
    pub angle_deg: f64,
    #[serde(default)]
    pub operation: BodyOp,
}

/// One section of a loft.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LoftSection {
    Profile(ProfileRef),
    /// A sketch point: the loft ends in a tip (pyramid, cone).
    Point {
        sketch: FeatureId,
        point: EntityId,
    },
}

/// Ruled loft through sections in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Loft {
    pub sections: Vec<LoftSection>,
    #[serde(default)]
    pub operation: BodyOp,
}

/// Construction plane parallel to another plane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstructionPlane {
    pub base: PlaneRef,
    /// mm along the base normal.
    pub offset: f64,
}

/// Involute spur gear, centred on its plane origin, extruded along the
/// normal (ADR-0032).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpurGear {
    pub plane: PlaneRef,
    /// Module m (mm): pitch diameter = m · teeth.
    pub module: f64,
    pub teeth: u32,
    pub pressure_angle_deg: f64,
    pub thickness: f64,
    /// Diameter of the centre hole, 0 = none.
    #[serde(default)]
    pub bore: f64,
    #[serde(default)]
    pub operation: BodyOp,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoilKind {
    /// Helical spring: rises by `pitch` per revolution.
    #[default]
    Helix,
    /// Flat spiral in the plane: grows outwards by `pitch` per revolution.
    Spiral,
}

/// Coil / spiral: a round wire swept along a helix or flat spiral (ADR-0032).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coil {
    pub plane: PlaneRef,
    #[serde(default)]
    pub kind: CoilKind,
    /// Diameter of the path (helix) or start diameter (spiral), mm.
    pub diameter: f64,
    pub revolutions: f64,
    /// mm per revolution.
    pub pitch: f64,
    /// Wire (section) diameter, mm.
    pub section: f64,
    #[serde(default)]
    pub operation: BodyOp,
}

/// Plausibility limits for untrusted files (ADR-0030).
pub(super) const MAX_LENGTH: f64 = 1.0e5;

pub(super) fn positive(name: &str, v: f64) -> Result<(), String> {
    if v.is_finite() && v > 1e-6 && v <= MAX_LENGTH {
        Ok(())
    } else {
        Err(format!("{name} must be positive and at most {MAX_LENGTH} mm (got {v})"))
    }
}

pub(super) fn finite(name: &str, v: f64) -> Result<(), String> {
    if v.is_finite() && v.abs() <= MAX_LENGTH {
        Ok(())
    } else {
        Err(format!("{name} is out of range (got {v})"))
    }
}

impl Extrude {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.profiles.is_empty() {
            return Err("no profile selected".into());
        }
        positive("distance", self.distance.abs())
    }
}

impl Revolve {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.profiles.is_empty() {
            return Err("no profile selected".into());
        }
        if !(self.angle_deg.is_finite() && self.angle_deg > 0.0 && self.angle_deg <= 360.0) {
            return Err(format!("angle must be in (0, 360] degrees (got {})", self.angle_deg));
        }
        Ok(())
    }
}

impl Loft {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.sections.len() < 2 {
            return Err("a loft needs at least two sections".into());
        }
        let points = self.sections.iter().filter(|s| matches!(s, LoftSection::Point { .. }));
        let inner_point = self.sections[1..self.sections.len() - 1]
            .iter()
            .any(|s| matches!(s, LoftSection::Point { .. }));
        if points.count() >= self.sections.len() || inner_point {
            return Err("points are only allowed as first or last section".into());
        }
        Ok(())
    }
}

impl SpurGear {
    /// Root diameter (mm), standard tooth proportions (dedendum 1.25 m).
    pub fn root_diameter(&self) -> f64 {
        self.module * (f64::from(self.teeth) - 2.5)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        positive("module", self.module)?;
        positive("thickness", self.thickness)?;
        if !(6..=400).contains(&self.teeth) {
            return Err(format!("teeth must be 6…400 (got {})", self.teeth));
        }
        if !(self.pressure_angle_deg.is_finite()
            && (10.0..=35.0).contains(&self.pressure_angle_deg))
        {
            return Err("pressure angle must be 10…35°".into());
        }
        if !(self.bore.is_finite() && self.bore >= 0.0 && self.bore < 0.9 * self.root_diameter()) {
            return Err("bore must be smaller than the root diameter".into());
        }
        Ok(())
    }
}

impl Coil {
    pub(super) fn validate(&self) -> Result<(), String> {
        positive("diameter", self.diameter)?;
        positive("pitch", self.pitch)?;
        positive("section", self.section)?;
        if !(self.revolutions.is_finite() && (0.1..=500.0).contains(&self.revolutions)) {
            return Err("revolutions must be 0.1…500".into());
        }
        if self.section >= self.diameter {
            return Err("the wire must be thinner than the coil diameter".into());
        }
        if self.section >= self.pitch {
            return Err("pitch must be larger than the wire diameter (turns would touch)".into());
        }
        Ok(())
    }
}
