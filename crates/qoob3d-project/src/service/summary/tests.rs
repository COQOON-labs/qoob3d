//! The counts of `summary.json` (WS-PRE-04).

use serde_json::json;

use super::of;

#[test]
fn ws_pre_04_every_state_is_counted_and_the_known_ones_always_appear() {
    let status = json!({
        "project": "p",
        "parts": [{"state": "current"}, {"state": "current"}, {"state": "rebuilding"}],
        "checks": [{"state": "passed"}, {"state": "skipped"}],
        "jobs": [{"state": "running"}, {"state": "done"}],
        "load_errors": [],
    });
    let summary = of(&status);
    assert_eq!(
        summary["parts"],
        json!({"current": 2, "outdated": 0, "failed": 0, "never_built": 0, "rebuilding": 1})
    );
    assert_eq!(
        summary["checks"],
        json!({"passed": 1, "failed": 0, "stale": 0, "never_run": 0, "skipped": 1})
    );
    assert_eq!(summary["jobs_running"], json!(1));
}
