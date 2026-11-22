//! The project as the service sees it at one moment (agent.md §2.1,
//! AG-PJ-02/04): configuration, values, declared parts in build order with
//! the bytes their cache keys hash, and the build state of every part.
//!
//! Declarations come from evaluating the part files with a recording
//! `part()` and without the CAD API (`PartMode::Declare`), so the service
//! never runs OCCT. Results are cached by the file and module bytes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use qoob3d_core::project::{self, Config, Graph, KeyInputs, PartDecl, Values};
use qoob3d_kernel::script::{self, DeclaredPart, PartMode, RunOptions, RunOutput};
use serde_json::Value;

use crate::worker::{PartError, key_version};

/// Rhai operations allowed for declaring the parts of one file: top-level
/// code is meant to be declarations, not computations.
const DECLARE_MAX_OPERATIONS: u64 = 1_000_000;

/// `.qoob3d/cache/<part>` of the project in `dir`.
pub(crate) fn cache_dir(dir: &Path, part: &str) -> PathBuf {
    dir.join(".qoob3d").join("cache").join(part)
}

/// The parts `text` (the part file `file`) declares.
///
/// # Errors
/// The script fails; the message names the file.
pub(crate) fn declare(dir: &Path, file: &Path, text: &str) -> Result<Vec<DeclaredPart>, String> {
    declarations(dir, file, text).map(|o| o.parts)
}

/// What `text` (the part or check file `file`) declares, without running
/// any closure.
///
/// # Errors
/// The script fails; the message names the file.
pub(crate) fn declarations(dir: &Path, file: &Path, text: &str) -> Result<RunOutput, String> {
    let options = RunOptions {
        max_operations: DECLARE_MAX_OPERATIONS,
        project_dir: Some(dir.to_owned()),
        parts: PartMode::Declare,
        ..RunOptions::default()
    };
    script::run(text, &options).map_err(|e| format!("{}: {e}", file.display()))
}

/// What a part file declares, or why it does not load.
type Declared = Result<Vec<DeclaredPart>, String>;

/// Declarations by file, valid while the file and module bytes are unchanged.
/// A load error is kept as well: `/status` is polled, and a file that runs
/// into the limits of discovery (AG-HD-02) must not cost that run every time.
#[derive(Default)]
pub(crate) struct Declarations(Mutex<HashMap<PathBuf, (String, Declared)>>);

/// One declared part with the inputs of its cache key.
pub(crate) struct PartSource {
    pub(crate) name: String,
    pub(crate) file: PathBuf,
    pub(crate) depends: Vec<String>,
    source: Vec<u8>,
    modules: Vec<Vec<u8>>,
}

pub(crate) struct Snapshot {
    pub(crate) config: Config,
    pub(crate) values: Values,
    /// In build order: every part after its dependencies.
    pub(crate) parts: Vec<PartSource>,
}

fn read(dir: &Path, file: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(dir.join(file)).map_err(|e| format!("{}: {e}", file.display()))
}

impl Snapshot {
    /// Reads the project.
    ///
    /// # Errors
    /// Any load error (AG-PJ-01..03), as a message naming files, ids or parts.
    pub(crate) fn load(dir: &Path, cache: &Declarations) -> Result<Self, String> {
        let config = Config::load(dir).map_err(|e| e.to_string())?;
        let values = Values::load(dir).map_err(|e| e.to_string())?;
        let mut sources = Vec::new();
        for file in project::part_files(dir).map_err(|e| e.to_string())? {
            let source = read(dir, &file)?;
            let modules = project::resolve_imports(dir, &file)
                .map_err(|e| e.to_string())?
                .iter()
                .map(|m| read(dir, m))
                .collect::<Result<Vec<_>, _>>()?;
            for part in declared(dir, &file, &source, &modules, cache)? {
                if !plain_name(&part.name) {
                    return Err(format!(
                        "{}: part name {:?} must not be empty, start with '.' or contain / or \\",
                        file.display(),
                        part.name
                    ));
                }
                sources.push(PartSource {
                    name: part.name,
                    file: file.clone(),
                    depends: part.depends,
                    source: source.clone(),
                    modules: modules.clone(),
                });
            }
        }
        let decls = sources
            .iter()
            .map(|p| PartDecl {
                name: p.name.clone(),
                file: p.file.clone(),
                depends: p.depends.clone(),
            })
            .collect();
        let graph = Graph::new(decls).map_err(|e| e.to_string())?;
        let mut by_name: HashMap<String, PartSource> =
            sources.into_iter().map(|p| (p.name.clone(), p)).collect();
        let parts = graph.order().into_iter().filter_map(|name| by_name.remove(name)).collect();
        Ok(Self { config, values, parts })
    }

