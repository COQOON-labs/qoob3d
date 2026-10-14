//! The document: the timeline of features, the single source of truth
//! (ADR-0030). Geometry is always derived by the kernel and never stored.
//!
//! File format `.qbd` (ADR-0030): JSON envelope
//! `{ "format": "qoob3d", "format_version": N, "document": {…} }`.
//! Loading validates everything before the document can reach the kernel.

pub mod examples;
pub mod feature;
pub mod sketch;

use serde::{Deserialize, Serialize};

pub use feature::{
    AxisRef, BodyOp, Coil, CoilKind, ConstructionPlane, Extrude, Loft, LoftSection, OriginAxis,
    ProfileRef, Revolve, SpurGear,
};
pub use sketch::{
    ArcGeometry, CurveKind, EntityId, OriginPlane, PlaneFrame, PlaneRef, Sketch, SketchCurve,
    SketchPoint,
};

pub const FORMAT: &str = "qoob3d";
/// File name extension of project files (ADR-0030).
pub const FILE_EXTENSION: &str = "qbd";
/// v2: solid features, arcs, construction planes (ADR-0031).
pub const FORMAT_VERSION: u32 = 2;
/// Refuse files larger than this before parsing (bytes).
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTITIES: usize = 100_000;
const MAX_FEATURES: usize = 10_000;
const MAX_COORD: f64 = 1.0e6;
const MAX_NAME: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FeatureId(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub id: FeatureId,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suppressed: bool,
    pub kind: FeatureKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FeatureKind {
    Sketch(Sketch),
    Plane(ConstructionPlane),
    Extrude(Extrude),
    Revolve(Revolve),
    Loft(Loft),
    SpurGear(SpurGear),
    Coil(Coil),
}

impl FeatureKind {
    /// Stable short id of the kind (i18n keys `feature-<id>-name`, icons,
    /// model checks in `cargo xtask models`).
    pub fn kind_id(&self) -> &'static str {
        match self {
            FeatureKind::Sketch(_) => "sketch",
            FeatureKind::Plane(_) => "plane",
            FeatureKind::Extrude(_) => "extrude",
            FeatureKind::Revolve(_) => "revolve",
            FeatureKind::Loft(_) => "loft",
            FeatureKind::SpurGear(_) => "gear",
            FeatureKind::Coil(_) => "coil",
        }
    }

    /// All kind ids, in toolbar order.
    pub const KIND_IDS: [&'static str; 7] =
        ["sketch", "plane", "extrude", "revolve", "loft", "gear", "coil"];

    /// Whether this feature creates or changes bodies.
    pub fn is_solid(&self) -> bool {
        !matches!(self, FeatureKind::Sketch(_) | FeatureKind::Plane(_))
    }

    /// Plane the feature lies on, if any.
    pub fn plane(&self) -> Option<PlaneRef> {
        match self {
            FeatureKind::Sketch(s) => Some(s.plane),
            FeatureKind::Plane(p) => Some(p.base),
            FeatureKind::SpurGear(g) => Some(g.plane),
            FeatureKind::Coil(c) => Some(c.plane),
            _ => None,
        }
    }

    /// Features this one refers to (must come earlier in the timeline).
    pub fn dependencies(&self) -> Vec<FeatureId> {
        let mut out = Vec::new();
        if let Some(PlaneRef::Construction(id)) = self.plane() {
            out.push(id);
        }
        match self {
            FeatureKind::Extrude(e) => out.extend(e.profiles.iter().map(|p| p.sketch)),
            FeatureKind::Revolve(r) => {
                out.extend(r.profiles.iter().map(|p| p.sketch));
                if let AxisRef::SketchLine { sketch, .. } = r.axis {
                    out.push(sketch);
                }
            }
            FeatureKind::Loft(l) => out.extend(l.sections.iter().map(|s| match s {
                LoftSection::Profile(p) => p.sketch,
                LoftSection::Point { sketch, .. } => *sketch,
            })),
            _ => {}
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn operation(&self) -> Option<BodyOp> {
        match self {
            FeatureKind::Extrude(e) => Some(e.operation),
            FeatureKind::Revolve(r) => Some(r.operation),
            FeatureKind::Loft(l) => Some(l.operation),
            FeatureKind::SpurGear(g) => Some(g.operation),
            FeatureKind::Coil(c) => Some(c.operation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub features: Vec<Feature>,
    /// Next free feature id (never reused).
    pub next_feature_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self { features: Vec::new(), next_feature_id: 1 }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LoadError {
    #[error("file is larger than {MAX_FILE_BYTES} bytes")]
    TooLarge,
    #[error("not valid JSON: {0}")]
    Json(String),
    #[error("not a QOOB3D file (format {0:?})")]
    WrongFormat(String),
    #[error(
        "file was written by a newer QOOB3D (format version {0}, supported up to {FORMAT_VERSION})"
    )]
    TooNew(u32),
    #[error("invalid document: {0}")]
    Invalid(String),
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    format: String,
    format_version: u32,
    document: T,
}

impl Document {
    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features.iter().find(|f| f.id == id)
    }

    pub fn feature_mut(&mut self, id: FeatureId) -> Option<&mut Feature> {
        self.features.iter_mut().find(|f| f.id == id)
    }

    pub fn sketch(&self, id: FeatureId) -> Option<&Sketch> {
        self.feature(id).and_then(|f| match &f.kind {
            FeatureKind::Sketch(s) => Some(s),
            _ => None,
        })
    }

    pub fn sketch_mut(&mut self, id: FeatureId) -> Option<&mut Sketch> {
        self.feature_mut(id).and_then(|f| match &mut f.kind {
            FeatureKind::Sketch(s) => Some(s),
            _ => None,
        })
    }

    /// Sketches with their ids, in timeline order.
    pub fn sketches(&self) -> impl Iterator<Item = (FeatureId, &Sketch)> {
        self.features.iter().filter_map(|f| match &f.kind {
            FeatureKind::Sketch(s) => Some((f.id, s)),
            _ => None,
        })
    }

    /// Construction planes with their ids, in timeline order.
    pub fn planes(&self) -> impl Iterator<Item = (FeatureId, &ConstructionPlane)> {
        self.features.iter().filter_map(|f| match &f.kind {
            FeatureKind::Plane(p) => Some((f.id, p)),
            _ => None,
        })
    }

    /// Resolves a plane reference to its frame; `None` if a construction
    /// plane is missing (or the chain is cyclic / absurdly deep).
    pub fn plane_frame(&self, plane: PlaneRef) -> Option<PlaneFrame> {
        let mut offset = 0.0;
        let mut current = plane;
        for _ in 0..64 {
            match current {
                PlaneRef::Origin(o) => return Some(o.frame().offset(offset)),
                PlaneRef::Construction(id) => match &self.feature(id)?.kind {
                    FeatureKind::Plane(p) => {
                        offset += p.offset;
                        current = p.base;
                    }
                    _ => return None,
                },
            }
        }
        None
    }

    /// Index of a feature in the timeline.
    pub fn position(&self, id: FeatureId) -> Option<usize> {
        self.features.iter().position(|f| f.id == id)
    }

    /// Appends a feature and returns its id.
    pub fn add_feature(&mut self, name: String, kind: FeatureKind) -> FeatureId {
        let id = FeatureId(self.next_feature_id);
        self.next_feature_id += 1;
        self.features.push(Feature { id, name, suppressed: false, kind });
        id
    }

    pub fn remove_feature(&mut self, id: FeatureId) {
        self.features.retain(|f| f.id != id);
    }

    /// Number of sketches (for default names like "Sketch3").
    pub fn sketch_count(&self) -> usize {
        self.count_of("sketch")
    }

    /// Number of features of a kind (see [`FeatureKind::kind_id`]).
    pub fn count_of(&self, kind_id: &str) -> usize {
        self.features.iter().filter(|f| f.kind.kind_id() == kind_id).count()
    }

    pub fn to_json(&self) -> String {
        let env =
            Envelope { format: FORMAT.into(), format_version: FORMAT_VERSION, document: self };
        serde_json::to_string_pretty(&env).expect("document is always serialisable")
    }

    /// Parses, migrates and validates a `.qbd` file (ADR-0030).
    pub fn from_json(text: &str) -> Result<Document, LoadError> {
        if text.len() > MAX_FILE_BYTES {
            return Err(LoadError::TooLarge);
        }
        let env: Envelope<serde_json::Value> =
            serde_json::from_str(text).map_err(|e| LoadError::Json(e.to_string()))?;
        if env.format != FORMAT {
            return Err(LoadError::WrongFormat(env.format));
        }
        if env.format_version > FORMAT_VERSION {
            return Err(LoadError::TooNew(env.format_version));
        }
        let migrated = migrate(env.format_version, env.document)?;
        let doc: Document =
            serde_json::from_value(migrated).map_err(|e| LoadError::Json(e.to_string()))?;
        doc.validate().map_err(LoadError::Invalid)?;
        Ok(doc)
    }

    /// Structural and numeric validation; untrusted files must pass this.
    pub fn validate(&self) -> Result<(), String> {
        if self.features.len() > MAX_FEATURES {
            return Err(format!("{} features (limit {MAX_FEATURES})", self.features.len()));
        }
        let mut ids = std::collections::BTreeSet::new();
        for (index, f) in self.features.iter().enumerate() {
            if !ids.insert(f.id) {
                return Err(format!("duplicate feature id {}", f.id.0));
            }
            for dep in f.kind.dependencies() {
                if !self.features[..index].iter().any(|g| g.id == dep) {
                    return Err(format!(
                        "{} ({}) refers to feature {} that does not come before it",
                        f.name, f.id.0, dep.0
                    ));
                }
            }
            if f.id.0 >= self.next_feature_id {
                return Err(format!("feature id {} ≥ next_feature_id", f.id.0));
            }
            if f.name.chars().count() > MAX_NAME {
                return Err(format!("feature {} name too long", f.id.0));
            }
            self.validate_feature(f).map_err(|e| format!("{} ({}): {e}", f.name, f.id.0))?;
        }
        Ok(())
    }

    fn validate_feature(&self, f: &Feature) -> Result<(), String> {
        let sketch_of = |id: FeatureId| self.sketch(id).ok_or("referenced feature is no sketch");
        let profile_ok = |p: &ProfileRef| -> Result<(), String> {
            sketch_of(p.sketch)?;
            if p.key.is_empty() || p.key.windows(2).any(|w| w[0] >= w[1]) {
                return Err("profile key must be sorted and non-empty".into());
            }
            Ok(())
        };
        match &f.kind {
            FeatureKind::Sketch(s) => validate_sketch(s)?,
            FeatureKind::Plane(p) => feature::finite("offset", p.offset)?,
            FeatureKind::Extrude(e) => {
                e.validate()?;
                e.profiles.iter().try_for_each(profile_ok)?;
            }
            FeatureKind::Revolve(r) => {
                r.validate()?;
                r.profiles.iter().try_for_each(profile_ok)?;
                if let AxisRef::SketchLine { sketch, line } = r.axis {
                    let s = sketch_of(sketch)?;
                    if !matches!(s.curve(line).map(|c| &c.kind), Some(CurveKind::Line { .. })) {
                        return Err("revolve axis is not a sketch line".into());
                    }
                }
            }
            FeatureKind::Loft(l) => {
                l.validate()?;
                for section in &l.sections {
                    match section {
                        LoftSection::Profile(p) => profile_ok(p)?,
                        LoftSection::Point { sketch, point } => {
                            if sketch_of(*sketch)?.point(*point).is_none() {
                                return Err("loft point does not exist".into());
                            }
                        }
                    }
                }
            }
            FeatureKind::SpurGear(g) => g.validate()?,
            FeatureKind::Coil(c) => c.validate()?,
        }
        if let Some(plane) = f.kind.plane()
            && self.plane_frame(plane).is_none()
        {
            return Err("plane does not exist".into());
        }
        Ok(())
    }
}

fn validate_sketch(s: &Sketch) -> Result<(), String> {
    if s.points.len() + s.curves.len() > MAX_ENTITIES {
        return Err(format!("more than {MAX_ENTITIES} entities"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let coord_ok = |v: f64| v.is_finite() && v.abs() <= MAX_COORD;
    for p in &s.points {
        if !ids.insert(p.id) {
            return Err(format!("duplicate entity id {}", p.id.0));
        }
        if !coord_ok(p.x) || !coord_ok(p.y) {
            return Err(format!("point {} has invalid coordinates", p.id.0));
        }
    }
    for c in &s.curves {
        if !ids.insert(c.id) {
            return Err(format!("duplicate entity id {}", c.id.0));
        }
        for p in c.points() {
            if s.point(p).is_none() {
                return Err(format!("curve {} references missing point {}", c.id.0, p.0));
            }
        }
        match c.kind {
            CurveKind::Line { start, end } if start == end => {
                return Err(format!("line {} starts and ends at the same point", c.id.0));
            }
            CurveKind::Circle { radius, .. } if !(radius > 0.0 && radius <= MAX_COORD) => {
                return Err(format!("circle {} has invalid radius", c.id.0));
            }
            CurveKind::Arc { center, start, end } => {
                if center == start || center == end || start == end {
                    return Err(format!("arc {} needs three different points", c.id.0));
                }
                let g = s.arc(center, start, end);
                let e = s.pos(end);
                let d = (e[0] - g.center[0]).hypot(e[1] - g.center[1]);
                if g.radius.is_nan()
                    || g.radius <= 1e-9
                    || (d - g.radius).abs() > 1e-6 * g.radius.max(1.0)
                {
                    return Err(format!("arc {} end point is not on its circle", c.id.0));
                }
            }
            _ => {}
        }
    }
    if let Some(max) = ids.iter().next_back()
        && max.0 >= s.next_id
    {
        return Err(format!("entity id {} ≥ next_id", max.0));
    }
    Ok(())
}

/// Upgrades older format versions step by step (v → v+1), each with a
/// fixture test (ADR-0030).
fn migrate(version: u32, mut doc: serde_json::Value) -> Result<serde_json::Value, LoadError> {
    let mut v = version;
    while v < FORMAT_VERSION {
        doc = match v {
            // v1 → v2 only added variants and fields with defaults.
            1 => doc,
            other => return Err(LoadError::Invalid(format!("unknown format version {other}"))),
        };
        v += 1;
    }
    if v == FORMAT_VERSION {
        Ok(doc)
    } else {
        Err(LoadError::Invalid(format!("unknown format version {version}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Document {
        let mut doc = Document::default();
        let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
        let a = s.add_point([0.0, 0.0]);
        let b = s.add_point([20.0, 0.0]);
        s.add_line(a, b);
        let c = s.add_point([5.0, 5.0]);
        s.add_circle(c, 3.0);
        doc.add_feature("Sketch1".into(), FeatureKind::Sketch(s));
        doc
    }

    #[test]
    fn json_round_trip() {
        let doc = sample();
        let json = doc.to_json();
        assert!(json.contains("\"format\": \"qoob3d\""));
        assert_eq!(Document::from_json(&json).unwrap(), doc);
    }

    #[test]
    fn version_1_fixture_still_loads() {
        // Never change this fixture; add one per new format version (ADR-0030).
        let text = include_str!("../../tests/fixtures/format_v1.qbd");
        let doc = Document::from_json(text).unwrap();
        let s = doc.sketch(FeatureId(1)).unwrap();
        assert_eq!(s.curves.len(), 5, "rectangle + circle");
        assert_eq!(s.plane, PlaneRef::Origin(OriginPlane::XY));
    }

    #[test]
    fn version_2_fixture_still_loads_with_every_kind() {
        // Never change this fixture (ADR-0030); it was written once from
        // `examples::all_kinds()`.
        let doc = Document::from_json(include_str!("../../tests/fixtures/format_v2.qbd")).unwrap();
        for kind in FeatureKind::KIND_IDS {
            assert!(doc.count_of(kind) > 0, "{kind}");
        }
        let arcs = doc.sketches().flat_map(|(_, s)| &s.curves);
        assert!(arcs.into_iter().any(|c| matches!(c.kind, CurveKind::Arc { .. })));
        assert!(doc.planes().count() == 1);
    }

    #[test]
    fn rejects_foreign_newer_and_broken_files() {
        let wrong = r#"{"format":"stl","format_version":1,"document":{}}"#;
        assert!(matches!(Document::from_json(wrong), Err(LoadError::WrongFormat(_))));
        let newer = r#"{"format":"qoob3d","format_version":99,"document":{}}"#;
        assert_eq!(Document::from_json(newer), Err(LoadError::TooNew(99)));
        assert!(matches!(Document::from_json("{"), Err(LoadError::Json(_))));
    }

    #[test]
    fn rejects_invalid_geometry() {
        let mut doc = sample();
        if let Some(s) = doc.sketch_mut(FeatureId(1)) {
            s.points[0].x = f64::NAN;
        }
        // NaN is not even valid JSON; check the validator directly.
        assert!(doc.validate().unwrap_err().contains("invalid coordinates"));

        let mut doc = sample();
        if let Some(s) = doc.sketch_mut(FeatureId(1)) {
            s.curves.push(SketchCurve {
                id: EntityId(99),
                kind: CurveKind::Line { start: EntityId(1), end: EntityId(42) },
                construction: false,
            });
            s.next_id = 100;
        }
        let json = doc.to_json();
        assert!(
            matches!(Document::from_json(&json), Err(LoadError::Invalid(m)) if m.contains("missing point"))
        );
    }
}
