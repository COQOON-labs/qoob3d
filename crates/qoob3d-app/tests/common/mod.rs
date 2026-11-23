//! Shared helpers for the project app tests (WA-*, docs/requirements/project-app.md,
//! ADR-0053): a fake `qoob3d serve` speaking the protocol of section 1
//! (WA-PR-01..03), a temporary project folder and a qmesh builder (WA-QM-01).
//!
//! R5 (docs/requirements/agent.md §6.1): `service.json` carries a token
//! (AG-HD-03); the fake records the `X-Qoob3d-Token` header of every request
//! so tests can assert what the client sent, and it decodes percent-encoded
//! part names in `/mesh/<name>` as the real service does (AG-HD-04).

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

// ------------------------------------------------------------------ qmesh

/// qmesh bytes (WA-QM-01) written field by field here, independent of the
/// production writer: magic `QMSH`, u32 version 1, u32 V, T, E, P, positions
/// f32×3×V, normals f32×3×V, indices u32×3×T, edge points f32×3×E, starts
/// u32×(P+1). Little endian.
pub(crate) fn qmesh(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    indices: &[u32],
    edge_points: &[[f32; 3]],
    starts: &[u32],
) -> Vec<u8> {
    let mut out = b"QMSH".to_vec();
    let counts = [
        1,
        positions.len() as u32,
        (indices.len() / 3) as u32,
        edge_points.len() as u32,
        (starts.len() - 1) as u32,
    ];
    for c in counts {
        out.extend_from_slice(&c.to_le_bytes());
    }
    for f in positions.iter().chain(normals).flatten() {
        out.extend_from_slice(&f.to_le_bytes());
    }
    for i in indices {
        out.extend_from_slice(&i.to_le_bytes());
    }
    for f in edge_points.iter().flatten() {
        out.extend_from_slice(&f.to_le_bytes());
    }
    for s in starts {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Axis-aligned cube from `min` with edge length `s` as qmesh: 8 shared
/// corners, 12 outward triangles, 12 two-point edge polylines.
pub(crate) fn cube_qmesh(min: [f32; 3], s: f32) -> Vec<u8> {
    let corners: Vec<[f32; 3]> = (0..8u8)
        .map(|i| {
            let bit = |k: u8| f32::from((i >> k) & 1);
            [min[0] + bit(0) * s, min[1] + bit(1) * s, min[2] + bit(2) * s]
        })
        .collect();
    // Corner index = x + 2y + 4z. Quads counter-clockwise seen from outside.
    let quads =
        [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let indices: Vec<u32> =
        quads.iter().flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]]).collect();
    let normals: Vec<[f32; 3]> = (0..8u8)
        .map(|i| {
            let sign = |k: u8| if (i >> k) & 1 == 1 { 1.0 } else { -1.0 };
            [sign(0), sign(1), sign(2)].map(|c: f32| c / 3f32.sqrt())
        })
        .collect();
    let pairs = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4)];
    let pairs = pairs.into_iter().chain([(1, 5), (2, 6), (3, 7)]);
    let points: Vec<[f32; 3]> = pairs.flat_map(|(a, b)| [corners[a], corners[b]]).collect();
    let starts: Vec<u32> = (0..=12).map(|i| i * 2).collect();
    qmesh(&corners, &normals, &indices, &points, &starts)
}

// ------------------------------------------------------------------ project folder

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A unique temporary folder `<tmp>/qoob3d-project-<tag>-<pid>-<n>/` with the
/// project in `project/` (with `qoob3d.toml`) and room for helper scripts
/// beside it. Removed on drop.
pub(crate) struct TempProject {
    pub(crate) root: PathBuf,
    pub(crate) dir: PathBuf,
}

