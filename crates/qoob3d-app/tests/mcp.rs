//! R4: the MCP server `qoob3d mcp [DIR]` over stdio against the real binary
//! (docs/requirements/agent.md §4.5 AG-MC-01..06 and §7 acceptance R4;
//! §6.2 AG-TK-05..07: the agent binds parts and checks with `task_update`;
//! ADR-0054). Every test names its requirement IDs.
//!
//! Transport: one JSON-RPC 2.0 message per line on stdin/stdout. [`Mcp`]
//! reads stdout in a thread and waits for every answer with a timeout, so a
//! hanging server fails the test instead of blocking it. Every line that
//! arrives on stdout is checked to be a JSON-RPC message (AG-MC-01: nothing
//! but protocol messages on stdout).
//!
//! Expected numbers are derived by hand next to the assertions; tolerances:
//! `VOL` = 1e-6 relative for volumes of boxes and cylinders (as in the R1
//! kernel tests), `GAP` = 1e-4 mm for distances between cylinders (see there).
//!
//! Processes and folders: [`Mcp`] closes stdin and kills the server on drop;
//! `support::Project` stops the service the server started on demand and
//! removes the folder (see `common/agent.rs`).

#[path = "common/agent.rs"]
mod support;

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{Value, json};
use support::{
    BIN, Project, Service, VOL, assert_rel, assert_vec, check_envelope, cli, git, git_init,
    git_tracked, png_colors, png_size, service_json, wait_until,
};

/// Smallest distance between a pin and its hole (coaxial cylinders): OCCT's
/// extrema search is good to about 1e-7 mm; 1e-4 mm is a thousandth of the
/// 0.1 mm error the acceptance test must detect.
const GAP: f64 = 1e-4;
/// Answers to protocol messages that start no work.
const QUICK: Duration = Duration::from_secs(30);
/// Answers to tool calls (service start, builds, renders on software Vulkan).
const TOOL: Duration = Duration::from_secs(120);
const CLI: Duration = Duration::from_secs(90);
const MIB: usize = 1024 * 1024;
const VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
/// The script API reference the binary must embed (AG-MC-05).
const SCRIPT_API: &str = include_str!("../../../docs/script-api.md");

// ===========================================================================
// MCP client
// ===========================================================================

/// At most 400 characters of a message for panic texts (renders are megabytes).
fn short(text: &str) -> String {
    if text.len() <= 400 {
        return text.to_owned();
    }
    let head: String = text.chars().take(400).collect();
    format!("{head}… ({} bytes)", text.len())
}

/// A stdout line as JSON-RPC 2.0 message: a response (`id` with exactly one
/// of `result`/`error`) or a notification/request (`method`).
fn protocol_message(line: &str) -> Value {
    let v: Value = serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {}", short(line)));
    assert_eq!(v["jsonrpc"], json!("2.0"), "not JSON-RPC 2.0: {}", short(line));
    let response = v.get("id").is_some() && (v.get("result").is_some() != v.get("error").is_some());
    assert!(response || v["method"].is_string(), "no response, no request: {}", short(line));
    v
}

/// The answer of one `tools/call` (AG-MC-02).
struct Tool {
    envelope: Value,
    content: Vec<Value>,
}

impl Tool {
    /// `result` of an envelope with `ok: true`.
    fn ok(&self) -> &Value {
        assert_eq!(self.envelope["ok"], json!(true), "{}", short(&self.envelope.to_string()));
        &self.envelope["result"]
    }

    /// The call was refused and the error names every word.
    fn refused(&self, words: &[&str], what: &str) {
        let text = self.envelope.to_string();
        assert_eq!(self.envelope["ok"], json!(false), "{what} must be refused: {}", short(&text));
        for w in words {
            assert!(text.contains(w), "{what}: the error must name {w}: {}", short(&text));
        }
    }

    fn error_kind(&self) -> &str {
        self.envelope["error"]["kind"].as_str().unwrap_or_default()
    }
}

/// A running `qoob3d mcp`.
struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    log: PathBuf,
    next_id: u64,
    /// Names of all tools called so far.
    called: BTreeSet<String>,
}

impl Mcp {
    /// `qoob3d mcp <DIR>`, working directory outside the project.
    fn spawn(p: &Project) -> Self {
        Self::start(Command::new(BIN).arg("mcp").arg(&p.dir).current_dir(&p.root), p)
    }

    /// `qoob3d mcp` in the project folder (AG-MC-01: DIR defaults to it).
    fn spawn_in_project(p: &Project) -> Self {
        Self::start(Command::new(BIN).arg("mcp").current_dir(&p.dir), p)
    }

    fn start(command: &mut Command, p: &Project) -> Self {
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

    fn stderr(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let start = text.len().saturating_sub(3000);
        text.get(start..).unwrap_or_default().to_owned()
    }

    /// Writes one line to the server.
    fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        let sent = writeln!(stdin, "{line}").and_then(|()| stdin.flush());
        if let Err(e) = sent {
            panic!("the server closed stdin ({e}); stderr:\n{}", self.stderr());
        }
    }

