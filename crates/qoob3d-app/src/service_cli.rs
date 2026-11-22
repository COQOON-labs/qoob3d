//! `qoob3d build|status|export|values` (AG-SV-02, ADR-0053): thin clients of
//! the project service in the working directory. The service is started on
//! demand (detached, without our stdio, WA-OP-02); stdout gets exactly the
//! JSON envelope, the exit code is 1 if `ok` is false or a part failed.

use std::time::Duration;

use serde_json::{Value, json};

use crate::cli::ProjectCommand as Command;
use crate::live::{Client, ProjectDir, START_TIMEOUT, service_command_from_env};

/// Requests that do not wait for builds.
const QUICK: Duration = Duration::from_secs(30);
/// Margin over `--wait` for the answer of a build request.
const WAIT_MARGIN: Duration = Duration::from_secs(30);
/// An export builds first; its answer may take as long as the builds.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(3600);

/// Runs `command` and prints its envelope; the process exit code.
pub fn run(command: &Command) -> i32 {
    let started = std::time::Instant::now();
    let envelope = request(command).unwrap_or_else(|e| {
        json!({
            "ok": false,
            "duration_s": started.elapsed().as_secs_f64(),
            "warnings": [],
            "result": null,
            "error": {"kind": "service", "message": format!("{e:#}")},
        })
    });
    println!("{envelope}");
    let part_failed = envelope["result"]["parts"]
        .as_object()
        .is_some_and(|parts| parts.values().any(|p| p["state"] == "failed"));
    if envelope["ok"] == json!(true) && !part_failed { 0 } else { 1 }
}

fn request(command: &Command) -> anyhow::Result<Value> {
    let dir = std::env::current_dir()?;
    let project = ProjectDir::open(&dir)?;
    let client = Client::connect(&project, &service_command_from_env(), START_TIMEOUT)?;
    match command {
        Command::Status => client.call("GET", "/status", None, QUICK),
        Command::Values => client.call("GET", "/values", None, QUICK),
        Command::Build { parts, wait } => {
            let parts = if parts.is_empty() { Value::Null } else { json!(parts) };
            let timeout =
                Duration::try_from_secs_f64(*wait).unwrap_or(Duration::ZERO) + WAIT_MARGIN;
            let body = json!({"parts": parts, "wait_s": wait});
            client.call("POST", "/build", Some(&body), timeout)
        }
        Command::Export { parts, format } => {
            let body = json!({"parts": parts, "format": format});
            client.call("POST", "/export", Some(&body), EXPORT_TIMEOUT)
        }
    }
}
