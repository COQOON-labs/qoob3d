//! Rebuild of the document into bodies (ADR-0031) with the hash-chain cache
//! of ADR-0012. Runs on the kernel worker thread only (OCCT shapes are not
//! `Send`); results leave as plain data.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use cxx::UniquePtr;
use qoob3d_core::document::{
    AxisRef, BodyOp, Coil, CoilKind, CurveKind, Document, Feature, FeatureId, FeatureKind,
    LoftSection, PlaneFrame, PlaneRef, ProfileRef, SpurGear,
};
use qoob3d_core::generate::{self, PieceKind};
use qoob3d_core::profile::Profile;
use qoob3d_core::{ShapeMesh, TriMesh};

use crate::occt::ffi;
use crate::profiles::{flatten, sketch_profiles};
use crate::tessellate::tessellate_raw;
use crate::{KernelError, VIEW_TESSELLATION_TOLERANCE_MM};

/// Bodies closer than this touch (Join / Cut participants), mm.
const TOUCH_MM: f64 = 1e-6;

/// A body: identified by the feature that created it (ADR-0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BodyId {
    pub feature: FeatureId,
    pub index: u32,
}

/// One body after a rebuild.
#[derive(Debug, Clone)]
pub struct BodyResult {
    pub id: BodyId,
    /// 1-based creation number ("Body3").
    pub number: u32,
    pub mesh: Arc<ShapeMesh>,
    /// Exact B-Rep volume (mm³).
    pub volume_mm3: f64,
    /// OCCT shape check passed.
    pub valid: bool,
}

/// Derived data of one sketch.
#[derive(Debug, Clone)]
pub struct SketchResult {
    pub feature: FeatureId,
    pub profiles: Result<Vec<Profile>, KernelError>,
}

/// Everything a rebuild produces.
#[derive(Debug, Clone, Default)]
pub struct RebuildResult {
    pub sketches: Vec<SketchResult>,
    pub bodies: Vec<BodyResult>,
    /// Features that failed, with the reason (shown red in the timeline).
    pub errors: Vec<(FeatureId, String)>,
}

/// A body for export: name and a finer mesh.
#[derive(Debug, Clone)]
pub struct ExportBody {
    pub number: u32,
    pub mesh: TriMesh,
}

pub(crate) struct Shape(UniquePtr<ffi::TopoDS_Shape>);

impl Shape {
    fn get(&self) -> &ffi::TopoDS_Shape {
        self.0.as_ref().expect("shapes are never null")
    }
}

fn occ<T>(r: Result<T, cxx::Exception>) -> Result<T, KernelError> {
    r.map_err(|e| KernelError::Occt(e.what().to_string()))
}

fn shape(r: Result<UniquePtr<ffi::TopoDS_Shape>, cxx::Exception>) -> Result<Shape, KernelError> {
    let s = occ(r)?;
    if s.is_null() {
        return Err(KernelError::Occt("no shape".into()));
    }
    Ok(Shape(s))
}

struct Body {
    id: BodyId,
    number: u32,
    shape: Rc<Shape>,
    mesh: Rc<OnceCell<Result<Arc<ShapeMesh>, KernelError>>>,
}

impl Clone for Body {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            number: self.number,
            shape: self.shape.clone(),
            mesh: self.mesh.clone(),
        }
    }
}

impl Body {
    fn new(id: BodyId, number: u32, shape: Shape) -> Self {
        Self { id, number, shape: Rc::new(shape), mesh: Rc::new(OnceCell::new()) }
    }
}

#[derive(Clone, Default)]
struct State {
    bodies: Vec<Body>,
    next_number: u32,
}

struct Step {
    state: Rc<State>,
    error: Option<String>,
}

/// The worker's model with its caches (ADR-0012).
#[derive(Default)]
pub(crate) struct Model {
    steps: HashMap<u64, Step>,
    profiles: HashMap<u64, Result<Vec<Profile>, KernelError>>,
    previous_keys: HashSet<u64>,
}

fn hash_of(seed: u64, value: &impl serde::Serialize) -> u64 {
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    serde_json::to_string(value).unwrap_or_default().hash(&mut h);
    h.finish()
}