    /// The next stdout line as checked protocol message.
    fn recv(&mut self, timeout: Duration) -> Value {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => protocol_message(&line),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                panic!("no answer within {timeout:?}; stderr:\n{}", self.stderr())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the server closed stdout without an answer; stderr:\n{}", self.stderr())
            }
        }
    }

    /// The next response; notifications of the server are checked and skipped.
    fn response(&mut self, timeout: Duration) -> Value {
        loop {
            let message = self.recv(timeout);
            if message.get("method").is_none() {
                return message;
            }
        }
    }

    /// A request with a fresh numeric id; the whole response.
    fn request(&mut self, method: &str, params: &Value, timeout: Duration) -> Value {
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
    fn result(&mut self, method: &str, params: &Value) -> Value {
        let response = self.request(method, params, QUICK);
        assert!(response.get("error").is_none(), "{method}: {}", short(&response.to_string()));
        response["result"].clone()
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({"jsonrpc": "2.0", "method": method}).to_string());
    }

    /// `initialize` with `version`, then `notifications/initialized`; the result.
    fn initialize(&mut self, version: &str) -> Value {
        let params = json!({
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "qoob3d-tests", "version": "0"},
        });
        let result = self.result("initialize", &params);
        self.notify("notifications/initialized");
        result
    }

    /// A fresh, initialized session for `p`.
    fn open(p: &Project) -> Self {
        let mut mcp = Self::spawn(p);
        mcp.initialize(VERSIONS[2]);
        mcp
    }

    /// `tools/call`: one text content with the JSON envelope, `isError` =
    /// `!ok` (AG-MC-02); only a successful `render` has a second content.
    fn call(&mut self, tool: &str, arguments: &Value) -> Tool {
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

    /// Closes stdin: the server must exit by itself and must not have
    /// written anything that was not read as an answer.
    fn finish(mut self) {
        drop(self.stdin.take());
        let child = &mut self.child;
        let exited =
            wait_until(Duration::from_secs(10), || child.try_wait().is_ok_and(|s| s.is_some()));
        assert!(exited, "the server does not exit when stdin closes; stderr:\n{}", self.stderr());
        let status = self.child.wait().expect("exit status");
        assert!(status.success(), "exit status {status}; stderr:\n{}", self.stderr());
        // The reader thread ends with the pipe: everything left is unexpected.
        let rest: Vec<String> = self.lines.iter().collect();
        assert!(rest.is_empty(), "unrequested output on stdout: {}", short(&rest.join("\n")));
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

// ===========================================================================
// Test projects
// ===========================================================================

/// A = box(a_size, 10, 10) with a_size = 20 → 2000 mm³, x ∈ [−10, 10].
const PART_A: &str = "part(\"A\", #{}, |deps| box(value(\"a_size\"), 10, 10));\n";
/// B = A moved by 30 in x → x ∈ [20, 40]; gap to A = 20 − 10 = 10 mm.
const PART_B: &str = "part(\"B\", #{ depends: [\"A\"] }, |deps| deps[\"A\"].move(30, 0, 0));\n";
const VALUES: &str = "\
[a_size]
value = 20
source = \"test drawing\"
status = \"assumption\"
";
const CHECK_A: &str = "\
check(\"a_volume\", || {
    expect_near(part(\"A\").volume(), 2000, 0.001, \"volume of A\");
});
";

/// A task file as a human writes it (AG-TK-01) in `state`.
fn task_file(title: &str, state: &str) -> String {
    format!(
        "+++\ntitle = \"{title}\"\nkind = \"task\"\nstate = \"{state}\"\nparts = []\nchecks = []\n\
         created = \"2026-11-20\"\n+++\n\nThe words of the human.\n"
    )
}

/// A, B, a value, the check `a_volume` and the open task T-0001.
fn small_project(name: &str) -> Project {
    let p = Project::new(name, "");
    p.write("parts/a.rhai", PART_A)
        .write("parts/b.rhai", PART_B)
        .write("values/dims.toml", VALUES)
        .write("checks/geometry.rhai", CHECK_A)
        .write("tasks/0001-first.md", &task_file("First", "open"));
    p
}

fn text_of(tool: &Tool) -> String {
    tool.ok()["text"]
        .as_str()
        .unwrap_or_else(|| panic!("result.text: {}", tool.envelope))
        .to_owned()
}

// ===========================================================================
// AG-MC-01: protocol
// ===========================================================================

#[test]
fn ag_mc_01_initialize_answers_with_a_supported_protocol_version() {
    let p = small_project("mcp-init");
    // Each supported version is echoed; anything else gets the newest.
    let cases = [VERSIONS[0], VERSIONS[1], VERSIONS[2], "1999-01-01", "2099-12-31", ""];
    for client in cases {
        let mut mcp = Mcp::spawn(&p);
        let r = mcp.initialize(client);
        let expected = if VERSIONS.contains(&client) { client } else { "2025-06-18" };
        assert_eq!(r["protocolVersion"], json!(expected), "client {client:?}: {r}");
        assert!(r["capabilities"]["tools"].is_object(), "capability tools: {r}");
        assert!(r["capabilities"]["resources"].is_object(), "capability resources: {r}");
        assert_eq!(r["serverInfo"]["name"], json!("qoob3d"), "{r}");
        assert_eq!(r["serverInfo"]["version"], json!(env!("CARGO_PKG_VERSION")), "{r}");
        // instructions = the workflow guide (AG-MC-06), also served as a tool.
        let instructions = r["instructions"].as_str().unwrap_or_default().to_owned();
        assert!(!instructions.trim().is_empty(), "instructions: {r}");
        if client == VERSIONS[2] {
            let guide = text_of(&mcp.call("guide", &json!({})));
            assert_eq!(instructions.trim(), guide.trim(), "instructions are the guide");
        }
        mcp.finish();
    }
}

#[test]
fn ag_mc_01_json_rpc_errors_notifications_and_clean_stdout() {
    let p = small_project("mcp-rpc");
    let mut mcp = Mcp::spawn_in_project(&p);
    mcp.initialize(VERSIONS[2]);

    // ping → empty result.
    assert_eq!(mcp.result("ping", &json!({})), json!({}));
    // Ids are echoed with their type; a request without params is fine.
    mcp.send(r#"{"jsonrpc":"2.0","id":"abc-1","method":"ping"}"#);
    let r = mcp.response(QUICK);
    assert_eq!((&r["id"], &r["result"]), (&json!("abc-1"), &json!({})), "{r}");

    // Unknown method → −32601 with the id of the request.
    let r = mcp.request("no/such/method", &json!({}), QUICK);
    assert_eq!(r["error"]["code"], json!(-32601), "{r}");
    assert!(r["error"]["message"].is_string(), "{r}");
    assert!(r.get("result").is_none(), "{r}");

    // Malformed JSON → −32700 with id null; the server keeps running.
    for garbage in ["{\"jsonrpc\": \"2.0\", \"id\": 7, \"method\": ", "not json at all", "[1, 2"] {
        mcp.send(garbage);
        let r = mcp.response(QUICK);
        assert_eq!(r["error"]["code"], json!(-32700), "{garbage:?}: {r}");
        assert!(r["id"].is_null(), "a parse error has id null: {r}");
    }

    // Notifications (no id) never get a response, known or unknown: the
    // next line on stdout is the answer to the ping after them.
    mcp.notify("notifications/initialized");
    mcp.notify("notifications/cancelled");
    mcp.notify("no/such/notification");
    assert_eq!(mcp.result("ping", &json!({})), json!({}), "only the ping is answered");

    // An unknown tool is an error of either layer and does not end the session.
    let params = json!({"name": "no_such_tool", "arguments": {}});
    let r = mcp.request("tools/call", &params, QUICK);
    assert!(r.get("error").is_some() || r["result"]["isError"] == json!(true), "{r}");
    // Missing required argument: the same.
    let r = mcp.request("tools/call", &json!({"name": "task_get", "arguments": {}}), TOOL);
    assert!(r.get("error").is_some() || r["result"]["isError"] == json!(true), "{r}");

    // DIR defaults to the working directory: the status is this project's.
    let st = mcp.call("project_status", &json!({}));
    assert_eq!(st.ok()["project"], json!("mcp-rpc"), "{}", st.envelope);
    // Every line so far was a JSON-RPC message (checked in `recv`), also
    // with a service started on demand; nothing follows after stdin closes.
    mcp.finish();
}

// ===========================================================================
// AG-MC-02: tools
// ===========================================================================

/// `(name, arguments, required arguments)` exactly as AG-MC-02 lists them
/// (`x?` = optional).
const TOOLS: [(&str, &[&str], &[&str]); 19] = [
    ("project_status", &[], &[]),
    ("guide", &[], &[]),
    ("api_reference", &["search"], &[]),
    ("task_list", &["state"], &[]),
    ("task_get", &["id"], &["id"]),
    ("task_create", &["title", "kind", "body", "parts", "checks"], &["title"]),
    // AG-TK-05: `task_update` accepts `parts` and `checks`.
    ("task_update", &["id", "state", "note", "parts", "checks"], &["id"]),
    ("file_list", &["dir"], &[]),
    ("file_read", &["path"], &["path"]),
    ("file_write", &["path", "content"], &["path", "content"]),
    ("build", &["parts", "wait_s"], &[]),
    ("job", &["id", "cancel"], &["id"]),
    ("run_script", &["source", "wait_s"], &["source"]),
    ("measure", &["a", "b"], &["a"]),
    ("check", &["names", "wait_s"], &[]),
    // AG-MC-09: `render` accepts `views` (2 to 6 named views, contact sheet).
    ("render", &["view", "views", "parts", "width", "height"], &[]),
    ("export", &["parts", "format"], &["parts", "format"]),
    ("values", &[], &[]),
    ("save_state", &["message"], &["message"]),
];

fn string_set(v: &Value) -> BTreeSet<String> {
    v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_owned)).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn ag_mc_02_tools_list_names_exactly_the_tools_with_described_schemas() {
    let p = small_project("mcp-tools-list");
    let mut mcp = Mcp::open(&p);
    let listed = mcp.result("tools/list", &json!({}))["tools"].clone();
    let names: Vec<&str> =
        listed.as_array().into_iter().flatten().filter_map(|t| t["name"].as_str()).collect();
    let expected: BTreeSet<&str> = TOOLS.iter().map(|t| t.0).collect();
    assert_eq!(names.iter().copied().collect::<BTreeSet<_>>(), expected);
    assert_eq!(names.len(), TOOLS.len(), "no tool is listed twice: {names:?}");

    for (name, arguments, required) in TOOLS {
        let tool = listed.as_array().into_iter().flatten().find(|t| t["name"] == json!(name));
        let tool = tool.unwrap_or_else(|| panic!("{name} not listed"));
        let description = tool["description"].as_str().unwrap_or_default();
        assert!(description.trim().len() >= 10, "{name}: description {description:?}");
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], json!("object"), "{name}: {schema}");
        // Exactly the arguments of the requirement: no hidden `role` on
        // task_update, nothing an agent has to guess.
        let properties: BTreeSet<String> = schema["properties"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        assert_eq!(properties, set(arguments), "{name}: arguments");
        assert_eq!(string_set(&schema["required"]), set(required), "{name}: required");
        for argument in arguments {
            let property = &schema["properties"][argument];
            assert!(property.is_object(), "{name}.{argument}: {property}");
            let text = property["description"].as_str().unwrap_or_default();
            assert!(!text.trim().is_empty(), "{name}.{argument}: description: {property}");
        }
    }
    // AG-MC-02: the agent can never set done or rejected; the schema must
    // not offer them.
    let update =
        listed.as_array().into_iter().flatten().find(|t| t["name"] == json!("task_update"));
    let offered =
        string_set(&update.expect("task_update")["inputSchema"]["properties"]["state"]["enum"]);
    assert!(!offered.contains("done") && !offered.contains("rejected"), "{offered:?}");
    mcp.finish();
}

#[test]
fn ag_mc_02_every_tool_answers_with_the_envelope() {
    let p = small_project("mcp-tools");
    git_init(&p.dir);
    // AG-MC-01: a running service is used, not replaced.
    let s = Service::start(&p);
    let running = service_json(&p.dir);
    let mut mcp = Mcp::open(&p);
    call_information_tools(&mut mcp);
    call_file_and_task_tools(&mut mcp, &p);
    call_build_tools(&mut mcp, &p);
    call_output_tools(&mut mcp, &p);

    let called: BTreeSet<&str> = mcp.called.iter().map(String::as_str).collect();
    assert_eq!(called, TOOLS.iter().map(|t| t.0).collect(), "every tool was called");
    assert_eq!(service_json(&p.dir), running, "the server used the running service");
    // The tools work on the service's state: HTTP sees what MCP built.
    let st = s.request("GET", "/status", None).ok();
    let parts = st["parts"].as_array().cloned().unwrap_or_default();
    assert_eq!(parts.len(), 3, "A, B and the part written over MCP: {st}");
    assert!(parts.iter().all(|part| part["state"] == json!("current")), "{st}");
    mcp.finish();
}

fn call_information_tools(mcp: &mut Mcp) {
    let st = mcp.call("project_status", &json!({}));
    assert_eq!(st.ok()["project"], json!("mcp-tools"));
    let parts = st.ok()["parts"].as_array().cloned().unwrap_or_default();
    let names: BTreeSet<String> =
        parts.iter().filter_map(|p| p["name"].as_str().map(str::to_owned)).collect();
    assert_eq!(names, set(&["A", "B"]), "{}", st.envelope);
    assert!(st.ok()["checks"].is_array(), "status lists the checks: {}", st.envelope);

    assert!(!text_of(&mcp.call("guide", &json!({}))).trim().is_empty());
    assert!(text_of(&mcp.call("api_reference", &json!({}))).contains("# Script API reference"));

    // All values plus the assumptions: a_size is an assumption (VALUES).
    let values = mcp.call("values", &json!({}));
    let v = &values.ok()["values"][0];
    assert_eq!(
        (&v["id"], v["value"].as_f64()),
        (&json!("a_size"), Some(20.0)),
        "{}",
        values.envelope
    );
    assert_eq!((&v["source"], &v["status"]), (&json!("test drawing"), &json!("assumption")));
    assert_eq!(values.ok()["assumptions"], json!(["a_size"]), "{}", values.envelope);
}

fn call_file_and_task_tools(mcp: &mut Mcp, p: &Project) {
    let list = mcp.call("file_list", &json!({}));
    assert!(string_set(&list.ok()["files"]).contains("parts/a.rhai"), "{}", list.envelope);
    let read = mcp.call("file_read", &json!({"path": "parts/a.rhai"}));
    assert_eq!(read.ok()["content"], json!(PART_A));
    // C = box(4, 4, 4) = 64 mm³.
    let part_c = "part(\"C\", #{}, |deps| box(4, 4, 4));\n";
    mcp.call("file_write", &json!({"path": "parts/c.rhai", "content": part_c})).ok();
    assert_eq!(p.read("parts/c.rhai"), part_c);

    let list = mcp.call("task_list", &json!({}));
    assert_eq!(list.ok().as_array().map(Vec::len), Some(1), "{}", list.envelope);
    assert_eq!(list.ok()[0]["id"], json!("T-0001"));
    let done = mcp.call("task_list", &json!({"state": "done"}));
    assert_eq!(done.ok(), &json!([]), "state filters the list");
    let open = mcp.call("task_list", &json!({"state": "open"}));
    assert_eq!(open.ok().as_array().map(Vec::len), Some(1));
    let shown = mcp.call("task_get", &json!({"id": "T-0001"}));
    assert!(shown.ok()["body"].as_str().is_some_and(|b| b.contains("The words of the human.")));
    assert!(shown.ok()["log"].is_array(), "{}", shown.envelope);
    mcp.call("task_get", &json!({"id": "T-0404"})).refused(&["T-0404"], "an unknown task");

    let args = json!({"title": "Second task", "kind": "idea", "body": "From the agent.",
        "parts": ["A"], "checks": ["a_volume"]});
    let created = mcp.call("task_create", &args);
    let t = created.ok();
    assert_eq!(
        (&t["id"], &t["kind"], &t["state"]),
        (&json!("T-0002"), &json!("idea"), &json!("open"))
    );
    assert_eq!(t["file"], json!("tasks/0002-second-task.md"), "{t}");
    assert!(p.read("tasks/0002-second-task.md").contains("From the agent."));

    // task_update acts as the agent: the log line says so, in the form
    // `<old> -> <new>: <note>` (AG-TK-07).
    let args = json!({"id": "T-0002", "state": "in_progress", "note": "taking it"});
    assert_eq!(mcp.call("task_update", &args).ok()["state"], json!("in_progress"));
    let file = p.read("tasks/0002-second-task.md");
    assert!(file.contains("state = \"in_progress\""), "{file}");
    let line = " agent: open -> in_progress: taking it";
    assert!(file.lines().any(|l| l.starts_with("- ") && l.ends_with(line)), "{file}");
}

fn call_build_tools(mcp: &mut Mcp, p: &Project) {
    // wait_s = 0 on unbuilt parts answers with a job (§3.1); `job` polls it.
    let started = mcp.call("build", &json!({"wait_s": 0}));
    let id = started.ok()["job"].clone();
    assert!(id.is_u64() || id.is_string(), "a job id: {}", started.envelope);
    let mut job = Value::Null;
    let finished = wait_until(Duration::from_secs(60), || {
        job = mcp.call("job", &json!({"id": id})).ok().clone();
        ["done", "failed", "cancelled"].contains(&job["state"].as_str().unwrap_or_default())
    });
    assert!(finished && job["state"] == json!("done"), "{job}");
    assert_eq!(string_set(&job["result"]["built"]), set(&["A", "B", "C"]), "{job}");
    mcp.call("job", &json!({"id": 999_999_999})).refused(&[], "an unknown job");

    // Everything is cached now; A = box(20, 10, 10) = 2000 mm³.
    let built = mcp.call("build", &json!({"parts": ["A"], "wait_s": 60}));
    assert_eq!(built.ok()["parts"]["A"]["state"], json!("cached"), "{}", built.envelope);
    assert_rel(&built.ok()["parts"]["A"]["volume"], 2000.0, VOL, "volume of A");
    assert!(p.path(".qoob3d/cache/A").is_dir(), "the cache of the project was used");

    let source = "show(part(\"A\").move(0, 0, 5), \"lifted\");\nprint(\"hi\");\n";
    let run = mcp.call("run_script", &json!({"source": source, "wait_s": 60}));
    assert_eq!(run.ok()["printed"], json!(["hi"]), "{}", run.envelope);
    let shape = &run.ok()["shapes"][0];
    assert_eq!(shape["name"], json!("lifted"));
    assert_rel(&shape["volume"], 2000.0, VOL, "volume of the lifted A");
    // A lifted by 5: centre of the box at z = 5 + 5.
    assert_vec(&shape["center"], [0.0, 0.0, 10.0], 1e-6, "centre of the lifted A");
    let bad = mcp.call("run_script", &json!({"source": "box(0, 1, 1);"}));
    bad.refused(&[], "a failing script");
    assert_eq!((bad.error_kind(), bad.envelope["error"]["line"].as_u64()), ("script", Some(1)));

    // A ends at x = 10, B starts at x = 20.
    let m = mcp.call("measure", &json!({"a": "A", "b": "B"}));
    assert_rel(&m.ok()["volume"], 2000.0, VOL, "volume of A");
    assert_rel(&m.ok()["distance"], 10.0, 1e-6, "distance A to B");
    assert_eq!(m.ok()["common_volume"].as_f64(), Some(0.0), "{}", m.envelope);
    let single = mcp.call("measure", &json!({"a": "C"}));
    assert_rel(&single.ok()["volume"], 64.0, VOL, "volume of C");
    mcp.call("measure", &json!({"a": "Nope"})).refused(&["Nope"], "an unknown part");

    let checked = mcp.call("check", &json!({"wait_s": 60}));
    let r = checked.ok();
    assert_eq!((r["passed"].as_u64(), r["failed"].as_u64()), (Some(1), Some(0)), "{r}");
    let named = mcp.call("check", &json!({"names": ["a_volume"]}));
    assert_eq!(named.ok()["checks"][0]["state"], json!("passed"), "{}", named.envelope);
}

fn call_output_tools(mcp: &mut Mcp, p: &Project) {
    let render =
        mcp.call("render", &json!({"view": "top", "parts": ["A"], "width": 160, "height": 120}));
    assert!(render.ok()["path"].is_string(), "{}", render.envelope);

    let export = mcp.call("export", &json!({"parts": ["A"], "format": "stl"}));
    export.ok();
    // Binary STL: 80 bytes header, u32 triangle count n, 50 bytes per
    // triangle; a box has at least 12 triangles.
    let stl = std::fs::read(p.path("out/A.stl")).expect("out/A.stl");
    assert!(stl.len() >= 84, "STL shorter than its header");
    let n = u32::from_le_bytes([stl[80], stl[81], stl[82], stl[83]]) as usize;
    assert!(
        n >= 12 && stl.len() == 84 + 50 * n,
        "binary STL with {n} triangles, {} bytes",
        stl.len()
    );
    mcp.call("export", &json!({"parts": ["A"], "format": "obj"})).refused(&[], "an unknown format");

    // All checks are green: save_state commits.
    let saved = mcp.call("save_state", &json!({"message": "All tools called"}));
    let commit = saved.ok()["commit"].as_str().unwrap_or_default().to_owned();
    let head = git(&p.dir, &["rev-parse", "HEAD"]).expect("a commit exists");
    assert!(commit.len() >= 7 && head.starts_with(&commit), "commit {commit:?} is HEAD {head}");
    assert_eq!(git(&p.dir, &["log", "-1", "--format=%s"]).as_deref(), Ok("All tools called"));
}

/// Calls `task_update` and expects a refusal on either protocol layer.
fn update_must_fail(mcp: &mut Mcp, arguments: &Value) {
    let params = json!({"name": "task_update", "arguments": arguments});
    let r = mcp.request("tools/call", &params, TOOL);
    assert!(
        r.get("error").is_some() || r["result"]["isError"] == json!(true),
        "task_update {arguments} must be refused: {r}"
    );
}

#[test]
fn ag_mc_02_task_update_can_never_set_done_or_rejected() {
    // One task in every state; ADR-0054: `done` is never set over MCP.
    let from = ["open", "in_progress", "blocked", "review", "done", "rejected"];
    let p = Project::new("mcp-roles", "");
    let files: Vec<String> = (1..=from.len()).map(|n| format!("tasks/{n:04}-state.md")).collect();
    for (file, state) in files.iter().zip(from) {
        p.write(file, &task_file("State", state));
    }
    let mut mcp = Mcp::open(&p);
    for (n, (file, state)) in files.iter().zip(from).enumerate() {
        let id = format!("T-{:04}", n + 1);
        let before = p.read(file);
        for target in ["done", "rejected"] {
            if target == state {
                continue;
            }
            let plain =
                mcp.call("task_update", &json!({"id": id, "state": target, "note": "sure"}));
            plain.refused(&[id.as_str(), target], &format!("{state} → {target} over MCP"));
            // Claiming to be the human does not help.
            update_must_fail(&mut mcp, &json!({"id": id, "state": target, "role": "human"}));
            update_must_fail(
                &mut mcp,
                &json!({"id": id, "state": target, "role": "human", "note": "n"}),
            );
            assert_eq!(p.read(file), before, "{file} changed by a refused update");
        }
        // The human's decisions are final for the agent.
        if ["done", "rejected"].contains(&state) {
            mcp.call("task_update", &json!({"id": id, "state": "in_progress"}))
                .refused(&[id.as_str()], "reopening a decided task over MCP");
            assert_eq!(p.read(file), before);
        }
    }
    // What the agent may do still works: open → in_progress, review → in_progress.
    for id in ["T-0001", "T-0004"] {
        let moved = mcp.call("task_update", &json!({"id": id, "state": "in_progress"}));
        assert_eq!(moved.ok()["state"], json!("in_progress"), "{}", moved.envelope);
    }
    assert!(p.read(&files[0]).contains("state = \"in_progress\""));
    mcp.finish();
}

#[test]
fn ag_tk_05_task_update_binds_parts_and_checks_while_open_or_in_progress() {
    // AG-TK-05: "MCP `task_update` accept[s] `parts` and `checks` (replace
    // the lists …)" while the task is `open` or `in_progress`. One task in
    // every state, written by the human without any binding.
    let from = ["open", "in_progress", "blocked", "review", "done", "rejected"];
    let p = Project::new("mcp-bind", "");
    let files: Vec<String> = (1..=from.len()).map(|n| format!("tasks/{n:04}-state.md")).collect();
    for (file, state) in files.iter().zip(from) {
        p.write(file, &task_file("State", state));
    }
    let mut mcp = Mcp::open(&p);
    for (n, (file, state)) in files.iter().zip(from).enumerate() {
        let id = format!("T-{:04}", n + 1);
        let before = p.read(file);
        // Unknown names are accepted at binding time (they fail the gate later).
        let args = json!({"id": id, "parts": ["Hook", "Crane column"], "checks": ["load"]});
        let bound = mcp.call("task_update", &args);
        if ["open", "in_progress"].contains(&state) {
            let t = bound.ok();
            assert_eq!(t["parts"], json!(["Hook", "Crane column"]), "{state}: {t}");
            assert_eq!(t["checks"], json!(["load"]), "{state}: {t}");
            assert_eq!(t["state"], json!(state), "binding is no state change: {t}");
            let shown = mcp.call("task_get", &json!({"id": id}));
            assert_eq!(shown.ok()["parts"], json!(["Hook", "Crane column"]), "{state}");
            assert_eq!(shown.ok()["checks"], json!(["load"]), "{state}");
            // The human's words stay.
            let file = p.read(file);
            assert!(file.contains("\n\nThe words of the human.\n"), "{file}");
            assert!(file.contains("title = \"State\"\nkind = \"task\""), "{file}");
        } else {
            bound.refused(&[id.as_str(), state], &format!("binding in state {state}"));
            assert_eq!(p.read(file), before, "{file} changed by a refused binding");
        }
    }
    // Replace, not merge; one list alone leaves the other; an empty list clears.
    let t = mcp.call("task_update", &json!({"id": "T-0001", "parts": ["Plate"]})).ok().clone();
    assert_eq!((&t["parts"], &t["checks"]), (&json!(["Plate"]), &json!(["load"])), "{t}");
    let t = mcp.call("task_update", &json!({"id": "T-0001", "checks": []})).ok().clone();
    assert_eq!((&t["parts"], &t["checks"]), (&json!(["Plate"]), &json!([])), "{t}");
    // Binding and taking the task in one call.
    let args = json!({"id": "T-0001", "state": "in_progress", "checks": ["fit"], "note": "mine"});
    let t = mcp.call("task_update", &args).ok().clone();
    assert_eq!((&t["state"], &t["checks"]), (&json!("in_progress"), &json!(["fit"])), "{t}");
    // Wrong types are refused, nothing is bound.
    update_must_fail(&mut mcp, &json!({"id": "T-0002", "parts": "Hook"}));
    update_must_fail(&mut mcp, &json!({"id": "T-0002", "checks": [1, 2]}));
    let shown = mcp.call("task_get", &json!({"id": "T-0002"}));
    assert_eq!(shown.ok()["parts"], json!(["Hook", "Crane column"]));
    mcp.finish();
}

#[test]
fn ag_mc_02_api_reference_whole_or_searched_sections() {
    let p = small_project("mcp-reference");
    let mut mcp = Mcp::open(&p);
    // Without search: the whole docs/script-api.md, embedded in the binary.
    let whole = text_of(&mcp.call("api_reference", &json!({})));
    assert_eq!(whole.trim(), SCRIPT_API.trim(), "the reference is docs/script-api.md");

    // Heading match: the section of `fillet`, not unrelated solids.
    let fillet = text_of(&mcp.call("api_reference", &json!({"search": "fillet"})));
    assert!(fillet.contains("### `fillet(s, edges, r) -> Shape`"), "{}", short(&fillet));
    assert!(fillet.contains("Rounds the selected edges"), "sections come with their text");
    assert!(
        fillet.contains("show(fillet(b, all_edges(b), 2), \"rounded\");"),
        "and their examples"
    );
    assert!(
        !fillet.contains("### `torus(") && !fillet.contains("### `sphere("),
        "only matching sections"
    );
    assert!(fillet.len() < whole.len() / 2, "a search returns less than the whole reference");
    // Case-insensitive.
    let upper = text_of(&mcp.call("api_reference", &json!({"search": "FILLET"})));
    assert_eq!(upper, fillet, "the search ignores case");

    // Text match: "Stadium" occurs only in the text of `slot`, not in a heading.
    assert_eq!(SCRIPT_API.matches("tadium").count(), 1, "the reference changed: adjust this test");
    let slot = text_of(&mcp.call("api_reference", &json!({"search": "stadium"})));
    assert!(slot.contains("`slot(l, w) -> Profile`"), "{}", short(&slot));
    assert!(slot.contains("Stadium (oblong hole)"), "{}", short(&slot));
    assert!(!slot.contains("### `torus(") && !slot.contains("### `fillet("), "{}", short(&slot));

    // The FDM helpers an agent needs for the acceptance task are findable.
    let hole = text_of(&mcp.call("api_reference", &json!({"search": "clearance"})));
    assert!(hole.contains("### `hole(d, l, clearance) -> Shape`"), "{}", short(&hole));

    // No match: not the whole reference.
    let none = mcp.call("api_reference", &json!({"search": "zzz-no-such-function"}));
    if none.envelope["ok"] == json!(true) {
        assert!(!text_of(&none).contains("### `box("), "no match must not return everything");
    }
    mcp.finish();
}

// ===========================================================================
// AG-MC-03: confined file tools
// ===========================================================================

const SECRET: &str = "// TOP-SECRET-OUTSIDE-THE-PROJECT\n";

fn write_refused(mcp: &mut Mcp, path: &str, why: &str) {
    let args = json!({"path": path, "content": "// written by a test\n"});
    mcp.call("file_write", &args).refused(&[], &format!("file_write {path:?} ({why})"));
}

fn read_refused(mcp: &mut Mcp, path: &str, why: &str) {
    let tool = mcp.call("file_read", &json!({"path": path}));
    tool.refused(&[], &format!("file_read {path:?} ({why})"));
    assert!(!tool.envelope.to_string().contains("TOP-SECRET"), "file_read {path:?} leaked a file");
}

/// Every file below `dir`, relative to `base`, sorted.
fn tree(dir: &Path, base: &Path, out: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() && !path.is_symlink() {
            tree(&path, base, out);
        } else {
            out.insert(path.strip_prefix(base).unwrap_or(&path).display().to_string());
        }
    }
}

