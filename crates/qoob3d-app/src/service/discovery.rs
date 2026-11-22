//! Declaration discovery (ADR-0056): what the part and check files declare.
//! The service never evaluates a script: every file whose bytes (or
//! literally imported modules) changed goes to a worker as `Probe::Declare`,
//! under the project's time limit and the worker's memory limit, on a thread
//! of its own. The answers, load errors included, are cached by those bytes,
//! so a file that hits a limit is not evaluated on every poll.
//!
//! Callers say how long they wait: `/status` and `/mesh` never wait and
//! answer from the last known declarations (AG-SV-01: 100 ms); builds,
//! checks and the review gate wait until every file is known; `POST
//! /discover` waits for chosen files (the MCP `file_write` reports their
//! declarations at once). The cache
//! lock is never held while a worker runs or a thread starts.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use qoob3d_core::project::{self, Config, Limits, Mesh};
use serde::Deserialize;
use serde_json::{Value, json};

use super::jobs::caught;
use super::snapshot::{Declared, FileDecl, Snapshot, load_error_of};
use super::{Failure, Service, body};
use crate::worker::{Answer, Declaration, ErrorKind, PartError, Probe, Request};

/// Declarations by file, valid while the fingerprint is unchanged.
#[derive(Default)]
pub(crate) struct Discovery {
    files: Mutex<HashMap<PathBuf, Entry>>,
    /// Signalled whenever a discovery ends.
    done: Condvar,
}

struct Entry {
    fingerprint: String,
    /// `None` while the worker runs.
    declared: Option<Declared>,
    /// The last declarations of the file that loaded.
    last: Option<Declaration>,
}

impl Discovery {
    fn lock(&self) -> MutexGuard<'_, HashMap<PathBuf, Entry>> {
        self.files.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stores the answer for `file` if it still belongs to its bytes; a
    /// cancelled discovery (`None`, shutdown) is forgotten.
    fn store(&self, file: &Path, fingerprint: &str, declared: Option<Declared>) {
        let mut files = self.lock();
        if files.get(file).is_some_and(|e| e.fingerprint == fingerprint) {
            match declared {
                Some(declared) => {
                    if let Some(entry) = files.get_mut(file) {
                        if let Ok(d) = &declared {
                            entry.last = Some(d.clone());
                        }
                        entry.declared = Some(declared);
                    }
                }
                None => {
                    files.remove(file);
                }
            }
        }
        drop(files);
        self.done.notify_all();
    }
}

/// The declarations in a worker's answer to `Probe::Declare` for `file`;
/// `None` if it was cancelled.
fn declared(file: &Path, answer: Result<Answer, PartError>) -> Option<Declared> {
    let named = |mut e: PartError| {
        if e.file.is_none() {
            e.message = format!("{}: {}", file.display(), e.message);
            e.file = Some(file.display().to_string());
        }
        e
    };
    match answer {
        Ok(Answer { ok: true, meta: Some(meta), .. }) => Some(
            serde_json::from_value(meta)
                .map_err(|e| load_error_of(file, format!("{}: discovery: {e}", file.display()))),
        ),
        Ok(answer) => Some(Err(named(answer.error.unwrap_or_else(|| {
            PartError::new(ErrorKind::Crash, "the worker answered without a result")
        })))),
        Err(e) if e.kind == ErrorKind::Cancelled => None,
        Err(e) => Some(Err(named(e))),
    }
}

/// `POST /discover`.
#[derive(Deserialize)]
struct DiscoverBody {
    /// Part or check files relative to the project; `None` = all.
    #[serde(default)]
    files: Option<Vec<PathBuf>>,
    #[serde(default)]
    wait_s: f64,
}

/// The files still being discovered, for `/status` and `/discover`.
pub(crate) fn discovering(files: &[FileDecl]) -> Vec<&Path> {
    files.iter().filter(|f| f.declared.is_none()).map(|f| f.file.as_path()).collect()
}

/// One file of the `/discover` answer: `declared`, `failed` (with
/// `error`) or `discovering`.
fn file_json(f: &FileDecl) -> Value {
    match &f.declared {
        Some(Ok(d)) => json!({"file": f.file, "state": "declared", "parts": d.parts,
            "checks": d.checks}),
        Some(Err(e)) => json!({"file": f.file, "state": "failed", "error": e}),
        None => json!({"file": f.file, "state": "discovering"}),
    }
}

impl Service {
    /// `POST /discover {"files": [..]|null, "wait_s": f64}` (ADR-0056):
    /// waits up to `wait_s` for the discovery of those part and check files
    /// (others are ignored) and answers `{"files": [{file, state, parts?,
    /// checks?, error?}], "discovering": [..]}`.
    pub(super) fn discover_request(&self, bytes: &[u8]) -> super::Answer {
        let request: DiscoverBody = body(bytes)?;
        let wait = Duration::try_from_secs_f64(request.wait_s).unwrap_or(Duration::ZERO);
        let listed = |files: Result<Vec<PathBuf>, project::ProjectError>| {
            files.map_err(|e| Failure::from(PartError::new(ErrorKind::Load, e)))
        };
        let mut files = listed(project::part_files(&self.dir))?;
        files.extend(listed(project::check_files(&self.dir))?);
        if let Some(wanted) = &request.files {
            files.retain(|f| wanted.contains(f));
        }
        let known = self.discover(files, Some(wait));
        Ok(json!({
            "files": known.iter().map(file_json).collect::<Vec<_>>(),
            "discovering": discovering(&known),
        }))
    }

