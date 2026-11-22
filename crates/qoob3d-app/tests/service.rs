//! R2/R3 without MCP: project service, worker processes, cache and CLI
//! against the real `qoob3d` binary (docs/requirements/agent.md §2, §2.1,
//! §3.1, §4; envelope and qmesh from docs/requirements/project-app.md;
//! ADR-0053). Every test names its requirement IDs.
//!
//! Test projects use only simple primitives (`box`, `cylinder`, `move`), so
//! every volume and bounding box is derived by hand next to the assertion.
//! Volumes of boxes and cylinders are exact in OCCT up to rounding; `VOL` =
//! 1e-6 relative is the same margin as the R1 kernel tests. Meshes are f32:
//! coordinates up to 40 mm round to < 4e-6 mm, so `F32_POS` = 1e-4 mm.
//!
//! Processes: every test owns its service through [`Service`], whose drop
//! sends `/shutdown` and kills the service and its workers if they linger.
//! The read-only checks share one built project in `built_project_*`.

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use qoob3d_app::live;
use qoob3d_core::qmesh;
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_qoob3d");
const VOL: f64 = 1e-6;
const F32_POS: f64 = 1e-4;
/// Upper bound for a service to come up (WA-OP-02 allows 20 s).
const START: Duration = Duration::from_secs(20);
/// `wait_s` for synchronous builds; simple boxes build in milliseconds.
const WAIT_S: f64 = 60.0;

// ===========================================================================
// Test projects
// ===========================================================================

/// A = box(a_size, 10, 10) with a_size = 20 → 20·10·10 = 2000 mm³,
/// bbox x ±10, y ±5, z 0..10.
const PART_A: &str = "part(\"A\", #{}, |deps| box(value(\"a_size\"), 10, 10));\n";
/// B = A moved by 30 in x → 2000 mm³, bbox x 20..40, y ±5, z 0..10.
const PART_B: &str = "part(\"B\", #{ depends: [\"A\"] }, |deps| deps[\"A\"].move(30, 0, 0));\n";
/// C = form::make() = box(4, 4, 4) → 64 mm³.
const PART_C: &str = "import \"lib/form\" as form;\npart(\"C\", #{}, |deps| form::make());\n";
const LIB_FORM: &str = "fn make() { box(4, 4, 4) }\n";
/// D = cylinder(10, 5) → π·5²·5 = 125π mm³, bbox x/y ±5, z 0..5.
const PART_D: &str = "part(\"D\", #{}, |deps| cylinder(10, 5));\n";
const VALUES: &str = "\
[a_size]
value = 20
source = \"test drawing\"
status = \"assumption\"

[unused_width]
value = 5
source = \"caliper\"
status = \"measured\"
";

/// Normal = box(10, 20, 30) → 6000 mm³.
const NORMAL: &str = "part(\"Normal\", #{}, |deps| box(10, 20, 30));\n";

/// Runs forever without hitting the operation limit (AG-SC-02, default 50 M):
/// 16 B doubled 14 times = 256 KiB; each loop iteration copies 512 KiB with a
/// handful of Rhai operations, so 50 M operations are minutes away and only
/// the time limit (worker kill, AG-PJ-05) stops it.
const ENDLESS: &str = "\
part(\"Endless\", #{}, |deps| {
    let s = \"0123456789abcdef\";
    for i in 0..14 { s += s; }
    loop { let t = s + s; }
});
";

/// Grows to 1200 · 512 KiB = 600 MiB resident (bounded, so a missed poll
/// cannot eat the machine), then idles like [`ENDLESS`]. With
/// `memory_mb = 300` only the memory limit can end it before `time_s = 30`.
const HOG: &str = "\
part(\"Hog\", #{}, |deps| {
    let s = \"0123456789abcdef\";
    for i in 0..14 { s += s; }
    let a = [];
    for i in 0..1200 { a.push(s + s); }
    loop { let t = s + s; }
});
";

/// Runtime error at line 3: `    let b = box(0, 10, 10);` → `box` starts at
/// column 13 (4 spaces + `let b = ` = 12 characters before it, 1-based).
const BAD: &str = "\
part(\"Bad\", #{}, |deps| {
    let a = box(10, 10, 10);
    let b = box(0, 10, 10);
    b
});
";

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A temporary project `<root>/project/` with `qoob3d.toml`; removed on drop
/// (after the [`Service`] guards declared later have stopped).
struct Project {
    root: PathBuf,
    dir: PathBuf,
}