fn files_below(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    tree(dir, dir, &mut out);
    out
}

/// The files of the project without `.qoob3d/` (service.json, cache, logs
/// belong to the service).
fn project_files(p: &Project) -> BTreeSet<String> {
    files_below(&p.dir).into_iter().filter(|f| !f.starts_with(".qoob3d/")).collect()
}

#[test]
fn ag_mc_03_file_tools_refuse_everything_outside_the_allowed_paths() {
    let p = small_project("mcp-confine");
    p.write("parts/notes.txt", "not a script\n").write(".qoob3d/private.toml", "x = 1\n");
    p.write("out/report.md", "# report\n").write("README.md", "# readme\n");
    std::fs::create_dir_all(p.root.join("outside")).expect("create outside");
    std::fs::write(p.root.join("outside/secret.rhai"), SECRET).expect("write secret");
    std::fs::write(p.root.join("secret.rhai"), SECRET).expect("write secret");
    let mut mcp = Mcp::open(&p);
    let before = (project_files(&p), files_below(&p.root));
    let absolute_outside = p.root.join("absolute.rhai").display().to_string();
    let absolute_inside = p.path("parts/absolute.rhai").display().to_string();
    let absolute_secret = p.root.join("secret.rhai").display().to_string();

    let writes = [
        ("../secret.rhai", "`..`"),
        ("parts/../../secret.rhai", "`..`"),
        ("parts/../parts/a.rhai", "`..` even if it stays inside"),
        ("lib/../../outside/new.rhai", "`..`"),
        (absolute_outside.as_str(), "absolute path"),
        (absolute_inside.as_str(), "absolute path into the project"),
        (".qoob3d/private.toml", "never .qoob3d/"),
        (".qoob3d/cache/x.rhai", "never .qoob3d/"),
        ("parts/../.qoob3d/x.toml", "never .qoob3d/"),
        ("out/report.md", "only parts, lib, values, checks, tasks"),
        ("src/main.rhai", "only parts, lib, values, checks, tasks"),
        ("top.rhai", "only qoob3d.toml in the project root"),
        ("README.md", "only qoob3d.toml in the project root"),
        (".gitignore", "only qoob3d.toml in the project root"),
        ("partsx/a.rhai", "a folder that only starts like an allowed one"),
        ("parts/notes.txt", "extension"),
        ("parts/script", "no extension"),
        ("parts/run.sh", "extension"),
        ("parts/a.rhai.bak", "extension"),
        ("parts", "a folder, not a file"),
        ("", "empty path"),
        ("tasks/0002-sneaky.md", "task files change only through the task tools"),
    ];
    for (path, why) in writes {
        write_refused(&mut mcp, path, why);
    }
    // An existing task cannot be overwritten either (it could set `done`).
    let done = json!({"path": "tasks/0001-first.md", "content": task_file("First", "done")});
    mcp.call("file_write", &done).refused(&[], "overwriting a task file");
    assert_eq!(p.read("tasks/0001-first.md"), task_file("First", "open"));

    for (path, why) in [
        ("../secret.rhai", "`..`"),
        ("parts/../../outside/secret.rhai", "`..`"),
        (absolute_secret.as_str(), "absolute path"),
        (".qoob3d/private.toml", "never .qoob3d/"),
        (".qoob3d/service.json", "never .qoob3d/"),
        ("out/report.md", "only parts, lib, values, checks, tasks"),
        ("README.md", "only qoob3d.toml in the project root"),
        ("parts/notes.txt", "extension"),
        ("parts/missing.rhai", "no such file"),
    ] {
        read_refused(&mut mcp, path, why);
    }
    for dir in ["..", ".qoob3d", "out", "/tmp", "parts/.."] {
        mcp.call("file_list", &json!({"dir": dir})).refused(&[], &format!("file_list {dir:?}"));
    }

    // Compared without the service's own files in .qoob3d/ and the test logs.
    let after = (project_files(&p), files_below(&p.root));
    assert_eq!(after.0, before.0, "a refused call wrote a file in the project");
    let outside = |all: &BTreeSet<String>| -> BTreeSet<String> {
        all.iter().filter(|f| !f.starts_with("project/") && !f.ends_with(".log")).cloned().collect()
    };
    assert_eq!(outside(&after.1), outside(&before.1), "a refused call wrote outside the project");
    assert_eq!(p.read(".qoob3d/private.toml"), "x = 1\n");
    assert!(!p.path(".qoob3d/cache/x.rhai").exists() && !p.path(".qoob3d/x.toml").exists());
    assert_eq!(std::fs::read_to_string(p.root.join("secret.rhai")).ok().as_deref(), Some(SECRET));
    assert_eq!(p.read("parts/a.rhai"), PART_A);
    mcp.finish();
}

