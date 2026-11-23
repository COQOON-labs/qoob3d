//! Shared helpers of the tests that run the real `qoob3d` binary as service
//! and CLI: `tests/agent.rs`, `tests/mcp.rs`, `tests/mcp_guide.rs`,
//! `tests/service.rs` and `tests/perf.rs` (docs/requirements/agent.md §2–5,
//! ADR-0053, ADR-0054): temporary projects, processes, HTTP with the service
//! token, the JSON envelope (docs/requirements/project-app.md §1), the
//! service and the CLI.
//!
//! Included with `#[path = "common/agent.rs"] mod support;` so that
//! `tests/project.rs` (which uses `common/mod.rs`) is not affected. Every
//! item here is used by each of those test files, directly or through
//! another item here, so no test binary sees dead code. Helpers that only
//! some of them need live in the other files of `common/`, each included by
//! exactly the files that use all of it (and found as `crate::support`):
//!
//! - `common/inspect.rs` (`mod inspect`): attach to a service, wait for
//!   discovery, read files, compare coordinates. agent, mcp, mcp_guide, service.
//! - `common/artifacts.rs` (`mod artifacts`): PNG and Git checks. agent, mcp, mcp_guide.
//! - `common/builds.rs` (`mod builds`): `/status` and `/build`. service, perf.
//! - `common/mcp.rs` (`mod mcp_client`): the MCP stdio client. mcp, mcp_guide.
//!
//! Processes: [`Project`] stops the service of its folder on drop (also one
//! that the CLI or the MCP server started on demand), kills what lingers and
//! removes the folder; a [`Service`] that a test spawned shuts down on drop
//! and kills its workers, so no test leaves processes or temp dirs behind.

use std::collections::BTreeSet;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub(crate) const BIN: &str = env!("CARGO_BIN_EXE_qoob3d");
/// Relative tolerance for volumes and areas of boxes and cylinders: exact
/// in OCCT up to rounding, the same margin as the R1 kernel tests.
pub(crate) const VOL: f64 = 1e-6;
/// Upper bound for a service to come up (WA-OP-02 allows 20 s).
pub(crate) const START: Duration = Duration::from_secs(20);

static NEXT: AtomicUsize = AtomicUsize::new(0);

// ------------------------------------------------------------------ project

/// A temporary project `<root>/project/` with `qoob3d.toml`. Drop stops its
/// service and removes `<root>`; declare [`Service`] guards after it.
pub(crate) struct Project {
    pub(crate) root: PathBuf,
    pub(crate) dir: PathBuf,
    name: String,
}

impl Project {
    /// Project named `name`; `extra` is appended to `qoob3d.toml` (limits, workers).
    pub(crate) fn new(name: &str, extra: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("qoob3d-agent-{name}-{}-{n}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let dir = root.join("project");
        std::fs::create_dir_all(dir.join("parts")).expect("create project");
        let p = Self { root, dir, name: name.to_owned() };
        p.config(extra);
        p
    }

    /// Rewrites `qoob3d.toml` with `extra` after the `[project]` table.
    pub(crate) fn config(&self, extra: &str) -> &Self {
        let name = &self.name;
        self.write("qoob3d.toml", &format!("[project]\nname = \"{name}\"\nunits = \"mm\"\n{extra}"))
    }

    pub(crate) fn write(&self, rel: &str, text: &str) -> &Self {
        let path = self.dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create folder");
        std::fs::write(&path, text).expect("write project file");
        self
    }

    pub(crate) fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        stop_service(&self.dir);
        std::fs::remove_dir_all(&self.root).ok();
    }
}

// ------------------------------------------------------------------ processes

/// `(pid, ppid, args)` of all processes (POSIX `ps`, Linux and macOS).
pub(crate) fn processes() -> Vec<(u32, u32, String)> {
    let out = Command::new("ps").args(["-A", "-o", "pid=,ppid=,args="]).output().expect("run ps");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let pid = w.next()?.parse().ok()?;
            let ppid = w.next()?.parse().ok()?;
            Some((pid, ppid, w.collect::<Vec<_>>().join(" ")))
        })
        .collect()
}

/// Alive and not a zombie.
pub(crate) fn alive(pid: u32) -> bool {
    let out = Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output();
    out.is_ok_and(|o| {
        let stat = String::from_utf8_lossy(&o.stdout).trim().to_owned();
        !stat.is_empty() && !stat.starts_with('Z')
    })
}

