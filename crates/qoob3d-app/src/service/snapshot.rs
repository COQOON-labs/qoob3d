//! The project as the service sees it at one moment (agent.md §2.1,
//! AG-PJ-02/04): configuration, values, declared parts in build order with
//! the bytes their cache keys hash, and the build state of every part.
//!
//! Declarations come from evaluating the part files with a recording
//! `part()` and without the CAD API (`PartMode::Declare`), so the service
//! never runs OCCT. Results are cached by the file and module bytes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use qoob3d_core::project::{self, Config, Graph, KeyInputs, PartDecl, Values};
use qoob3d_kernel::script::{self, DeclaredPart, PartMode, RunOptions};
use serde_json::Value;

use crate::worker::{PROGRAM_VERSION, PartError};

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
    let options = RunOptions {
        max_operations: DECLARE_MAX_OPERATIONS,
        project_dir: Some(dir.to_owned()),
        parts: PartMode::Declare,
        ..RunOptions::default()
    };
    script::run(text, &options).map(|o| o.parts).map_err(|e| format!("{}: {e}", file.display()))
}

/// Declarations by file, valid while the file and module bytes are unchanged.
#[derive(Default)]
pub(crate) struct Declarations(Mutex<HashMap<PathBuf, (String, Vec<DeclaredPart>)>>);

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

    /// Cache key of `part` with the current values of the ids `reads` and
    /// the keys of its dependencies (§2.1).
    pub(crate) fn key(
        &self,
        part: &PartSource,
        reads: &[String],
        dependency_keys: Vec<String>,
    ) -> String {
        let values_read = reads
            .iter()
            .map(|id| (id.clone(), self.values.get(id).map_or(Value::Null, |v| v.value.clone())))
            .collect();
        project::cache_key(&KeyInputs {
            program_version: PROGRAM_VERSION.to_owned(),
            part_file: part.source.clone(),
            modules: part.modules.clone(),
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
) -> Result<Vec<DeclaredPart>, String> {
    let fingerprint = project::cache_key(&KeyInputs {
        program_version: String::new(),
        part_file: source.to_vec(),
        modules: modules.to_vec(),
        values_read: Vec::new(),
        dependency_keys: Vec::new(),
    });
    let mut cache = cache.0.lock().expect("declaration cache lock poisoned");
    if let Some((f, parts)) = cache.get(file)
        && *f == fingerprint
    {
        return Ok(parts.clone());
    }
    let text = std::str::from_utf8(source).map_err(|e| format!("{}: {e}", file.display()))?;
    let parts = declare(dir, file, text)?;
    cache.insert(file.to_owned(), (fingerprint, parts.clone()));
    Ok(parts)
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

impl Record {
    /// Value ids the last successful build read.
    pub(crate) fn reads(&self) -> Vec<String> {
        self.built
            .as_ref()
            .and_then(|(_, meta)| meta["values_read"].as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
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
        let expected = snapshot.key(part, &r.reads(), dependency_keys);
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