#[cfg(unix)]
#[test]
fn ag_mc_03_file_tools_do_not_follow_symlinks_out_of_their_folders() {
    use std::os::unix::fs::symlink;
    let p = small_project("mcp-symlink");
    let outside = p.root.join("outside");
    std::fs::create_dir_all(&outside).expect("create outside");
    std::fs::write(outside.join("secret.rhai"), SECRET).expect("write secret");
    // A folder link and a file link that leave the project, and a folder
    // link that stays inside but leads to the task files.
    symlink(&outside, p.path("parts/away")).expect("symlink folder");
    symlink(outside.join("secret.rhai"), p.path("parts/leak.rhai")).expect("symlink file");
    symlink(p.path("tasks"), p.path("lib")).expect("symlink lib → tasks");
    symlink(p.path(".qoob3d"), p.path("values/internal")).expect("symlink → .qoob3d");
    p.write(".qoob3d/private.toml", "# TOP-SECRET internal\n");
    let mut mcp = Mcp::open(&p);

    read_refused(&mut mcp, "parts/away/secret.rhai", "folder symlink out of the project");
    read_refused(&mut mcp, "parts/leak.rhai", "file symlink out of the project");
    read_refused(&mut mcp, "values/internal/private.toml", "symlink into .qoob3d/");
    write_refused(&mut mcp, "parts/away/new.rhai", "folder symlink out of the project");
    write_refused(&mut mcp, "parts/leak.rhai", "file symlink out of the project");
    write_refused(&mut mcp, "values/internal/new.toml", "symlink into .qoob3d/");
    // lib/ is allowed, but here it is tasks/: the task rule holds.
    write_refused(&mut mcp, "lib/0002-sneaky.md", "symlink to the task files");
    let done = json!({"path": "lib/0001-first.md", "content": task_file("First", "done")});
    mcp.call("file_write", &done).refused(&[], "overwriting a task through a symlink");

    assert_eq!(std::fs::read_to_string(outside.join("secret.rhai")).ok().as_deref(), Some(SECRET));
    assert_eq!(files_below(&outside), set(&["secret.rhai"]), "nothing was written outside");
    assert_eq!(files_below(&p.path("tasks")), set(&["0001-first.md"]));
    assert_eq!(p.read("tasks/0001-first.md"), task_file("First", "open"));
    assert!(!p.path(".qoob3d/new.toml").exists());

    // Listing does not walk out of the project either.
    let list = mcp.call("file_list", &json!({}));
    if list.envelope["ok"] == json!(true) {
        let files = string_set(&list.ok()["files"]);
        let escaped: Vec<_> = files
            .iter()
            .filter(|f| f.starts_with("parts/away/") || f.starts_with("values/internal/"))
            .collect();
        assert!(escaped.is_empty(), "file_list followed a symlink: {escaped:?}");
    }
    mcp.finish();
}

