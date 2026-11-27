//! Build jobs of the service (agent.md §3.1, AG-PJ-04, AG-SV-01): a queue
//! worked off one job at a time; inside a job, parts whose dependencies are
//! done build in parallel on the worker pool. A part whose cache key has a
//! complete cache entry is `cached`, not built again.

use std::collections::{BTreeMap, HashMap};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use qoob3d_core::project::layout;
use serde_json::{Value, json};

use super::Service;
use super::snapshot::{Discovered, PartSource, Snapshot, cached, evict, record};
use crate::worker::{Answer, ErrorKind, PartError, Request};

/// Finished jobs kept for `/job/<id>`.
const KEEP_JOBS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub(crate) fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// What a job does.
#[derive(Clone)]
pub(crate) enum Work {
    /// Builds the parts and their dependencies; `None` = all.
    Build(Option<Vec<String>>),
    /// Builds all parts, then runs the checks; `None` = all (AG-CK-03).
    Check(Option<Vec<String>>),
    /// Runs a throwaway script (AG-SV-05); with `meshes` its shapes are
    /// meshed for a picture (AG-MC-12).
    Run { source: String, meshes: bool },
}

pub(crate) struct Job {
    pub(crate) state: JobState,
    work: Work,
    /// Parts to build including dependencies.
    total: usize,
    /// Result per finished part (§3.1).
    results: BTreeMap<String, Value>,
    /// Result of the checks or of the script.
    output: Option<Value>,
    /// Load, limit or script error that stopped the job.
    pub(crate) error: Option<PartError>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// `build`, `check` or `run` (AG-HD-10).
    pub(crate) fn kind(&self) -> &'static str {
        match self.work {
            Work::Build(_) => "build",
            Work::Check(_) => "check",
            Work::Run { .. } => "run",
        }
    }

    /// `GET /job/<id>` (§3.1).
    pub(crate) fn json(&self, id: u64) -> Value {
        let mut j = json!({
            "id": id,
            "state": self.state,
            "progress": {"done": self.results.len(), "total": self.total},
        });
        if self.state.finished() {
            j["result"] = self.result();
        }
        if let Some(e) = &self.error {
            j["error"] = json!(e);
        }
        j
    }

    /// The output of checks or a script; otherwise the build result: per
    /// part, and the names built, cached and failed.
    pub(crate) fn result(&self) -> Value {
        if let Some(output) = &self.output {
            return output.clone();
        }
        let named = |state: &str| -> Vec<&String> {
            self.results.iter().filter(|(_, r)| r["state"] == state).map(|(n, _)| n).collect()
        };
        json!({
            "parts": self.results,
            "built": named("built"),
            "cached": named("cached"),
            "failed": named("failed"),
        })
    }

    /// Requested parts that did not build, with their errors.
    pub(crate) fn failures(&self) -> Vec<String> {
        self.results
            .iter()
            .filter(|(_, r)| r["state"] == "failed")
            .map(|(n, r)| format!("{n}: {}", r["error"]["message"].as_str().unwrap_or_default()))
            .collect()
    }

    pub(crate) fn key(&self, part: &str) -> Option<String> {
        self.results.get(part)?["key"].as_str().map(str::to_owned)
    }
}

/// How one part of a job ended.
enum Outcome {
    Built { key: String, meta: Value },
    Cached { meta: Value },
    Failed(PartError),
    Cancelled,
}

impl Outcome {
    fn from_worker(answer: Result<Answer, PartError>) -> Self {
        match answer {
            Ok(Answer { ok: true, key: Some(key), meta: Some(meta), .. }) if meta.is_object() => {
                Self::Built { key, meta }
            }
            Ok(a) => Self::Failed(a.error.unwrap_or_else(|| {
                PartError::new(ErrorKind::Crash, "the worker answered without a result")
            })),
            Err(e) if e.kind == ErrorKind::Cancelled => Self::Cancelled,
            Err(e) => Self::Failed(e),
        }
    }
}

/// Runs `work`; a panic in it becomes an error of kind `crash` instead of
/// taking the thread (and with it the job queue) down (AG-HD-01).
pub(super) fn caught<T>(work: impl FnOnce() -> T) -> Result<T, PartError> {
    std::panic::catch_unwind(AssertUnwindSafe(work)).map_err(|panic| {
        let text = panic
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        PartError::new(ErrorKind::Crash, format!("the job thread panicked: {text}"))
    })
}

impl Service {
    /// Queues a build of `parts` (`None` = all) and returns the job id.
    ///
    /// # Errors
    /// The project does not load, or a part name is unknown.
    pub(crate) fn submit(&self, parts: Option<Vec<String>>) -> Result<u64, PartError> {
        let snapshot = self.snapshot(None).map_err(|m| PartError::new(ErrorKind::Load, m))?;
        let total = snapshot
            .closure(parts.as_deref())
            .map_err(|m| PartError::new(ErrorKind::BadRequest, m))?
            .len();
        Ok(self.enqueue(Work::Build(parts), total))
    }