impl TempProject {
    pub(crate) fn new(tag: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("qoob3d-project-{tag}-{}-{n}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let dir = root.join("project");
        std::fs::create_dir_all(dir.join("parts")).expect("create project dir");
        // Project file as in AG-PJ-01 (docs/requirements/agent.md).
        std::fs::write(dir.join("qoob3d.toml"), "[project]\nname = \"probe\"\nunits = \"mm\"\n")
            .expect("write qoob3d.toml");
        Self { root, dir }
    }

    /// A folder next to the project that is *not* a project (no `qoob3d.toml`).
    pub(crate) fn not_a_project(&self) -> PathBuf {
        let dir = self.root.join("not-a-project");
        std::fs::create_dir_all(&dir).expect("create dir");
        dir
    }

    /// `.qoob3d/service.json` as written by the service (section 1), with
    /// the token of the service on `port` (AG-HD-03, [`token_for`]). Written
    /// atomically (rename), as the service does: a poller may read it at any
    /// time.
    pub(crate) fn write_service_json(&self, port: u16) {
        let dir = self.dir.join(".qoob3d");
        std::fs::create_dir_all(&dir).expect("create .qoob3d");
        let text =
            format!("{{\"port\": {port}, \"pid\": 4242, \"token\": \"{}\"}}", token_for(port));
        let tmp = dir.join("service.json.test-tmp");
        std::fs::write(&tmp, text).expect("write service.json");
        std::fs::rename(&tmp, dir.join("service.json")).expect("rename service.json");
    }

    /// A port nobody listens on (bound once, then released).
    pub(crate) fn dead_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().expect("addr").port()
    }

    /// Stand-in for the service program (WA-OP-02): a `sh` script that records its
    /// arguments in `<root>/calls.txt` and, when called as
    /// `serve <DIR>`, writes `<DIR>/.qoob3d/service.json` pointing to
    /// `port` (atomically via rename). Returns the command words
    /// (`["sh", "<script>"]`), i.e. what `QOOB3D_SERVICE_CMD` would split into.
    pub(crate) fn start_script(&self, port: u16) -> Vec<String> {
        let body = format!(
            "printf '%s\\n' \"$@\" > \"{calls}\"\n\
             if [ \"$#\" != 2 ] || [ \"$1\" != \"serve\" ]; then exit 3; fi\n\
             mkdir -p \"$2/.qoob3d\"\n\
             printf '{{\"port\": %d, \"pid\": %d, \"token\": \"%s\"}}\\n' {port} $$ {token} \
             > \"$2/.qoob3d/service.json.tmp\"\n\
             mv \"$2/.qoob3d/service.json.tmp\" \"$2/.qoob3d/service.json\"\n",
            calls = self.calls_file().display(),
            token = token_for(port),
        );
        self.script("start.sh", &body)
    }

    /// A service stand-in that records its call but never starts anything.
    pub(crate) fn dud_script(&self) -> Vec<String> {
        let body = format!("printf '%s\\n' \"$@\" > \"{}\"\nexit 0\n", self.calls_file().display());
        self.script("dud.sh", &body)
    }

    fn script(&self, name: &str, body: &str) -> Vec<String> {
        let path = self.root.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write script");
        vec!["sh".to_owned(), path.display().to_string()]
    }

    fn calls_file(&self) -> PathBuf {
        self.root.join("calls.txt")
    }