    pub(crate) fn part(&self, name: &str) -> Option<&PartSource> {
        self.parts.iter().find(|p| p.name == name)
    }

    /// Cache key of `part` with the current values and module files that
    /// its last build read (`reads`) and the keys of its dependencies (§2.1).
    /// Before the first build the modules are the literal imports found by
    /// the scan (AG-HD-06).
    pub(crate) fn key(
        &self,
        dir: &Path,
        part: &PartSource,
        reads: &Reads,
        dependency_keys: Vec<String>,
    ) -> String {
        let values_read = reads
            .values
            .iter()
            .map(|id| (id.clone(), self.values.get(id).map_or(Value::Null, |v| v.value.clone())))
            .collect();
        // A module that is gone hashes as empty: the key changes, the part
        // is outdated and its build reports the missing file.
        let modules = reads.modules.as_ref().map_or_else(
            || part.modules.clone(),
            |files| files.iter().map(|m| std::fs::read(dir.join(m)).unwrap_or_default()).collect(),
        );
        project::cache_key(&KeyInputs {
            program_version: key_version(&self.config.mesh),
            part_file: part.source.clone(),
            modules,
            values_read,
            dependency_keys,
        })
    }

    /// `names` and all their dependencies in build order; `None` = all parts.
    ///
    /// # Errors
    /// An unknown part name.
    pub(crate) fn closure(&self, names: Option<&[String]>) -> Result<Vec<String>, String> {
        let Some(names) = names else {
            return Ok(self.parts.iter().map(|p| p.name.clone()).collect());
        };
        let mut wanted: Vec<&str> = Vec::new();
        for n in names {
            if self.part(n).is_none() {
                return Err(format!("unknown part {n:?}"));
            }
            wanted.push(n);
        }
        // Reverse build order: a part's dependents come before it.
        let mut needed = std::collections::HashSet::new();
        for p in self.parts.iter().rev() {
            if wanted.contains(&p.name.as_str()) || needed.contains(p.name.as_str()) {
                needed.insert(p.name.as_str());
                needed.extend(p.depends.iter().map(String::as_str));
            }
        }
        Ok(self
            .parts
            .iter()
            .filter(|p| needed.contains(p.name.as_str()))
            .map(|p| p.name.clone())
            .collect())
    }
}

/// A part name usable as a folder name.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\'])
}

fn declared(
    dir: &Path,
    file: &Path,
    source: &[u8],
    modules: &[Vec<u8>],
    cache: &Declarations,
) -> Declared {
    let fingerprint = project::cache_key(&KeyInputs {
        program_version: String::new(),
        part_file: source.to_vec(),
        modules: modules.to_vec(),
        values_read: Vec::new(),
        dependency_keys: Vec::new(),
    });
    let mut cache = cache.0.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((f, parts)) = cache.get(file)
        && *f == fingerprint
    {
        return parts.clone();
    }
    let parts = std::str::from_utf8(source)
        .map_err(|e| format!("{}: {e}", file.display()))
        .and_then(|text| declare(dir, file, text));
    cache.insert(file.to_owned(), (fingerprint, parts.clone()));
    parts
}

// ------------------------------------------------------------------ state

/// The last build outcomes of one part, kept by the service.
#[derive(Debug, Default)]
pub(crate) struct Record {
    /// Last successful build: key and cache `.json` contents.
    pub(crate) built: Option<(String, Value)>,
    /// Key and error of the last failed build.
    pub(crate) failed: Option<(String, PartError)>,
    /// The cache folder was searched for an older build.
    scanned: bool,
}

/// What the last successful build of a part read: the inputs of its cache
/// key that only a run can know.
#[derive(Debug, Default)]
pub(crate) struct Reads {
    /// Value ids.
    values: Vec<String>,
    /// Module files the engine resolved, relative to the project; `None`
    /// before the first build.
    modules: Option<Vec<PathBuf>>,
}

