//! Shared helpers of `tests/agent.rs` and `tests/mcp.rs` (R4,
//! docs/requirements/agent.md §4.2–4.5, ADR-0054): temporary projects, the
//! real `qoob3d` binary as service and CLI, HTTP, the JSON envelope
//! (docs/requirements/project-app.md §1), PNG and Git checks.
//!
//! Included with `#[path = "common/agent.rs"] mod support;` so that
//! `tests/project.rs` (which uses `common/mod.rs`) is not affected. Every
//! item here is used by both test files; helpers only one of them needs live
//! in that file.
//!
//! Processes: [`Project`] stops the service of its folder on drop (also one
//! that the CLI or the MCP server started on demand), kills what lingers and
//! removes the folder, so no test leaves processes or temp dirs behind.

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub(crate) const BIN: &str = env!("CARGO_BIN_EXE_qoob3d");
/// Relative tolerance for volumes and areas of boxes and cylinders: exact
/// in OCCT up to rounding, the same margin as the R1 kernel tests.
pub(crate) const VOL: f64 = 1e-6;
/// Upper bound for a service to come up (WA-OP-02 allows 20 s).
const START: Duration = Duration::from_secs(20);

static NEXT: AtomicUsize = AtomicUsize::new(0);

// ------------------------------------------------------------------ project

/// A temporary project `<root>/project/` with `qoob3d.toml`. Drop stops its
/// service and removes `<root>`.
pub(crate) struct Project {
    pub(crate) root: PathBuf,
    pub(crate) dir: PathBuf,
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
        let p = Self { root, dir };
        p.write("qoob3d.toml", &format!("[project]\nname = \"{name}\"\nunits = \"mm\"\n{extra}"));
        p
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

