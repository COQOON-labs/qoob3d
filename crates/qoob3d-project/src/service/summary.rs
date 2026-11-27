//! `.qoob3d/summary.json` (WS-PRE-04): the counts of part, check, job and
//! load-error states, for an overview that reads files and never asks a
//! service (WS-OV-03). A thread of the service takes `/status` every
//! [`INTERVAL`] and writes the file atomically when a count changed, so
//! changes of files nobody asked about arrive too, and it is written at
//! most twice per second. It stays when the service stops, with the last
//! state written after the jobs were cancelled.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use qoob3d_core::fs::write_atomic;
use qoob3d_core::project::layout;
use serde_json::{Map, Value, json};

use super::Service;
use super::tasks::utc_now;

/// How often the state is looked at, and so the shortest time between two
/// writes.
const INTERVAL: Duration = Duration::from_millis(500);
/// How often the thread looks whether the service stops.
const STEP: Duration = Duration::from_millis(50);

/// `{state: count}` of the `state` fields in `list` for every one of `states`.
fn counts(list: &Value, states: &[&str]) -> Value {
    let mut out = Map::new();
    for state in states {
        let n = list.as_array().into_iter().flatten().filter(|e| e["state"] == *state).count();
        out.insert((*state).to_owned(), json!(n));
    }
    Value::Object(out)
}

/// The summary of a `/status` result, without `updated`.
pub(crate) fn of(status: &Value) -> Value {
    let jobs = status["jobs"].as_array().into_iter().flatten();
    let load_errors = status["load_errors"].as_array().map_or(0, Vec::len);
    // A load error of the whole project has no file entry.
    let load_errors =
        if load_errors == 0 && status.get("error").is_some() { 1 } else { load_errors };
    json!({
        "project": status["project"],
        "parts": counts(&status["parts"], &["current", "outdated", "failed", "never_built"]),
        "checks": counts(&status["checks"], &["passed", "failed", "stale", "never_run"]),
        "jobs_running": jobs.filter(|j| j["state"] == "running").count(),
        "load_errors": load_errors,
    })
}

impl Service {
    /// Writes the summary if it differs from `last`; the summary written.
    pub(crate) fn write_summary(&self, last: Option<&Value>) -> Option<Value> {
        let summary = of(&self.status());
        if last == Some(&summary) {
            return None;
        }
        let mut file = summary.clone();
        file["updated"] = json!(utc_now());
        let path = layout::summary_json(&self.dir);
        if let Err(e) = write_atomic(&path, file.to_string().as_bytes()) {
            tracing::warn!("writing {}: {e}", path.display());
            return None;
        }
        Some(summary)
    }

    /// The summary thread, until the service stops.
    pub(crate) fn summarise(&self) {
        let mut last = None;
        while !self.stopping.load(Ordering::SeqCst) {
            let next = Instant::now() + INTERVAL;
            if let Some(written) = self.write_summary(last.as_ref()) {
                last = Some(written);
            }
            while Instant::now() < next && !self.stopping.load(Ordering::SeqCst) {
                std::thread::sleep(STEP);
            }
        }
    }
}