    /// Arguments of the last call of a stand-in script, `None` if never called.
    pub(crate) fn script_call(&self) -> Option<Vec<String>> {
        let text = std::fs::read_to_string(self.calls_file()).ok()?;
        Some(text.lines().map(str::to_owned).collect())
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

/// Same folder, ignoring `/private` symlinks on macOS and relative spelling.
pub(crate) fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

// ------------------------------------------------------------------ fake service

/// The token a fake service on `port` puts into `service.json` (AG-HD-03):
/// 32 hex characters, different for every port, so a test can tell which
/// `service.json` a client read.
pub(crate) fn token_for(port: u16) -> String {
    format!("{port:032x}")
}

/// Percent-decoding of a path segment (RFC 3986 §2.1): `%XX` is the byte
/// with the hex value XX; anything else stands for itself.
fn percent_decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(byte) if bytes[i] == b'%' => {
                out.push(byte);
                i += 3;
            }
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One part as the fake service reports it.
#[derive(Clone)]
pub(crate) struct FakePart {
    pub(crate) name: String,
    /// `current`, `outdated`, `failed`, `never_built` (WA-PR-01).
    pub(crate) state: String,
    /// Cache key of the last successful build, `None` → JSON `null`.
    pub(crate) key: Option<String>,
    /// Bytes served by `/mesh/<name>` (normally qmesh); `None` → 404 (never built).
    pub(crate) mesh: Option<Vec<u8>>,
}

impl FakePart {
    pub(crate) fn built(name: &str, state: &str, key: &str, mesh: Vec<u8>) -> Self {
        Self { name: name.into(), state: state.into(), key: Some(key.into()), mesh: Some(mesh) }
    }

    pub(crate) fn unbuilt(name: &str, state: &str) -> Self {
        Self { name: name.into(), state: state.into(), key: None, mesh: None }
    }
}

/// How the fake reacts to connections.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Serve,
    /// Listener closed: connections are refused (the service died).
    Refuse,
    /// Connections are accepted but never answered.
    Hang,
}

struct State {
    parts: Vec<FakePart>,
    mode: Mode,
    /// Requests in arrival order, e.g. `GET /status`, `GET /mesh/shaft`
    /// (the path as sent, not decoded).
    log: Vec<String>,
    /// The `X-Qoob3d-Token` header of every request, in arrival order.
    tokens: Vec<Option<String>>,
    /// Bodies of `POST /build`.
    builds: Vec<String>,
    /// Next `/mesh/<name>` answers with this header key instead (one shot).
    header_key_once: HashMap<String, String>,
    status_delay: Duration,
    status_error: bool,
}

/// A fake `qoob3d serve` on 127.0.0.1 (section 1 of the requirements).
pub(crate) struct FakeService {
    pub(crate) port: u16,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeService {
    pub(crate) fn start(parts: Vec<FakePart>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake service");
        let port = listener.local_addr().expect("addr").port();
        let state = Arc::new(Mutex::new(State {
            parts,
            mode: Mode::Serve,
            log: Vec::new(),
            tokens: Vec::new(),
            builds: Vec::new(),
            header_key_once: HashMap::new(),
            status_delay: Duration::ZERO,
            status_error: false,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (state, stop) = (state.clone(), stop.clone());
            std::thread::spawn(move || accept_loop(Some(listener), port, &state, &stop))
        };
        Self { port, state, stop, thread: Some(thread) }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("fake state poisoned")
    }

    pub(crate) fn set_mode(&self, mode: Mode) {
        self.lock().mode = mode;
        // Let the accept loop close or reopen the listener.
        std::thread::sleep(Duration::from_millis(30));
    }

    pub(crate) fn set_parts(&self, parts: Vec<FakePart>) {
        self.lock().parts = parts;
    }

    pub(crate) fn set_header_key_once(&self, part: &str, key: &str) {
        self.lock().header_key_once.insert(part.into(), key.into());
    }

    pub(crate) fn set_status_delay(&self, delay: Duration) {
        self.lock().status_delay = delay;
    }

    pub(crate) fn set_status_error(&self, error: bool) {
        self.lock().status_error = error;
    }

    pub(crate) fn log(&self) -> Vec<String> {
        self.lock().log.clone()
    }

    /// `(request, X-Qoob3d-Token header)` of every request so far.
    pub(crate) fn tokens(&self) -> Vec<(String, Option<String>)> {
        let s = self.lock();
        s.log.iter().cloned().zip(s.tokens.iter().cloned()).collect()
    }

    pub(crate) fn count(&self, request: &str) -> usize {
        self.lock().log.iter().filter(|r| *r == request).count()
    }

    pub(crate) fn builds(&self) -> Vec<String> {
        self.lock().builds.clone()
    }
}

impl Drop for FakeService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.join().ok();
        }
    }
}

fn accept_loop(
    mut listener: Option<TcpListener>,
    port: u16,
    state: &Arc<Mutex<State>>,
    stop: &AtomicBool,
) {
    let mut held = Vec::new(); // connections in Mode::Hang stay open, unanswered
    while !stop.load(Ordering::Relaxed) {
        let mode = state.lock().expect("fake state poisoned").mode;
        if mode == Mode::Refuse {
            listener = None;
            held.clear();
        } else if listener.is_none() {
            // std sets SO_REUSEADDR on Unix, so the same port can be reopened.
            listener = TcpListener::bind(("127.0.0.1", port)).ok();
        }
        let Some(l) = &listener else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        l.set_nonblocking(true).expect("nonblocking listener");
        match l.accept() {
            Ok((stream, _)) => {
                // Accepted sockets inherit O_NONBLOCK on macOS.
                stream.set_nonblocking(false).expect("blocking stream");
                if mode == Mode::Hang {
                    held.push(stream);
                } else {
                    let state = state.clone();
                    std::thread::spawn(move || handle(stream, &state));
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

/// Method, path, body and the `X-Qoob3d-Token` header of one request.
fn read_request(stream: &mut TcpStream) -> Option<(String, String, Vec<u8>, Option<String>)> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split_whitespace();
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let headers: Vec<(&str, &str)> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim(), v.trim())).collect();
    let header =
        |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|h| h.1);
    let length = header("content-length").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let token = header("x-qoob3d-token").map(str::to_owned);
    let mut body = buf[head_end..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some((method, path, body, token))
}

fn envelope(result: &serde_json::Value) -> Vec<u8> {
    serde_json::json!({"ok": true, "duration_s": 0.001, "warnings": [], "result": result})
        .to_string()
        .into_bytes()
}

fn error_envelope(kind: &str, message: &str) -> Vec<u8> {
    serde_json::json!({
        "ok": false, "duration_s": 0.001, "warnings": [], "result": null,
        "error": {"kind": kind, "message": message}
    })
    .to_string()
    .into_bytes()
}

fn status_body(state: &mut State) -> (u16, Vec<u8>) {
    if state.status_error {
        return (500, error_envelope("internal", "broken"));
    }
    let list: Vec<_> = state
        .parts
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "file": format!("parts/{}.rhai", p.name),
                "state": p.state,
                "key": p.key,
                // Extra fields must be ignored (WA-PR-01).
                "volume_mm3": 1000.0,
            })
        })
        .collect();
    let result = serde_json::json!({
        "project": "probe",
        "parts": list,
        "jobs": [],
        "workers": {"count": 2, "free": 2},
    });
    (200, envelope(&result))
}