#[test]
fn ag_mc_03_files_round_trip_through_write_read_and_list() {
    let p = small_project("mcp-files");
    p.write("parts/notes.txt", "not listed\n").write("out/report.md", "# not listed\n");
    let mut mcp = Mcp::open(&p);
    // Content with non-ASCII, tabs, CRLF and no final newline comes back
    // byte for byte; folders are created.
    let cases = [
        ("parts/new.rhai", "// Ø 3 mm – 0.2 µm ✓\r\n\tpart(\"N\", #{}, |deps| box(1, 2, 3));"),
        ("lib/util/deep.rhai", "fn one() { 1 }\n"),
        ("values/fit.toml", "[fit]\nvalue = 0.2\nsource = \"task\"\nstatus = \"one_source\"\n"),
        ("checks/new.rhai", "check(\"n\", || { expect(true, \"ok\"); });\n"),
        ("lib/readme.md", "# Notes\n"),
        ("lib/empty.rhai", ""),
    ];
    for (path, content) in cases {
        mcp.call("file_write", &json!({"path": path, "content": content})).ok();
        assert_eq!(
            std::fs::read(p.path(path)).ok().as_deref(),
            Some(content.as_bytes()),
            "{path} on disk"
        );
        let read = mcp.call("file_read", &json!({"path": path}));
        assert_eq!(read.ok()["content"], json!(content), "{path} read back");
    }
    // Overwriting replaces; qoob3d.toml is the one allowed file in the root.
    mcp.call("file_write", &json!({"path": "lib/readme.md", "content": "# Changed\n"})).ok();
    assert_eq!(
        mcp.call("file_read", &json!({"path": "lib/readme.md"})).ok()["content"],
        json!("# Changed\n")
    );
    let toml = format!("{}# edited over MCP\n", p.read("qoob3d.toml"));
    mcp.call("file_write", &json!({"path": "qoob3d.toml", "content": toml})).ok();
    assert_eq!(p.read("qoob3d.toml"), toml);
    // Task files can be read (not written) through the file tools.
    let task = mcp.call("file_read", &json!({"path": "tasks/0001-first.md"}));
    assert_eq!(task.ok()["content"], json!(task_file("First", "open")));

    // file_list: project-relative paths of the allowed files only.
    let all = string_set(&mcp.call("file_list", &json!({})).ok()["files"]);
    let expected = [
        "qoob3d.toml",
        "parts/a.rhai",
        "parts/b.rhai",
        "parts/new.rhai",
        "lib/util/deep.rhai",
        "lib/readme.md",
        "lib/empty.rhai",
        "values/dims.toml",
        "values/fit.toml",
        "checks/geometry.rhai",
        "checks/new.rhai",
        "tasks/0001-first.md",
    ];
    assert_eq!(all, set(&expected), "exactly the allowed files, no .qoob3d/, out/, .txt");
    let parts = string_set(&mcp.call("file_list", &json!({"dir": "parts"})).ok()["files"]);
    assert_eq!(parts, set(&["parts/a.rhai", "parts/b.rhai", "parts/new.rhai"]));
    let lib = string_set(&mcp.call("file_list", &json!({"dir": "lib/util"})).ok()["files"]);
    assert_eq!(lib, set(&["lib/util/deep.rhai"]));

    // The service sees a written part at once.
    let st = mcp.call("project_status", &json!({}));
    let n =
        st.ok()["parts"].as_array().into_iter().flatten().find(|part| part["name"] == json!("N"));
    assert_eq!(n.map(|part| part["state"].clone()), Some(json!("never_built")), "{}", st.envelope);
    mcp.finish();
}

