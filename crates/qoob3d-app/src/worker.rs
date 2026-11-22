//! `qoob3d worker` (AG-PJ-05, docs/requirements/agent.md §2.1, ADR-0053): the
//! process that runs OCCT for project builds. It reads one JSON request per
//! line from stdin and answers one JSON line per job on stdout, staying warm
//! between jobs. The service owns limits and replaces a worker that crashes
//! or is killed.
//!
//! OCCT's STEP translator prints to stdout too, so answers are framed: every
//! answer line contains `{"job":` and the service ignores everything before
//! it and every line without it.

use std::collections::BTreeMap;
use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Instant;

use qoob3d_core::project::{self, KeyInputs, Mesh, Values};
use qoob3d_core::qmesh;
use qoob3d_kernel::script::{self, PartMode, RunOptions, ScriptError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod probe;

pub(crate) use self::probe::Probe;
use crate::service::snapshot::{cache_dir, declare};

/// Enters every cache key (§2.1): another program version rebuilds.
const PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Marks the start of an answer in the worker's stdout.
pub(crate) const ANSWER_MARK: &str = "{\"job\":";

/// The program part of a cache key: the version and the tessellation settings
/// (AG-PF-05), so changing either rebuilds.
pub(crate) fn key_version(mesh: &Mesh) -> String {
    format!("{PROGRAM_VERSION} mesh {} {}", mesh.deflection_mm, mesh.angle_deg)
}

/// One job: build `part`, with `step` write the shapes of `deps` as STEP, or
/// with `probe` answer a question about the built parts `deps`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Request {
    pub(crate) job: u64,
    pub(crate) dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) part: Option<String>,
    /// Declaring file of `part`; without it the worker searches `parts/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) file: Option<PathBuf>,
    /// BREP files of the dependencies (or of the shapes to export).
    #[serde(default)]
    pub(crate) deps: BTreeMap<String, PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) step: Option<PathBuf>,
    /// Tessellation of the cached mesh (AG-PF-05); STL and 3MF are exported
    /// from that mesh.
    #[serde(default)]
    pub(crate) mesh: Mesh,
    /// Folder that takes the cache entry instead of the project cache
    /// (`qoob3d bench`, AG-PF-06).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cache: Option<PathBuf>,
    /// Check, throwaway script or measure (R4); writes nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) probe: Option<Probe>,
}

/// The answer to one job; `job` comes first, see [`ANSWER_MARK`].
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Answer {
    pub(crate) job: u64,
    pub(crate) ok: bool,
    /// Cache key of the built part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) key: Option<String>,
    /// Contents of the cache `.json` of the built part, or the result of a probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) meta: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<PartError>,
}

/// Why a part did not build (AG-PJ-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorKind {
    /// The script failed (syntax, runtime, geometry, operation limit).
    Script,
    /// The project could not be read (values, imports, declarations).
    Load,
    /// Writing the cache or an export failed.
    Io,
    TimeLimit,
    MemoryLimit,
    /// The worker died without the service killing it.
    Crash,
    /// A dependency failed.
    Dependency,
    Cancelled,
    /// Service requests: unknown endpoint, part or job; malformed request.
    NotFound,
    BadRequest,
    /// Review or save refused: parts or checks are not green (AG-TK-04, AG-SV-07).
    Gate,
    /// `git` failed or the project is no repository (AG-SV-07).
    Git,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PartError {
    pub(crate) kind: ErrorKind,
    pub(crate) message: String,
    /// Script file relative to the project, for script errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) column: Option<usize>,
}

impl PartError {
    pub(crate) fn new(kind: ErrorKind, message: impl ToString) -> Self {
        Self { kind, message: message.to_string(), file: None, line: None, column: None }
    }

    /// A script error in `file` (`None`: a throwaway script).
    fn script(file: Option<&Path>, error: &ScriptError) -> Self {
        let (ScriptError::Syntax { line, column, .. }
        | ScriptError::Runtime { line, column, .. }
        | ScriptError::Geometry { line, column, .. }
        | ScriptError::Limit { line, column, .. }) = error;
        Self {
            kind: ErrorKind::Script,
            message: error.to_string(),
            file: file.map(|f| f.display().to_string()),
            line: Some(*line),
            column: Some(*column),
        }
    }
}

/// Main loop of `qoob3d worker`.
///
/// # Errors
/// Writing to stdout failed (the service is gone).
pub fn run() -> anyhow::Result<()> {
    let (tx, requests) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
        // The service is gone: end at once, also in the middle of a job, so
        // an endless script never outlives its service.
        std::process::exit(0);
    });
    for line in requests {
        if line.trim().is_empty() {
            continue;
        }
        let answer = handle(&line);
        let mut out = std::io::stdout().lock();
        writeln!(out, "{}", serde_json::to_string(&answer)?)?;
        out.flush()?;
    }
    Ok(())
}

fn handle(line: &str) -> Answer {
    let request: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return failed(0, PartError::new(ErrorKind::Load, format!("bad request: {e}"))),
    };
    let job = request.job;
    match (&request.probe, &request.step, &request.part) {
        (Some(probe), ..) => match probe::answer(&request, probe) {
            Ok(meta) => Answer { job, ok: true, key: None, meta: Some(meta), error: None },
            Err(e) => failed(job, e),
        },
        (None, Some(out), _) => match write_step(&request, out) {
            Ok(()) => Answer { job, ok: true, key: None, meta: None, error: None },
            Err(e) => failed(job, e),
        },
        (None, None, Some(part)) => match build(&request, part) {
            Ok((key, meta)) => {
                Answer { job, ok: true, key: Some(key), meta: Some(meta), error: None }
            }
            Err(e) => failed(job, e),
        },
        (None, None, None) => {
            failed(job, PartError::new(ErrorKind::Load, "request without part, step or probe"))
        }
    }
}

