//! `qoob3d mcp [DIR]` (AG-MC-01..06, ADR-0054): the MCP server of a project
//! over stdio. JSON-RPC 2.0 by hand, one message per line, no async
//! runtime: a request is answered before the next line is read. stdout
//! carries nothing but protocol messages; logs go to stderr.
//!
//! `tools.rs` holds the tools (most forward to the project service, started
//! on demand), `files.rs` the confined file tools, `render.rs` the image,
//! `resources.rs` the embedded reference, guide and examples.

mod files;
mod render;
mod resources;
mod tools;

use std::io::{BufRead as _, Write as _};
use std::path::Path;

use serde_json::{Value, json};

use crate::live::ProjectDir;

/// Protocol versions this server speaks; the last one is offered to
/// clients that ask for anything else.
const VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Code and message of a JSON-RPC error.
type RpcError = (i64, String);

struct Server {
    project: ProjectDir,
}

/// Serves the project in `dir` (default: the working directory) until
/// stdin closes.
///
/// # Errors
/// `dir` is no project, or stdin/stdout fail.
pub fn run(dir: Option<&Path>) -> anyhow::Result<()> {
    let project = match dir {
        Some(dir) => ProjectDir::open(dir)?,
        None => ProjectDir::open(&std::env::current_dir()?)?,
    };
    tracing::info!(dir = %project.dir().display(), "MCP server on stdio");
    let server = Server { project };
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = server.handle(&line) {
            writeln!(out, "{answer}")?;
            out.flush()?;
        }
    }
    Ok(())
}

fn error(id: Value, (code, message): RpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// `initialize`: the client's protocol version if we speak it, else our newest.
fn initialize(params: &Value) -> Value {
    let wanted = params["protocolVersion"].as_str().unwrap_or_default();
    let version = VERSIONS.iter().find(|v| **v == wanted).unwrap_or(&VERSIONS[2]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}, "resources": {}},
        "serverInfo": {"name": "qoob3d", "version": env!("CARGO_PKG_VERSION")},
        "instructions": resources::GUIDE,
    })
}

impl Server {
    /// The answer to one line; `None` for notifications and responses.
    fn handle(&self, line: &str) -> Option<Value> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Some(error(Value::Null, (PARSE_ERROR, "the line is not valid JSON".into())));
        };
        let id = message.get("id").cloned();
        let Some(method) = message["method"].as_str() else {
            // A response to a request of ours (we send none) is ignored.
            let response = message.get("result").is_some() || message.get("error").is_some();
            let invalid = (INVALID_REQUEST, "a request needs a method".to_owned());
            return (!response).then(|| error(id.unwrap_or(Value::Null), invalid));
        };
        // Notifications (no id) are never answered.
        let id = id?;
        let params = &message["params"];
        let result = match method {
            "initialize" => Ok(initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(tools::list()),
            "tools/call" => self.call(params),
            "resources/list" => Ok(resources::list()),
            "resources/read" => resources::read(params),
            _ => Err((METHOD_NOT_FOUND, format!("unknown method {method:?}"))),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => error(id, e),
        })
    }
}
