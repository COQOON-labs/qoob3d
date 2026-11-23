//! `qoob3d build|status|export|values|check|task` (AG-SV-02, AG-CK-03,
//! AG-SV-04, ADR-0053): thin clients of the project service in the working
//! directory. The service is started on demand (detached, without our stdio,
//! WA-OP-02); stdout gets exactly the JSON envelope, the exit code is 1 if
//! `ok` is false or a part or check failed.

use std::time::Duration;

use serde_json::{Value, json};

use crate::cli::{ProjectCommand as Command, TaskCommand};
use crate::client::{Client, ProjectDir, START_TIMEOUT, segment, service_command_from_env};

/// Requests that do not wait for builds.
const QUICK: Duration = Duration::from_secs(30);
/// Seconds `qoob3d status` waits for the discovery of changed files.
const SETTLE_S: f64 = 10.0;
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
    let check_failed = envelope["result"]["failed"].as_u64() > Some(0);
    if envelope["ok"] == json!(true) && !part_failed && !check_failed { 0 } else { 1 }
}

fn task(client: &Client, command: &TaskCommand) -> anyhow::Result<Value> {
    match command {
        TaskCommand::List => client.call("GET", "/tasks", None, QUICK),
        TaskCommand::Show { id } => {
            client.call("GET", &format!("/task/{}", segment(id)), None, QUICK)
        }
        TaskCommand::New { title, kind, body } => {
            let body = json!({"title": title, "kind": kind, "body": body.as_deref().unwrap_or("")});
            client.call("POST", "/task", Some(&body), QUICK)
        }
        TaskCommand::Set { id, state, note } => {
            let body = json!({"state": state, "note": note, "role": "human"});
            client.call("POST", &format!("/task/{}", segment(id)), Some(&body), QUICK)
        }
    }
}

fn request(command: &Command) -> anyhow::Result<Value> {
    let dir = std::env::current_dir()?;
    let project = ProjectDir::open(&dir)?;
    let client = Client::connect(&project, &service_command_from_env(), START_TIMEOUT)?;
    match command {
        Command::Status => {
            // A settled state for humans: at most `SETTLE_S` for discovery
            // (ADR-0056), then `/status` whatever it knows.
            let body = json!({"files": null, "wait_s": SETTLE_S});
            client.call("POST", "/discover", Some(&body), QUICK)?;
            client.call("GET", "/status", None, QUICK)
        }
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
        Command::Check { names } => {
            let names = if names.is_empty() { Value::Null } else { json!(names) };
            let body = json!({"names": names, "wait_s": EXPORT_TIMEOUT.as_secs()});
            client.call("POST", "/check", Some(&body), EXPORT_TIMEOUT + WAIT_MARGIN)
        }
        Command::Task { command } => task(&client, command),
    }
}