/// Sends `-<signal>` (`KILL`, `SEGV`, …) to `pid`.
pub(crate) fn kill(pid: u32, signal: &str) {
    Command::new("kill").args([format!("-{signal}"), pid.to_string()]).output().ok();
}

/// Live `qoob3d worker` children of process `parent`.
pub(crate) fn workers_of(parent: u32) -> Vec<u32> {
    processes()
        .into_iter()
        .filter(|(pid, ppid, args)| *ppid == parent && args.contains("worker") && alive(*pid))
        .map(|(pid, ..)| pid)
        .collect()
}

/// Waits until `cond` holds, polling every 20 ms; false on timeout.
pub(crate) fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `(port, pid)` from `.qoob3d/service.json` (project-app.md §1).
pub(crate) fn service_json(dir: &Path) -> Option<(u16, u32)> {
    let text = std::fs::read_to_string(dir.join(".qoob3d/service.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some((u16::try_from(v["port"].as_u64()?).ok()?, u32::try_from(v["pid"].as_u64()?).ok()?))
}

/// The token from `.qoob3d/service.json` that every request carries in the
/// header `X-Qoob3d-Token` (AG-HD-03); `None` if the file has none.
pub(crate) fn service_token(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(".qoob3d/service.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v["token"].as_str().map(str::to_owned)
}

/// Stops whatever serves `dir`: `POST /shutdown`, then SIGKILL for the
/// service, its workers and any process that still names the folder.
pub(crate) fn stop_service(dir: &Path) {
    let all = processes();
    let needle = dir.display().to_string();
    let mut victims: Vec<u32> = Vec::new();
    if let Some((port, pid)) = service_json(dir) {
        victims.extend(all.iter().filter(|(_, ppid, _)| *ppid == pid).map(|(p, ..)| *p));
        let token = service_token(dir);
        let shutdown = Some(&json!({}));
        http(port, token.as_deref(), "POST", "/shutdown", shutdown, Duration::from_secs(2)).ok();
        if !wait_until(Duration::from_secs(10), || !alive(pid)) {
            victims.push(pid);
        }
    }
    let me = std::process::id();
    let named = all.iter().filter(|(pid, _, args)| *pid != me && args.contains(&needle));
    for (pid, ..) in named {
        victims.extend(all.iter().filter(|(_, ppid, _)| ppid == pid).map(|(p, ..)| *p));
        victims.push(*pid);
    }
    for pid in victims {
        if alive(pid) {
            kill(pid, "KILL");
        }
    }
}

// ------------------------------------------------------------------ envelope and HTTP

/// The JSON envelope `{ok, duration_s, warnings, result}` with `error`
/// `{kind, message}` when not ok (project-app.md §1, AG-MC-02), checked
/// field by field. Returns `ok`.
pub(crate) fn check_envelope(v: &Value) -> bool {
    let ok = v["ok"].as_bool().unwrap_or_else(|| panic!("envelope without bool ok: {v}"));
    let d = v["duration_s"].as_f64().unwrap_or_else(|| panic!("envelope without duration_s: {v}"));
    assert!(d >= 0.0 && d.is_finite(), "duration_s {d}");
    assert!(v["warnings"].is_array(), "envelope without warnings array: {v}");
    assert!(v.get("result").is_some(), "envelope without result: {v}");
    if ok {
        assert!(v.get("error").is_none_or(Value::is_null), "ok envelope with an error: {v}");
    } else {
        assert!(v["error"]["kind"].is_string(), "error without kind: {v}");
        let message = v["error"]["message"].as_str().unwrap_or_default();
        assert!(!message.is_empty(), "error without message: {v}");
    }
    ok
}

pub(crate) struct Reply {
    pub(crate) code: u16,
    head: String,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    /// The value of header `name` (case-insensitive).
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The checked JSON envelope of the answer.
    pub(crate) fn envelope(&self) -> Value {
        let v: Value = serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!("no JSON envelope ({e}), HTTP {}: {}", self.code, self.text())
        });
        check_envelope(&v);
        v
    }

    /// `result` of an envelope with `ok: true`.
    pub(crate) fn ok(&self) -> Value {
        let v = self.envelope();
        assert_eq!(v["ok"], json!(true), "HTTP {}: {v}", self.code);
        v["result"].clone()
    }
}

/// One HTTP/1.1 request to 127.0.0.1:`port` (`Connection: close`), with the
/// service token in `X-Qoob3d-Token` when there is one (AG-HD-03).
fn http(
    port: u16,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<Reply, String> {
    let body = body.map(Value::to_string).unwrap_or_default();
    let token = token.map(|t| format!("X-Qoob3d-Token: {t}\r\n")).unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{token}Connection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    exchange(port, &req, timeout)
}

/// Sends `request` (a complete HTTP/1.1 request, written by the caller) to
/// 127.0.0.1:`port` and reads the answer: the body up to `Content-Length`,
/// or up to the end of the connection without one.
pub(crate) fn exchange(port: u16, request: &str, timeout: Duration) -> Result<Reply, String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("connect: {e}"))?;
    s.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    s.write_all(request.as_bytes()).map_err(|e| format!("write: {e}"))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = s.read(&mut chunk).map_err(|e| format!("read head: {e}"))?;
        if n == 0 {
            return Err("closed before the header ended".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let code = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("status line")?;
    let mut reply = Reply { code, head, body: buf[end..].to_vec() };
    match reply.header("content-length").and_then(|l| l.parse::<usize>().ok()) {
        Some(len) => {
            while reply.body.len() < len {
                let n = s.read(&mut chunk).map_err(|e| format!("read body: {e}"))?;
                if n == 0 {
                    return Err(format!("body truncated at {} of {len}", reply.body.len()));
                }
                reply.body.extend_from_slice(&chunk[..n]);
            }
        }
        None => {
            s.read_to_end(&mut reply.body).map_err(|e| format!("read body: {e}"))?;
        }
    }
    Ok(reply)
}

// ------------------------------------------------------------------ service

/// A running `qoob3d serve` of a project. Declare it after its [`Project`]:
/// drop stops the service before the folder goes away.
pub(crate) struct Service {
    pub(crate) port: u16,
    pub(crate) pid: u32,
    /// From `service.json`, sent with every request (AG-HD-03).
    token: Option<String>,
    dir: PathBuf,
    /// Set when this handle spawned `serve` itself (stopped and reaped on drop).
    pub(crate) child: Option<Child>,
    log: PathBuf,
    /// Every worker pid ever seen, killed on drop if still alive.
    seen_workers: Mutex<BTreeSet<u32>>,
}

impl Service {
    /// Spawns `qoob3d serve <DIR>` and waits until it answers `/status`.
    pub(crate) fn start(p: &Project) -> Self {
        let mut command = Command::new(BIN);
        command.arg("serve").arg(&p.dir);
        Self::start_with(p, &mut command)
    }

    /// Spawns `command` (which must end up as `qoob3d serve <DIR>` under the
    /// spawned pid) and waits until it answers `/status`.
    pub(crate) fn start_with(p: &Project, command: &mut Command) -> Self {
        let log = p.root.join("serve.log");
        let err = std::fs::File::create(&log).expect("create serve.log");
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(err)
            .spawn()
            .expect("spawn qoob3d serve");
        let s = Self::wait_ready(p, Some(child), log);
        let pid = s.child.as_ref().map(Child::id);
        assert_eq!(Some(s.pid), pid, "service.json names the serve process");
        s
    }

    /// Waits until `service.json` names a service that answers `/status`
    /// (with `child`: that child, not a stale or foreign entry).
    pub(crate) fn wait_ready(p: &Project, mut child: Option<Child>, log: PathBuf) -> Self {
        let deadline = Instant::now() + START;
        loop {
            if let Some((port, pid)) = service_json(&p.dir) {
                let token = service_token(&p.dir);
                let wait = Duration::from_secs(1);
                let ours = child.as_ref().is_none_or(|c| c.id() == pid);
                if ours
                    && http(port, token.as_deref(), "GET", "/status", None, wait)
                        .is_ok_and(|r| r.code == 200)
                {
                    let (dir, seen_workers) = (p.dir.clone(), Mutex::new(BTreeSet::new()));
                    return Self { port, pid, token, dir, child, log, seen_workers };
                }
            }
            if let Some(Ok(Some(status))) = child.as_mut().map(Child::try_wait) {
                let log = std::fs::read_to_string(&log).unwrap_or_default();
                panic!("qoob3d serve exited early with {status}:\n{log}");
            }
            if Instant::now() >= deadline {
                if let Some(c) = child.as_mut() {
                    c.kill().ok();
                    c.wait().ok();
                }
                panic!("service for {} not up within {START:?}", p.dir.display());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The last 3000 bytes of the service log.
    pub(crate) fn log(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let start = text.len().saturating_sub(3000);
        text.get(start..).unwrap_or_default().to_owned()
    }

    /// One request; panics with the tail of the service log if it gets no answer.
    pub(crate) fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Reply {
        self.try_request(method, path, body, Duration::from_secs(90))
            .unwrap_or_else(|e| panic!("{method} {path}: {e}\nservice log:\n{}", self.log()))
    }

    /// One request with the service token; `Err` if there is no answer in `timeout`.
    pub(crate) fn try_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Reply, String> {
        http(self.port, self.token.as_deref(), method, path, body, timeout)
    }

    /// Live `qoob3d worker` children of the service.
    pub(crate) fn workers(&self) -> Vec<u32> {
        let w = workers_of(self.pid);
        self.seen_workers.lock().unwrap_or_else(PoisonError::into_inner).extend(&w);
        w
    }

    /// `POST /shutdown`; waits for the service to exit (AG-SV-01).
    pub(crate) fn shutdown(&mut self) -> bool {
        self.workers();
        let sent = self.try_request("POST", "/shutdown", Some(&json!({})), Duration::from_secs(2));
        let pid = self.pid;
        let gone = match self.child.as_mut() {
            Some(c) => {
                wait_until(Duration::from_secs(10), || c.try_wait().is_ok_and(|s| s.is_some()))
            }
            None => wait_until(Duration::from_secs(10), || !alive(pid)),
        };
        sent.is_ok() && gone
    }
}

impl Drop for Service {
    /// Stops only a service this handle spawned: shutdown, then SIGKILL for
    /// the service, its workers (also those seen earlier) and whatever names
    /// the folder. An attached handle is a view: its service (and an MCP
    /// server that names the folder) must outlive it and is stopped by
    /// [`Project`]'s drop.
    fn drop(&mut self) {
        if self.child.is_none() {
            return;
        }
        if !self.shutdown() {
            kill(self.pid, "KILL");
        }
        stop_service(&self.dir);
        if let Some(c) = self.child.as_mut() {
            c.kill().ok();
            c.wait().ok();
        }
        let seen = self.seen_workers.get_mut().unwrap_or_else(PoisonError::into_inner);
        for pid in seen.iter() {
            if alive(*pid) {
                kill(*pid, "KILL");
            }
        }
    }
}

// ------------------------------------------------------------------ CLI

pub(crate) struct CliOut {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl CliOut {
    /// stdout as the checked JSON envelope (AG-SV-02).
    pub(crate) fn envelope(&self) -> Value {
        let v: Value = serde_json::from_str(self.stdout.trim()).unwrap_or_else(|e| {
            panic!("stdout is no JSON envelope ({e}):\n{}\nstderr:\n{}", self.stdout, self.stderr)
        });
        check_envelope(&v);
        v
    }
}

fn reader(mut r: impl std::io::Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut s = String::new();
        r.read_to_string(&mut s).ok();
        tx.send(s).ok();
    });
    rx
}

/// `qoob3d <args>` with working directory `dir`, killed after `timeout`.
pub(crate) fn cli(dir: &Path, args: &[&str], timeout: Duration) -> CliOut {
    let mut child = Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qoob3d CLI");
    let out = reader(child.stdout.take().expect("stdout"));
    let err = reader(child.stderr.take().expect("stderr"));
    let exited = wait_until(timeout, || child.try_wait().is_ok_and(|s| s.is_some()));
    if !exited {
        child.kill().ok();
        child.wait().ok();
        panic!("qoob3d {args:?} did not finish within {timeout:?}");
    }
    let code = child.wait().expect("wait CLI").code();
    // Whatever the CLI starts (a service on demand, bench workers) must not
    // hold its pipes open.
    let pipe = Duration::from_secs(3);
    let stdout = out.recv_timeout(pipe).expect("stdout closed: nothing may inherit it");
    let stderr = err.recv_timeout(pipe).expect("stderr closed: nothing may inherit it");
    CliOut { code, stdout, stderr }
}

// ------------------------------------------------------------------ numbers

/// A JSON number against `expected`, relative tolerance `tol`.
pub(crate) fn assert_rel(actual: &Value, expected: f64, tol: f64, what: &str) {
    let a = actual.as_f64().unwrap_or_else(|| panic!("{what}: no number: {actual}"));
    assert_rel_f64(a, expected, tol, what);
}

/// `actual` against `expected`, relative tolerance `tol`.
pub(crate) fn assert_rel_f64(actual: f64, expected: f64, tol: f64, what: &str) {
    let rel = (actual - expected).abs() / expected.abs();
    assert!(rel < tol, "{what}: got {actual}, expected {expected} (rel {rel:e})");
}
