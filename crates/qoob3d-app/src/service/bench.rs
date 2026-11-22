//! `qoob3d bench <DIR>` (AG-PF-06, agent.md §5): builds every part of the
//! project uncached, [`RUNS`] times, and reports the faster run per part.
//! Every build gets a fresh worker, so its resident memory belongs to that
//! part alone. The entries go to an own folder that is removed afterwards:
//! the project cache and a running service are left alone.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use qoob3d_core::qmesh;
use serde_json::{Value, json};

use super::pool::Pool;
use super::snapshot::{Declarations, Snapshot};
use super::{Answer, Failure};
use crate::live::ProjectDir;
use crate::worker::{ErrorKind, PartError, Request};

/// Builds per part; the faster one is reported.
const RUNS: usize = 2;

/// The bench result `{"parts": {name: {build_s, peak_memory_mb, triangles,
/// mesh_bytes}}}`.
pub(super) fn run(dir: &Path) -> Answer {
    let project = ProjectDir::open(dir).map_err(|e| Failure::not_found(format!("{e:#}")))?;
    let dir = project.dir();
    let snapshot = Snapshot::load(dir, &Declarations::default())
        .map_err(|m| PartError::new(ErrorKind::Load, m))?;
    let exe = std::env::current_exe().map_err(|e| Failure::new(500, ErrorKind::Io, e))?;
    let cache = dir.join(".qoob3d").join(format!("bench-{}", std::process::id()));
    let result = measure(dir, &snapshot, &exe, &cache);
    std::fs::remove_dir_all(&cache).ok();
    result
}

fn measure(dir: &Path, snapshot: &Snapshot, exe: &Path, cache: &Path) -> Answer {
    let mut parts: BTreeMap<&str, Value> = BTreeMap::new();
    for _ in 0..RUNS {
        // BREP of every part built in this run, for its dependents.
        let mut breps: HashMap<&str, PathBuf> = HashMap::new();
        for part in &snapshot.parts {
            let name = part.name.as_str();
            let fail = |kind, message: String| {
                Failure::from(PartError::new(kind, format!("part {name}: {message}")))
            };
            let request = Request {
                job: 1,
                dir: dir.to_owned(),
                part: Some(part.name.clone()),
                file: Some(part.file.clone()),
                deps: part.depends.iter().map(|d| (d.clone(), breps[d.as_str()].clone())).collect(),
                step: None,
                mesh: snapshot.config.mesh,
                cache: Some(cache.to_owned()),
                probe: None,
            };
            // Never recycled: the pool only lives for this build.
            let pool = Pool::start(exe.to_owned(), 1, u64::MAX);
            let answer = pool.run(&request, &snapshot.config.limits, &AtomicBool::new(false));
            let peak_memory_mb = pool.peak_mb();
            pool.close(Duration::ZERO);
            let answer = answer.map_err(|e| fail(e.kind, e.message))?;
            let (Some(key), Some(meta)) = (answer.key, answer.meta) else {
                let e = answer.error.unwrap_or_else(|| {
                    PartError::new(ErrorKind::Crash, "the worker answered without a result")
                });
                return Err(fail(e.kind, e.message));
            };
            let base = cache.join(name).join(key);
            let qmsh = base.with_extension("qmsh");
            let io =
                |message: String| fail(ErrorKind::Io, format!("{}: {message}", qmsh.display()));
            let bytes = std::fs::read(&qmsh).map_err(|e| io(e.to_string()))?;
            let mesh = qmesh::read(&bytes).map_err(|e| io(format!("{e:?}")))?;
            let build_s = meta["duration_s"].as_f64().unwrap_or(f64::NAN);
            let faster = parts.get(name).is_none_or(|p| p["build_s"].as_f64() > Some(build_s));
            if faster {
                let measures = json!({
                    "build_s": build_s,
                    "peak_memory_mb": peak_memory_mb,
                    "triangles": mesh.faces.indices.len() / 3,
                    "mesh_bytes": bytes.len(),
                });
                parts.insert(name, measures);
            }
            breps.insert(name, base.with_extension("brep"));
        }
    }
    Ok(json!({"parts": parts}))
}
