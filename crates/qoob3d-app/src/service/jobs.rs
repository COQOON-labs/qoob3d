//! Build jobs of the service (agent.md §3.1, AG-PJ-04, AG-SV-01): a queue
//! worked off one job at a time; inside a job, parts whose dependencies are
//! done build in parallel on the worker pool. A part whose cache key has a
//! complete cache entry is `cached`, not built again.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::Service;
use super::snapshot::{PartSource, Snapshot, cache_dir, cached, record};
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

pub(crate) struct Job {
    pub(crate) state: JobState,
    /// Requested parts; `None` = all.
    requested: Option<Vec<String>>,
    /// Parts to build including dependencies.
    total: usize,
    /// Result per finished part (§3.1).
    results: BTreeMap<String, Value>,
    /// Load error that stopped the job.
    pub(crate) error: Option<PartError>,
    cancel: Arc<AtomicBool>,
}

impl Job {
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

    /// The build result: per part, and the names built, cached and failed.
    pub(crate) fn result(&self) -> Value {
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

impl Service {
    /// Queues a build of `parts` (`None` = all) and returns the job id.
    ///
    /// # Errors
    /// The project does not load, or a part name is unknown.
    pub(crate) fn submit(&self, parts: Option<Vec<String>>) -> Result<u64, PartError> {
        let snapshot = Snapshot::load(&self.dir, &self.declarations)
            .map_err(|m| PartError::new(ErrorKind::Load, m))?;
        let total = snapshot
            .closure(parts.as_deref())
            .map_err(|m| PartError::new(ErrorKind::BadRequest, m))?
            .len();
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
            requested: parts,
            total,
            results: BTreeMap::new(),
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        st.jobs.insert(id, job);
        st.queue.push_back(id);
        self.changed.notify_all();
        Ok(id)
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
            st = self.changed.wait_timeout(st, left).expect("service lock poisoned").0;
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
                    st = self.changed.wait(st).expect("service lock poisoned");
                }
            };
            self.run_job(id);
        }
    }

    fn run_job(&self, id: u64) {
        let Some((requested, cancel)) =
            self.lock().jobs.get(&id).map(|j| (j.requested.clone(), Arc::clone(&j.cancel)))
        else {
            return;
        };
        let loaded = Snapshot::load(&self.dir, &self.declarations).and_then(|s| {
            let names = s.closure(requested.as_deref())?;
            Ok((s, names))
        });
        match loaded {
            Ok((snapshot, names)) => {
                if let Some(job) = self.lock().jobs.get_mut(&id) {
                    job.total = names.len();
                }
                self.build_parts(id, &snapshot, &names, &cancel);
            }
            Err(message) => {
                if let Some(job) = self.lock().jobs.get_mut(&id) {
                    job.error = Some(PartError::new(ErrorKind::Load, message));
                }
            }
        }
        let mut st = self.lock();
        if let Some(job) = st.jobs.get_mut(&id) {
            job.state = if cancel.load(Ordering::SeqCst) {
                JobState::Cancelled
            } else if job.error.is_some() || job.results.values().any(|r| r["state"] == "failed") {
                JobState::Failed
            } else {
                JobState::Done
            };
        }
        self.changed.notify_all();
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
                    let key = snapshot.key(part, &reads, dependency_keys);
                    let failed_dependency = part.depends.iter().find(|d| !done[d.as_str()].1);
                    let outcome = if cancel.load(Ordering::SeqCst) {
                        Outcome::Cancelled
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
                                let brep = cache_dir(&self.dir, d).join(&done[d.as_str()].0);
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
                        };
                        let tx = tx.clone();
                        scope.spawn(move || {
                            let answer = self.pool.run(&request, limits, cancel);
                            tx.send((part, key, Outcome::from_worker(answer))).ok();
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