#[test]
fn ag_mc_03_files_are_limited_to_one_mebibyte() {
    let p = small_project("mcp-size");
    let mut mcp = Mcp::open(&p);
    // Exactly 1 MiB = 1 048 576 bytes is allowed ("at most").
    let full = format!("//{}", "x".repeat(MIB - 2));
    assert_eq!(full.len(), MIB);
    mcp.call("file_write", &json!({"path": "lib/full.rhai", "content": full})).ok();
    assert_eq!(std::fs::metadata(p.path("lib/full.rhai")).map(|m| m.len()).ok(), Some(MIB as u64));
    let read = mcp.call("file_read", &json!({"path": "lib/full.rhai"}));
    assert_eq!(read.ok()["content"].as_str().map(str::len), Some(MIB));

    // One byte more is refused and writes nothing.
    let over = format!("//{}", "x".repeat(MIB - 1));
    mcp.call("file_write", &json!({"path": "lib/over.rhai", "content": over}))
        .refused(&[], "1 MiB + 1 byte");
    assert!(!p.path("lib/over.rhai").exists());
    // The limit counts bytes, not characters: 524 289 × "é" (2 bytes) = 1 MiB + 2.
    let wide = "é".repeat(MIB / 2 + 1);
    mcp.call("file_write", &json!({"path": "lib/wide.rhai", "content": wide}))
        .refused(&[], "1 MiB + 2 bytes in 524 289 characters");
    assert!(!p.path("lib/wide.rhai").exists());
    // An existing file is not truncated by a refused write.
    mcp.call("file_write", &json!({"path": "lib/full.rhai", "content": over}))
        .refused(&[], "overwriting with too much");
    assert_eq!(p.read("lib/full.rhai"), full);

    // Reading is limited as well.
    p.write("lib/huge.rhai", &over);
    let read = mcp.call("file_read", &json!({"path": "lib/huge.rhai"}));
    read.refused(&[], "reading 1 MiB + 1 byte");
    assert!(read.envelope.to_string().len() < 10_000, "the refusal does not carry the content");
    mcp.finish();
}

// ===========================================================================
// AG-MC-04: render
// ===========================================================================

/// Standard base64 (RFC 4648 §4, alphabet `A–Z a–z 0–9 + /`, `=` padding),
/// decoded here independently of the server.
fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            other => panic!("not standard base64: byte {other:#x}"),
        }
    };
    let bytes = text.as_bytes();
    assert!(bytes.len().is_multiple_of(4), "base64 length {} is no multiple of 4", bytes.len());
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (i, quad) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let last = i + 1 == bytes.len() / 4;
        let pad = quad.iter().rev().take_while(|c| **c == b'=').count();
        assert!(pad <= 2 && (pad == 0 || last), "padding only at the end");
        let n = quad[..4 - pad].iter().fold(0u32, |n, c| (n << 6) | value(*c)) << (6 * pad);
        let triple = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&triple[..3 - pad]);
    }
    out
}

/// A successful `render`: text envelope with `path` under `out/renders/`
/// and an image content with the same PNG; the PNG bytes.
fn rendered(mcp: &mut Mcp, p: &Project, arguments: &Value, size: (u32, u32)) -> Vec<u8> {
    let tool = mcp.call("render", arguments);
    let path =
        tool.ok()["path"].as_str().unwrap_or_else(|| panic!("result.path: {}", tool.envelope));
    assert!(path.ends_with(".png"), "{path}");
    // `path` is relative to the project (or absolute); it lies in out/renders/.
    let file = p.dir.join(path).canonicalize().unwrap_or_else(|e| panic!("{path}: {e}"));
    let renders = p.path("out/renders").canonicalize().expect("out/renders exists");
    assert_eq!(file.parent(), Some(renders.as_path()), "{path} is not directly in out/renders/");

    let image = &tool.content[1];
    assert_eq!(image["type"], json!("image"), "second content: {}", short(&image.to_string()));
    assert_eq!(image["mimeType"], json!("image/png"));
    let png = base64_decode(image["data"].as_str().expect("image data"));
    assert_eq!(png_size(&png), size, "render {arguments}");
    assert!(png_colors(&png) > 1, "render {arguments}: the image is one flat colour");
    assert!(std::fs::read(&file).ok().as_deref() == Some(&png[..]), "{path} is the returned image");
    png
}

#[test]
fn ag_mc_04_render_returns_the_envelope_and_a_png_image() {
    // RFC 4648 §10 test vectors for the decoder used below.
    assert_eq!(base64_decode("Zm9vYmFy"), b"foobar");
    assert_eq!(base64_decode("Zm9vYg=="), b"foob");
    assert_eq!(base64_decode("Zm9vYmE="), b"fooba");

    let p = small_project("mcp-render");
    let mut mcp = Mcp::open(&p);
    // Nothing is built: render builds first, like render-project (AG-RD-01).
    let small = json!({"width": 320, "height": 240});
    let iso = rendered(&mut mcp, &p, &small, (320, 240));
    let st = mcp.call("project_status", &json!({}));
    let parts = st.ok()["parts"].as_array().cloned().unwrap_or_default();
    assert!(parts.iter().all(|part| part["state"] == json!("current")), "{}", st.envelope);

    // Defaults: 1024x768 (AG-RD-01), view iso.
    rendered(&mut mcp, &p, &json!({}), (1024, 768));
    let explicit =
        rendered(&mut mcp, &p, &json!({"view": "iso", "width": 320, "height": 240}), (320, 240));
    assert!(iso == explicit, "the default view is iso");
    // View and part selection change the image.
    let top =
        rendered(&mut mcp, &p, &json!({"view": "top", "width": 320, "height": 240}), (320, 240));
    let top_a = json!({"view": "top", "parts": ["A"], "width": 320, "height": 240});
    let top_a = rendered(&mut mcp, &p, &top_a, (320, 240));
    assert!(iso != top && top != top_a, "views and selections differ");

    // Errors have the text envelope only.
    mcp.call("render", &json!({"parts": ["Nope"]})).refused(&["Nope"], "rendering an unknown part");
    mcp.call("render", &json!({"view": "sideways"})).refused(&[], "an unknown view");
    mcp.finish();
}

// ===========================================================================
// AG-MC-05, AG-MC-06: resources and guide
// ===========================================================================

/// `resources/read` of one text resource; `(mimeType, text)`.
fn read_resource(mcp: &mut Mcp, uri: &str) -> (String, String) {
    let r = mcp.result("resources/read", &json!({"uri": uri}));
    let contents = r["contents"].as_array().cloned().unwrap_or_default();
    assert_eq!(contents.len(), 1, "{uri}: {}", short(&r.to_string()));
    assert_eq!(contents[0]["uri"], json!(uri));
    let text = contents[0]["text"].as_str().unwrap_or_else(|| panic!("{uri}: no text"));
    (contents[0]["mimeType"].as_str().unwrap_or_default().to_owned(), text.to_owned())
}

#[test]
fn ag_mc_05_resources_serve_the_reference_the_guide_and_examples() {
    let p = small_project("mcp-resources");
    let mut mcp = Mcp::open(&p);
    let listed = mcp.result("resources/list", &json!({}))["resources"].clone();
    let resources = listed.as_array().cloned().unwrap_or_default();
    for r in &resources {
        assert!(r["uri"].as_str().is_some_and(|u| u.starts_with("qoob3d://")), "{r}");
        assert!(r["name"].as_str().is_some_and(|n| !n.is_empty()), "resources have names: {r}");
    }
    let uris: Vec<&str> = resources.iter().filter_map(|r| r["uri"].as_str()).collect();
    assert!(uris.contains(&"qoob3d://script-api") && uris.contains(&"qoob3d://guide"), "{uris:?}");
    assert_eq!(uris.iter().collect::<BTreeSet<_>>().len(), uris.len(), "no URI twice: {uris:?}");

    // The reference is docs/script-api.md of this version, as Markdown.
    let (mime, text) = read_resource(&mut mcp, "qoob3d://script-api");
    assert_eq!(mime, "text/markdown");
    assert_eq!(text.trim(), SCRIPT_API.trim());
    // The guide resource is the guide tool.
    let (_, guide) = read_resource(&mut mcp, "qoob3d://guide");
    assert_eq!(guide.trim(), text_of(&mcp.call("guide", &json!({}))).trim());

    // One resource per example script, each readable and a script.
    let examples: Vec<&str> =
        uris.iter().copied().filter(|u| u.starts_with("qoob3d://examples/")).collect();
    assert!(!examples.is_empty(), "at least one example: {uris:?}");
    for uri in &examples {
        let name = uri.trim_start_matches("qoob3d://examples/");
        assert!(name.ends_with(".rhai") && name.len() > 5 && !name.contains('/'), "{uri}");
        let (_, script) = read_resource(&mut mcp, uri);
        assert!(script.contains('(') && script.trim().len() > 20, "{uri} is no script: {script}");
    }
    assert_eq!(resources.len(), 2 + examples.len(), "nothing but reference, guide and examples");

    // Unknown resources are JSON-RPC errors; the session goes on.
    for uri in ["qoob3d://examples/no-such-example.rhai", "qoob3d://nope", "file:///etc/passwd"] {
        let r = mcp.request("resources/read", &json!({"uri": uri}), QUICK);
        assert!(r["error"]["code"].is_i64() && r.get("result").is_none(), "{uri}: {r}");
    }
    assert_eq!(mcp.result("ping", &json!({})), json!({}));
    mcp.finish();
}