impl Project {
    /// Project named `name`; `extra` is appended to `qoob3d.toml` (limits, workers).
    fn new(name: &str, extra: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("qoob3d-service-{name}-{}-{n}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let dir = root.join("project");
        std::fs::create_dir_all(dir.join("parts")).expect("create project");
        let p = Self { root, dir };
        p.write("qoob3d.toml", &format!("[project]\nname = \"{name}\"\nunits = \"mm\"\n{extra}"));
        p
    }

    /// The four-part project of acceptance R2: A, B (depends on A), C
    /// (imports lib/form.rhai), D independent, plus values.
    fn abcd(name: &str) -> Self {
        let p = Self::new(name, "[workers]\ncount = 2\n");
        p.write("parts/a.rhai", PART_A)
            .write("parts/b.rhai", PART_B)
            .write("parts/c.rhai", PART_C)
            .write("lib/form.rhai", LIB_FORM)
            .write("parts/d.rhai", PART_D)
            .write("values/dims.toml", VALUES);
        p
    }

    fn write(&self, rel: &str, text: &str) -> &Self {
        let path = self.dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create folder");
        std::fs::write(&path, text).expect("write project file");
        self
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

// ===========================================================================
// HTTP and envelope
// ===========================================================================

struct Reply {
    code: u16,
    head: String,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The JSON envelope `{ok, duration_s, warnings, result}` with `error`
    /// `{kind, message}` when not ok (project-app.md §1), checked field by field.
    fn envelope(&self) -> Value {
        let v: Value = serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!("no JSON envelope ({e}), HTTP {}: {}", self.code, self.text())
        });
        let ok = v["ok"].as_bool().unwrap_or_else(|| panic!("envelope without bool ok: {v}"));
        let d =
            v["duration_s"].as_f64().unwrap_or_else(|| panic!("envelope without duration_s: {v}"));
        assert!(d >= 0.0 && d.is_finite(), "duration_s {d}");
        assert!(v["warnings"].is_array(), "envelope without warnings array: {v}");
        assert!(v.get("result").is_some(), "envelope without result: {v}");
        if !ok {
            assert!(v["error"]["kind"].is_string(), "error without kind: {v}");
            assert!(v["error"]["message"].is_string(), "error without message: {v}");
        }
        v
    }

    /// `result` of an envelope with `ok: true`.
    fn ok(&self) -> Value {
        let v = self.envelope();
        assert_eq!(v["ok"], json!(true), "HTTP {}: {v}", self.code);
        v["result"].clone()
    }
}

/// One HTTP/1.1 request to 127.0.0.1:`port` (`Connection: close`).
fn http(
    port: u16,
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
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| format!("write: {e}"))?;
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

// ===========================================================================
// Processes
// ===========================================================================

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

fn kill(pid: u32, signal: &str) {
    Command::new("kill").args([format!("-{signal}"), pid.to_string()]).output().ok();
}

/// Waits until `cond` holds, polling every 20 ms; false on timeout.
fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
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
fn service_json(dir: &Path) -> Option<(u16, u32)> {
    let text = std::fs::read_to_string(dir.join(".qoob3d/service.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some((u16::try_from(v["port"].as_u64()?).ok()?, u32::try_from(v["pid"].as_u64()?).ok()?))
}

/// A running `qoob3d serve` for a project. Drop shuts it down and kills
/// whatever is left (service and workers), so no test leaks processes.
struct Service {
    port: u16,
    pid: u32,
    /// Set when this test spawned `serve` itself (reaped on drop).
    child: Option<Child>,
    log: PathBuf,
    /// Every worker pid ever seen, killed on drop if still alive.
    seen_workers: std::cell::RefCell<BTreeSet<u32>>,
}

impl Service {
    /// Spawns `qoob3d serve <DIR>` and waits until it answers `/status`.
    fn start(p: &Project) -> Self {
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
        let s = Self::wait_ready(p, Some(child), log);
        let pid = s.child.as_ref().map(Child::id);
        assert_eq!(Some(s.pid), pid, "service.json names the serve process");
        s
    }

    /// Guard for a service started by someone else (the CLI, AG-SV-02).
    fn attach(p: &Project) -> Self {
        Self::wait_ready(p, None, p.root.join("serve.log"))
    }

    fn wait_ready(p: &Project, mut child: Option<Child>, log: PathBuf) -> Self {
        let deadline = Instant::now() + START;
        loop {
            if let Some((port, pid)) = service_json(&p.dir) {
                let probe = http(port, "GET", "/status", None, Duration::from_secs(1));
                if probe.is_ok_and(|r| r.code == 200) {
                    let seen = std::cell::RefCell::new(BTreeSet::new());
                    return Self { port, pid, child, log, seen_workers: seen };
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

    fn log(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let start = text.len().saturating_sub(3000);
        text.get(start..).unwrap_or_default().to_owned()
    }

    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Reply {
        http(self.port, method, path, body, Duration::from_secs(90))
            .unwrap_or_else(|e| panic!("{method} {path}: {e}\nservice log:\n{}", self.log()))
    }

    fn get(&self, path: &str) -> Reply {
        self.request("GET", path, None)
    }

    fn post(&self, path: &str, body: &Value) -> Reply {
        self.request("POST", path, Some(body))
    }

    fn status(&self) -> Value {
        let r = self.get("/status");
        assert_eq!(r.code, 200);
        r.ok()
    }

    /// `POST /build` with `wait_s` = [`WAIT_S`]; the build result (§3.1),
    /// also when parts failed.
    fn build(&self, parts: Option<&[&str]>) -> Value {
        let r = self.post("/build", &json!({"parts": parts, "wait_s": WAIT_S}));
        let v = r.envelope();
        let result = v["result"].clone();
        assert!(result["parts"].is_object(), "no build result: {v}\nlog:\n{}", self.log());
        result
    }

    /// `POST /build` with `wait_s = 0` → the job id (§3.1).
    fn start_job(&self, parts: Option<&[&str]>) -> Value {
        let result = self.post("/build", &json!({"parts": parts, "wait_s": 0})).ok();
        let state = result["state"].as_str().unwrap_or_default();
        assert!(["running", "queued"].contains(&state), "wait_s 0 returns a job: {result}");
        assert!(result["job"].is_u64() || result["job"].is_string(), "job id: {result}");
        result["job"].clone()
    }

    /// Polls `GET /job/<id>` every 50 ms until a final state; the job and
    /// the time it was first seen final.
    fn wait_job(&self, id: &Value, timeout: Duration) -> (Value, Instant) {
        let deadline = Instant::now() + timeout;
        loop {
            let job = self.get(&format!("/job/{}", job_path(id))).ok();
            let state = job["state"].as_str().unwrap_or_default().to_owned();
            if ["done", "failed", "cancelled"].contains(&state.as_str()) {
                return (job, Instant::now());
            }
            assert!(Instant::now() < deadline, "job {id} still {state} after {timeout:?}: {job}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Live `qoob3d worker` children of the service.
    fn workers(&self) -> Vec<u32> {
        let w: Vec<u32> = processes()
            .into_iter()
            .filter(|(pid, ppid, args)| *ppid == self.pid && args.contains("worker") && alive(*pid))
            .map(|(pid, ..)| pid)
            .collect();
        self.seen_workers.borrow_mut().extend(&w);
        w
    }

    /// `POST /shutdown`; waits for the service to exit (AG-SV-01).
    fn shutdown(&mut self) -> bool {
        self.workers();
        let sent = http(self.port, "POST", "/shutdown", Some(&json!({})), Duration::from_secs(2));
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
    fn drop(&mut self) {
        let clean = self.shutdown();
        if !clean {
            kill(self.pid, "KILL");
        }
        if let Some(c) = self.child.as_mut() {
            c.kill().ok();
            c.wait().ok();
        }
        for pid in self.seen_workers.borrow().iter() {
            if alive(*pid) {
                kill(*pid, "KILL");
            }
        }
    }
}

fn job_path(id: &Value) -> String {
    id.as_str().map_or_else(|| id.to_string(), str::to_owned)
}

// ===========================================================================
// Result helpers
// ===========================================================================

fn names(list: &Value) -> BTreeSet<String> {
    list.as_array()
        .unwrap_or_else(|| panic!("expected a list of names: {list}"))
        .iter()
        .map(|n| n.as_str().unwrap_or_else(|| panic!("name {n}")).to_owned())
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// `(built, cached)` of a build result.
fn built_cached(result: &Value) -> (BTreeSet<String>, BTreeSet<String>) {
    (names(&result["built"]), names(&result["cached"]))
}

/// `{name: part}` of `/status` `result.parts`.
fn status_parts(status: &Value) -> std::collections::BTreeMap<String, Value> {
    status["parts"]
        .as_array()
        .unwrap_or_else(|| panic!("status without parts: {status}"))
        .iter()
        .map(|p| (p["name"].as_str().unwrap_or_default().to_owned(), p.clone()))
        .collect()
}

fn states(status: &Value) -> std::collections::BTreeMap<String, String> {
    status_parts(status)
        .into_iter()
        .map(|(n, p)| (n, p["state"].as_str().unwrap_or_default().to_owned()))
        .collect()
}

fn key_of(status: &Value, part: &str) -> String {
    let parts = status_parts(status);
    let p = parts.get(part).unwrap_or_else(|| panic!("no part {part} in {status}"));
    p["key"].as_str().unwrap_or_else(|| panic!("{part} has no key: {p}")).to_owned()
}

fn is_sha256_hex(key: &str) -> bool {
    key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit())
}

fn assert_rel(actual: f64, expected: f64, tol: f64, what: &str) {
    let rel = (actual - expected).abs() / expected.abs();
    assert!(rel < tol, "{what}: got {actual}, expected {expected} (rel {rel:e})");
}

fn assert_vec(v: &Value, expected: [f64; 3], tol: f64, what: &str) {
    for (i, e) in expected.iter().enumerate() {
        let a = v[i].as_f64().unwrap_or_else(|| panic!("{what}[{i}] missing: {v}"));
        assert!((a - e).abs() < tol, "{what}: got {v}, expected {expected:?}");
    }
}

/// A built part in a build result: state, volume, bbox, valid, duration (§3.1).
fn assert_built(result: &Value, part: &str, state: &str, volume: f64, bbox: ([f64; 3], [f64; 3])) {
    let p = &result["parts"][part];
    assert_eq!(p["state"], json!(state), "{part}: {p}");
    assert_rel(p["volume"].as_f64().unwrap_or(f64::NAN), volume, VOL, &format!("volume of {part}"));
    assert_vec(&p["bbox"]["min"], bbox.0, 1e-6, &format!("bbox.min of {part}"));
    assert_vec(&p["bbox"]["max"], bbox.1, 1e-6, &format!("bbox.max of {part}"));
    let size = [bbox.1[0] - bbox.0[0], bbox.1[1] - bbox.0[1], bbox.1[2] - bbox.0[2]];
    assert_vec(&p["bbox"]["size"], size, 1e-6, &format!("bbox.size of {part}"));
    assert_eq!(p["valid"], json!(true), "{part}: {p}");
    assert!(p["duration_s"].as_f64().is_some_and(|d| d >= 0.0), "{part}: {p}");
    assert!(p.get("error").is_none_or(Value::is_null), "{part}: {p}");
}

fn error_kind(result: &Value, part: &str) -> String {
    let p = &result["parts"][part];
    assert_eq!(p["state"], json!("failed"), "{part}: {p}");
    p["error"]["kind"]
        .as_str()
        .unwrap_or_else(|| panic!("{part} without error kind: {p}"))
        .to_owned()
}

const A_BOX: ([f64; 3], [f64; 3]) = ([-10.0, -5.0, 0.0], [10.0, 5.0, 10.0]);
const B_BOX: ([f64; 3], [f64; 3]) = ([20.0, -5.0, 0.0], [40.0, 5.0, 10.0]);
const C_BOX: ([f64; 3], [f64; 3]) = ([-2.0, -2.0, 0.0], [2.0, 2.0, 4.0]);
const D_BOX: ([f64; 3], [f64; 3]) = ([-5.0, -5.0, 0.0], [5.0, 5.0, 5.0]);
const D_VOLUME: f64 = 125.0 * std::f64::consts::PI;

// ===========================================================================
// CLI
// ===========================================================================

struct CliOut {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl CliOut {
    fn envelope(&self) -> Value {
        serde_json::from_str(self.stdout.trim()).unwrap_or_else(|e| {
            panic!("stdout is no JSON envelope ({e}):\n{}\nstderr:\n{}", self.stdout, self.stderr)
        })
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

/// `qoob3d <args>` in the project folder (AG-SV-02), killed after `timeout`.
fn cli(dir: &Path, args: &[&str], timeout: Duration) -> CliOut {
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

// ===========================================================================
// Mesh and file checks
// ===========================================================================

/// Volume (divergence theorem, Σ v1·(v2×v3)/6) and bbox of a binary STL,
/// parsed here independently of the writer.
fn stl_volume_bbox(bytes: &[u8]) -> (f64, [f64; 3], [f64; 3]) {
    assert!(bytes.len() >= 84, "STL shorter than its header");
    let n = u32::from_le_bytes(bytes[80..84].try_into().expect("4 bytes")) as usize;
    assert_eq!(bytes.len(), 84 + 50 * n, "binary STL size for {n} triangles");
    assert!(n >= 12, "a box has at least 12 triangles, got {n}");
    let f = |o: usize| f64::from(f32::from_le_bytes(bytes[o..o + 4].try_into().expect("4 bytes")));
    let (mut vol, mut lo, mut hi) = (0.0, [f64::MAX; 3], [f64::MIN; 3]);
    for t in 0..n {
        let base = 84 + 50 * t + 12;
        let v: Vec<[f64; 3]> = (0..3)
            .map(|k| [f(base + 12 * k), f(base + 12 * k + 4), f(base + 12 * k + 8)])
            .collect();
        let cross = [
            v[1][1] * v[2][2] - v[1][2] * v[2][1],
            v[1][2] * v[2][0] - v[1][0] * v[2][2],
            v[1][0] * v[2][1] - v[1][1] * v[2][0],
        ];
        vol += (v[0][0] * cross[0] + v[0][1] * cross[1] + v[0][2] * cross[2]) / 6.0;
        for p in &v {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
    }
    (vol, lo, hi)
}

/// Bounding box of qmesh positions.
fn qmesh_bbox(bytes: &[u8]) -> ([f64; 3], [f64; 3]) {
    let mesh = qmesh::read(bytes).unwrap_or_else(|e| panic!("invalid qmesh: {e:?}"));
    assert!(!mesh.faces.indices.is_empty(), "qmesh without triangles");
    assert!(!mesh.edges.points.is_empty(), "qmesh without edges (§2.1: edges included)");
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in &mesh.faces.positions {
        for i in 0..3 {
            lo[i] = lo[i].min(f64::from(p[i]));
            hi[i] = hi[i].max(f64::from(p[i]));
        }
    }
    (lo, hi)
}

fn assert_box(got: ([f64; 3], [f64; 3]), want: ([f64; 3], [f64; 3]), what: &str) {
    for i in 0..3 {
        assert!(
            (got.0[i] - want.0[i]).abs() < F32_POS && (got.1[i] - want.1[i]).abs() < F32_POS,
            "{what}: bbox {got:?}, expected {want:?}"
        );
    }
}

// ===========================================================================
// R2 acceptance (agent.md §4)
// ===========================================================================

/// Builds `part` of the running service and checks it is built (worker pool usable).
fn assert_normal_builds(s: &Service) {
    let r = s.build(Some(&["Normal"]));
    let p = &r["parts"]["Normal"];
    assert!(["built", "cached"].contains(&p["state"].as_str().unwrap_or_default()), "{r}");
    // box(10, 20, 30) = 6000 mm³.
    assert_rel(p["volume"].as_f64().unwrap_or(f64::NAN), 6000.0, VOL, "volume of Normal");
}

#[test]
fn ag_pj_05_ag_sv_01_endless_part_is_killed_after_the_time_limit_while_status_answers() {
    // §4 R2: time_s = 3 → killed after 3–5 s; /status < 100 ms on each of 20 polls.
    let p = Project::new("time", "[limits]\ntime_s = 3\n[workers]\ncount = 2\n");
    p.write("parts/endless.rhai", ENDLESS).write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    let t0 = Instant::now();
    let job = s.start_job(Some(&["Endless"]));
    let mut running_polls = 0;
    for i in 0..20 {
        let t = Instant::now();
        let r = http(s.port, "GET", "/status", None, Duration::from_secs(1))
            .unwrap_or_else(|e| panic!("poll {i}: {e}"));
        let took = t.elapsed();
        assert!(took < Duration::from_millis(100), "poll {i}: /status took {took:?}");
        let status = r.ok();
        let jobs = status["jobs"].as_array().cloned().unwrap_or_default();
        if jobs.iter().any(|j| j["id"] == job && j["state"] == json!("running")) {
            running_polls += 1;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // 20 polls take < 3 s, i.e. they ran while the job was running.
    assert!(running_polls >= 10, "the job was reported running in only {running_polls} polls");
    let (done, at) = s.wait_job(&job, Duration::from_secs(15));
    let took = at - t0;
    assert!(took >= Duration::from_secs(3), "killed before the limit: {took:?}");
    assert!(took <= Duration::from_secs(5), "killed too late: {took:?}");
    assert_eq!(error_kind(&done["result"], "Endless"), "time_limit", "{done}");
    assert_eq!(states(&s.status())["Endless"], "failed");
    assert_normal_builds(&s);
    assert_eq!(s.status()["workers"]["count"], json!(2), "the killed worker was replaced");
}

#[test]
fn ag_pj_05_memory_limit_kills_a_growing_part_within_ten_seconds() {
    // §4 R2 (5): RSS polled every 0.2 s; 600 MiB > memory_mb = 300.
    let p =
        Project::new("memory", "[limits]\ntime_s = 30\nmemory_mb = 300\n[workers]\ncount = 1\n");
    p.write("parts/hog.rhai", HOG).write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    let t0 = Instant::now();
    let job = s.start_job(Some(&["Hog"]));
    let (done, at) = s.wait_job(&job, Duration::from_secs(20));
    assert!(at - t0 <= Duration::from_secs(10), "memory limit took {:?}", at - t0);
    assert_eq!(error_kind(&done["result"], "Hog"), "memory_limit", "{done}");
    // The service survived (it never runs builds itself) and builds again.
    assert_normal_builds(&s);
}

#[test]
fn ag_pj_05_script_error_names_file_line_column_and_the_pool_stays_usable() {
    // §4 R2 (6). An unknown value id is a script error too (AG-PJ-03).
    let p = Project::new("script-error", "[workers]\ncount = 1\n");
    p.write("parts/bad.rhai", BAD)
        .write("parts/unknown.rhai", "part(\"Unknown\", #{}, |deps| box(value(\"nope\"), 1, 1));\n")
        .write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    let r = s.build(None);
    assert_eq!(error_kind(&r, "Bad"), "script");
    let e = &r["parts"]["Bad"]["error"];
    assert_eq!(e["file"], json!("parts/bad.rhai"), "{e}");
    assert_eq!((e["line"].as_u64(), e["column"].as_u64()), (Some(3), Some(13)), "{e}");
    assert_eq!(error_kind(&r, "Unknown"), "script");
    let msg = r["parts"]["Unknown"]["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("nope"), "the error names the unknown value: {msg}");
    // Other parts are unaffected, and after the errors a change still builds.
    assert!(names(&r["built"]).contains("Normal"), "{r}");
    p.write("parts/normal.rhai", &format!("{NORMAL}// changed\n"));
    assert_eq!(built_cached(&s.build(Some(&["Normal"]))).0, set(&["Normal"]));
    let st = s.status();
    assert_eq!(states(&st)["Bad"], "failed");
    let bad = &status_parts(&st)["Bad"];
    assert!(bad["error"].is_object(), "status carries the error: {bad}");
}

#[test]
fn ag_pj_05_crashed_worker_is_reported_as_crash_and_replaced() {
    // §4 R2 (7): a worker killed from outside (SIGKILL, portable) is a crash
    // for the service: it did not kill it for a limit.
    let p = Project::new("crash", "[limits]\ntime_s = 60\n[workers]\ncount = 2\n");
    p.write("parts/endless.rhai", ENDLESS).write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    let job = s.start_job(Some(&["Endless"]));
    let running = wait_until(Duration::from_secs(10), || {
        let st = s.status();
        let jobs = st["jobs"].as_array().cloned().unwrap_or_default();
        jobs.iter().any(|j| j["id"] == job && j["state"] == json!("running"))
            && !s.workers().is_empty()
    });
    assert!(running, "job never ran in a worker");
    std::thread::sleep(Duration::from_millis(500));
    let victims = s.workers();
    for pid in &victims {
        kill(*pid, "KILL");
    }
    let (done, _) = s.wait_job(&job, Duration::from_secs(10));
    assert_eq!(error_kind(&done["result"], "Endless"), "crash", "{done}");
    assert_normal_builds(&s);
    let now = s.workers();
    assert!(!now.is_empty(), "workers were replaced");
    assert!(now.iter().all(|w| !victims.contains(w)), "old {victims:?}, now {now:?}");
    assert_eq!(s.status()["workers"]["count"], json!(2));
}

/// Changes a project file and builds all parts; returns `(built, cached)`.
fn change_and_build(
    s: &Service,
    p: &Project,
    rel: &str,
    text: &str,
) -> (BTreeSet<String>, BTreeSet<String>) {
    p.write(rel, text);
    built_cached(&s.build(None))
}

#[test]
fn ag_pj_04_changes_rebuild_exactly_the_changed_parts_and_dependents() {
    // §4 R2 (2) and (3).
    let p = Project::abcd("incremental");
    let s = Service::start(&p);
    let all = set(&["A", "B", "C", "D"]);
    assert_eq!(built_cached(&s.build(None)), (all.clone(), set(&[])), "first build");
    let t = Instant::now();
    let r = s.build(None);
    let took = t.elapsed();
    assert_eq!(built_cached(&r), (set(&[]), all.clone()), "second build");
    assert!(took < Duration::from_millis(500), "fully cached build took {took:?}");
    assert_built(&r, "B", "cached", 2000.0, B_BOX);
    let keys0 = s.status();

    // A's file: A, and B through its dependency key.
    let a2 = PART_A.replace("10, 10)", "10, 12)");
    assert_eq!(change_and_build(&s, &p, "parts/a.rhai", &a2), (set(&["A", "B"]), set(&["C", "D"])));
    // B = A' moved: 20·10·12 = 2400 mm³.
    let r = s.build(None);
    assert_rel(r["parts"]["B"]["volume"].as_f64().unwrap_or(f64::NAN), 2400.0, VOL, "B after A");
    assert_eq!(key_of(&s.status(), "C"), key_of(&keys0, "C"));
    assert_ne!(key_of(&s.status(), "B"), key_of(&keys0, "B"));

    // The imported module: exactly C (box(4, 4, 5) = 80 mm³).
    let form2 = "fn make() { box(4, 4, 5) }\n";
    assert_eq!(
        change_and_build(&s, &p, "lib/form.rhai", form2),
        (set(&["C"]), set(&["A", "B", "D"]))
    );

    // A value A read: status first shows A and B outdated, then both rebuild.
    p.write("values/dims.toml", &VALUES.replace("value = 20", "value = 21"));
    let st = states(&s.status());
    assert_eq!((st["A"].as_str(), st["B"].as_str()), ("outdated", "outdated"), "{st:?}");
    assert_eq!((st["C"].as_str(), st["D"].as_str()), ("current", "current"), "{st:?}");
    assert_eq!(built_cached(&s.build(None)), (set(&["A", "B"]), set(&["C", "D"])));

    // A value nobody read: nothing.
    let keys1 = s.status();
    let unread = VALUES.replace("value = 20", "value = 21").replace("value = 5", "value = 6");
    p.write("values/dims.toml", &unread);
    assert!(states(&s.status()).values().all(|v| v == "current"), "unread value changes nothing");
    assert_eq!(built_cached(&s.build(None)), (set(&[]), all.clone()));
    for part in ["A", "B", "C", "D"] {
        assert_eq!(key_of(&s.status(), part), key_of(&keys1, part), "key of {part}");
    }

    // Rewriting D with identical bytes (new mtime only) changes nothing: keys hash bytes.
    assert_eq!(change_and_build(&s, &p, "parts/d.rhai", PART_D), (set(&[]), all));
}

#[test]
fn ag_pj_03_value_without_source_is_an_error_naming_the_file() {
    // §4 R2 (4), AG-SV-02 exit code 1.
    let p = Project::new("no-source", "");
    p.write("parts/normal.rhai", NORMAL)
        .write("values/bad.toml", "[wall]\nvalue = 1.2\nstatus = \"measured\"\n");
    let _s = Service::start(&p);
    for args in [&["values"][..], &["build", "--wait", "30"][..]] {
        let out = cli(&p.dir, args, Duration::from_secs(60));
        assert_eq!(out.code, Some(1), "qoob3d {args:?}: {}", out.stdout);
        let env = out.envelope();
        assert!(env["ok"].is_boolean(), "{env}");
        assert!(out.stdout.contains("bad.toml"), "{args:?} must name the file: {}", out.stdout);
        assert!(out.stdout.contains("wall"), "{args:?} must name the id: {}", out.stdout);
    }
}

// ===========================================================================
// AG-PJ-02: declarations, load errors
// ===========================================================================

#[test]
fn ag_pj_02_several_parts_per_file_with_dependencies() {
    // M2 = M1 moved up by 10 + a 2 mm cube on top:
    // M1 = box(10, 10, 10) = 1000, M2 = 1000 + 8 = 1008 mm³ (touching union).
    let p = Project::new("multi", "");
    p.write(
        "parts/multi.rhai",
        "part(\"M1\", #{ material: \"PLA\", color: \"white\" }, |deps| box(10, 10, 10));\n\
         part(\"M2\", #{ depends: [\"M1\"] }, |deps| deps[\"M1\"].move(0, 0, 10) + box(2, 2, 2).move(0, 0, 20));\n",
    );
    let s = Service::start(&p);
    let st = s.status();
    let parts = status_parts(&st);
    assert_eq!(parts.keys().cloned().collect::<BTreeSet<_>>(), set(&["M1", "M2"]));
    for part in parts.values() {
        assert_eq!(part["file"], json!("parts/multi.rhai"), "{part}");
        assert_eq!(part["state"], json!("never_built"), "{part}");
        assert!(part["key"].is_null(), "no key before the first build: {part}");
    }
    let r = s.build(None);
    assert_built(&r, "M1", "built", 1000.0, ([-5.0, -5.0, 0.0], [5.0, 5.0, 10.0]));
    assert_built(&r, "M2", "built", 1008.0, ([-5.0, -5.0, 10.0], [5.0, 5.0, 22.0]));
    let st = s.status();
    for part in ["M1", "M2"] {
        assert_eq!(states(&st)[part], "current");
        assert!(is_sha256_hex(&key_of(&st, part)));
    }
}

/// Builds all parts of a project with a load error; the answer must not be
/// a success and must name `words`.
fn assert_load_error(tag: &str, files: &[(&str, &str)], words: &[&str]) {
    let p = Project::new(tag, "");
    for (rel, text) in files {
        p.write(rel, text);
    }
    let s = Service::start(&p);
    let r = s.post("/build", &json!({"parts": null, "wait_s": WAIT_S}));
    let v = r.envelope();
    let failed = v["result"]["parts"]
        .as_object()
        .is_some_and(|m| m.values().any(|p| p["state"] == json!("failed")));
    assert!(v["ok"] == json!(false) || failed, "{tag}: load error reported as success: {v}");
    for w in words {
        assert!(r.text().contains(w), "{tag}: the error must name {w}: {v}");
    }
}

#[test]
fn ag_pj_02_unknown_dependency_is_a_load_error_naming_the_parts() {
    let b = "part(\"B\", #{ depends: [\"Ghost\"] }, |deps| box(1, 1, 1));\n";
    assert_load_error("unknown-dep", &[("parts/b.rhai", b)], &["B", "Ghost"]);
}

#[test]
fn ag_pj_02_cycle_is_a_load_error_naming_the_parts() {
    let x = "part(\"Alpha\", #{ depends: [\"Gamma\"] }, |deps| box(1, 1, 1));\n\
             part(\"Beta\", #{ depends: [\"Alpha\"] }, |deps| box(1, 1, 1));\n";
    let y = "part(\"Gamma\", #{ depends: [\"Beta\"] }, |deps| box(1, 1, 1));\n";
    assert_load_error(
        "cycle",
        &[("parts/x.rhai", x), ("parts/y.rhai", y)],
        &["Alpha", "Beta", "Gamma"],
    );
}

#[test]
fn ag_pj_02_duplicate_names_are_a_load_error() {
    let lid = "part(\"Lid\", #{}, |deps| box(1, 1, 1));\n";
    assert_load_error("duplicate", &[("parts/a.rhai", lid), ("parts/b.rhai", lid)], &["Lid"]);
}

// ===========================================================================
// Built project: read-only checks on one service
// ===========================================================================

#[test]
fn built_project_status_mesh_cache_and_export() {
    let p = Project::abcd("built");
    let s = Service::start(&p);
    let r = s.build(None);
    assert_built(&r, "A", "built", 2000.0, A_BOX);
    assert_built(&r, "B", "built", 2000.0, B_BOX);
    assert_built(&r, "C", "built", 64.0, C_BOX);
    assert_built(&r, "D", "built", D_VOLUME, D_BOX);
    let st = s.status();
    check_status_shape(&st);
    check_cache_files(&p, &st);
    check_mesh_endpoint(&s, &st);
    check_live_client(&p, &st);
    check_not_found(&s);
    check_export(&s, &p);
}

/// AG-SV-01, §3.1: `{project, parts: [{name, file, state, key}], jobs, workers}`.
fn check_status_shape(st: &Value) {
    assert_eq!(st["project"], json!("built"));
    let parts = status_parts(st);
    let files: Vec<(&str, &str)> = vec![
        ("A", "parts/a.rhai"),
        ("B", "parts/b.rhai"),
        ("C", "parts/c.rhai"),
        ("D", "parts/d.rhai"),
    ];
    assert_eq!(parts.len(), 4, "{st}");
    for (name, file) in files {
        let part = &parts[name];
        assert_eq!(part["file"], json!(file), "{part}");
        assert_eq!(part["state"], json!("current"), "{part}");
        assert!(is_sha256_hex(part["key"].as_str().unwrap_or_default()), "{part}");
    }
    assert!(st["jobs"].is_array(), "{st}");
    for j in st["jobs"].as_array().into_iter().flatten() {
        assert!(j.get("id").is_some() && j["state"].is_string(), "job entry {j}");
    }
    assert_eq!(st["workers"]["count"], json!(2), "[workers] count = 2: {st}");
    assert!(st["workers"]["busy"].as_u64().is_some_and(|b| b <= 2), "{st}");
}

/// §2.1: `.qoob3d/cache/<part>/<key>.{brep,qmsh,json}` with the listed JSON keys.
fn check_cache_files(p: &Project, st: &Value) {
    for (part, volume, bbox) in
        [("A", 2000.0, A_BOX), ("B", 2000.0, B_BOX), ("C", 64.0, C_BOX), ("D", D_VOLUME, D_BOX)]
    {
        let key = key_of(st, part);
        let base = p.path(&format!(".qoob3d/cache/{part}/{key}"));
        let brep = std::fs::read(base.with_extension("brep")).expect("cache .brep");
        assert!(!brep.is_empty(), "{part}.brep is empty");
        let qmsh = std::fs::read(base.with_extension("qmsh")).expect("cache .qmsh");
        assert_box(qmesh_bbox(&qmsh), bbox, &format!("cached mesh of {part}"));
        let text = std::fs::read_to_string(base.with_extension("json")).expect("cache .json");
        let j: Value = serde_json::from_str(&text).expect("cache json");
        assert_rel(
            j["volume"].as_f64().unwrap_or(f64::NAN),
            volume,
            VOL,
            &format!("cached volume {part}"),
        );
        for k in ["min", "max", "size"] {
            assert!(j["bbox"][k].is_array(), "{part}: bbox.{k} in {j}");
        }
        assert_eq!(j["valid"], json!(true), "{part}: {j}");
        assert_eq!(j["solids"], json!(1), "{part}: one solid: {j}");
        assert!(j["duration_s"].as_f64().is_some_and(|d| d >= 0.0), "{part}: {j}");
        assert!(j["warnings"].is_array(), "{part}: {j}");
        let reads = j["values_read"].to_string();
        assert!(!j["values_read"].is_null(), "{part}: values_read missing: {j}");
        // Only A calls value("a_size"); nobody reads unused_width.
        assert_eq!(reads.contains("a_size"), part == "A", "{part}: values_read {reads}");
        assert!(!reads.contains("unused_width"), "{part}: values_read {reads}");
    }
}

/// WA-PR-02: qmesh body, `application/octet-stream`, `X-Qoob3d-Key` = status key.
fn check_mesh_endpoint(s: &Service, st: &Value) {
    for (part, bbox) in [("A", A_BOX), ("B", B_BOX), ("C", C_BOX), ("D", D_BOX)] {
        let r = s.get(&format!("/mesh/{part}"));
        assert_eq!(r.code, 200, "/mesh/{part}: {}", r.text());
        let ct = r.header("content-type").unwrap_or_default();
        assert!(ct.starts_with("application/octet-stream"), "{part}: {ct}");
        assert_eq!(r.header("x-qoob3d-key"), Some(key_of(st, part)), "{part}");
        assert_box(qmesh_bbox(&r.body), bbox, &format!("mesh of {part}"));
    }
    // The cylinder D is tessellated to 0.01 mm: no vertex is outside r = 5.
    let mesh = qmesh::read(&s.get("/mesh/D").body).expect("qmesh of D");
    for p in &mesh.faces.positions {
        let r = f64::from(p[0]).hypot(f64::from(p[1]));
        assert!(r <= 5.0 + F32_POS, "vertex outside the cylinder: r = {r}");
    }
}

/// The W1 client (live.rs) talks to the real service.
fn check_live_client(p: &Project, st: &Value) {
    let dir = live::ProjectDir::open(&p.dir).expect("project dir");
    let client = live::Client::connect(&dir, &[BIN.to_owned()], START).expect("connect");
    let parts = client.status().expect("status via the W1 client");
    assert!(parts.iter().all(|part| part.state == live::PartState::Current), "{parts:?}");
    let mesh = client.mesh("B").expect("mesh via the W1 client");
    assert_eq!(mesh.key, key_of(st, "B"));
}

/// Envelope on every endpoint, also for errors (AG-SV-01).
fn check_not_found(s: &Service) {
    for path in ["/mesh/Nope", "/job/999999999", "/no/such/endpoint"] {
        let r = s.get(path);
        assert_eq!(r.code, 404, "{path}: {}", r.text());
        assert_eq!(r.envelope()["ok"], json!(false), "{path}");
    }
    let r = s.post("/build", &json!({"parts": ["Nope"], "wait_s": 5}));
    let v = r.envelope();
    assert_eq!(v["ok"], json!(false), "unknown part: {v}");
    assert!(r.text().contains("Nope"), "{v}");
}

/// §3.1 `/export`: `out/<part>.<ext>`; STL volume by an own parser, 3MF by `unzip -t`.
fn check_export(s: &Service, p: &Project) {
    s.post("/export", &json!({"parts": ["A", "C"], "format": "stl"})).ok();
    let (vol, lo, hi) = stl_volume_bbox(&std::fs::read(p.path("out/A.stl")).expect("out/A.stl"));
    // f32 vertices of a planar box: rel. error ~1e-7 per coordinate.
    assert_rel(vol, 2000.0, 1e-5, "STL volume of A");
    assert_box((lo, hi), A_BOX, "STL bbox of A");
    let (vol, ..) = stl_volume_bbox(&std::fs::read(p.path("out/C.stl")).expect("out/C.stl"));
    assert_rel(vol, 64.0, 1e-5, "STL volume of C");

    s.post("/export", &json!({"parts": ["A"], "format": "3mf"})).ok();
    let path = p.path("out/A.3mf");
    let t = Command::new("unzip").arg("-t").arg(&path).output().expect("unzip installed");
    assert!(t.status.success(), "unzip -t: {}", String::from_utf8_lossy(&t.stdout));
    let model = Command::new("unzip")
        .arg("-p")
        .arg(&path)
        .arg("3D/3dmodel.model")
        .output()
        .expect("unzip -p");
    let model = String::from_utf8_lossy(&model.stdout);
    assert_eq!(model.matches("<object ").count(), 1, "one object per part:\n{model}");

    s.post("/export", &json!({"parts": ["A"], "format": "step"})).ok();
    let step = std::fs::read_to_string(p.path("out/A.step")).expect("out/A.step");
    assert!(step.starts_with("ISO-10303-21;"), "STEP header: {}", &step[..step.len().min(80)]);

    let r = s.post("/export", &json!({"parts": ["A"], "format": "obj"}));
    assert_eq!(r.envelope()["ok"], json!(false), "unknown format is an error");
}

// ===========================================================================
// AG-SV-01: jobs
// ===========================================================================

#[test]
fn ag_sv_01_job_lifecycle_with_progress() {
    let p = Project::abcd("job");
    let s = Service::start(&p);
    let job = s.start_job(None);
    let mut last_done = 0;
    let deadline = Instant::now() + Duration::from_secs(60);
    let final_job = loop {
        let j = s.get(&format!("/job/{}", job_path(&job))).ok();
        assert_eq!(j["id"], job, "{j}");
        let state = j["state"].as_str().unwrap_or_default().to_owned();
        assert!(["queued", "running", "done"].contains(&state.as_str()), "{j}");
        let (done, total) = (j["progress"]["done"].as_u64(), j["progress"]["total"].as_u64());
        assert_eq!(total, Some(4), "four parts to build: {j}");
        let done = done.unwrap_or_else(|| panic!("progress.done: {j}"));
        assert!(done >= last_done && done <= 4, "progress {last_done} → {done}");
        last_done = done;
        if state == "done" {
            break j;
        }
        assert!(Instant::now() < deadline, "job not done: {j}");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(final_job["progress"]["done"], json!(4));
    let result = &final_job["result"];
    assert_eq!(built_cached(result).0, set(&["A", "B", "C", "D"]), "{final_job}");
    let st = s.status();
    let jobs = st["jobs"].as_array().cloned().unwrap_or_default();
    assert!(jobs.iter().any(|j| j["id"] == job && j["state"] == json!("done")), "{st}");
}

#[test]
fn ag_sv_01_cancel_stops_a_running_job() {
    let p = Project::new("cancel", "[limits]\ntime_s = 60\n[workers]\ncount = 1\n");
    p.write("parts/endless.rhai", ENDLESS).write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    let job = s.start_job(Some(&["Endless"]));
    std::thread::sleep(Duration::from_millis(500));
    s.post(&format!("/job/{}/cancel", job_path(&job)), &json!({})).ok();
    let (j, _) = s.wait_job(&job, Duration::from_secs(5));
    assert_eq!(j["state"], json!("cancelled"), "{j}");
    // With one worker, Normal only builds if the endless script was stopped.
    assert_normal_builds(&s);
    assert_eq!(s.status()["workers"]["busy"], json!(0));
    assert_ne!(states(&s.status())["Endless"], "current");
}

// ===========================================================================
// AG-SV-01: service lifecycle
// ===========================================================================

#[test]
fn ag_sv_01_shutdown_removes_service_json_and_stops_all_workers() {
    let p = Project::new("shutdown", "[workers]\ncount = 2\n");
    p.write("parts/normal.rhai", NORMAL);
    let mut s = Service::start(&p);
    assert_normal_builds(&s);
    let workers = s.workers();
    assert!(!workers.is_empty(), "workers stay warm after a build (§2.1)");
    let r = http(s.port, "POST", "/shutdown", Some(&json!({})), Duration::from_secs(5))
        .expect("/shutdown");
    r.ok();
    let child = s.child.as_mut().expect("own child");
    let exited =
        wait_until(Duration::from_secs(10), || child.try_wait().is_ok_and(|x| x.is_some()));
    assert!(exited, "serve did not exit after /shutdown");
    let status = child.wait().expect("exit status");
    assert!(status.success(), "serve exit status {status}");
    assert!(!p.path(".qoob3d/service.json").exists(), "service.json removed on exit");
    let gone = wait_until(Duration::from_secs(5), || workers.iter().all(|w| !alive(*w)));
    assert!(gone, "worker processes left behind: {workers:?}");
}

#[test]
fn ag_sv_01_one_service_per_folder() {
    let p = Project::new("single", "");
    p.write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    let before = service_json(&p.dir);
    let mut second = Command::new(BIN)
        .arg("serve")
        .arg(&p.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn second serve");
    let exited =
        wait_until(Duration::from_secs(10), || second.try_wait().is_ok_and(|x| x.is_some()));
    if !exited {
        second.kill().ok();
        second.wait().ok();
    }
    assert!(exited, "a second service for the same folder must not stay running");
    assert_eq!(service_json(&p.dir), before, "service.json still names the first service");
    s.status();
}

#[test]
fn ag_sv_01_stale_service_json_does_not_block_a_new_service() {
    let p = Project::new("stale", "");
    p.write("parts/normal.rhai", NORMAL);
    // A pid that has exited and a port nobody listens on.
    let mut dead = Command::new("true").spawn().expect("spawn true");
    let dead_pid = dead.id();
    dead.wait().expect("wait true");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("port")
        .port();
    p.write(".qoob3d/service.json", &format!("{{\"port\": {port}, \"pid\": {dead_pid}}}"));
    let s = Service::start(&p);
    assert_ne!(s.port, port);
    assert_normal_builds(&s);
}

// ===========================================================================
// AG-PJ-05 / §2.1: worker protocol
// ===========================================================================

fn send(stdin: &mut ChildStdin, request: &Value) {
    writeln!(stdin, "{request}").expect("write request");
    stdin.flush().expect("flush request");
}

#[test]
fn ag_pj_05_worker_answers_one_json_line_per_job_and_stays_warm() {
    let p = Project::abcd("worker");
    let mut w = Command::new(BIN)
        .arg("worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn qoob3d worker");
    let mut stdin = w.stdin.take().expect("stdin");
    let (tx, lines) = mpsc::channel();
    let out = BufReader::new(w.stdout.take().expect("stdout"));
    std::thread::spawn(move || {
        for line in out.lines().map_while(Result::ok) {
            tx.send(line).ok();
        }
    });
    let dir = p.dir.display().to_string();
    for (job, part) in [(1, "D"), (2, "A")] {
        send(&mut stdin, &json!({"job": job, "dir": dir, "part": part, "deps": {}}));
        let line = lines.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|e| {
            w.kill().ok();
            panic!("no answer to job {job}: {e}")
        });
        let v: Value =
            serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"));
        assert_eq!(v["job"], json!(job), "the answer names its job: {v}");
        assert!(w.try_wait().is_ok_and(|s| s.is_none()), "worker stays warm after job {job}");
    }
    drop(stdin);
    let exited = wait_until(Duration::from_secs(5), || w.try_wait().is_ok_and(|s| s.is_some()));
    if !exited {
        w.kill().ok();
    }
    w.wait().ok();
    assert!(exited, "the worker ends when its stdin closes");
    assert!(lines.recv_timeout(Duration::from_millis(200)).is_err(), "exactly one line per job");
}

// ===========================================================================
// AG-SV-02: CLI
// ===========================================================================

#[test]
fn ag_sv_02_cli_starts_the_service_on_demand_and_prints_envelopes() {
    let p = Project::abcd("cli");
    assert!(service_json(&p.dir).is_none());
    let t = Duration::from_secs(90);
    let out = cli(&p.dir, &["status"], t);
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    assert_eq!(out.envelope()["ok"], json!(true));
    let s = Service::attach(&p);
    let first = service_json(&p.dir);

    let all = set(&["A", "B", "C", "D"]);
    let out = cli(&p.dir, &["build", "--wait", "60"], t);
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    assert_eq!(built_cached(&out.envelope()["result"]), (all.clone(), set(&[])));
    let out = cli(&p.dir, &["build", "--wait", "60"], t);
    assert_eq!(built_cached(&out.envelope()["result"]), (set(&[]), all));
    let out = cli(&p.dir, &["build", "D", "--wait", "60"], t);
    assert_eq!(out.code, Some(0), "{}", out.stdout);
    assert!(out.envelope()["result"]["parts"]["D"].is_object());

    let out = cli(&p.dir, &["export", "A", "--format", "stl"], t);
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    out.envelope();
    let (vol, ..) = stl_volume_bbox(&std::fs::read(p.path("out/A.stl")).expect("out/A.stl"));
    assert_rel(vol, 2000.0, 1e-5, "CLI STL volume of A");

    let out = cli(&p.dir, &["values"], t);
    assert_eq!(out.code, Some(0), "{}", out.stdout);
    assert_eq!(out.envelope()["ok"], json!(true));
    assert!(out.stdout.contains("a_size") && out.stdout.contains("unused_width"), "{}", out.stdout);

    assert_eq!(service_json(&p.dir), first, "every CLI call used the same service");
    p.write("parts/bad.rhai", BAD);
    let out = cli(&p.dir, &["build", "--wait", "60"], t);
    assert_eq!(out.code, Some(1), "a failed part gives exit 1: {}", out.stdout);
    assert_eq!(out.envelope()["result"]["parts"]["Bad"]["state"], json!("failed"));
    drop(s);
}
