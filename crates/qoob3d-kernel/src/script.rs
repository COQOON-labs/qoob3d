//! Script kernel (ADR-0053, `docs/requirements/agent.md` §1): models are Rhai
//! scripts on our OCCT wrappers. The script is glue only; every geometric
//! operation is one call into `cpp/qoob3d_occt.cpp` (ADR-0004, ADR-0014).
//! The API reference for scripts is `docs/script-api.md` (AG-SC-16).
//!
//! Shapes live on the thread that runs the script (OCCT handles are not
//! `Send`); R2 runs scripts in worker processes. Project parts and target
//! values (`part`, `value`, agent.md §2.1) are in `project.rs`.

mod api;
mod check;
mod measure;
mod profile;
mod project;
mod select;

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use qoob3d_core::ShapeMesh;
use qoob3d_core::export::{self, ExportObject};
use rhai::module_resolvers::{DummyModuleResolver, FileModuleResolver};
use rhai::{Engine, EvalAltResult, Module, ModuleResolver, Position, Shared};

use crate::model::{self, occ};
use crate::occt::ffi;
use crate::tessellate::{tessellate_angle, tessellate_raw};
use crate::{EXPORT_TESSELLATION_TOLERANCE_MM, KernelError};

pub use check::{CheckOptions, CheckResult, Expectation};
pub use measure::{common_volume, distance};
pub use project::{DeclaredPart, PartMode};

/// Call depth limit (AG-SC-02). Rhai's own default differs between debug
/// and release builds; scripts must behave the same in both.
const MAX_CALL_LEVELS: usize = 64;

/// How a script runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOptions {
    /// Rhai operations before the script stops with [`ScriptError::Limit`].
    pub max_operations: u64,
    /// Project folder: `import` and `import_step` resolve inside it only.
    /// Without a project, scripts cannot read any file.
    pub project_dir: Option<PathBuf>,
    /// Target values `value(id)` returns (AG-PJ-03); other ids are errors.
    pub values: BTreeMap<String, serde_json::Value>,
    /// What `part(...)` does (AG-PJ-02).
    pub parts: PartMode,
    /// Built parts for `part("Name")` and the selection of checks (AG-CK-01).
    pub check: CheckOptions,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_operations: 50_000_000,
            project_dir: None,
            values: BTreeMap::new(),
            parts: PartMode::All,
            check: CheckOptions::default(),
        }
    }
}

/// What a script produced (AG-SC-01).
#[derive(Debug, Default)]
pub struct RunOutput {
    /// Shapes registered with `show(shape, "name")`, in call order.
    pub shapes: Vec<(String, Shape)>,
    /// Lines printed with `print`.
    pub printed: Vec<String>,
    /// Parts declared with `part(...)`, in call order.
    pub parts: Vec<DeclaredPart>,
    /// Values read with `value(id)`, the inputs of the cache key (§2.1).
    pub values_read: BTreeMap<String, serde_json::Value>,
    /// Checks declared with `check(...)`, in call order (AG-CK-01).
    pub checks: Vec<CheckResult>,
}

/// A failed script run (AG-SC-03). Lines and columns are 1-based; 0 means
/// the position is unknown.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    #[error("line {line}, column {column}: syntax error: {message}")]
    Syntax { line: usize, column: usize, message: String },
    #[error("line {line}, column {column}: {message}")]
    Runtime { line: usize, column: usize, message: String },
    /// An OCCT operation failed or produced an implausible result.
    #[error("line {line}, column {column}: {function} failed: {message}")]
    Geometry { line: usize, column: usize, function: String, message: String },
    /// Operation or call depth limit reached (AG-SC-02).
    #[error("line {line}, column {column}: limit reached: {message}")]
    Limit { line: usize, column: usize, message: String },
}

impl ScriptError {
    fn at(mut self, pos: Position) -> Self {
        let (l, c) = line_column(pos);
        let (ScriptError::Syntax { line, column, .. }
        | ScriptError::Runtime { line, column, .. }
        | ScriptError::Geometry { line, column, .. }
        | ScriptError::Limit { line, column, .. }) = &mut self;
        (*line, *column) = (l, c);
        self
    }

    fn line(&self) -> usize {
        let (ScriptError::Syntax { line, .. }
        | ScriptError::Runtime { line, .. }
        | ScriptError::Geometry { line, .. }
        | ScriptError::Limit { line, .. }) = self;
        *line
    }
}

/// A solid (or a compound of solids) built by a script. Cloning shares the
/// OCCT shape.
#[derive(Clone)]
pub struct Shape(Rc<ShapeData>);

struct ShapeData {
    occt: model::Shape,
    volume: OnceCell<f64>,
}