#[test]
fn ag_mc_06_07_the_guide_fits_120_lines_and_names_the_loop() {
    let p = small_project("mcp-guide");
    let mut mcp = Mcp::open(&p);
    let guide = text_of(&mcp.call("guide", &json!({})));
    let lines = guide.trim().lines().count();
    // AG-MC-07 raised the bound of AG-MC-06 from 60 to 120 lines.
    assert!((10..=120).contains(&lines), "the guide has {lines} lines (at most 120)");
    let lower = guide.to_lowercase();
    // Project layout.
    for folder in ["qoob3d.toml", "parts/", "lib/", "values/", "checks/", "tasks/"] {
        assert!(guide.contains(folder), "the guide names {folder}");
    }
    // The loop, by the names of the tools and states an agent has to use.
    let tools =
        ["task_list", "task_update", "api_reference", "file_write", "build", "check", "render"];
    for word in tools.into_iter().chain(["in_progress", "review", "blocked"]) {
        assert!(guide.contains(word), "the guide names {word}");
    }
    // The rules: sources for values, never guess, batched booleans, mm.
    assert!(lower.contains("source"), "values need sources");
    let ask = ["guess", "ask", "invent", "assum"].iter().any(|w| lower.contains(w));
    assert!(ask, "never guess a dimension, ask via blocked");
    assert!(
        guide.contains("union(") || guide.contains("cut(") || lower.contains("batch"),
        "batched booleans"
    );
    assert!(guide.contains("mm"), "units");
    // How errors are reported.
    assert!(lower.contains("error"), "how errors are reported");
    mcp.finish();
}

// ===========================================================================
// §7 acceptance R4 (with the binding of R5, AG-TK-05/06): box with lid over MCP
// ===========================================================================

const TASK_FILE: &str = "tasks/0001-box-with-lid.md";
/// The task of the human: free text. It binds no parts and checks; the
/// agent binds what it builds (AG-TK-05), and the gate needs that (AG-TK-06).
const TASK: &str = "\
+++
title = \"Box with lid and pin connection, 0.2 mm clearance\"
kind = \"task\"
state = \"open\"
created = \"2026-11-22\"
+++