impl Model {
    pub(crate) fn rebuild(&mut self, doc: &Document) -> RebuildResult {
        let (state, sketches, errors) = self.run(doc);
        let bodies = state
            .bodies
            .iter()
            .filter_map(|b| {
                let mesh = b
                    .mesh
                    .get_or_init(|| {
                        tessellate_raw(b.shape.get(), VIEW_TESSELLATION_TOLERANCE_MM).map(Arc::new)
                    })
                    .clone();
                let mesh = match mesh {
                    Ok(m) => m,
                    Err(err) => {
                        tracing::error!(%err, "body tessellation failed");
                        return None;
                    }
                };
                Some(BodyResult {
                    id: b.id,
                    number: b.number,
                    mesh,
                    volume_mm3: occ(ffi::qoob3d_volume(b.shape.get())).unwrap_or(f64::NAN),
                    valid: occ(ffi::qoob3d_is_valid(b.shape.get())).unwrap_or(false),
                })
            })
            .collect();
        RebuildResult { sketches, bodies, errors }
    }

    pub(crate) fn export(
        &mut self,
        doc: &Document,
        tolerance: f64,
    ) -> Result<Vec<ExportBody>, KernelError> {
        let (state, _, _) = self.run(doc);
        state
            .bodies
            .iter()
            .map(|b| {
                let mesh = tessellate_raw(b.shape.get(), tolerance)?;
                Ok(ExportBody { number: b.number, mesh: mesh.faces })
            })
            .collect()
    }

    #[allow(clippy::type_complexity)] // internal tuple, one caller pattern
    fn run(&mut self, doc: &Document) -> (Rc<State>, Vec<SketchResult>, Vec<(FeatureId, String)>) {
        let mut state = Rc::new(State { bodies: Vec::new(), next_number: 1 });
        let mut sketches = Vec::new();
        let mut errors = Vec::new();
        let mut used = HashSet::new();
        let mut chain = 0u64;
        for f in &doc.features {
            chain = hash_of(chain, f);
            if f.suppressed {
                continue;
            }
            match &f.kind {
                FeatureKind::Sketch(s) => {
                    let key = hash_of(0, s);
                    used.insert(key);
                    let profiles =
                        self.profiles.entry(key).or_insert_with(|| sketch_profiles(s)).clone();
                    if let Err(err) = &profiles {
                        errors.push((f.id, err.to_string()));
                    }
                    sketches.push(SketchResult { feature: f.id, profiles });
                }
                FeatureKind::Plane(_) => {}
                _ => {
                    used.insert(chain);
                    let step =
                        self.steps.entry(chain).or_insert_with(|| match apply(doc, f, &state) {
                            Ok(next) => Step { state: Rc::new(next), error: None },
                            Err(err) => Step { state: state.clone(), error: Some(err) },
                        });
                    state = step.state.clone();
                    if let Some(err) = &step.error {
                        errors.push((f.id, err.clone()));
                    }
                }
            }
        }
        // Keep this and the previous rebuild (cheap undo/redo), drop the rest.
        let keep: HashSet<u64> = used.union(&self.previous_keys).copied().collect();
        self.steps.retain(|k, _| keep.contains(k));
        self.profiles.retain(|k, _| keep.contains(k));
        self.previous_keys = used;
        (state, sketches, errors)
    }
}

/// Applies one solid feature to the bodies.
fn apply(doc: &Document, f: &Feature, state: &State) -> Result<State, String> {
    let tool = tool_shape(doc, &f.kind).map_err(|e| e.to_string())?;
    let op = f.kind.operation().unwrap_or_default();
    combine(state, f.id, tool, op).map_err(|e| e.to_string())
}

fn frame_of(doc: &Document, plane: PlaneRef) -> Result<PlaneFrame, KernelError> {
    doc.plane_frame(plane).ok_or_else(|| KernelError::Model("plane does not exist".into()))
}

/// Faces of profiles (grouped per sketch, in the given order) in world space.
fn profile_faces(doc: &Document, refs: &[&ProfileRef]) -> Result<Shape, KernelError> {
    let mut out: Option<Shape> = None;
    for r in refs {
        let sketch = doc
            .sketch(r.sketch)
            .ok_or_else(|| KernelError::Model("profile sketch does not exist".into()))?;
        let frame = frame_of(doc, sketch.plane)?;
        let (curves, _) = flatten(sketch);
        let keys: Vec<u32> = r.key.iter().map(|id| id.0).collect();
        let faces = shape(ffi::qoob3d_profile_faces(
            &curves,
            &keys,
            &[0, keys.len() as u32],
            &frame.to_array(),
        ))
        .map_err(|e| match e {
            KernelError::Occt(m) if m.contains("profile not found") => {
                KernelError::Model("a selected profile no longer exists".into())
            }
            other => other,
        })?;
        out = Some(match out {
            None => faces,
            Some(prev) => shape(ffi::qoob3d_append(prev.get(), faces.get()))?,
        });
    }
    out.ok_or_else(|| KernelError::Model("no profile selected".into()))
}