impl Record {
    /// What the last successful build read.
    pub(crate) fn reads(&self) -> Reads {
        let Some((_, meta)) = &self.built else {
            return Reads::default();
        };
        let values = meta["values_read"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        let modules = meta["modules_read"]
            .as_array()
            .map(|files| files.iter().filter_map(Value::as_str).map(PathBuf::from).collect());
        Reads { values, modules }
    }
}

/// Cache `.json` of `part` with `key`, if the entry is complete.
pub(crate) fn cached(dir: &Path, part: &str, key: &str) -> Option<Value> {
    let base = cache_dir(dir, part).join(key);
    if !base.with_extension("brep").is_file() || !base.with_extension("qmsh").is_file() {
        return None;
    }
    let text = std::fs::read_to_string(base.with_extension("json")).ok()?;
    serde_json::from_str(&text).ok().filter(Value::is_object)
}

/// Removes all but the newest `keep` cache entries of `part` (by the
/// modification time of the `.json`), never the entry `current` (AG-PF-04).
pub(crate) fn evict(dir: &Path, part: &str, current: &str, keep: usize) {
    let folder = cache_dir(dir, part);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return;
    };
    let mut others: Vec<(std::time::SystemTime, String)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let key = e.path().file_stem()?.to_string_lossy().into_owned();
            Some((e.metadata().ok()?.modified().ok()?, key))
        })
        .filter(|(_, key)| key != current)
        .collect();
    others.sort();
    let excess = (others.len() + 1).saturating_sub(keep);
    for (_, key) in others.iter().take(excess) {
        // The `.json` first: without it the entry no longer counts as complete.
        for extension in ["json", "brep", "qmsh"] {
            std::fs::remove_file(folder.join(key).with_extension(extension)).ok();
        }
    }
}

/// The newest complete cache entry of `part`: the last build of an earlier
/// service run.
fn newest(dir: &Path, part: &str) -> Option<(String, Value)> {
    let entries = std::fs::read_dir(cache_dir(dir, part)).ok()?;
    let newest = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max()?;
    let key = newest.1.file_stem()?.to_string_lossy().into_owned();
    cached(dir, part, &key).map(|meta| (key, meta))
}

/// Build state of a part (WA-PR-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Current,
    Outdated,
    Failed,
    NeverBuilt,
}

/// One part of `/status`.
#[derive(Debug, serde::Serialize)]
pub(crate) struct PartView {
    pub(crate) name: String,
    pub(crate) file: String,
    pub(crate) state: State,
    /// Key of the last successful build.
    pub(crate) key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<PartError>,
}

/// The record of `name`, completed from the cache folder on first use.
pub(crate) fn record<'a>(
    records: &'a mut HashMap<String, Record>,
    dir: &Path,
    name: &str,
) -> &'a mut Record {
    let r = records.entry(name.to_owned()).or_default();
    if !r.scanned {
        r.scanned = true;
        if r.built.is_none() {
            r.built = newest(dir, name);
        }
    }
    r
}

/// Expected key and state of every part, in build order.
pub(crate) fn views(
    dir: &Path,
    snapshot: &Snapshot,
    records: &mut HashMap<String, Record>,
) -> Vec<PartView> {
    let mut keys: HashMap<&str, String> = HashMap::new();
    let mut out = Vec::with_capacity(snapshot.parts.len());
    for part in &snapshot.parts {
        let r = record(records, dir, &part.name);
        let dependency_keys = part
            .depends
            .iter()
            .map(|d| keys.get(d.as_str()).cloned().unwrap_or_default())
            .collect();
        let expected = snapshot.key(dir, part, &r.reads(), dependency_keys);
        let mut error = None;
        let state = if r.failed.as_ref().is_some_and(|(k, _)| *k == expected) {
            error = r.failed.as_ref().map(|(_, e)| e.clone());
            State::Failed
        } else if r.built.as_ref().is_some_and(|(k, _)| *k == expected) {
            State::Current
        } else if let Some(meta) = cached(dir, &part.name, &expected) {
            r.built = Some((expected.clone(), meta));
            State::Current
        } else if r.built.is_some() || r.failed.is_some() {
            State::Outdated
        } else {
            State::NeverBuilt
        };
        out.push(PartView {
            name: part.name.clone(),
            file: part.file.display().to_string(),
            state,
            key: r.built.as_ref().map(|(k, _)| k.clone()),
            error,
        });
        keys.insert(&part.name, expected);
    }
    out
}