    /// Queues `work` and returns the job id.
    pub(crate) fn enqueue(&self, work: Work, total: usize) -> u64 {
        let mut st = self.lock();
        st.next_job += 1;
        let id = st.next_job;
        if st.jobs.len() >= KEEP_JOBS
            && let Some(old) = st.jobs.iter().find(|(_, j)| j.state.finished()).map(|(i, _)| *i)
        {
            st.jobs.remove(&old);
        }
        let job = Job {
            state: JobState::Queued,
            work,
            total,
            results: BTreeMap::new(),
            output: None,
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        st.jobs.insert(id, job);
        st.queue.push_back(id);
        self.changed.notify_all();
        id
    }

    /// Waits until job `id` is finished or `timeout` passed (`None` = no
    /// limit); true if finished.
    pub(crate) fn wait(&self, id: u64, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|t| Instant::now() + t);
        let mut st = self.lock();
        loop {
            match st.jobs.get(&id) {
                Some(j) if !j.state.finished() => {}
                Some(_) => return true,
                None => return false,
            }
            let left = deadline
                .map_or(Duration::from_secs(1), |d| d.saturating_duration_since(Instant::now()));
            if left.is_zero() {
                return false;
            }
            st = self.changed.wait_timeout(st, left).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// Cancels job `id` (§3.1); its state, `None` if unknown.
    pub(crate) fn cancel(&self, id: u64) -> Option<JobState> {
        let mut st = self.lock();
        let job = st.jobs.get_mut(&id)?;
        job.cancel.store(true, Ordering::SeqCst);
        if job.state == JobState::Queued {
            job.state = JobState::Cancelled;
            st.queue.retain(|q| *q != id);
        }
        let state = st.jobs.get(&id).map(|j| j.state);
        self.changed.notify_all();
        state
    }

    /// Cancels every job (shutdown).
    pub(crate) fn cancel_all(&self) {
        let ids: Vec<u64> = self.lock().jobs.keys().copied().collect();
        for id in ids {
            self.cancel(id);
        }
    }

    /// The job runner thread: one job after the other until shutdown.
    pub(crate) fn run_jobs(&self) {
        loop {
            let id = {
                let mut st = self.lock();
                loop {
                    if self.stopping.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Some(id) = st.queue.pop_front() {
                        if let Some(job) = st.jobs.get_mut(&id) {
                            job.state = JobState::Running;
                        }
                        self.changed.notify_all();
                        break id;
                    }
                    st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
                }
            };
            self.run_job(id);
        }
    }

    fn run_job(&self, id: u64) {
        let Some((work, cancel)) =
            self.lock().jobs.get(&id).map(|j| (j.work.clone(), Arc::clone(&j.cancel)))
        else {
            return;
        };
        let output = caught(|| match &work {
            Work::Run { source, meshes } => {
                self.run_script(source, meshes.then_some(id), &cancel).map(Some)
            }
            Work::Build(parts) => self.build_job(id, parts.as_deref(), &cancel).map(|_| None),
            Work::Check(names) => self
                .build_job(id, None, &cancel)
                .and_then(|snapshot| self.run_checks(&snapshot, names.as_deref(), &cancel))
                .map(Some),
        })
        .flatten();
        let mut st = self.lock();
        if let Some(job) = st.jobs.get_mut(&id) {
            match output {
                Ok(output) => job.output = output,
                Err(e) => job.error = Some(e),
            }
            let red = job.results.values().any(|r| r["state"] == "failed")
                || job.output.as_ref().is_some_and(|o| o["failed"].as_u64() > Some(0));
            job.state = if cancel.load(Ordering::SeqCst) {
                JobState::Cancelled
            } else if job.error.is_some() || red {
                JobState::Failed
            } else {
                JobState::Done
            };
        }
        self.changed.notify_all();
    }

    /// Loads the project and builds `requested` (`None` = all) with their
    /// dependencies; the snapshot that was built. Waits for the discovery of
    /// every part file (ADR-0055). A build of all parts builds what loads and
    /// then fails with the load errors of the files that do not.
    fn build_job(
        &self,
        id: u64,
        requested: Option<&[String]>,
        cancel: &AtomicBool,
    ) -> Result<Snapshot, PartError> {
        let load = |m| PartError::new(ErrorKind::Load, m);
        let snapshot = self.snapshot(None).map_err(load)?;
        let names = snapshot.closure(requested).map_err(load)?;
        if let Some(job) = self.lock().jobs.get_mut(&id) {
            job.total = names.len();
        }
        self.build_parts(id, &snapshot, &names, cancel);
        match snapshot.load_errors.as_slice() {
            [] => Ok(snapshot),
            _ if requested.is_some() => Ok(snapshot),
            [one] => Err(one.clone()),
            many => {
                let messages: Vec<&str> = many.iter().map(|e| e.message.as_str()).collect();
                Err(load(messages.join("; ")))
            }
        }
    }

    /// Builds `names` (build order) of `snapshot`, in parallel where the
    /// dependencies allow.
    fn build_parts(&self, id: u64, snapshot: &Snapshot, names: &[String], cancel: &AtomicBool) {
        let limits = &snapshot.config.limits;
        let mut pending: Vec<&PartSource> = names.iter().filter_map(|n| snapshot.part(n)).collect();
        // Key (the final one, or the expected one if the part did not build)
        // and success of every finished part.
        let mut done: HashMap<&str, (String, bool)> = HashMap::new();
        std::thread::scope(|scope| {
            let (tx, rx) = mpsc::channel();
            let mut running = 0;
            loop {
                while let Some(i) = pending
                    .iter()
                    .position(|p| p.depends.iter().all(|d| done.contains_key(d.as_str())))
                {
                    let part = pending.remove(i);
                    let dependency_keys: Vec<String> =
                        part.depends.iter().map(|d| done[d.as_str()].0.clone()).collect();
                    let reads = record(&mut self.lock().records, &self.dir, &part.name).reads();
                    let key = snapshot.key(&self.dir, part, &reads, dependency_keys);
                    let failed_dependency = part.depends.iter().find(|d| !done[d.as_str()].1);
                    let outcome = if cancel.load(Ordering::SeqCst) {
                        Outcome::Cancelled
                    } else if let Discovered::Failed(e) = &part.discovered {
                        Outcome::Failed(e.clone())
                    } else if matches!(part.discovered, Discovered::Running) {
                        let message = format!("{}: discovery did not finish", part.file.display());
                        Outcome::Failed(PartError::new(ErrorKind::Load, message))
                    } else if let Some(d) = failed_dependency {
                        let message = format!("dependency {d:?} did not build");
                        Outcome::Failed(PartError::new(ErrorKind::Dependency, message))
                    } else if let Some(meta) = cached(&self.dir, &part.name, &key) {
                        Outcome::Cached { meta }
                    } else {
                        let deps = part
                            .depends
                            .iter()
                            .map(|d| {
                                let brep =
                                    layout::cache_dir(&self.dir, d).join(&done[d.as_str()].0);
                                (d.clone(), brep.with_extension("brep"))
                            })
                            .collect();
                        let request = Request {
                            job: self.next_request(),
                            dir: self.dir.clone(),
                            part: Some(part.name.clone()),
                            file: Some(part.file.clone()),
                            deps,
                            step: None,
                            mesh: snapshot.config.mesh,
                            cache: None,
                            probe: None,
                        };
                        let tx = tx.clone();
                        scope.spawn(move || {
                            // Caught here, so the part fails and the loop
                            // below still gets its message (AG-HD-01).
                            let outcome = caught(|| {
                                let run = self.pool.run(&request, limits, cancel);
                                let outcome = Outcome::from_worker(run);
                                if let Outcome::Built { key, .. } = &outcome {
                                    let keep = snapshot.config.cache.keep;
                                    evict(&self.dir, &part.name, key, keep);
                                }
                                outcome
                            })
                            .unwrap_or_else(Outcome::Failed);
                            tx.send((part, key, outcome)).ok();
                        });
                        running += 1;
                        continue;
                    };
                    done.insert(&part.name, self.finish(id, &part.name, key, outcome));
                }
                if running == 0 {
                    break;
                }
                let Ok((part, key, outcome)) = rx.recv() else { break };
                running -= 1;
                done.insert(&part.name, self.finish(id, &part.name, key, outcome));
            }
        });
    }

    /// Records how `part` ended in the job and in the part's record;
    /// returns its key and success.
    fn finish(&self, id: u64, part: &str, key: String, outcome: Outcome) -> (String, bool) {
        let mut st = self.lock();
        let record = record(&mut st.records, &self.dir, part);
        let (key, result) = match outcome {
            Outcome::Built { key, meta } => {
                record.built = Some((key.clone(), meta.clone()));
                record.failed = None;
                (key.clone(), part_result("built", &key, meta))
            }
            Outcome::Cached { meta } => {
                record.built = Some((key.clone(), meta.clone()));
                record.failed = None;
                (key.clone(), part_result("cached", &key, meta))
            }
            Outcome::Failed(error) => {
                record.failed = Some((key.clone(), error.clone()));
                (key, json!({"state": "failed", "error": error}))
            }
            Outcome::Cancelled => {
                let error = PartError::new(ErrorKind::Cancelled, "cancelled");
                (key, json!({"state": "failed", "error": error}))
            }
        };
        let ok = result["state"] != "failed";
        if let Some(job) = st.jobs.get_mut(&id) {
            job.results.insert(part.to_owned(), result);
        }
        self.changed.notify_all();
        (key, ok)
    }
}

/// A built or cached part: the cache `.json` fields (an object) plus
/// `state` and `key` (§3.1).
fn part_result(state: &str, key: &str, mut meta: Value) -> Value {
    meta["state"] = json!(state);
    meta["key"] = json!(key);
    meta
}
