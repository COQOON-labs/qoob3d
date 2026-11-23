//! MCP client for `tests/mcp.rs` and `tests/mcp_guide.rs` (every item is
//! used by each of them, see `common/agent.rs`): the server `qoob3d mcp
//! [DIR]` over stdio (docs/requirements/agent.md §4.5, AG-MC-01..02,
//! ADR-0054).
//!
//! Transport: one JSON-RPC 2.0 message per line on stdin/stdout. [`Mcp`]
//! reads stdout in a thread and waits for every answer with a timeout, so a
//! hanging server fails the test instead of blocking it. Every line that
//! arrives on stdout is checked to be a JSON-RPC message (AG-MC-01: nothing
//! but protocol messages on stdout). The timeouts are those of the including
//! test file: `crate::QUICK` for protocol messages that start no work,
//! `crate::TOOL` for tool calls.
//!
//! Included with `#[path = "common/mcp.rs"] mod mcp_client;`. [`Mcp`] closes
//! stdin and kills the server on drop.

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{Value, json};

use crate::support::{BIN, Project, check_envelope, wait_until};
use crate::{QUICK, TOOL};

/// The protocol versions the server supports (AG-MC-01), oldest first.
pub(crate) const VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];

/// At most 400 characters of a message for panic texts (renders are megabytes).
pub(crate) fn short(text: &str) -> String {
    if text.len() <= 400 {
        return text.to_owned();
    }
    let head: String = text.chars().take(400).collect();
    format!("{head}… ({} bytes)", text.len())
}

/// A stdout line as JSON-RPC 2.0 message: a response (`id` with exactly one
/// of `result`/`error`) or a notification/request (`method`).
pub(crate) fn protocol_message(line: &str) -> Value {
    let v: Value = serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {}", short(line)));
    assert_eq!(v["jsonrpc"], json!("2.0"), "not JSON-RPC 2.0: {}", short(line));
    let response = v.get("id").is_some() && (v.get("result").is_some() != v.get("error").is_some());
    assert!(response || v["method"].is_string(), "no response, no request: {}", short(line));
    v
}

/// The answer of one `tools/call` (AG-MC-02).
pub(crate) struct Tool {
    pub(crate) envelope: Value,
    pub(crate) content: Vec<Value>,
}

impl Tool {
    /// `result` of an envelope with `ok: true`.
    pub(crate) fn ok(&self) -> &Value {
        assert_eq!(self.envelope["ok"], json!(true), "{}", short(&self.envelope.to_string()));
        &self.envelope["result"]
    }

    /// `result.text` of a text tool (`guide`, `api_reference`).
    pub(crate) fn text(&self) -> String {
        self.ok()["text"]
            .as_str()
            .unwrap_or_else(|| panic!("result.text: {}", short(&self.envelope.to_string())))
            .to_owned()
    }

    /// The call was refused and the error names every word.
    pub(crate) fn refused(&self, words: &[&str], what: &str) {
        let text = self.envelope.to_string();
        assert_eq!(self.envelope["ok"], json!(false), "{what} must be refused: {}", short(&text));
        for w in words {
            assert!(text.contains(w), "{what}: the error must name {w}: {}", short(&text));
        }
        assert_eq!(self.content.len(), 1, "{what}: an error has the text envelope only");
    }
}

/// A running `qoob3d mcp`.
pub(crate) struct Mcp {
    pub(crate) child: Child,
    pub(crate) stdin: Option<ChildStdin>,
    pub(crate) lines: Receiver<String>,
    log: PathBuf,
    next_id: u64,
    /// Names of all tools called so far.
    pub(crate) called: BTreeSet<String>,
}

impl Mcp {
    /// `qoob3d mcp <DIR>`, working directory outside the project.
    pub(crate) fn spawn(p: &Project) -> Self {
        Self::start(Command::new(BIN).arg("mcp").arg(&p.dir).current_dir(&p.root), p)
    }