    pub(crate) fn read(&self, rel: &str) -> String {
        let path = self.path(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
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
fn processes() -> Vec<(u32, u32, String)> {
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
fn alive(pid: u32) -> bool {
    let out = Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output();
    out.is_ok_and(|o| {
        let stat = String::from_utf8_lossy(&o.stdout).trim().to_owned();
        !stat.is_empty() && !stat.starts_with('Z')
    })
}

fn kill(pid: u32) {
    Command::new("kill").args(["-KILL".to_owned(), pid.to_string()]).output().ok();
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
fn stop_service(dir: &Path) {
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
            kill(pid);
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
    if ok {
        assert!(v.get("result").is_some(), "ok envelope without result: {v}");
        assert!(v.get("error").is_none_or(Value::is_null), "ok envelope with an error: {v}");
    } else {
        assert!(v["error"]["kind"].is_string(), "error without kind: {v}");
        let message = v["error"]["message"].as_str().unwrap_or_default();
        assert!(!message.is_empty(), "error without message: {v}");
    }
    ok
}

pub(crate) struct Reply {
    code: u16,
    body: Vec<u8>,
}

impl Reply {
    /// The checked JSON envelope of the answer.
    pub(crate) fn envelope(&self) -> Value {
        let text = String::from_utf8_lossy(&self.body);
        let v: Value = serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("no JSON envelope ({e}), HTTP {}: {text}", self.code));
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
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("connect: {e}"))?;
    s.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    let body = body.map(Value::to_string).unwrap_or_default();
    let token = token.map(|t| format!("X-Qoob3d-Token: {t}\r\n")).unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{token}Connection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| format!("write: {e}"))?;
    // `Connection: close`: the answer ends when the service closes.
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).map_err(|e| format!("read: {e}"))?;
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n").ok_or("no header end")? + 4;
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let code = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("status line")?;
    Ok(Reply { code, body: buf[end..].to_vec() })
}

// ------------------------------------------------------------------ service

/// A running `qoob3d serve` of a project. Declare it after its [`Project`]:
/// drop stops the service before the folder goes away.
pub(crate) struct Service {
    port: u16,
    /// From `service.json`, sent with every request (AG-HD-03).
    token: Option<String>,
    dir: PathBuf,
    child: Option<Child>,
    log: PathBuf,
}

impl Service {
    /// Spawns `qoob3d serve <DIR>` and waits until it answers `/status`.
    pub(crate) fn start(p: &Project) -> Self {
        let log = p.root.join("serve.log");
        let err = std::fs::File::create(&log).expect("create serve.log");
        let child = Command::new(BIN)
            .arg("serve")
            .arg(&p.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(err)
            .spawn()
            .expect("spawn qoob3d serve");
        Self::wait_ready(p, Some(child), log)
    }

    /// Handle for a service started by someone else (CLI or MCP, on demand);
    /// dropping it leaves that service running.
    pub(crate) fn attach(p: &Project) -> Self {
        Self::wait_ready(p, None, p.root.join("serve.log"))
    }

    fn wait_ready(p: &Project, mut child: Option<Child>, log: PathBuf) -> Self {
        let deadline = Instant::now() + START;
        loop {
            if let Some((port, _)) = service_json(&p.dir) {
                let token = service_token(&p.dir);
                let wait = Duration::from_secs(1);
                let probe = http(port, token.as_deref(), "GET", "/status", None, wait);
                if probe.is_ok_and(|r| r.code == 200) {
                    return Self { port, token, dir: p.dir.clone(), child, log };
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

    /// One request; panics with the tail of the service log if it gets no answer.
    pub(crate) fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Reply {
        let (token, wait) = (self.token.as_deref(), Duration::from_secs(90));
        http(self.port, token, method, path, body, wait).unwrap_or_else(|e| {
            let text = std::fs::read_to_string(&self.log).unwrap_or_default();
            let start = text.len().saturating_sub(3000);
            let tail = text.get(start..).unwrap_or_default();
            panic!("{method} {path}: {e}\nservice log:\n{tail}")
        })
    }
}

impl Drop for Service {
    /// Stops only a service this handle spawned. An attached handle is a
    /// view: its service (and an MCP server that names the folder) must
    /// outlive it and is stopped by [`Project`]'s drop.
    fn drop(&mut self) {
        if let Some(c) = self.child.as_mut() {
            stop_service(&self.dir);
            c.kill().ok();
            c.wait().ok();
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
    // A service started on demand must not hold the CLI's pipes open.
    let pipe = Duration::from_secs(3);
    let stdout = out.recv_timeout(pipe).expect("stdout closed: the service must not inherit it");
    let stderr = err.recv_timeout(pipe).expect("stderr closed: the service must not inherit it");
    CliOut { code, stdout, stderr }
}

// ------------------------------------------------------------------ numbers

pub(crate) fn assert_rel(actual: &Value, expected: f64, tol: f64, what: &str) {
    let a = actual.as_f64().unwrap_or_else(|| panic!("{what}: no number: {actual}"));
    let rel = (a - expected).abs() / expected.abs();
    assert!(rel < tol, "{what}: got {a}, expected {expected} (rel {rel:e})");
}

/// A JSON array `[x, y, z]` against `expected`, absolute tolerance `tol` (mm).
pub(crate) fn assert_vec(v: &Value, expected: [f64; 3], tol: f64, what: &str) {
    for (i, e) in expected.iter().enumerate() {
        let a = v[i].as_f64().unwrap_or_else(|| panic!("{what}[{i}] missing: {v}"));
        assert!((a - e).abs() < tol, "{what}: got {v}, expected {expected:?}");
    }
}

// ------------------------------------------------------------------ PNG

/// Width and height from the PNG header, read by hand (independent of the
/// writer): 8 signature bytes `89 50 4E 47 0D 0A 1A 0A`, then the first
/// chunk, which must be `IHDR` with length 13: u32 BE width at byte 16,
/// u32 BE height at byte 20 (PNG specification §5.2, §11.2.2).
pub(crate) fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.len() > 33, "PNG shorter than signature + IHDR: {} bytes", bytes.len());
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(&bytes[8..12], &[0, 0, 0, 13], "IHDR length");
    assert_eq!(&bytes[12..16], b"IHDR", "first chunk is IHDR");
    let be = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    (be(16), be(20))
}

/// Number of distinct pixel values (capped at 64) of a PNG, decoded with
/// the `png` crate: a render that shows a shaded part on a background has
/// more than a handful, an empty or failed render has one.
pub(crate) fn png_colors(bytes: &[u8]) -> usize {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("valid PNG");
    let size = reader.output_buffer_size().expect("PNG size fits");
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).expect("PNG pixels decode");
    let px = (info.line_size / info.width as usize).max(1);
    let mut seen = std::collections::BTreeSet::new();
    for pixel in buf[..info.buffer_size()].chunks_exact(px) {
        seen.insert(pixel.to_vec());
        if seen.len() >= 64 {
            break;
        }
    }
    seen.len()
}

// ------------------------------------------------------------------ Git

/// `git <args>` in `dir`; trimmed stdout, or `Err` with stderr.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git not runnable: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// A fresh repository in `dir` with a local identity (no global config
/// needed, nothing outside the temp project is touched) and no signing.
pub(crate) fn git_init(dir: &Path) {
    for args in [
        &["init", "--quiet"][..],
        &["config", "user.name", "QOOB3D Test"][..],
        &["config", "user.email", "test@qoob3d.invalid"][..],
        &["config", "commit.gpgsign", "false"][..],
    ] {
        git(dir, args).unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    }
}

/// Files tracked at `HEAD` (`git ls-tree`), relative to the repository.
pub(crate) fn git_tracked(dir: &Path) -> Vec<String> {
    let out = git(dir, &["ls-tree", "-r", "--name-only", "HEAD"]).expect("git ls-tree HEAD");
    out.lines().map(str::to_owned).collect()
}
