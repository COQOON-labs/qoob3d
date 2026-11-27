//! Checks of the service (AG-CK-01..03, ADR-0054): the checks declared in
//! `checks/*.rhai` (discovered like parts, in a worker), their last results and
//! `POST /check`. Every check runs as its own worker request under the
//! project limits, so an endless check costs only its own result.
//!
//! A result is `stale` when the check file (or a module it imports) changed
//! or a part the check asked for was built with another key since.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use qoob3d_core::tasks::GateInput;
use serde::Deserialize;
use serde_json::{Value, json};

use super::jobs::Work;
use super::snapshot::{FileDecl, Hashing, Record, Snapshot};
use super::{Answer, Failure, Service, State, body};
use crate::worker::{ErrorKind, PartError, Probe};

/// One declared check.
pub(crate) struct CheckSource {
    pub(crate) name: String,
    pub(crate) file: PathBuf,
    /// Hash of the file and its imported modules.
    fingerprint: String,
}

/// The checks of the project and the check files that do not load.
pub(crate) struct CheckSet {
    pub(crate) sources: Vec<CheckSource>,
    pub(crate) load_errors: Vec<PartError>,
}

impl CheckSet {
    /// The checks, or the load errors as one error (AG-CK-03 runs no check
    /// of a project whose check files do not all load).
    fn complete(self) -> Result<Vec<CheckSource>, PartError> {
        if self.load_errors.is_empty() {
            return Ok(self.sources);
        }
        let messages: Vec<&str> = self.load_errors.iter().map(|e| e.message.as_str()).collect();
        Err(PartError::new(ErrorKind::Load, messages.join("; ")))
    }
}

/// The last run of a check.
pub(crate) struct CheckRecord {
    passed: bool,
    fingerprint: String,
    /// Parts the check asked for, with the key of their last build then.
    parts: BTreeMap<String, Option<String>>,
}

/// All checks of the check files `files` (discovered, ADR-0055), by file
/// and call order. A file under rediscovery contributes its last checks.
///
/// # Errors
/// A check name is declared twice; the message names files and the check.
pub(crate) fn load(files: Vec<FileDecl>) -> Result<CheckSet, String> {
    let mut sources: Vec<CheckSource> = Vec::new();
    let mut load_errors = Vec::new();
    for f in files {
        let names = match f.declared {
            Some(Ok(declared)) => declared.checks,
            Some(Err(error)) => {
                load_errors.push(error);
                continue;
            }
            None => f.last.map(|d| d.checks).unwrap_or_default(),
        };
        for name in names {
            if let Some(first) = sources.iter().find(|s| s.name == name) {
                return Err(format!(
                    "check {name:?} is declared in {} and in {}",
                    first.file.display(),
                    f.file.display()
                ));
            }
            sources.push(CheckSource {
                name,
                file: f.file.clone(),
                fingerprint: f.fingerprint.clone(),
            });
        }
    }
    Ok(CheckSet { sources, load_errors })
}

impl Service {
    /// The checks once every check file is discovered.
    fn check_set(&self) -> Result<CheckSet, PartError> {
        self.declarations(None).1.and_then(load).map_err(|m| PartError::new(ErrorKind::Load, m))
    }
}

/// Key of the last successful build of `part`.
fn built_key<'a>(records: &'a HashMap<String, Record>, part: &str) -> Option<&'a String> {
    records.get(part).and_then(|r| r.built.as_ref()).map(|(key, _)| key)
}

/// `passed`, `failed`, `stale` or `never_run` (AG-CK-03).
fn state(source: &CheckSource, st: &State) -> &'static str {
    let Some(record) = st.checks.get(&source.name) else {
        return "never_run";
    };
    let rebuilt =
        record.parts.iter().any(|(part, key)| built_key(&st.records, part) != key.as_ref());
    if rebuilt || record.fingerprint != source.fingerprint {
        "stale"
    } else if record.passed {
        "passed"
    } else {
        "failed"
    }
}

/// The `checks` list of `/status`.
pub(crate) fn status(sources: &[CheckSource], st: &State) -> Vec<Value> {
    sources.iter().map(|s| json!({"name": s.name, "file": s.file, "state": state(s, st)})).collect()
}

