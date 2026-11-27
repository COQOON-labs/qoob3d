//! Project folder (docs/requirements/agent.md §2, AG-PJ-01..04): `qoob3d.toml`,
//! part files, target values, module imports, build graph and cache keys.
//!
//! Pure logic without OCCT (ADR-0053): evaluating parts happens in the
//! kernel's script engine inside a worker, this module only reads files and
//! decides what has to be built. Paths handed out are relative to the
//! project root.

mod graph;
mod imports;
pub mod layout;
mod plates;
mod pose;
mod values;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use graph::{Graph, PartDecl};
pub use imports::resolve_imports;
pub use plates::{COLOR_NAMES, Placement, parse_color, plates};
pub use pose::{ARRANGE_GAP_MM, PrintPose, arrange};
pub use values::{Value, ValueStatus, Values};

/// Longest time limit of a job: one day (AG-HD-01).
const MAX_TIME_S: f64 = 86400.0;

/// Largest memory limit: 1 TiB, so the value in bytes fits every integer type
/// it meets (AG-HD-01).
const MAX_MEMORY_MB: u64 = 1_048_576;

/// Why a project could not be loaded. Every variant names the file, id or
/// parts concerned, so the message alone locates the problem.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("cannot read {}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{}: {message}", file.display())]
    Toml { file: PathBuf, message: String },
    #[error("{}: {message}", file.display())]
    Config { file: PathBuf, message: String },
    #[error("{}: value {id:?}: {message}", file.display())]
    Value { file: PathBuf, id: String, message: String },
    #[error("value {id:?} is defined in {} and in {}", first.display(), second.display())]
    DuplicateValue { id: String, first: PathBuf, second: PathBuf },
    #[error("{}: import {module:?}: {message}", file.display())]
    Import { file: PathBuf, module: String, message: String },
    #[error("part {name:?} is declared in {} and in {}", first.display(), second.display())]
    DuplicatePart { name: String, first: PathBuf, second: PathBuf },
    #[error("part {part:?} depends on unknown part {dependency:?}")]
    UnknownDependency { part: String, dependency: String },
    #[error("dependency cycle: {}", cycle.join(" -> "))]
    Cycle { cycle: Vec<String> },
}

/// Contents of `qoob3d.toml` (AG-PJ-01).
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub name: String,
    /// Always `"mm"` (ADR-0006); other units are refused on load.
    pub units: String,
    pub limits: Limits,
    pub workers: Workers,
    pub cache: Cache,
    pub mesh: Mesh,
    pub service: ServiceSettings,
}

/// Longest idle time before a service stops: a week (WS-PRE-05).
const MAX_IDLE_MIN: f64 = 10080.0;

/// The project service (WS-PRE-05).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ServiceSettings {
    /// Minutes without requests and without jobs after which the service
    /// stops; fractions allowed.
    pub idle_min: f64,
}

impl Default for ServiceSettings {
    fn default() -> Self {
        Self { idle_min: 30.0 }
    }
}

/// Per-build limits of a worker (AG-PJ-05).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Limits {
    /// Wall clock time per build.
    pub time_s: f64,
    /// Resident set size.
    pub memory_mb: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { time_s: 60.0, memory_mb: 4096 }
    }
}

/// Worker pool size.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Workers {
    pub count: usize,
    /// Resident memory (MB) above which a worker is replaced after its job
    /// (AG-PF-03).
    pub recycle_mb: u64,
}

impl Default for Workers {
    fn default() -> Self {
        Self { count: 2, recycle_mb: 512 }
    }
}

/// Cache eviction (AG-PF-04).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Cache {
    /// Cache entries kept per part, the newest ones; at least 1.
    pub keep: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self { keep: 5 }
    }
}

/// Tessellation quality of the cached meshes (AG-PF-05).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Mesh {
    /// Largest distance between mesh and surface.
    pub deflection_mm: f64,
    /// Largest angle between neighbouring segments of a curve.
    pub angle_deg: f64,
}

impl Default for Mesh {
    fn default() -> Self {
        Self { deflection_mm: 0.01, angle_deg: 20.0 }
    }
}

/// `qoob3d.toml` as written; `[project]` is required, the rest has defaults.
#[derive(Deserialize)]
struct RawConfig {
    project: RawProject,
    #[serde(default)]
    limits: Limits,
    #[serde(default)]
    workers: Workers,
    #[serde(default)]
    cache: Cache,
    #[serde(default)]
    mesh: Mesh,
    #[serde(default)]
    service: ServiceSettings,
}

#[derive(Deserialize)]
struct RawProject {
    name: String,
    #[serde(default = "default_units")]
    units: String,
}

fn default_units() -> String {
    "mm".to_owned()
}