impl Shape {
    fn new(occt: model::Shape) -> Self {
        Self(Rc::new(ShapeData { occt, volume: OnceCell::new() }))
    }

    fn get(&self) -> &ffi::TopoDS_Shape {
        self.0.occt.get()
    }

    fn same(&self, other: &Shape) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Volume in mm³ (NaN if OCCT cannot compute it). Computed once.
    pub fn volume(&self) -> f64 {
        *self.0.volume.get_or_init(|| occ(ffi::qoob3d_volume(self.get())).unwrap_or(f64::NAN))
    }

    /// `BRepCheck_Analyzer` finds no defect.
    pub fn is_valid(&self) -> bool {
        occ(ffi::qoob3d_is_valid(self.get())).unwrap_or(false)
    }

    /// Number of solids (compounds are flattened).
    pub fn solid_count(&self) -> usize {
        occ(ffi::qoob3d_solid_count(self.get())).map_or(0, |n| n as usize)
    }

    /// Tight bounding box `(min, max)` in mm (NaN if the shape is empty).
    pub fn bbox(&self) -> ([f64; 3], [f64; 3]) {
        self.bounds().unwrap_or(([f64::NAN; 3], [f64::NAN; 3]))
    }

    /// Triangles and feature edges with chordal deviation `tolerance_mm`.
    pub fn mesh(&self, tolerance_mm: f64) -> Result<ShapeMesh, KernelError> {
        tessellate_raw(self.get(), tolerance_mm)
    }

    /// As [`Shape::mesh`] with the largest angle `angle_deg` between
    /// neighbouring segments chosen by the caller (AG-PF-05).
    pub fn mesh_with_angle(
        &self,
        tolerance_mm: f64,
        angle_deg: f64,
    ) -> Result<ShapeMesh, KernelError> {
        tessellate_angle(self.get(), tolerance_mm, angle_deg.to_radians())
    }

    /// Writes the shape as an OCCT BREP file (cache entry, AG-PJ-04).
    pub fn write_brep(&self, path: &Path) -> Result<(), KernelError> {
        occ(ffi::qoob3d_write_brep(self.get(), utf8(path)?))
    }

    fn bounds(&self) -> Result<([f64; 3], [f64; 3]), KernelError> {
        let b = occ(ffi::qoob3d_bbox(self.get()))?;
        Ok(([b[0], b[1], b[2]], [b[3], b[4], b[5]]))
    }
}

impl fmt::Debug for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Shape")
    }
}

/// Reads a BREP file written by [`Shape::write_brep`].
pub fn read_brep(path: &Path) -> Result<Shape, KernelError> {
    model::shape(ffi::qoob3d_read_brep(utf8(path)?)).map(Shape::new)
}

fn utf8(path: &Path) -> Result<&str, KernelError> {
    path.to_str().ok_or_else(|| KernelError::Io(format!("{}: not UTF-8", path.display())))
}

/// One compound of `parts` (nested compounds flattened), e.g. the tools of
/// a batched boolean.
fn compound<'a>(parts: impl IntoIterator<Item = &'a Shape>) -> Result<Shape, KernelError> {
    let mut c = occ(ffi::qoob3d_compound())?;
    if c.is_null() {
        return Err(KernelError::Occt("no compound".into()));
    }
    for p in parts {
        occ(ffi::qoob3d_compound_add(c.pin_mut(), p.get()))?;
    }
    model::shape(Ok(c)).map(Shape::new)
}

/// Evaluates a script (AG-SC-01).
pub fn run(source: &str, options: &RunOptions) -> Result<RunOutput, ScriptError> {
    let output = Rc::new(RefCell::new(RunOutput::default()));
    let project =
        options.project_dir.as_ref().map(|d| d.canonicalize().unwrap_or_else(|_| d.clone()));

    let mut engine = Engine::new();
    engine.set_max_operations(options.max_operations);
    engine.set_max_call_levels(MAX_CALL_LEVELS);
    match &project {
        Some(root) => engine.set_module_resolver(ProjectModules {
            root: root.clone(),
            files: FileModuleResolver::new_with_path(root),
        }),
        None => engine.set_module_resolver(DummyModuleResolver::new()),
    };
    let printed = Rc::clone(&output);
    engine.on_print(move |line| printed.borrow_mut().printed.push(line.to_string()));
    engine.on_debug(|line, _, pos| tracing::debug!(%pos, "script: {line}"));
    // Declaring parts must not reach OCCT (§2.1): without the CAD API a
    // geometry call outside a part closure is an error, not a computation.
    if options.parts != PartMode::Declare {
        api::register(&mut engine, &output, project);
    }
    project::register(&mut engine, &output, options);
    check::register(&mut engine, &output, options);

    let result = engine.run(source);
    // The engine's closures hold clones of `output`.
    drop(engine);
    result.map_err(|e| script_error(*e))?;
    Ok(output.take())
}