A box 30 x 20 x 12 mm with a lid of 2 mm. The lid is held by two pins of
3 mm diameter and 3 mm length; the holes have 0.2 mm clearance.
";
const FIT_VALUES: &str = "\
[clearance]
value = 0.2
source = \"task T-0001: 0.2 mm clearance\"
status = \"one_source\"
";
/// Box: 30·20·12 minus the cavity 22·16·10 (the tool is 11 high and ends
/// above the box) minus two holes from the top face at x = ±13. `{C}` is the
/// clearance: the first attempt has it wrong.
const BOX_PART: &str = "\
part(\"Box\", #{ material: \"PLA\" }, |deps| {
    let c = {C};
    cut(box(30, 20, 12), [
        box(22, 16, 11).move(0, 0, 2),
        hole(3, 3, c).move(13, 0, 12),
        hole(3, 3, c).move(-13, 0, 12),
    ])
});
";
/// Lid in its mounted position: plate 30·20·2 on top of the box (z ∈ [12, 14])
/// with two pins of d = 3, l = 3 pointing down at x = ±13.
const LID_PART: &str = "\
part(\"Lid\", #{ material: \"PLA\" }, |deps| {
    union([
        box(30, 20, 2),
        pin(3, 3).move(13, 0, 0),
        pin(3, 3).move(-13, 0, 0),
    ]).move(0, 0, 12)
});
";
/// Lifted by 1 mm, the plate is 1 mm above the box and the pins (r = 1.5)
/// are still 2 mm deep in their holes (r = 1.5 + clearance): the smallest
/// distance between the parts is the radial gap = clearance.
const FIT_CHECKS: &str = "\
check(\"pin_clearance\", || {
    let gap = distance(part(\"Box\"), part(\"Lid\").move(0, 0, 1));
    expect_near(gap, value(\"clearance\"), 0.0001, \"radial clearance of the pins\");
});
check(\"lid_volume\", || {
    let plate = 30 * 20 * 2;
    let pins = 2 * PI() * 1.5 * 1.5 * 3;
    expect_near(part(\"Lid\").volume(), plate + pins, 0.001, \"plate plus two pins\");
    expect(part(\"Lid\").solids() == 1, \"the lid is one solid\");
});
";
/// V(Box) = 7200 − 3520 − 2·π·r²·depth with hole r = 1.5 + c, depth = 3 + c.
fn box_volume(clearance: f64) -> f64 {
    let r = 1.5 + clearance;
    7200.0 - 3520.0 - 2.0 * std::f64::consts::PI * r * r * (3.0 + clearance)
}
/// V(Lid) = 30·20·2 + 2·π·1.5²·3 = 1200 + 13.5π.
const LID_VOLUME: f64 = 1200.0 + 13.5 * std::f64::consts::PI;

fn write_file(mcp: &mut Mcp, path: &str, content: &str) {
    mcp.call("file_write", &json!({"path": path, "content": content})).ok();
}

/// `task_update` to review that the gate must refuse naming `words`.
fn gate_refuses(mcp: &mut Mcp, words: &[&str], what: &str) {
    let args = json!({"id": "T-0001", "state": "review", "note": "ready for review"});
    let tool = mcp.call("task_update", &args);
    tool.refused(words, what);
    assert_eq!(tool.error_kind(), "gate", "{what}: {}", tool.envelope);
    let state = mcp.call("task_get", &json!({"id": "T-0001"})).ok()["state"].clone();
    assert_eq!(state, json!("in_progress"), "{what}: a refused hand-over changed the task");
}

fn part_states(mcp: &mut Mcp) -> Vec<(String, String)> {
    let st = mcp.call("project_status", &json!({}));
    let parts = st.ok()["parts"].as_array().cloned().unwrap_or_default();
    let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let mut states: Vec<_> = parts.iter().map(|p| (text(&p["name"]), text(&p["state"]))).collect();
    states.sort();
    states
}

#[test]
fn r4_acceptance_an_agent_builds_box_and_lid_over_mcp_and_the_human_sets_done() {
    // The human's side: an empty project in Git with one task file.
    let p = Project::new("box-with-lid", "");
    p.write(TASK_FILE, TASK);
    git_init(&p.dir);
    assert!(service_json(&p.dir).is_none(), "no service runs: the server starts it on demand");

    // The agent's side: nothing but the MCP server.
    let mut mcp = Mcp::spawn(&p);
    let init = mcp.initialize(VERSIONS[2]);
    assert!(init["instructions"].as_str().is_some_and(|g| g.contains("task_list")), "{init}");
    accept_take_the_task(&mut mcp);
    accept_first_attempt(&mut mcp, &p);
    accept_fix(&mut mcp);
    accept_hand_over(&mut mcp, &p);
    accept_human_sets_done(&mut mcp, &p);
    mcp.finish();
}

/// task_list → task_get → task_update in_progress → read the API reference.
fn accept_take_the_task(mcp: &mut Mcp) {
    let open = mcp.call("task_list", &json!({"state": "open"}));
    let t = &open.ok()[0];
    assert_eq!(t["id"], json!("T-0001"), "{}", open.envelope);
    assert_eq!(t["title"], json!("Box with lid and pin connection, 0.2 mm clearance"));
    assert_eq!((&t["parts"], &t["checks"]), (&json!([]), &json!([])), "a free-text task");
    let shown = mcp.call("task_get", &json!({"id": "T-0001"}));
    assert!(shown.ok()["body"].as_str().is_some_and(|b| b.contains("0.2 mm clearance")));

    let taken = mcp.call("task_update", &json!({"id": "T-0001", "state": "in_progress"}));
    assert_eq!(taken.ok()["state"], json!("in_progress"), "{}", taken.envelope);
    // AG-TK-06: nothing is bound, so a note alone opens no gate (this is
    // how the first fresh-agent run of R5 handed over unchecked work).
    gate_refuses(mcp, &["part", "check"], "review of a task with nothing bound");

    // AG-TK-05: the agent binds what it is going to build and check.
    let args = json!({"id": "T-0001", "parts": ["Box", "Lid"],
        "checks": ["pin_clearance", "lid_volume"]});
    let bound = mcp.call("task_update", &args);
    assert_eq!(
        (&bound.ok()["parts"], &bound.ok()["checks"], &bound.ok()["state"]),
        (&json!(["Box", "Lid"]), &json!(["pin_clearance", "lid_volume"]), &json!("in_progress")),
        "{}",
        bound.envelope
    );
    // Nothing exists yet: the gate names both parts and both checks.
    gate_refuses(mcp, &["Box", "Lid", "pin_clearance", "lid_volume"], "review before any work");

    let reference = text_of(&mcp.call("api_reference", &json!({"search": "pin"})));
    assert!(
        reference.contains("`pin(d, l) -> Shape`") && reference.contains("`hole(d, l, clearance)")
    );
}

/// Parts and checks are written; the box has 0.1 mm clearance by mistake.
fn accept_first_attempt(mcp: &mut Mcp, p: &Project) {
    write_file(mcp, "values/fit.toml", FIT_VALUES);
    write_file(mcp, "parts/box.rhai", &BOX_PART.replace("{C}", "0.1"));
    write_file(mcp, "parts/lid.rhai", LID_PART);
    write_file(mcp, "checks/fit.rhai", FIT_CHECKS);
    // The first tool that needs it started the service (AG-MC-01).
    let s = Service::attach(p);
    let st = s.request("GET", "/status", None).ok();
    assert_eq!(st["parts"].as_array().map(Vec::len), Some(2), "HTTP sees the files of MCP: {st}");

    let built = mcp.call("build", &json!({"wait_s": 60}));
    let parts = &built.ok()["parts"];
    assert_eq!(
        (&parts["Box"]["state"], &parts["Lid"]["state"]),
        (&json!("built"), &json!("built"))
    );
    assert_rel(&parts["Box"]["volume"], box_volume(0.1), VOL, "volume of the first box");
    assert_rel(&parts["Lid"]["volume"], LID_VOLUME, VOL, "volume of the lid");
    // Box z ∈ [0, 12]; lid plate z ∈ [12, 14], pins down to z = 9.
    assert_vec(&parts["Box"]["bbox"]["size"], [30.0, 20.0, 12.0], 1e-6, "size of the box");
    assert_vec(&parts["Lid"]["bbox"]["min"], [-15.0, -10.0, 9.0], 1e-6, "lid bbox.min");
    assert_vec(&parts["Lid"]["bbox"]["max"], [15.0, 10.0, 14.0], 1e-6, "lid bbox.max");

    // The check measures 0.1 mm instead of 0.2 mm.
    let checked = mcp.call("check", &json!({"wait_s": 60}));
    let r = checked.ok();
    assert_eq!((r["passed"].as_u64(), r["failed"].as_u64()), (Some(1), Some(1)), "{r}");
    let checks = r["checks"].as_array().cloned().unwrap_or_default();
    let fit = checks.iter().find(|c| c["name"] == json!("pin_clearance")).expect("pin_clearance");
    assert_eq!(fit["state"], json!("failed"), "{fit}");
    let e = &fit["expectations"][0];
    assert_eq!((&e["text"], &e["passed"]), (&json!("radial clearance of the pins"), &json!(false)));
    let (actual, expected) = (e["actual"].as_f64().unwrap_or(f64::NAN), e["expected"].as_f64());
    assert!((actual - 0.1).abs() < GAP && expected == Some(0.2), "measured 0.1, wanted 0.2: {e}");

    // Red: no hand-over and no commit.
    gate_refuses(mcp, &["pin_clearance"], "review with a failed check");
    let saved = mcp.call("save_state", &json!({"message": "must not be committed"}));
    saved.refused(&["pin_clearance"], "save_state with a failed check");
    assert_eq!(saved.error_kind(), "gate", "{}", saved.envelope);
    assert!(git(&p.dir, &["rev-parse", "HEAD"]).is_err(), "a refused save_state committed");
}

/// The box reads the clearance from the value; stale states close the gate.
fn accept_fix(mcp: &mut Mcp) {
    write_file(mcp, "parts/box.rhai", &BOX_PART.replace("{C}", "value(\"clearance\")"));
    let states = part_states(mcp);
    let expected =
        [("Box", "outdated"), ("Lid", "current")].map(|(n, s)| (n.to_owned(), s.to_owned()));
    assert_eq!(states, expected, "only the changed part is stale");
    gate_refuses(mcp, &["Box"], "review with a stale part");

    let built = mcp.call("build", &json!({"wait_s": 60}));
    let r = built.ok();
    assert_eq!(
        (string_set(&r["built"]), string_set(&r["cached"])),
        (set(&["Box"]), set(&["Lid"])),
        "{r}"
    );
    assert_rel(&r["parts"]["Box"]["volume"], box_volume(0.2), VOL, "volume of the fixed box");
    // Built, but the check has not run after this build.
    gate_refuses(mcp, &["pin_clearance"], "review before the checks ran again");

    let checked = mcp.call("check", &json!({"wait_s": 60}));
    let r = checked.ok();
    assert_eq!((r["passed"].as_u64(), r["failed"].as_u64()), (Some(2), Some(0)), "{r}");
    let checks = r["checks"].as_array().cloned().unwrap_or_default();
    let fit = checks.iter().find(|c| c["name"] == json!("pin_clearance")).expect("pin_clearance");
    let actual = fit["expectations"][0]["actual"].as_f64().unwrap_or(f64::NAN);
    assert!((actual - 0.2).abs() < GAP, "the clearance is 0.2 mm: {fit}");
}

/// Measure, look, hand over with `review`, commit.
fn accept_hand_over(mcp: &mut Mcp, p: &Project) {
    // Mounted, the lid lies on the box: they touch (distance 0) and share
    // no volume (a press fit or a collision would show as common volume).
    let m = mcp.call("measure", &json!({"a": "Box", "b": "Lid"}));
    assert_rel(&m.ok()["volume"], box_volume(0.2), VOL, "volume of the box");
    assert_eq!(m.ok()["solids"], json!(1), "{}", m.envelope);
    let (distance, common) = (m.ok()["distance"].as_f64(), m.ok()["common_volume"].as_f64());
    assert!(distance.is_some_and(|d| d.abs() < 1e-6), "lid and box touch: {}", m.envelope);
    assert!(common.is_some_and(|v| v.abs() < 1e-6), "no interference: {}", m.envelope);
    // The same question with a throwaway script, lid lifted by 1 mm.
    let source = "let lifted = part(\"Lid\").move(0, 0, 1);\n\
                  print(distance(part(\"Box\"), lifted) < 0.2001);\n\
                  show(lifted, \"lifted\");\n";
    let run = mcp.call("run_script", &json!({"source": source, "wait_s": 60}));
    assert_eq!(run.ok()["printed"], json!(["true"]), "{}", run.envelope);
    // Pins from z = 9 + 1, plate up to z = 14 + 1; the volume is the lid's.
    assert_vec(&run.ok()["shapes"][0]["bbox"]["min"], [-15.0, -10.0, 10.0], 1e-6, "lifted lid");
    assert_vec(&run.ok()["shapes"][0]["bbox"]["max"], [15.0, 10.0, 15.0], 1e-6, "lifted lid");
    assert_rel(&run.ok()["shapes"][0]["volume"], LID_VOLUME, VOL, "lifted lid");

    rendered(mcp, p, &json!({"view": "iso", "width": 320, "height": 240}), (320, 240));

    // Green: the gate opens. The agent can still not accept its own work.
    let args = json!({"id": "T-0001", "state": "review", "note": "Box and lid built, checks pass"});
    assert_eq!(mcp.call("task_update", &args).ok()["state"], json!("review"));
    mcp.call("task_update", &json!({"id": "T-0001", "state": "done"}))
        .refused(&["T-0001", "done"], "the agent sets done");

    let saved = mcp.call("save_state", &json!({"message": "Box with lid, T-0001 for review"}));
    let commit = saved.ok()["commit"].as_str().unwrap_or_default().to_owned();
    let head = git(&p.dir, &["rev-parse", "HEAD"]).expect("a commit exists");
    assert!(commit.len() >= 7 && head.starts_with(&commit), "commit {commit:?} is HEAD {head}");
    let tracked = git_tracked(&p.dir);
    for file in [
        "qoob3d.toml",
        "parts/box.rhai",
        "parts/lid.rhai",
        "values/fit.toml",
        "checks/fit.rhai",
        TASK_FILE,
    ] {
        assert!(tracked.iter().any(|t| t == file), "{file} not committed: {tracked:?}");
    }
    // out/renders/*.png and the cache exist and stay out of Git.
    assert!(
        !tracked.iter().any(|t| t.starts_with("out/") || t.starts_with(".qoob3d/")),
        "{tracked:?}"
    );
    assert!(
        git(&p.dir, &["show", "HEAD:tasks/0001-box-with-lid.md"])
            .is_ok_and(|t| t.contains("state = \"review\""))
    );
}

/// The human accepts with the CLI; the task file keeps the human's words.
fn accept_human_sets_done(mcp: &mut Mcp, p: &Project) {
    let out =
        cli(&p.dir, &["task", "set", "T-0001", "done", "--note", "printed, the lid fits"], CLI);
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    assert_eq!(out.envelope()["result"]["state"], json!("done"));

    // Only the `state` line changed and the `parts`/`checks` lines were
    // added by the binding (AG-TK-05); the human's words are byte-identical.
    let file = p.read(TASK_FILE);
    let kept: String = file
        .split_inclusive('\n')
        .filter(|l| !l.starts_with("parts") && !l.starts_with("checks"))
        .collect();
    let expected = TASK.replace("state = \"open\"", "state = \"done\"");
    assert!(kept.starts_with(expected.trim_end()), "header and body byte-identical:\n{file}");
    for key in ["parts", "checks"] {
        let lines = file.lines().filter(|l| l.starts_with(key)).count();
        assert_eq!(lines, 1, "one {key} line in the header:\n{file}");
    }
    let log: Vec<&str> =
        file.split_once("\n## Log").map(|(_, l)| l.lines().collect()).unwrap_or_default();
    // AG-TK-07: `<old> -> <new>` and the note; AG-TK-06: the hand-over entry
    // lists the bound parts and checks.
    let agent = log.iter().position(|l| {
        let bound = ["Box", "Lid", "pin_clearance", "lid_volume"];
        l.contains(" agent: in_progress -> review")
            && l.contains("Box and lid built, checks pass")
            && bound.iter().all(|name| l.contains(name))
    });
    let human =
        log.iter().position(|l| l.ends_with(" human: review -> done: printed, the lid fits"));
    assert!(agent.is_some() && agent < human, "log in order, agent then human:\n{file}");

    let shown = mcp.call("task_get", &json!({"id": "T-0001"}));
    assert_eq!(shown.ok()["state"], json!("done"), "{}", shown.envelope);
    mcp.call("task_update", &json!({"id": "T-0001", "state": "in_progress"}))
        .refused(&["T-0001"], "the agent reopens an accepted task");
    assert!(
        mcp.call("task_list", &json!({"state": "open"})).ok().as_array().is_some_and(Vec::is_empty)
    );
}
