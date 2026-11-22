//! Checks of the service (AG-CK-01..03, ADR-0054): the checks declared in
//! `checks/*.rhai` (found like parts, without OCCT), their last results and
//! `POST /check`. Every check runs as its own worker request under the
//! project limits, so an endless check costs only its own result.
//!
//! A result is `stale` when the check file (or a module it imports) changed
//! or a part the check asked for was built with another key since.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use qoob3d_core::project::{self, KeyInputs};
use qoob3d_core::tasks::GateInput;
use serde::Deserialize;
use serde_json::{Value, json};

use super::jobs::Work;
use super::snapshot::{Record, Snapshot, declarations, views};
use super::{Answer, Failure, Service, State, body};
use crate::worker::{ErrorKind, PartError, Probe};

/// One declared check.
pub(crate) struct CheckSource {
    pub(crate) name: String,
    pub(crate) file: PathBuf,
    /// Hash of the file and its imported modules.
    fingerprint: String,
}

/// Declared check names by file, valid while the fingerprint is unchanged.
#[derive(Default)]
pub(crate) struct CheckFiles(Mutex<HashMap<PathBuf, (String, Vec<String>)>>);

/// The last run of a check.
pub(crate) struct CheckRecord {
    passed: bool,
    fingerprint: String,
    /// Parts the check asked for, with the key of their last build then.
    parts: BTreeMap<String, Option<String>>,
}

/// All checks of the project in `dir`, by file and call order.
///
/// # Errors
/// A check file cannot be read or evaluated, or a name is declared twice;
/// the message names files and the check.
pub(crate) fn load(dir: &Path, cache: &CheckFiles) -> Result<Vec<CheckSource>, String> {
    let read =
        |file: &Path| std::fs::read(dir.join(file)).map_err(|e| format!("{}: {e}", file.display()));
    let mut sources: Vec<CheckSource> = Vec::new();
    for file in project::check_files(dir).map_err(|e| e.to_string())? {
        let source = read(&file)?;
        let modules = project::resolve_imports(dir, &file)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|m| read(m))
            .collect::<Result<Vec<_>, _>>()?;
        let text =
            String::from_utf8(source.clone()).map_err(|e| format!("{}: {e}", file.display()))?;
        let fingerprint = project::cache_key(&KeyInputs {
            program_version: String::new(),
            part_file: source,
            modules,
            values_read: Vec::new(),
            dependency_keys: Vec::new(),
        });
        let mut cache = cache.0.lock().expect("check file cache lock poisoned");
        let names = match cache.get(&file) {
            Some((f, names)) if *f == fingerprint => names.clone(),
            _ => {
                let names: Vec<String> =
                    declarations(dir, &file, &text)?.checks.into_iter().map(|c| c.name).collect();
                cache.insert(file.clone(), (fingerprint.clone(), names.clone()));
                names
            }
        };
        drop(cache);
        for name in names {
            if let Some(first) = sources.iter().find(|s| s.name == name) {
                return Err(format!(
                    "check {name:?} is declared in {} and in {}",
                    first.file.display(),
                    file.display()
                ));
            }
            sources.push(CheckSource {
                name,
                file: file.clone(),
                fingerprint: fingerprint.clone(),
            });
        }
    }
    Ok(sources)
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
        let sources =
            load(&self.dir, &self.check_files).map_err(|m| PartError::new(ErrorKind::Load, m))?;
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
        let sources =
            load(&self.dir, &self.check_files).map_err(|m| PartError::new(ErrorKind::Load, m))?;
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
    /// and the result of every check that is neither stale nor missing.
    pub(crate) fn gate_input(&self) -> GateInput {
        let snapshot = Snapshot::load(&self.dir, &self.declarations);
        let sources = load(&self.dir, &self.check_files).unwrap_or_default();
        let mut st = self.lock();
        let parts = snapshot
            .map(|s| views(&self.dir, &s, &mut st.records))
            .unwrap_or_default()
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
        GateInput { parts, checks }
    }
}
