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
mod limits;
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
use rhai::{Engine, EvalAltResult, Module, ModuleResolver, Position, Scope, Shared};

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

/// Bounding boxes are reported in steps of 1e-9 mm (AG-HD-11): OCCT's box of
/// a shape read back from a BREP file differs from the built one in the last
/// bits, and every place that reports a box must show the same numbers.
const BBOX_STEPS_PER_MM: f64 = 1e9;

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
    /// Module files the engine resolved for `import`, relative to the
    /// project folder, each once in the order of first use (AG-HD-06).
    pub modules_read: Vec<PathBuf>,
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

    /// Tight bounding box `(min, max)` in mm, in steps of 1e-9 mm (NaN if
    /// the shape is empty). The same numbers as `bbox()` in scripts.
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
        // Adding 0 turns −0 into 0.
        let step = |v: f64| (v * BBOX_STEPS_PER_MM).round() / BBOX_STEPS_PER_MM + 0.0;
        Ok(([b[0], b[1], b[2]].map(step), [b[3], b[4], b[5]].map(step)))
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

/// Evaluates a script (AG-SC-01). Declaration discovery
/// (`PartMode::Declare`) runs with at most 1 000 000 operations and with size
/// limits for strings, arrays and maps (AG-HD-02).
pub fn run(source: &str, options: &RunOptions) -> Result<RunOutput, ScriptError> {
    if options.parts != PartMode::Declare {
        let (outcome, output, _) = evaluate(source, options, options.max_operations);
        outcome.map_err(|e| script_error(*e))?;
        return Ok(output);
    }
    // 0 means no limit to Rhai.
    let operations = match options.max_operations {
        0 => limits::MAX_OPERATIONS,
        n => n.min(limits::MAX_OPERATIONS),
    };
    // In stages, see `limits`: after each one the variables are measured.
    let mut budget = 0;
    loop {
        let clean = budget.max(1);
        budget = (budget + limits::STAGE_OPERATIONS).min(operations);
        let (outcome, output, oversize) = evaluate(source, options, budget);
        if let Some((what, started)) = oversize {
            let over = budget.min(started + 1).max(clean + 1);
            return Err(limits::locate(what, (clean, over), |n| {
                let (outcome, _, oversize) = evaluate(source, options, n);
                (outcome, oversize.is_some())
            }));
        }
        if budget == operations || !limits::out_of_operations(&outcome) {
            outcome.map_err(|e| script_error(*e))?;
            return Ok(output);
        }
    }
}

/// One run with at most `operations` Rhai operations: how it ended, what it
/// produced, and what the size checks of declaration discovery found, with
/// the number of operations the run started.
fn evaluate(
    source: &str,
    options: &RunOptions,
    operations: u64,
) -> (limits::Outcome, RunOutput, Option<(limits::Oversize, u64)>) {
    let output = Rc::new(RefCell::new(RunOutput::default()));
    let project =
        options.project_dir.as_ref().map(|d| d.canonicalize().unwrap_or_else(|_| d.clone()));
    let declare = options.parts == PartMode::Declare;

    let mut engine = Engine::new();
    engine.set_max_operations(operations);
    engine.set_max_call_levels(MAX_CALL_LEVELS);
    match &project {
        Some(root) => engine.set_module_resolver(ProjectModules {
            root: root.clone(),
            files: FileModuleResolver::new_with_path(root),
            script: source.to_owned(),
            output: Rc::clone(&output),
        }),
        None => engine.set_module_resolver(DummyModuleResolver::new()),
    };
    let printed = Rc::clone(&output);
    engine.on_print(move |line| printed.borrow_mut().printed.push(line.to_string()));
    engine.on_debug(|line, _, pos| tracing::debug!(%pos, "script: {line}"));
    // Declaring parts must not reach OCCT (§2.1): without the CAD API a
    // geometry call outside a part closure is an error, not a computation.
    let started = if declare {
        Some(limits::apply(&mut engine))
    } else {
        api::register(&mut engine, &output, project);
        None
    };
    project::register(&mut engine, &output, options);
    check::register(&mut engine, &output, options);

    let mut scope = Scope::new();
    let outcome = engine.run_with_scope(&mut scope, source);
    let oversize = started.and_then(|n| Some((limits::found(&scope)?, n.get())));
    // The engine's closures hold clones of `output`.
    drop(engine);
    (outcome, output.take(), oversize)
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
    /// Text of the script that runs.
    script: String,
    output: Rc<RefCell<RunOutput>>,
}

impl ModuleResolver for ProjectModules {
    fn resolve(
        &self,
        engine: &Engine,
        source: Option<&str>,
        path: &str,
        pos: Position,
    ) -> Result<Shared<Module>, Box<EvalAltResult>> {
        let file = format!("{path}.rhai");
        let full = inside_project(&self.root, &file)
            .map_err(|m| EvalAltResult::ErrorModuleNotFound(m, pos))?;
        // The importing code: the script itself, or the module named `source`.
        let module_text = source.map(|module| {
            std::fs::read_to_string(self.root.join(format!("{module}.rhai"))).unwrap_or_default()
        });
        let text = module_text.as_deref().unwrap_or(&self.script);
        let statement = import_statement(text, pos, path);
        if !matches!(statement, Some((_, true))) {
            // The cache key knows the literal paths only (AG-HD-06).
            let message = "import needs its path as a plain string literal: import \"lib/x\" as x;";
            let at = statement.map_or(pos, |(at, _)| at);
            return Err(EvalAltResult::ErrorRuntime(message.into(), at).into());
        }
        let module = self.files.resolve(engine, None, path, pos)?;
        let relative =
            full.strip_prefix(&self.root).map_or_else(|_| file.into(), Path::to_path_buf);
        let read = &mut self.output.borrow_mut().modules_read;
        if !read.contains(&relative) {
            read.push(relative);
        }
        Ok(module)
    }
}

/// The `import` statement of `text` at `pos` (`pos` is the statement or a
/// place in its path expression): where its keyword stands, and whether its
/// path is the double-quoted literal `path` and nothing else.
fn import_statement(text: &str, pos: Position, path: &str) -> Option<(Position, bool)> {
    let line = pos.line()?;
    let row = text.lines().nth(line - 1)?;
    // Up to the end of a keyword that starts at `pos`.
    let head: String = row.chars().take(pos.position()? + 5).collect();
    let start = head.rfind("import")?;
    let after = row[start + "import".len()..]
        .trim_start()
        .strip_prefix('"')
        .and_then(|r| r.strip_prefix(path))
        .and_then(|r| r.strip_prefix('"'))
        .map(str::trim_start);
    let literal = after.is_some_and(|r| r.is_empty() || r.starts_with("as") || r.starts_with(';'));
    let column = row[..start].chars().count() + 1;
    Some((Position::new(u16::try_from(line).ok()?, u16::try_from(column).ok()?), literal))
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