fn normal_of_sketch(doc: &Document, id: FeatureId) -> Result<[f64; 3], KernelError> {
    let s = doc.sketch(id).ok_or_else(|| KernelError::Model("sketch does not exist".into()))?;
    Ok(frame_of(doc, s.plane)?.n)
}

fn tool_shape(doc: &Document, kind: &FeatureKind) -> Result<Shape, KernelError> {
    match kind {
        FeatureKind::Extrude(e) => {
            // One prism per sketch (each has its own normal), then combined.
            let mut sketches: Vec<FeatureId> = e.profiles.iter().map(|p| p.sketch).collect();
            sketches.dedup();
            let mut out: Option<Shape> = None;
            for id in sketches {
                let refs: Vec<&ProfileRef> = e.profiles.iter().filter(|p| p.sketch == id).collect();
                let faces = profile_faces(doc, &refs)?;
                let n = normal_of_sketch(doc, id)?;
                let v = n.map(|c| c * e.distance);
                let prism = shape(ffi::qoob3d_extrude(faces.get(), &v, e.symmetric))?;
                out = Some(match out {
                    None => prism,
                    Some(prev) => shape(ffi::qoob3d_append(prev.get(), prism.get()))?,
                });
            }
            out.ok_or_else(|| KernelError::Model("no profile selected".into()))
        }
        FeatureKind::Revolve(r) => {
            let refs: Vec<&ProfileRef> = r.profiles.iter().collect();
            let faces = profile_faces(doc, &refs)?;
            let axis: [f64; 6] = match r.axis {
                AxisRef::Origin(a) => {
                    let d = a.direction();
                    [0.0, 0.0, 0.0, d[0], d[1], d[2]]
                }
                AxisRef::SketchLine { sketch, line } => {
                    let s = doc
                        .sketch(sketch)
                        .ok_or_else(|| KernelError::Model("axis sketch does not exist".into()))?;
                    let Some(CurveKind::Line { start, end }) =
                        s.curve(line).map(|c| c.kind.clone())
                    else {
                        return Err(KernelError::Model("axis line does not exist".into()));
                    };
                    let frame = frame_of(doc, s.plane)?;
                    let (a, b) = (frame.to_world(s.pos(start)), frame.to_world(s.pos(end)));
                    [a[0], a[1], a[2], b[0] - a[0], b[1] - a[1], b[2] - a[2]]
                }
            };
            shape(ffi::qoob3d_revolve(faces.get(), &axis, r.angle_deg.to_radians()))
        }
        FeatureKind::Loft(l) => {
            let point = |sketch: FeatureId, point| -> Result<Vec<f64>, KernelError> {
                let s = doc
                    .sketch(sketch)
                    .ok_or_else(|| KernelError::Model("loft sketch does not exist".into()))?;
                let p = s
                    .point(point)
                    .ok_or_else(|| KernelError::Model("loft point does not exist".into()))?;
                Ok(frame_of(doc, s.plane)?.to_world([p.x, p.y]).to_vec())
            };
            let (mut start, mut end) = (Vec::new(), Vec::new());
            let mut refs = Vec::new();
            for (i, section) in l.sections.iter().enumerate() {
                match section {
                    LoftSection::Profile(p) => refs.push(p),
                    LoftSection::Point { sketch, point: id } if i == 0 => {
                        start = point(*sketch, *id)?;
                    }
                    LoftSection::Point { sketch, point: id } => end = point(*sketch, *id)?,
                }
            }
            let faces = profile_faces(doc, &refs)?;
            shape(ffi::qoob3d_loft(faces.get(), &start, &end))
        }
        FeatureKind::SpurGear(g) => gear(doc, g),
        FeatureKind::Coil(c) => coil(doc, c),
        FeatureKind::Sketch(_) | FeatureKind::Plane(_) => {
            Err(KernelError::Model("not a solid feature".into()))
        }
    }
}