#[derive(Deserialize)]
struct CheckBody {
    #[serde(default)]
    names: Option<Vec<String>>,
    #[serde(default)]
    wait_s: f64,
}

impl Service {
    /// `POST /check` (AG-CK-03).
    pub(super) fn check(&self, bytes: &[u8]) -> Answer {
        let request: CheckBody = body(bytes)?;
        let sources = self.check_set().and_then(CheckSet::complete)?;
        let unknown =
            request.names.iter().flatten().find(|name| !sources.iter().any(|s| s.name == **name));
        if let Some(name) = unknown {
            return Err(Failure::bad_request(format!("unknown check {name:?}")));
        }
        let id = self.enqueue(Work::Check(request.names), 0);
        self.answer_job(id, request.wait_s)
    }

    /// Runs the checks `names` (`None` = all) against the current parts of
    /// `snapshot` and records the results: `{checks, passed, failed}`.
    ///
    /// # Errors
    /// The check files do not load.
    pub(crate) fn run_checks(
        &self,
        snapshot: &Snapshot,
        names: Option<&[String]>,
        cancel: &AtomicBool,
    ) -> Result<Value, PartError> {
        let sources = self.check_set().and_then(CheckSet::complete)?;
        let built = self.built(snapshot);
        let mut checks = Vec::new();
        for source in sources.iter().filter(|s| names.is_none_or(|n| n.contains(&s.name))) {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let probe = Probe::Check { name: source.name.clone() };
            let limits = &snapshot.config.limits;
            let start = std::time::Instant::now();
            let (meta, error) =
                match self.probe(probe, Some(source.file.clone()), built.clone(), limits, cancel) {
                    Ok(meta) => {
                        let error = meta["error"].clone();
                        (meta, error)
                    }
                    Err(e) if e.kind == ErrorKind::Cancelled => break,
                    Err(e) => (Value::Null, json!(e)),
                };
            let expectations = meta["expectations"].as_array().cloned().unwrap_or_default();
            let passed = error.is_null() && expectations.iter().all(|e| e["passed"] == true);
            let mut check = json!({
                "name": source.name,
                "file": source.file,
                "state": if passed { "passed" } else { "failed" },
                "duration_s": start.elapsed().as_secs_f64(),
                "expectations": expectations,
            });
            if !error.is_null() {
                check["error"] = error;
            }
            checks.push(check);

            let mut st = self.lock();
            let parts = meta["parts_read"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|part| (part.to_owned(), built_key(&st.records, part).cloned()))
                .collect();
            let record = CheckRecord { passed, fingerprint: source.fingerprint.clone(), parts };
            st.checks.insert(source.name.clone(), record);
        }
        let passed = checks.iter().filter(|c| c["state"] == "passed").count();
        Ok(json!({"checks": checks, "passed": passed, "failed": checks.len() - passed}))
    }

    /// What the review gate looks at (AG-TK-04): the state of every part
    /// and the result of every check that is neither stale nor missing;
    /// and the load errors of the part and check files (AG-HD-09).
    pub(crate) fn gate_input(&self) -> (GateInput, Vec<String>) {
        let (part_files, check_files) = self.declarations(None);
        let snapshot = part_files.and_then(|files| Snapshot::load(&self.dir, files));
        let set = check_files.and_then(load);
        let mut load_errors: Vec<String> =
            [snapshot.as_ref().err(), set.as_ref().err()].into_iter().flatten().cloned().collect();
        let file_errors = snapshot.iter().flat_map(|s| &s.load_errors);
        let file_errors = file_errors.chain(set.iter().flat_map(|c| &c.load_errors));
        load_errors.extend(file_errors.map(|e| e.message.clone()));
        let sources = set.map(|c| c.sources).unwrap_or_default();
        let views = snapshot.map(|s| self.views(&s, Hashing::Wait)).unwrap_or_default();
        let st = self.lock();
        let parts = views
            .into_iter()
            .map(|v| (v.name, json!(v.state).as_str().unwrap_or_default().to_owned()))
            .collect();
        let checks = sources
            .iter()
            .filter_map(|s| match state(s, &st) {
                "passed" => Some((s.name.clone(), true)),
                "failed" => Some((s.name.clone(), false)),
                _ => None,
            })
            .collect();
        (GateInput { parts, checks }, load_errors)
    }
}