    /// Spawns `command` (a `qoob3d mcp`) with piped stdio; stderr goes to
    /// `mcp.log` of the project root.
    pub(crate) fn start(command: &mut Command, p: &Project) -> Self {
        let log = p.root.join("mcp.log");
        let err = std::fs::OpenOptions::new().create(true).append(true).open(&log);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(err.expect("open mcp.log"))
            .spawn()
            .expect("spawn qoob3d mcp");
        let stdin = child.stdin.take();
        let out = BufReader::new(child.stdout.take().expect("stdout"));
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in out.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, stdin, lines, log, next_id: 0, called: BTreeSet::new() }
    }

    /// The last 3000 bytes of the server's stderr.
    pub(crate) fn stderr(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let start = text.len().saturating_sub(3000);
        text.get(start..).unwrap_or_default().to_owned()
    }

    /// Writes one line to the server.
    pub(crate) fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        let sent = writeln!(stdin, "{line}").and_then(|()| stdin.flush());
        if let Err(e) = sent {
            panic!("the server closed stdin ({e}); stderr:\n{}", self.stderr());
        }
    }

    /// The next stdout line as checked protocol message.
    pub(crate) fn recv(&mut self, timeout: Duration) -> Value {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => protocol_message(&line),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let (called, stderr) = (&self.called, self.stderr());
                panic!("no answer within {timeout:?} (tools called: {called:?}); stderr:\n{stderr}")
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the server closed stdout without an answer; stderr:\n{}", self.stderr())
            }
        }
    }

    /// The next response; notifications of the server are checked and skipped.
    pub(crate) fn response(&mut self, timeout: Duration) -> Value {
        loop {
            let message = self.recv(timeout);
            if message.get("method").is_none() {
                return message;
            }
        }
    }

    /// A request with a fresh numeric id; the whole response.
    pub(crate) fn request(&mut self, method: &str, params: &Value, timeout: Duration) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        );
        let response = self.response(timeout);
        assert_eq!(response["id"], json!(id), "{method}: {}", short(&response.to_string()));
        response
    }

    /// `result` of a request that must succeed.
    pub(crate) fn result(&mut self, method: &str, params: &Value) -> Value {
        let response = self.request(method, params, QUICK);
        assert!(response.get("error").is_none(), "{method}: {}", short(&response.to_string()));
        response["result"].clone()
    }

    pub(crate) fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}).to_string());
    }

    /// `initialize` with `version`, then `notifications/initialized`; the result.
    pub(crate) fn initialize(&mut self, version: &str) -> Value {
        let params = json!({
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "qoob3d-tests", "version": "0"},
        });
        let result = self.result("initialize", &params);
        self.notify("notifications/initialized");
        result
    }

    /// A fresh session for `p`, initialized with the newest version.
    pub(crate) fn open(p: &Project) -> Self {
        let mut mcp = Self::spawn(p);
        mcp.initialize(VERSIONS[2]);
        mcp
    }

    /// `tools/call`: one text content with the JSON envelope, `isError` =
    /// `!ok` (AG-MC-02); only a successful `render` has a second content.
    pub(crate) fn call(&mut self, tool: &str, arguments: &Value) -> Tool {
        self.called.insert(tool.to_owned());
        let params = json!({"name": tool, "arguments": arguments});
        let response = self.request("tools/call", &params, TOOL);
        let text = short(&response.to_string());
        assert!(response.get("error").is_none(), "{tool} {arguments}: protocol error: {text}");
        let result = &response["result"];
        let content = result["content"].as_array().cloned().unwrap_or_default();
        assert!(!content.is_empty(), "{tool}: no content: {text}");
        assert_eq!(content[0]["type"], json!("text"), "{tool}: {text}");
        let body = content[0]["text"].as_str().unwrap_or_default();
        let envelope: Value = serde_json::from_str(body).unwrap_or_else(|e| {
            panic!("{tool}: content[0].text is no JSON envelope ({e}): {text}")
        });
        let ok = check_envelope(&envelope);
        assert_eq!(result["isError"], json!(!ok), "{tool}: isError = !ok: {text}");
        let contents = if tool == "render" && ok { 2 } else { 1 };
        assert_eq!(content.len(), contents, "{tool}: number of contents: {text}");
        Tool { envelope, content }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let child = &mut self.child;
        if !wait_until(Duration::from_secs(3), || child.try_wait().is_ok_and(|s| s.is_some())) {
            self.child.kill().ok();
        }
        self.child.wait().ok();
    }
}