fn gear(doc: &Document, g: &SpurGear) -> Result<Shape, KernelError> {
    let frame = frame_of(doc, g.plane)?;
    let outline = generate::gear_outline(g);
    let (mut points, mut starts, mut kinds) = (Vec::new(), Vec::new(), Vec::new());
    let mut count = 0u32;
    for (kind, pts) in &outline.pieces {
        starts.push(count);
        kinds.push(match kind {
            PieceKind::Lines => 0,
            PieceKind::Spline => 1,
        });
        for p in pts {
            points.extend(frame.to_world(*p));
            count += 1;
        }
    }
    starts.push(count);
    let holes: Vec<f64> = if g.bore > 0.0 {
        let o = frame.origin;
        vec![o[0], o[1], o[2], 0.5 * g.bore]
    } else {
        Vec::new()
    };
    let v = frame.n.map(|c| c * g.thickness);
    shape(ffi::qoob3d_outline_prism(&points, &starts, &kinds, &holes, &frame.n, &v))
}

fn coil(doc: &Document, c: &Coil) -> Result<Shape, KernelError> {
    let frame = frame_of(doc, c.plane)?;
    let path: Vec<f64> = generate::coil_path(c)
        .iter()
        .flat_map(|p| {
            let w = frame.to_world([p[0], p[1]]);
            [0, 1, 2].map(|k| w[k] + frame.n[k] * p[2])
        })
        .collect();
    let binormal: &[f64] = match c.kind {
        CoilKind::Helix => &[],
        CoilKind::Spiral => &frame.n,
    };
    shape(ffi::qoob3d_sweep_circle(&path, 0.5 * c.section, binormal))
}

fn solids(s: &Shape) -> Result<Vec<Shape>, KernelError> {
    let n = occ(ffi::qoob3d_solid_count(s.get()))?;
    (0..n).map(|i| shape(ffi::qoob3d_solid(s.get(), i))).collect()
}

fn touches(a: &Shape, b: &Shape) -> Result<bool, KernelError> {
    Ok(occ(ffi::qoob3d_distance(a.get(), b.get()))? <= TOUCH_MM)
}

fn combine(
    state: &State,
    feature: FeatureId,
    tool: Shape,
    op: BodyOp,
) -> Result<State, KernelError> {
    let mut next = state.clone();
    let pieces = solids(&tool)?;
    if pieces.is_empty() {
        return Err(KernelError::Model("the feature produced no solid".into()));
    }
    let new_bodies = |next: &mut State, pieces: Vec<Shape>| {
        for (i, s) in pieces.into_iter().enumerate() {
            let id = BodyId { feature, index: i as u32 };
            next.bodies.push(Body::new(id, next.next_number, s));
            next.next_number += 1;
        }
    };
    match op {
        BodyOp::NewBody => new_bodies(&mut next, pieces),
        BodyOp::Join => {
            let mut targets = Vec::new();
            for (i, b) in next.bodies.iter().enumerate() {
                if touches(&b.shape, &tool)? {
                    targets.push(i);
                }
            }
            let Some(&first) = targets.first() else {
                new_bodies(&mut next, pieces);
                return Ok(next);
            };
            let mut merged =
                shape(ffi::qoob3d_boolean(next.bodies[first].shape.get(), tool.get(), 0))?;
            for &i in &targets[1..] {
                merged = shape(ffi::qoob3d_boolean(merged.get(), next.bodies[i].shape.get(), 0))?;
            }
            let (id, number) = (next.bodies[first].id, next.bodies[first].number);
            next.bodies[first] = Body::new(id, number, merged);
            for &i in targets[1..].iter().rev() {
                next.bodies.remove(i);
            }
        }
        BodyOp::Cut => {
            let mut hit = false;
            let mut bodies = Vec::new();
            for b in &next.bodies {
                if !touches(&b.shape, &tool)? {
                    bodies.push(b.clone());
                    continue;
                }
                hit = true;
                let rest = shape(ffi::qoob3d_boolean(b.shape.get(), tool.get(), 1))?;
                if occ(ffi::qoob3d_solid_count(rest.get()))? > 0 {
                    bodies.push(Body::new(b.id, b.number, rest));
                }
            }
            if !hit {
                return Err(KernelError::Model("cut does not touch any body".into()));
            }
            next.bodies = bodies;
        }
    }
    Ok(next)
}
