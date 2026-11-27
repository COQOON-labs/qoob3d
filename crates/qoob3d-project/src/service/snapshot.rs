//! The project as the service sees it at one moment (agent.md §2.1,
//! AG-PJ-02/04): configuration, values, declared parts in build order with
//! the bytes their cache keys hash, and the build state of every part.
//!
//! Declarations come from discovery (`discovery.rs`): a worker evaluates
//! each part and check file with a recording `part()` and without the CAD
//! API, so the service never evaluates a script (ADR-0055). A file that does
//! not load is a load error of that file; the other files' parts stay.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use qoob3d_core::project::layout;
use qoob3d_core::project::{self, Config, FileDigest, Graph, KeyInputs, PartDecl, Values};
use serde_json::Value;

use super::Service;
use super::hashes::Hashes;
use crate::worker::{Declaration, ErrorKind, PartError, key_version};

/// What a part or check file declares, or why it does not load.
pub(crate) type Declared = Result<Declaration, PartError>;

/// A part or check file with what discovery knows of it.
pub(crate) struct FileDecl {
    pub(crate) file: PathBuf,
    /// Hash of the file and its literally imported modules: the key of the
    /// declaration cache.
    pub(crate) fingerprint: String,
    /// Digest of the file.
    digest: FileDigest,
    /// The modules it imports literally (AG-HD-06: the cache key before the
    /// first build).
    modules: Vec<PathBuf>,
    /// The declarations of these bytes; `None` while discovery runs.
    pub(crate) declared: Option<Declared>,
    /// The last declarations of the file that loaded, from earlier bytes
    /// too: what `/status` shows while the file is rediscovered or broken.
    pub(crate) last: Option<Declaration>,
}

impl FileDecl {
    /// Finds the digests of `file` (relative to `dir`) and of the modules
    /// it imports literally, through `hashes` (WS-PRE-01); a file that
    /// cannot be read is a load error of its own.
    pub(crate) fn read(dir: &Path, file: PathBuf, hashes: &Hashes) -> Self {
        // The scan reads the file, so a file that cannot be read fails here.
        match hashes.imports(dir, &file) {
            Ok(modules) => {
                let digest = hashes.digest(&dir.join(&file));
                let fingerprint = project::cache_key(&KeyInputs {
                    program_version: String::new(),
                    part_file: digest,
                    modules: modules.iter().map(|m| hashes.digest(&dir.join(m))).collect(),
                    values_read: Vec::new(),
                    dependency_keys: Vec::new(),
                });
                Self { file, fingerprint, digest, modules, declared: None, last: None }
            }
            Err(e) => Self {
                declared: Some(Err(load_error_of(&file, e.to_string()))),
                file,
                fingerprint: String::new(),
                digest: FileDigest::default(),
                modules: Vec::new(),
                last: None,
            },
        }
    }
}

/// A load error of `file` (kind `load`).
pub(crate) fn load_error_of(file: &Path, message: String) -> PartError {
    let mut error = PartError::new(ErrorKind::Load, message);
    error.file = Some(file.display().to_string());
    error
}

/// How far the declaration of a part is known.
#[derive(Debug, Clone)]
pub(crate) enum Discovered {
    /// From the current bytes of its file.
    Current,
    /// From earlier bytes; discovery of the current ones runs.
    Running,
    /// From earlier bytes; the current ones do not load.
    Failed(PartError),
}

/// One declared part with the inputs of its cache key.
pub(crate) struct PartSource {
    pub(crate) name: String,
    pub(crate) file: PathBuf,
    pub(crate) depends: Vec<String>,
    /// How it lies on the bed for printing (AG-PJ-06).
    pub(crate) print: Option<qoob3d_core::project::PrintPose>,
    /// `#RRGGBB` and the part it prints with (AG-PJ-07).
    pub(crate) color: Option<String>,
    pub(crate) print_with: Option<String>,
    pub(crate) discovered: Discovered,
    digest: FileDigest,
    modules: Vec<PathBuf>,
}

pub(crate) struct Snapshot {
    pub(crate) config: Config,
    pub(crate) values: Values,
    /// In build order: every part after its dependencies.
    pub(crate) parts: Vec<PartSource>,
    /// Part files that do not load (AG-PJ-02, ADR-0055).
    pub(crate) load_errors: Vec<PartError>,
}

