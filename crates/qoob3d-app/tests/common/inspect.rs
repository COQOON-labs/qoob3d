//! Looking at a project from outside, for `tests/agent.rs`, `tests/mcp.rs`,
//! `tests/mcp_guide.rs` and `tests/service.rs` (every item is used by each
//! of them, see `common/agent.rs`): a handle for a service someone else
//! started, waiting for discovery, file contents and coordinates.
//!
//! Included with `#[path = "common/inspect.rs"] mod inspect;` next to
//! `common/agent.rs` as `mod support`.

use serde_json::{Value, json};

use crate::support::{Project, Service};

impl Project {
    pub(crate) fn read(&self, rel: &str) -> String {
        let path = self.path(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }
}

impl Service {
    /// Handle for a service started by someone else (CLI or MCP, on demand,
    /// AG-SV-02); dropping it leaves that service running.
    pub(crate) fn attach(p: &Project) -> Self {
        Self::wait_ready(p, None, p.root.join("serve.log"))
    }

    /// `POST /discover {"files": null, "wait_s": 20}` (ADR-0055, §3.1):
    /// waits until every part and check file is discovered and asserts that
    /// nothing is left in `discovering`. Discovery runs in a worker, so the
    /// first `/status` after a start or a file write may not list the
    /// declarations yet; call this before reading parts, checks or
    /// `load_errors` from `/status`. 20 s: the start-up budget (WA-OP-02),
    /// far above one worker round trip per file. Returns the result.
    pub(crate) fn wait_discovery(&self) -> Value {
        let r = self.request("POST", "/discover", Some(&json!({"files": null, "wait_s": 20})));
        let result = r.ok();
        assert_eq!(result["discovering"], json!([]), "discovery did not finish: {result}");
        result
    }
}

/// A JSON array `[x, y, z]` against `expected`, absolute tolerance `tol` (mm).
pub(crate) fn assert_vec(v: &Value, expected: [f64; 3], tol: f64, what: &str) {
    for (i, e) in expected.iter().enumerate() {
        let a = v[i].as_f64().unwrap_or_else(|| panic!("{what}[{i}] missing: {v}"));
        assert!((a - e).abs() < tol, "{what}: got {v}, expected {expected:?}");
    }
}