fn failed(job: u64, error: PartError) -> Answer {
    Answer { job, ok: false, key: None, meta: None, error: Some(error) }
}

fn load_error(e: impl ToString) -> PartError {
    PartError::new(ErrorKind::Load, e)
}

fn io_error(path: &Path, e: impl std::fmt::Display) -> PartError {
    PartError::new(ErrorKind::Io, format!("{}: {e}", path.display()))
}

/// Builds `name` and writes its cache entry `<key>.{brep,qmsh,json}` (§2.1).
fn build(request: &Request, name: &str) -> Result<(String, Value), PartError> {
    let start = Instant::now();
    let dir = &request.dir;
    let values = Values::load(dir).map_err(load_error)?;
    let (file, depends) = locate(dir, request.file.as_deref(), name)?;
    let source = std::fs::read(dir.join(&file)).map_err(|e| io_error(&file, e))?;
    let text = String::from_utf8(source.clone())
        .map_err(|e| load_error(format!("{}: {e}", file.display())))?;
    let modules = project::resolve_imports(dir, &file)
        .map_err(load_error)?
        .iter()
        .map(|m| std::fs::read(dir.join(m)).map_err(|e| io_error(m, e)))
        .collect::<Result<Vec<_>, _>>()?;
    let deps: BTreeMap<String, PathBuf> =
        request.deps.iter().map(|(n, p)| (n.clone(), dir.join(p))).collect();
    let dependency_keys = depends
        .iter()
        .map(|d| {
            deps.get(d)
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
                .ok_or_else(|| {
                    PartError::new(ErrorKind::Dependency, format!("dependency {d:?} was not built"))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let options = RunOptions {
        project_dir: Some(dir.clone()),
        values: values.iter().map(|v| (v.id.clone(), v.value.clone())).collect(),
        parts: PartMode::One { name: name.to_owned(), deps },
        ..RunOptions::default()
    };
    let output = script::run(&text, &options).map_err(|e| PartError::script(Some(&file), &e))?;
    let Some((_, shape)) = output.shapes.iter().find(|(n, _)| n == name) else {
        return Err(load_error(format!("{}: part {name:?} was not built", file.display())));
    };
    let key = project::cache_key(&KeyInputs {
        program_version: key_version(&request.mesh),
        part_file: source,
        modules,
        values_read: output.values_read.clone().into_iter().collect(),
        dependency_keys,
    });

    let folder = request.cache.as_ref().map_or_else(|| cache_dir(dir, name), |c| c.join(name));
    std::fs::create_dir_all(&folder).map_err(|e| io_error(&folder, e))?;
    let base = folder.join(&key);
    let brep = base.with_extension("brep");
    shape.write_brep(&brep).map_err(|e| io_error(&brep, e))?;
    let mesh = shape
        .mesh_with_angle(request.mesh.deflection_mm, request.mesh.angle_deg)
        .map_err(|e| io_error(&base, e))?;
    let qmsh = base.with_extension("qmsh");
    std::fs::write(&qmsh, qmesh::write(&mesh)).map_err(|e| io_error(&qmsh, e))?;
    drop(mesh);

    let valid = shape.is_valid();
    let warnings: Vec<String> =
        if valid { Vec::new() } else { vec!["the shape is not valid (BRepCheck)".to_owned()] };
    let meta = json!({
        "volume": shape.volume(),
        "bbox": probe::bbox(shape),
        "valid": valid,
        "solids": shape.solid_count(),
        "duration_s": start.elapsed().as_secs_f64(),
        "values_read": output.values_read,
        "warnings": warnings,
    });
    // The `.json` comes last and atomically: it marks the entry complete.
    let json_path = base.with_extension("json");
    crate::service::write_atomic(&json_path, meta.to_string().as_bytes())
        .map_err(|e| io_error(&json_path, e))?;
    Ok((key, meta))
}

/// The file declaring `name` and its dependencies.
fn locate(
    dir: &Path,
    file: Option<&Path>,
    name: &str,
) -> Result<(PathBuf, Vec<String>), PartError> {
    let files = match file {
        Some(f) => vec![f.to_owned()],
        None => project::part_files(dir).map_err(load_error)?,
    };
    for file in files {
        let text = std::fs::read_to_string(dir.join(&file)).map_err(|e| io_error(&file, e))?;
        let declared = declare(dir, &file, &text).map_err(load_error)?;
        if let Some(part) = declared.into_iter().find(|p| p.name == name) {
            return Ok((file, part.depends));
        }
    }
    Err(load_error(format!("no part {name:?} in the project")))
}

/// Writes the shapes of `request.deps` as one STEP file `out`.
fn write_step(request: &Request, out: &Path) -> Result<(), PartError> {
    let shapes = request
        .deps
        .iter()
        .map(|(name, brep)| {
            let path = request.dir.join(brep);
            script::read_brep(&path).map(|s| (name.clone(), s)).map_err(|e| io_error(&path, e))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let path = request.dir.join(out);
    script::export_step(&shapes, &path).map_err(|e| io_error(&path, e))
}