impl Snapshot {
    /// The project with the part files `files` as discovery knows them.
    ///
    /// # Errors
    /// A load error of the whole project (AG-PJ-01..03: configuration,
    /// values, part names, duplicates, dependencies), as a message naming
    /// files, ids or parts.
    pub(crate) fn load(dir: &Path, files: Vec<FileDecl>) -> Result<Self, String> {
        let config = Config::load(dir).map_err(|e| e.to_string())?;
        let values = Values::load(dir).map_err(|e| e.to_string())?;
        let mut sources = Vec::new();
        let mut load_errors = Vec::new();
        for f in files {
            let (declaration, discovered) = match f.declared {
                Some(Ok(declaration)) => (Some(declaration), Discovered::Current),
                Some(Err(error)) => {
                    load_errors.push(error.clone());
                    (f.last, Discovered::Failed(error))
                }
                None => (f.last, Discovered::Running),
            };
            for part in declaration.map(|d| d.parts).unwrap_or_default() {
                if !plain_name(&part.name) {
                    return Err(format!(
                        "{}: part name {:?} must not be empty, start with '.' or contain / or \\",
                        f.file.display(),
                        part.name
                    ));
                }
                sources.push(PartSource {
                    name: part.name,
                    file: f.file.clone(),
                    depends: part.depends,
                    print: part.print,
                    color: part.color,
                    print_with: part.print_with,
                    discovered: discovered.clone(),
                    digest: f.digest,
                    modules: f.modules.clone(),
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
        Ok(Self { config, values, parts, load_errors })
    }

    pub(crate) fn part(&self, name: &str) -> Option<&PartSource> {
        self.parts.iter().find(|p| p.name == name)
    }

    /// Cache key of `part` with the current values and module files that
    /// its last build read (`reads`) and the keys of its dependencies (§2.1).
    /// Before the first build the modules are the literal imports found by
    /// the scan (AG-HD-06). `digest` gives the digest of an absolute path,
    /// `None` while it is not known yet (WS-PRE-01); then the key is `None`.
    pub(crate) fn key(
        &self,
        dir: &Path,
        part: &PartSource,
        reads: &Reads,
        dependency_keys: Vec<String>,
        digest: &mut dyn FnMut(&Path) -> Option<FileDigest>,
        hashes: &Hashes,
    ) -> Option<String> {
        let values_read = reads
            .values
            .iter()
            .map(|id| (id.clone(), self.values.get(id).map_or(Value::Null, |v| v.value.clone())))
            .collect();
        // A module or imported file that is gone hashes as empty: the key
        // changes, the part is outdated and its build reports the missing
        // file. Imported files follow the modules, as in the worker.
        let modules = reads.modules.as_deref().unwrap_or(&part.modules);
        let modules = modules
            .iter()
            .chain(&reads.files)
            .map(|m| digest(&dir.join(m)))
            .collect::<Option<Vec<_>>>()?;
        let inputs = KeyInputs {
            program_version: key_version(&self.config.mesh),
            part_file: part.digest,
            modules,
            values_read,
            dependency_keys,
        };
        Some(hashes.key(&part.name, &inputs))
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
#[derive(Debug, Default, Clone)]
pub(crate) struct Reads {
    /// Value ids.
    values: Vec<String>,
    /// Module files the engine resolved, relative to the project; `None`
    /// before the first build.
    modules: Option<Vec<PathBuf>>,
    /// Files `import_step` and `import_mesh` read (AG-SC-24).
    files: Vec<PathBuf>,
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
        let paths = |key: &str| {
            meta[key]
                .as_array()
                .map(|files| files.iter().filter_map(Value::as_str).map(PathBuf::from).collect())
        };
        Reads {
            values,
            modules: paths("modules_read"),
            files: paths("files_read").unwrap_or_default(),
        }
    }
}

/// Cache `.json` of `part` with `key`, if the entry is complete.
pub(crate) fn cached(dir: &Path, part: &str, key: &str) -> Option<Value> {
    let base = layout::cache_dir(dir, part).join(key);
    if !base.with_extension("brep").is_file() || !base.with_extension("qmsh").is_file() {
        return None;
    }
    let text = std::fs::read_to_string(base.with_extension("json")).ok()?;
    serde_json::from_str(&text).ok().filter(Value::is_object)
}

/// Removes all but the newest `keep` cache entries of `part` (by the
/// modification time of the `.json`), never the entry `current` (AG-PF-04).
pub(crate) fn evict(dir: &Path, part: &str, current: &str, keep: usize) {
    let folder = layout::cache_dir(dir, part);
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
    let entries = std::fs::read_dir(layout::cache_dir(dir, part)).ok()?;
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
    /// The print pose, for parts that declare one (AG-PJ-06).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pose: Option<qoob3d_core::project::PrintPose>,
    /// `#RRGGBB`, for parts that declare a colour (AG-PJ-07).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) color: Option<String>,
    /// The part it prints with as one multi-colour object (AG-PJ-07).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) print_with: Option<String>,
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

/// How [`Service::views`] gets the digests of files.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hashing {
    /// Hashes what it needs (builds, checks, the review gate).
    Wait,
    /// Never waits for a large file (`/status`, `/mesh`): parts whose key
    /// needs one that is still hashed are `outdated` meanwhile.
    Background,
}

/// What [`Service::views`] needs of a part's record, copied out of the lock.
struct Seen {
    reads: Reads,
    built: Option<String>,
    failed: Option<(String, PartError)>,
}

impl Service {
    /// The records of the parts of `snapshot`, completed from their cache
    /// folders on first use; the folders are read outside the lock.
    fn seen(&self, snapshot: &Snapshot) -> Vec<Seen> {
        let dir = &self.dir;
        let unscanned: Vec<&str> = {
            let st = self.lock();
            snapshot
                .parts
                .iter()
                .map(|p| p.name.as_str())
                .filter(|name| st.records.get(*name).is_none_or(|r| !r.scanned))
                .collect()
        };
        let found: Vec<(&str, Option<(String, Value)>)> =
            unscanned.into_iter().map(|name| (name, newest(dir, name))).collect();
        let mut st = self.lock();
        for (name, newest) in found {
            let r = st.records.entry(name.to_owned()).or_default();
            if !r.scanned {
                r.scanned = true;
                if r.built.is_none() {
                    r.built = newest;
                }
            }
        }
        snapshot
            .parts
            .iter()
            .map(|p| {
                let r = st.records.get(&p.name);
                Seen {
                    reads: r.map(Record::reads).unwrap_or_default(),
                    built: r.and_then(|r| r.built.as_ref()).map(|(k, _)| k.clone()),
                    failed: r.and_then(|r| r.failed.clone()),
                }
            })
            .collect()
    }

    /// Expected key and state of every part, in build order. The service
    /// lock is held only to copy and update records, never while files are
    /// read or hashed (WS-PRE-01).
    pub(crate) fn views(&self, snapshot: &Snapshot, hashing: Hashing) -> Vec<PartView> {
        let dir = &self.dir;
        let seen = self.seen(snapshot);
        let hashes = &self.hashes;
        // Once per call: parts share modules and imported files.
        let mut asked: HashMap<PathBuf, Option<FileDigest>> = HashMap::new();
        let mut digest = |path: &Path| {
            if let Some(known) = asked.get(path) {
                return *known;
            }
            let digest = match hashing {
                Hashing::Wait => Some(hashes.digest(path)),
                Hashing::Background => hashes.known(path),
            };
            asked.insert(path.to_owned(), digest);
            digest
        };
        // `None`: not known while a file is hashed.
        let mut keys: HashMap<&str, Option<String>> = HashMap::new();
        // Complete cache entries found for the expected key: (part, built
        // key seen, new built entry).
        let mut entries = Vec::new();
        let mut out = Vec::with_capacity(snapshot.parts.len());
        for (part, seen) in snapshot.parts.iter().zip(seen) {
            let dependency_keys: Option<Vec<String>> = part
                .depends
                .iter()
                .map(|d| keys.get(d.as_str()).cloned().unwrap_or(Some(String::new())))
                .collect();
            let expected = dependency_keys
                .and_then(|deps| snapshot.key(dir, part, &seen.reads, deps, &mut digest, hashes));
            let mut error = None;
            let mut built = seen.built.clone();
            let state = if let Discovered::Failed(e) = &part.discovered {
                error = Some(e.clone());
                State::Failed
            } else if matches!(part.discovered, Discovered::Running) {
                // ADR-0055: earlier declarations while the file is rediscovered.
                State::Outdated
            } else if let Some(expected) = &expected {
                if seen.failed.as_ref().is_some_and(|(k, _)| k == expected) {
                    error = seen.failed.as_ref().map(|(_, e)| e.clone());
                    State::Failed
                } else if seen.built.as_ref() == Some(expected) {
                    State::Current
                } else if let Some(meta) = cached(dir, &part.name, expected) {
                    entries.push((
                        part.name.as_str(),
                        seen.built.clone(),
                        (expected.clone(), meta),
                    ));
                    built = Some(expected.clone());
                    State::Current
                } else if seen.built.is_some() || seen.failed.is_some() {
                    State::Outdated
                } else {
                    State::NeverBuilt
                }
            } else if seen.built.is_some() || seen.failed.is_some() {
                State::Outdated
            } else {
                State::NeverBuilt
            };
            out.push(PartView {
                name: part.name.clone(),
                file: part.file.display().to_string(),
                state,
                key: built,
                error,
                pose: part.print.clone(),
                color: part.color.clone(),
                print_with: part.print_with.clone(),
            });
            keys.insert(&part.name, expected);
        }
        if !entries.is_empty() {
            let mut st = self.lock();
            for (name, seen, entry) in entries {
                // Unless a build recorded another entry meanwhile.
                if let Some(r) = st.records.get_mut(name)
                    && r.built.as_ref().map(|(k, _)| k) == seen.as_ref()
                {
                    r.built = Some(entry);
                }
            }
        }
        out
    }
}