fn line_column(pos: Position) -> (usize, usize) {
    (pos.line().unwrap_or(0), pos.position().unwrap_or(0))
}

fn script_error(err: EvalAltResult) -> ScriptError {
    let (line, column) = line_column(err.position());
    match err {
        EvalAltResult::ErrorParsing(kind, _) => {
            ScriptError::Syntax { line, column, message: kind.to_string() }
        }
        EvalAltResult::ErrorInFunctionCall(_, source, inner, pos) => {
            // Inside a script function of this file the inner position is the
            // useful one; for functions of imported modules, the call site.
            let e = script_error(*inner);
            if source.is_empty() && e.line() > 0 { e } else { e.at(pos) }
        }
        EvalAltResult::ErrorInModule(name, inner, pos) => match script_error(*inner).at(pos) {
            ScriptError::Runtime { line, column, message } => {
                ScriptError::Runtime { line, column, message: format!("module {name}: {message}") }
            }
            other => other,
        },
        EvalAltResult::ErrorRuntime(ref value, _) if value.is::<api::GeometryFault>() => {
            let fault = value.clone_cast::<api::GeometryFault>();
            ScriptError::Geometry {
                line,
                column,
                function: fault.function.to_string(),
                message: fault.message,
            }
        }
        EvalAltResult::ErrorTooManyOperations(_)
        | EvalAltResult::ErrorTooManyVariables(_)
        | EvalAltResult::ErrorTooManyModules(_)
        | EvalAltResult::ErrorStackOverflow(_)
        | EvalAltResult::ErrorDataTooLarge(..) => {
            ScriptError::Limit { line, column, message: message_of(err) }
        }
        _ => ScriptError::Runtime { line, column, message: message_of(err) },
    }
}

fn message_of(mut err: EvalAltResult) -> String {
    err.clear_position();
    err.to_string()
}

/// The file `relative` inside the project `root`: no absolute paths, no
/// `..`, and symlinks must not lead outside (AG-SC-02, AG-SC-14).
fn inside_project(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    let plain = path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if path.is_absolute() || !plain {
        return Err(format!("{relative:?} must be a path inside the project"));
    }
    let full = root.join(path).canonicalize().map_err(|e| format!("{relative:?}: {e}"))?;
    if full.starts_with(root) {
        Ok(full)
    } else {
        Err(format!("{relative:?} leads outside the project"))
    }
}

/// `import "lib/x"` resolves to `<project>/lib/x.rhai`, also inside modules.
struct ProjectModules {
    root: PathBuf,
    files: FileModuleResolver,
}

impl ModuleResolver for ProjectModules {
    fn resolve(
        &self,
        engine: &Engine,
        _source: Option<&str>,
        path: &str,
        pos: Position,
    ) -> Result<Shared<Module>, Box<EvalAltResult>> {
        inside_project(&self.root, &format!("{path}.rhai"))
            .map_err(|m| EvalAltResult::ErrorModuleNotFound(m, pos))?;
        self.files.resolve(engine, None, path, pos)
    }
}

/// Binary STL of all shapes (ADR-0034), meshed at the export tolerance.
pub fn export_stl(shapes: &[(String, Shape)], path: &Path) -> Result<(), KernelError> {
    write_meshes(shapes, path, export::stl)
}

/// 3MF with one object per shape (ADR-0034).
pub fn export_3mf(shapes: &[(String, Shape)], path: &Path) -> Result<(), KernelError> {
    write_meshes(shapes, path, export::three_mf)
}

/// STEP (AP214) of all shapes as one compound.
pub fn export_step(shapes: &[(String, Shape)], path: &Path) -> Result<(), KernelError> {
    let all = compound(shapes.iter().map(|(_, s)| s))?;
    occ(ffi::qoob3d_write_step(all.get(), utf8(path)?))
}

fn write_meshes(
    shapes: &[(String, Shape)],
    path: &Path,
    encode: fn(&[ExportObject<'_>]) -> Vec<u8>,
) -> Result<(), KernelError> {
    let meshes = shapes
        .iter()
        .map(|(name, s)| Ok((name, tessellate_raw(s.get(), EXPORT_TESSELLATION_TOLERANCE_MM)?)))
        .collect::<Result<Vec<_>, KernelError>>()?;
    let objects: Vec<ExportObject<'_>> = meshes
        .iter()
        .map(|(name, mesh)| ExportObject { name: (*name).clone(), mesh: &mesh.faces })
        .collect();
    std::fs::write(path, encode(&objects))
        .map_err(|e| KernelError::Io(format!("{}: {e}", path.display())))
}
