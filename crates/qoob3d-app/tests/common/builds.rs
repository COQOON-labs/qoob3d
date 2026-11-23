//! `/status` and `/build` of a [`Service`] for `tests/service.rs` and
//! `tests/perf.rs` (every item is used by each of them, see
//! `common/agent.rs`; docs/requirements/agent.md §3.1).
//!
//! Included with `#[path = "common/builds.rs"] mod builds;`.

use serde_json::{Value, json};

use crate::support::{Reply, Service};

/// `wait_s` for synchronous builds; simple boxes build in milliseconds.
pub(crate) const WAIT_S: f64 = 60.0;

impl Service {
    pub(crate) fn get(&self, path: &str) -> Reply {
        self.request("GET", path, None)
    }

    pub(crate) fn post(&self, path: &str, body: &Value) -> Reply {
        self.request("POST", path, Some(body))
    }

    /// `GET /status`: HTTP 200 and its `result`.
    pub(crate) fn status(&self) -> Value {
        let r = self.get("/status");
        assert_eq!(r.code, 200);
        r.ok()
    }

    /// `POST /build` with `wait_s` = [`WAIT_S`]; the build result (§3.1),
    /// also when parts failed.
    pub(crate) fn build(&self, parts: Option<&[&str]>) -> Value {
        let r = self.post("/build", &json!({"parts": parts, "wait_s": WAIT_S}));
        let v = r.envelope();
        let result = v["result"].clone();
        assert!(result["parts"].is_object(), "no build result: {v}\nlog:\n{}", self.log());
        result
    }
}