impl Config {
    /// Reads `<root>/qoob3d.toml`; a folder without it is no project.
    ///
    /// # Errors
    /// Missing or malformed file, units other than mm, limits that are not
    /// positive or above their maximum (AG-HD-01),
    /// `[cache] keep = 0`, non-positive mesh settings, an idle time that is
    /// not positive or above a week.
    pub fn load(root: &Path) -> Result<Self, ProjectError> {
        let file = PathBuf::from(layout::CONFIG_FILE);
        let text = read_text(&root.join(&file))?;
        let raw: RawConfig = toml::from_str(&text)
            .map_err(|e| ProjectError::Toml { file: file.clone(), message: e.to_string() })?;
        let invalid = |message: &str| ProjectError::Config {
            file: file.clone(),
            message: message.to_owned(),
        };
        if raw.project.units != "mm" {
            return Err(invalid("units must be \"mm\""));
        }
        // TOML allows `nan` and `inf`; a job thread must be able to wait for
        // the limit (AG-HD-01).
        let time_s = raw.limits.time_s;
        if !(time_s.is_finite() && time_s > 0.0 && time_s <= MAX_TIME_S) {
            return Err(invalid("[limits] time_s must be greater than 0 and at most 86400"));
        }
        if !(1..=MAX_MEMORY_MB).contains(&raw.limits.memory_mb) {
            return Err(invalid("[limits] memory_mb must be greater than 0 and at most 1048576"));
        }
        if raw.workers.recycle_mb > MAX_MEMORY_MB {
            return Err(invalid("[workers] recycle_mb must be at most 1048576"));
        }
        if raw.workers.count == 0 {
            return Err(invalid("[workers] count must be greater than 0"));
        }
        if raw.cache.keep == 0 {
            return Err(invalid("[cache] keep must be greater than 0"));
        }
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if !positive(raw.mesh.deflection_mm) {
            return Err(invalid("[mesh] deflection_mm must be greater than 0"));
        }
        if !positive(raw.mesh.angle_deg) {
            return Err(invalid("[mesh] angle_deg must be greater than 0"));
        }
        if !(positive(raw.service.idle_min) && raw.service.idle_min <= MAX_IDLE_MIN) {
            return Err(invalid("[service] idle_min must be greater than 0 and at most 10080"));
        }
        Ok(Self {
            name: raw.project.name,
            units: raw.project.units,
            limits: raw.limits,
            workers: raw.workers,
            cache: raw.cache,
            mesh: raw.mesh,
            service: raw.service,
        })
    }
}

/// The part files `parts/*.rhai` of the project, sorted, relative to `root`.
/// Sub folders are not searched; a project without `parts/` has none.
///
/// # Errors
/// The folder exists but cannot be listed.
pub fn part_files(root: &Path) -> Result<Vec<PathBuf>, ProjectError> {
    files_with_extension(root, "parts", "rhai")
}

/// The check files `checks/*.rhai` of the project (AG-CK-01), as [`part_files`].
///
/// # Errors
/// The folder exists but cannot be listed.
pub fn check_files(root: &Path) -> Result<Vec<PathBuf>, ProjectError> {
    files_with_extension(root, "checks", "rhai")
}

/// Files `<root>/<dir>/*.<extension>`, sorted, relative to `root`.
fn files_with_extension(
    root: &Path,
    dir: &str,
    extension: &str,
) -> Result<Vec<PathBuf>, ProjectError> {
    let path = root.join(dir);
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    let io = |source| ProjectError::Io { path: path.clone(), source };
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(io)? {
        let entry = entry.map_err(io)?;
        let name = PathBuf::from(entry.file_name());
        if name.extension().is_some_and(|e| e == extension) && entry.path().is_file() {
            files.push(Path::new(dir).join(name));
        }
    }
    files.sort();
    Ok(files)
}

fn read_text(path: &Path) -> Result<String, ProjectError> {
    std::fs::read_to_string(path)
        .map_err(|source| ProjectError::Io { path: path.to_owned(), source })
}

/// SHA-256 of a file's bytes: how a file enters a cache key (§2.1), so the
/// service can remember it per file instead of reading every file for
/// every key (WS-PRE-01).
pub type FileDigest = [u8; 32];

/// The [`FileDigest`] of `bytes`.
#[must_use]
pub fn file_digest(bytes: &[u8]) -> FileDigest {
    Sha256::digest(bytes).into()
}

/// The [`FileDigest`] of the file `path`, read in chunks, so a large import
/// is never held in memory as a whole.
///
/// # Errors
/// The file cannot be opened or read.
pub fn read_digest(path: &Path) -> std::io::Result<FileDigest> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut chunk = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            return Ok(hash.finalize().into());
        }
        hash.update(&chunk[..n]);
    }
}

/// The inputs of a part's cache key (§2.1).
#[derive(Debug, Clone, PartialEq)]
pub struct KeyInputs {
    /// Everything about the building program that changes its output: the
    /// version, the kernel and the tessellation settings (AG-PF-05,
    /// WS-PRE-02), composed by the caller.
    pub program_version: String,
    /// Digest of the part file.
    pub part_file: FileDigest,
    /// Digests of every transitively imported module, in [`resolve_imports`]
    /// order, followed by those of the files the build imported with
    /// `import_step` and `import_mesh` (AG-SC-24).
    pub modules: Vec<FileDigest>,
    /// Values read in the last build; empty before the first build. The
    /// order does not matter, the key sorts them.
    pub values_read: Vec<(String, serde_json::Value)>,
    /// Cache keys of the dependencies, in declaration order.
    pub dependency_keys: Vec<String>,
}

/// SHA-256 over the inputs in the order of §2.1, as lowercase hex (AG-PJ-04).
///
/// Every input and every list is length-prefixed, so moving bytes from one
/// input into the next changes the key.
#[must_use]
pub fn cache_key(inputs: &KeyInputs) -> String {
    fn field(hash: &mut Sha256, bytes: &[u8]) {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    let mut values: Vec<(&str, String)> =
        inputs.values_read.iter().map(|(id, v)| (id.as_str(), v.to_string())).collect();
    values.sort_unstable();

    let mut hash = Sha256::new();
    field(&mut hash, inputs.program_version.as_bytes());
    field(&mut hash, &inputs.part_file);
    hash.update((inputs.modules.len() as u64).to_le_bytes());
    for module in &inputs.modules {
        field(&mut hash, module);
    }
    hash.update((values.len() as u64).to_le_bytes());
    for (id, value) in &values {
        field(&mut hash, id.as_bytes());
        field(&mut hash, value.as_bytes());
    }
    hash.update((inputs.dependency_keys.len() as u64).to_le_bytes());
    for key in &inputs.dependency_keys {
        field(&mut hash, key.as_bytes());
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