    /// The part files and the check files of the project with what
    /// discovery knows of them, after waiting up to `wait` (`None`: until
    /// every file is known). An error lists a folder that cannot be read.
    pub(crate) fn declarations(
        &self,
        wait: Option<Duration>,
    ) -> (Result<Vec<FileDecl>, String>, Result<Vec<FileDecl>, String>) {
        let parts = project::part_files(&self.dir).map_err(|e| e.to_string());
        let checks = project::check_files(&self.dir).map_err(|e| e.to_string());
        let files = [&parts, &checks].into_iter().flatten().flatten().cloned().collect();
        let mut known = self.discover(files, wait).into_iter();
        let parts = parts.map(|files| known.by_ref().take(files.len()).collect());
        let checks = checks.map(|_| known.collect());
        (parts, checks)
    }

    /// The project with its part files, after waiting up to `wait` for
    /// their discovery (`None`: until all are known).
    ///
    /// # Errors
    /// A load error of the whole project (see [`Snapshot::load`]).
    pub(crate) fn snapshot(&self, wait: Option<Duration>) -> Result<Snapshot, String> {
        let files = project::part_files(&self.dir).map_err(|e| e.to_string())?;
        Snapshot::load(&self.dir, self.discover(files, wait))
    }

    /// `files` with their declarations: starts a discovery for every file
    /// whose bytes are not known yet and waits up to `wait` for the running
    /// ones.
    fn discover(&self, files: Vec<PathBuf>, wait: Option<Duration>) -> Vec<FileDecl> {
        let mut decls: Vec<FileDecl> =
            files.into_iter().map(|f| FileDecl::read(&self.dir, f)).collect();
        let deadline = wait.map(|w| Instant::now() + w);
        let mut known = self.discovery.lock();
        let mut new = Vec::new();
        for f in decls.iter().filter(|f| f.declared.is_none()) {
            if known.get(&f.file).is_some_and(|e| e.fingerprint == f.fingerprint) {
                continue;
            }
            let last = known.remove(&f.file).and_then(|e| e.last);
            let entry = Entry { fingerprint: f.fingerprint.clone(), declared: None, last };
            known.insert(f.file.clone(), entry);
            new.push(f);
        }
        drop(known);
        for f in new {
            if let Err(e) = self.start(f) {
                self.discovery.store(&f.file, &f.fingerprint, Some(Err(e)));
            }
        }
        let mut known = self.discovery.lock();
        loop {
            let running = decls.iter().any(|f| {
                f.declared.is_none()
                    && known
                        .get(&f.file)
                        .is_some_and(|e| e.fingerprint == f.fingerprint && e.declared.is_none())
            });
            let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            if !running || left.is_some_and(|l| l.is_zero()) {
                break;
            }
            let chunk = left.unwrap_or(Duration::from_secs(1));
            known = self
                .discovery
                .done
                .wait_timeout(known, chunk)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        for f in &mut decls {
            let Some(entry) = known.get(&f.file) else { continue };
            f.last.clone_from(&entry.last);
            if f.declared.is_none() && entry.fingerprint == f.fingerprint {
                f.declared.clone_from(&entry.declared);
            }
        }
        decls
    }

    /// Sends `file` to a worker on a thread of its own; the answer goes to
    /// the discovery cache.
    ///
    /// # Errors
    /// The thread could not be started.
    fn start(&self, file: &FileDecl) -> Result<(), PartError> {
        let limits = Config::load(&self.dir).map_or_else(|_| Limits::default(), |c| c.limits);
        let request = Request {
            job: self.next_request(),
            dir: self.dir.clone(),
            part: None,
            file: Some(file.file.clone()),
            deps: BTreeMap::new(),
            step: None,
            mesh: Mesh::default(),
            cache: None,
            probe: Some(Probe::Declare),
        };
        let (pool, discovery) = (Arc::clone(&self.pool), Arc::clone(&self.discovery));
        let (path, fingerprint) = (file.file.clone(), file.fingerprint.clone());
        std::thread::Builder::new()
            .name("qoob3d-declare".into())
            .spawn(move || {
                let answer = caught(|| pool.run(&request, &limits, &AtomicBool::new(false)));
                discovery.store(&path, &fingerprint, declared(&path, answer.flatten()));
            })
            .map(drop)
            .map_err(|e| {
                load_error_of(&file.file, format!("{}: discovery: {e}", file.file.display()))
            })
    }
}
