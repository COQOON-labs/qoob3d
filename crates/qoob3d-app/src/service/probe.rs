//! `POST /run` and `POST /measure` (AG-SV-05, AG-SV-06): questions about
//! the built parts, answered by a worker that writes nothing.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use qoob3d_core::project::{Limits, Mesh};
use serde::Deserialize;
use serde_json::Value;

use super::jobs::Work;
use super::snapshot::{Snapshot, State, cache_dir, views};
use super::{Answer, Failure, Service, body, unknown_job};
use crate::worker::{self, ErrorKind, PartError, Probe, Request};

#[derive(Deserialize)]
struct RunBody {
    source: String,
    #[serde(default)]
    wait_s: f64,
}

#[derive(Deserialize)]
struct MeasureBody {
    a: String,
    #[serde(default)]
    b: Option<String>,
}

impl Service {
    /// BREP files of the parts of `snapshot` that are built and current.
    pub(crate) fn built(&self, snapshot: &Snapshot) -> BTreeMap<String, PathBuf> {
        let views = views(&self.dir, snapshot, &mut self.lock().records);
        views
            .into_iter()
            .filter(|v| v.state == State::Current)
            .filter_map(|v| {
                let brep = cache_dir(&self.dir, &v.name).join(v.key?).with_extension("brep");
                Some((v.name, brep))
            })
            .collect()
    }

    /// Runs `probe` on a worker within `limits`; its result.
    ///
    /// # Errors
    /// Limit, crash, cancellation, or the failure the worker reports.
    pub(crate) fn probe(
        &self,
        probe: Probe,
        file: Option<PathBuf>,
        deps: BTreeMap<String, PathBuf>,
        limits: &Limits,
        cancel: &AtomicBool,
    ) -> Result<Value, PartError> {
        let request = Request {
            job: self.next_request(),
            dir: self.dir.clone(),
            part: None,
            file,
            deps,
            step: None,
            mesh: Mesh::default(),
            cache: None,
            probe: Some(probe),
        };
        match self.pool.run(&request, limits, cancel)? {
            worker::Answer { ok: true, meta: Some(meta), .. } => Ok(meta),
            answer => Err(answer.error.unwrap_or_else(|| {
                PartError::new(ErrorKind::Crash, "the worker answered without a result")
            })),
        }
    }

    /// `POST /run` (AG-SV-05).
    pub(super) fn run(&self, bytes: &[u8]) -> Answer {
        let request: RunBody = body(bytes)?;
        let id = self.enqueue(Work::Run(request.source), 0);
        self.answer_job(id, request.wait_s)
    }

    /// The job behind `/run`: `{shapes, printed}`.
    ///
    /// # Errors
    /// The project does not load, a limit, or the script error.
    pub(crate) fn run_script(&self, source: &str, cancel: &AtomicBool) -> Result<Value, PartError> {
        let snapshot = self.snapshot(None).map_err(|m| PartError::new(ErrorKind::Load, m))?;
        let probe = Probe::Run { source: source.to_owned() };
        self.probe(probe, None, self.built(&snapshot), &snapshot.config.limits, cancel)
    }

    /// `POST /measure` (AG-SV-06): builds the parts if needed, then measures
    /// them in a worker.
    pub(super) fn measure(&self, bytes: &[u8]) -> Answer {
        let request: MeasureBody = body(bytes)?;
        let parts: Vec<String> = std::iter::once(request.a).chain(request.b).collect();
        let id = self.submit(Some(parts.clone()))?;
        self.wait(id, None);
        let deps = {
            let st = self.lock();
            let job = st.jobs.get(&id).ok_or_else(|| unknown_job(id))?;
            if let Some(e) = &job.error {
                return Err(Failure::from(e.clone()));
            }
            let failures = job.failures();
            if !failures.is_empty() {
                let message = format!("not measured, parts failed: {}", failures.join("; "));
                return Err(Failure::new(400, ErrorKind::Script, message));
            }
            parts
                .iter()
                .map(|p| {
                    let key = job.key(p).unwrap_or_default();
                    (p.clone(), cache_dir(&self.dir, p).join(key).with_extension("brep"))
                })
                .collect()
        };
        let limits =
            self.snapshot(None).map_err(|m| PartError::new(ErrorKind::Load, m))?.config.limits;
        let probe = Probe::Measure { parts };
        Ok(self.probe(probe, None, deps, &limits, &AtomicBool::new(false))?)
    }
}
