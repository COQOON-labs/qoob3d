//! `POST /run` and `POST /measure` (AG-SV-05, AG-SV-06): questions about
//! the built parts, answered by a worker that writes nothing.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use qoob3d_core::project::layout;
use qoob3d_core::project::{Config, Limits, Mesh};
use serde::Deserialize;
use serde_json::Value;

use super::jobs::Work;
use super::snapshot::{Hashing, Snapshot, State};
use super::{Answer, Failure, Service, body, unknown_job};
use crate::worker::{self, ErrorKind, PartError, Probe, Request};

#[derive(Deserialize)]
struct RunBody {
    source: String,
    #[serde(default)]
    wait_s: f64,
    /// Write the meshes of the shown shapes (AG-MC-12).
    #[serde(default)]
    meshes: bool,
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
        let views = self.views(snapshot, Hashing::Wait);
        views
            .into_iter()
            .filter(|v| v.state == State::Current)
            .filter_map(|v| {
                let brep =
                    layout::cache_dir(&self.dir, &v.name).join(v.key?).with_extension("brep");
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
            // `mesh_report` in checks tessellates like the parts (AG-SC-34).
            mesh: Config::load(&self.dir).map_or_else(|_| Mesh::default(), |c| c.mesh),
            cache: None,
            probe: Some(probe),
        };
        match self.work(&request, limits, cancel)? {
            worker::Answer { ok: true, meta: Some(meta), .. } => Ok(meta),
            answer => Err(answer.error.unwrap_or_else(|| {
                PartError::new(ErrorKind::Crash, "the worker answered without a result")
            })),
        }
    }

    /// `POST /run` (AG-SV-05).
    pub(super) fn run(&self, bytes: &[u8]) -> Answer {
        let request: RunBody = body(bytes)?;
        let id = self.enqueue(Work::Run { source: request.source, meshes: request.meshes }, 0);
        self.answer_job(id, request.wait_s)
    }

    /// The job behind `/run`: `{shapes, printed}`; with `meshes` (the job
    /// id) the shapes' meshes go to `.qoob3d/run/<job>/` (AG-MC-12), in the
    /// project's `[mesh]` quality.
    ///
    /// # Errors
    /// The project does not load, a limit, or the script error.
    pub(crate) fn run_script(
        &self,
        source: &str,
        meshes: Option<u64>,
        cancel: &AtomicBool,
    ) -> Result<Value, PartError> {
        let snapshot = self.snapshot(None).map_err(|m| PartError::new(ErrorKind::Load, m))?;
        let folder = meshes.map(|job| {
            let full = layout::run_dir(&self.dir, job);
            full.strip_prefix(&self.dir).map_or(full.clone(), std::path::Path::to_path_buf)
        });
        let probe = Probe::Run { source: source.to_owned(), meshes: folder };
        let request = Request {
            job: self.next_request(),
            dir: self.dir.clone(),
            part: None,
            file: None,
            deps: self.built(&snapshot),
            step: None,
            mesh: snapshot.config.mesh,
            cache: None,
            probe: Some(probe),
        };
        match self.work(&request, &snapshot.config.limits, cancel)? {
            worker::Answer { ok: true, meta: Some(meta), .. } => Ok(meta),
            answer => Err(answer.error.unwrap_or_else(|| {
                PartError::new(ErrorKind::Crash, "the worker answered without a result")
            })),
        }
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
                    (p.clone(), layout::cache_dir(&self.dir, p).join(key).with_extension("brep"))
                })
                .collect()
        };
        let limits =
            self.snapshot(None).map_err(|m| PartError::new(ErrorKind::Load, m))?.config.limits;
        let probe = Probe::Measure { parts };
        Ok(self.probe(probe, None, deps, &limits, &AtomicBool::new(false))?)
    }
}