fn handle(mut stream: TcpStream, state: &Arc<Mutex<State>>) {
    let Some((method, path, body, token)) = read_request(&mut stream) else { return };
    let request = format!("{method} {path}");
    let lock = || state.lock().expect("fake state poisoned");
    {
        let mut s = lock();
        s.log.push(request.clone());
        s.tokens.push(token);
    }
    let mut extra_header = String::new();
    let (code, content_type, payload) = if request == "GET /status" {
        let delay = lock().status_delay;
        std::thread::sleep(delay);
        let (code, payload) = status_body(&mut lock());
        (code, "application/json", payload)
    } else if let Some(segment) = path.strip_prefix("/mesh/").filter(|_| method == "GET") {
        // The name is one percent-encoded path segment (AG-HD-04).
        let name = percent_decode(segment);
        let mut s = lock();
        let part = s.parts.iter().find(|p| p.name == name).cloned();
        match part {
            Some(FakePart { mesh: Some(bytes), key, .. }) => {
                let key = s.header_key_once.remove(&name).or(key).unwrap_or_default();
                extra_header = format!("X-Qoob3d-Key: {key}\r\n");
                (200, "application/octet-stream", bytes)
            }
            _ => (404, "application/json", error_envelope("not_found", &name)),
        }
    } else if request == "POST /build" {
        let mut s = lock();
        s.builds.push(String::from_utf8_lossy(&body).into_owned());
        drop(s);
        let result = serde_json::json!({"job": "j1", "state": "running"});
        (200, "application/json", envelope(&result))
    } else {
        (404, "application/json", error_envelope("unknown", &request))
    };
    let reason = match code {
        200 => "OK",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         {extra_header}Connection: close\r\n\r\n",
        payload.len()
    );
    stream.write_all(head.as_bytes()).ok();
    stream.write_all(&payload).ok();
    stream.flush().ok();
    stream.shutdown(Shutdown::Both).ok();
}
