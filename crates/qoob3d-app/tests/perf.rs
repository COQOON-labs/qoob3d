//! Performance package P1, service side (docs/requirements/agent.md §5)
//! against the real `qoob3d` binary: worker recycling (AG-PF-03), cache
//! eviction (AG-PF-04), configurable tessellation (AG-PF-05) and
//! `qoob3d bench` (AG-PF-06). Envelope, `/status`, `/build` and the cache
//! layout are those of §2.1 and §3.1; the qmesh layout is WA-QM-01
//! (docs/requirements/project-app.md §3), read here by an own parser.
//!
//! Test projects use boxes and one cylinder, so volumes are derived by hand
//! (`VOL` = 1e-6 relative as in `tests/service.rs`) and triangle counts have
//! analytic bounds from the chord geometry of a circle.
//!
//! Processes: every test owns its service through [`Service`], whose drop
//! sends `/shutdown` and kills the service and its workers if they linger
//! (same pattern as `tests/service.rs`; the helpers are repeated here because
//! `tests/common/` belongs to the fake service of the W1 tests).
//!
//! Names decided by this file where the requirement leaves them open (see the
//! report): `/status` → `workers.recycled`; bench → `result.parts.<name>` with
//! `build_s`, `peak_memory_mb`, `triangles`, `mesh_bytes`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_qoob3d");
const VOL: f64 = 1e-6;
/// f32 rounding of coordinates up to 20 mm is < 2e-6 mm.
const F32_POS: f64 = 1e-4;
/// Upper bound for a service to come up (WA-OP-02 allows 20 s).
const START: Duration = Duration::from_secs(20);
/// `wait_s` for synchronous builds.
const WAIT_S: f64 = 60.0;

// ===========================================================================
// Test projects
// ===========================================================================

/// `Name` = box(x, 10, 10) → 100 x mm³.
fn box_part(name: &str, x: u32) -> String {
    format!("part(\"{name}\", #{{}}, |deps| box({x}, 10, 10));\n")
}

/// Cyl = cylinder(40, 10): r = 20, V = π · 400 · 10 = 4000π.
const CYL: &str = "part(\"Cyl\", #{}, |deps| cylinder(40, 10));\n";
const CYL_VOLUME: f64 = 4000.0 * std::f64::consts::PI;
const CYL_R: f64 = 20.0;

/// Keeps 20 000 unit boxes alive until the closure ends and returns
/// box(10, 10, 10) = 1000 mm³. The B-reps (about 16 kB each) leave the
/// worker's resident memory far above 100 MB after the job; measured on
/// Linux: 325 MB after the job, 1.2 s in a debug build. [`heavy_rss_mb`]
/// measures it again in every test run, so a platform whose allocator
/// behaves differently fails as a test premise, not as a product bug.
const HEAVY: &str = "\
part(\"Heavy\", #{}, |deps| {
    let keep = [];
    for i in 0..20000 { keep.push(box(1, 1, 1)); }
    box(10, 10, 10)
});
";

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A temporary project `<root>/project/` with `qoob3d.toml`; removed on drop
/// (after the [`Service`] guards declared later have stopped).
struct Project {
    root: PathBuf,
    dir: PathBuf,
    name: String,
}

impl Project {
    /// Project named `name`; `extra` is appended to `qoob3d.toml`.
    fn new(name: &str, extra: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("qoob3d-perf-{name}-{}-{n}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let dir = root.join("project");
        std::fs::create_dir_all(dir.join("parts")).expect("create project");
        let p = Self { root, dir, name: name.to_owned() };
        p.config(extra);
        p
    }

    /// Rewrites `qoob3d.toml` with `extra` after the `[project]` table.
    fn config(&self, extra: &str) -> &Self {
        let name = &self.name;
        self.write("qoob3d.toml", &format!("[project]\nname = \"{name}\"\nunits = \"mm\"\n{extra}"))
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

    /// The JSON envelope `{ok, duration_s, warnings, result}` (project-app.md §1).
    fn envelope(&self) -> Value {
        let v: Value = serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!("no JSON envelope ({e}), HTTP {}: {}", self.code, self.text())
        });
        assert_envelope(&v);
        v
    }

    /// `result` of an envelope with `ok: true`.
    fn ok(&self) -> Value {
        let v = self.envelope();
        assert_eq!(v["ok"], json!(true), "HTTP {}: {v}", self.code);
        v["result"].clone()
    }
}

/// Field by field: `ok`, `duration_s`, `warnings`, `result`, and `error`
/// `{kind, message}` when not ok.
fn assert_envelope(v: &Value) {
    let ok = v["ok"].as_bool().unwrap_or_else(|| panic!("envelope without bool ok: {v}"));
    let d = v["duration_s"].as_f64().unwrap_or_else(|| panic!("envelope without duration_s: {v}"));
    assert!(d >= 0.0 && d.is_finite(), "duration_s {d}");
    assert!(v["warnings"].is_array(), "envelope without warnings array: {v}");
    assert!(v.get("result").is_some(), "envelope without result: {v}");
    if !ok {
        assert!(v["error"]["kind"].is_string(), "error without kind: {v}");
        assert!(v["error"]["message"].is_string(), "error without message: {v}");
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

/// Resident set size in MB (`ps -o rss=` prints KiB on Linux and macOS).
fn rss_mb(pid: u32) -> f64 {
    let out =
        Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output().expect("run ps");
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let kib: f64 = text.parse().unwrap_or_else(|e| panic!("rss of {pid}: {text:?} ({e})"));
    kib * 1024.0 / 1e6
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

/// The token from `.qoob3d/service.json` that every request carries in the
/// header `X-Qoob3d-Token` (AG-HD-03); `None` if the file has none.
fn service_token(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(".qoob3d/service.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v["token"].as_str().map(str::to_owned)
}

/// Live `qoob3d worker` children of process `parent`.
fn workers_of(parent: u32) -> Vec<u32> {
    processes()
        .into_iter()
        .filter(|(pid, ppid, args)| *ppid == parent && args.contains("worker") && alive(*pid))
        .map(|(pid, ..)| pid)
        .collect()
}

/// A running `qoob3d serve` for a project. Drop shuts it down and kills
/// whatever is left (service and workers), so no test leaks processes.
struct Service {
    port: u16,
    pid: u32,
    /// From `service.json`, sent with every request (AG-HD-03).
    token: Option<String>,
    child: Child,
    log: PathBuf,
    /// Every worker pid ever seen, killed on drop if still alive.
    seen_workers: std::cell::RefCell<BTreeSet<u32>>,
}

impl Service {
    /// Spawns `qoob3d serve <DIR>` and waits until it answers `/status`.
    fn start(p: &Project) -> Self {
        let log = p.root.join("serve.log");
        let err = std::fs::File::create(&log).expect("create serve.log");
        let mut child = Command::new(BIN)
            .arg("serve")
            .arg(&p.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(err)
            .spawn()
            .expect("spawn qoob3d serve");
        let deadline = Instant::now() + START;
        loop {
            if let Some((port, pid)) = service_json(&p.dir) {
                let token = service_token(&p.dir);
                let wait = Duration::from_secs(1);
                let probe = http(port, token.as_deref(), "GET", "/status", None, wait);
                if pid == child.id() && probe.is_ok_and(|r| r.code == 200) {
                    let seen = std::cell::RefCell::new(BTreeSet::new());
                    return Self { port, pid, token, child, log, seen_workers: seen };
                }
            }
            if let Ok(Some(status)) = child.try_wait() {
                let log = std::fs::read_to_string(&log).unwrap_or_default();
                panic!("qoob3d serve exited early with {status}:\n{log}");
            }
            if Instant::now() >= deadline {
                child.kill().ok();
                child.wait().ok();
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
        http(self.port, self.token.as_deref(), method, path, body, Duration::from_secs(90))
            .unwrap_or_else(|e| panic!("{method} {path}: {e}\nservice log:\n{}", self.log()))
    }

    fn status(&self) -> Value {
        let r = self.request("GET", "/status", None);
        assert_eq!(r.code, 200);
        r.ok()
    }

    /// `POST /build` with `wait_s` = [`WAIT_S`]; the build result (§3.1).
    fn build(&self, parts: Option<&[&str]>) -> Value {
        let r = self.request("POST", "/build", Some(&json!({"parts": parts, "wait_s": WAIT_S})));
        let v = r.envelope();
        let result = v["result"].clone();
        assert!(result["parts"].is_object(), "no build result: {v}\nlog:\n{}", self.log());
        result
    }

    /// Builds one part and checks its state (`built` or `cached`) and volume.
    fn build_one(&self, part: &str, state: &str, volume: f64) {
        let r = self.build(Some(&[part]));
        let p = &r["parts"][part];
        assert_eq!(p["state"], json!(state), "{part}: {r}\nlog:\n{}", self.log());
        let v = p["volume"].as_f64().unwrap_or(f64::NAN);
        assert!((v - volume).abs() / volume < VOL, "volume of {part}: got {v}, expected {volume}");
    }

    /// Key of `part` in `/status` (64 hex digits, §2.1).
    fn key(&self, part: &str) -> String {
        let st = self.status();
        let parts = st["parts"].as_array().unwrap_or_else(|| panic!("status without parts: {st}"));
        let p = parts
            .iter()
            .find(|p| p["name"] == json!(part))
            .unwrap_or_else(|| panic!("no part {part} in {st}"));
        let key = p["key"].as_str().unwrap_or_else(|| panic!("{part} has no key: {p}")).to_owned();
        assert!(key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()), "key {key:?}");
        key
    }

    /// `workers.recycled` of `/status` (AG-PF-03): a count.
    fn recycled(&self) -> u64 {
        let st = self.status();
        st["workers"]["recycled"]
            .as_u64()
            .unwrap_or_else(|| panic!("/status without the count workers.recycled: {st}"))
    }

    /// Live `qoob3d worker` children of the service.
    fn workers(&self) -> Vec<u32> {
        let w = workers_of(self.pid);
        self.seen_workers.borrow_mut().extend(&w);
        w
    }

    /// `POST /shutdown`; waits for the service to exit.
    fn shutdown(&mut self) -> bool {
        self.workers();
        let (token, wait) = (self.token.as_deref(), Duration::from_secs(2));
        let sent = http(self.port, token, "POST", "/shutdown", Some(&json!({})), wait);
        let child = &mut self.child;
        let gone =
            wait_until(Duration::from_secs(10), || child.try_wait().is_ok_and(|s| s.is_some()));
        sent.is_ok() && gone
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if !self.shutdown() {
            kill(self.pid, "KILL");
        }
        self.child.kill().ok();
        self.child.wait().ok();
        for pid in self.seen_workers.borrow().iter() {
            if alive(*pid) {
                kill(*pid, "KILL");
            }
        }
    }
}

// ===========================================================================
// AG-PF-03: worker recycling
// ===========================================================================

/// Test premise for AG-PF-03: resident memory (MB) of a bare `qoob3d worker`
/// (§2.1 worker protocol) after it built [`HEAVY`], measured once per test
/// run in an own project so the service projects keep an empty cache.
fn heavy_rss_mb() -> f64 {
    static RSS: OnceLock<f64> = OnceLock::new();
    *RSS.get_or_init(|| {
        let p = Project::new("premise", "");
        p.write("parts/heavy.rhai", HEAVY);
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
        let request =
            json!({"job": 1, "dir": p.dir.display().to_string(), "part": "Heavy", "deps": {}});
        writeln!(stdin, "{request}").expect("write request");
        stdin.flush().expect("flush request");
        // Answers start with `{"job":`; other stdout lines come from OCCT.
        let deadline = Instant::now() + Duration::from_secs(60);
        let answered = loop {
            match lines.recv_timeout(Duration::from_secs(1)) {
                Ok(line) if line.starts_with("{\"job\":") => break true,
                Ok(_) => {}
                Err(_) if Instant::now() >= deadline => break false,
                Err(_) => {}
            }
        };
        // Give the allocator a moment to return what it is going to return.
        std::thread::sleep(Duration::from_millis(300));
        let rss = if answered { rss_mb(w.id()) } else { f64::NAN };
        drop(stdin);
        let exited = wait_until(Duration::from_secs(5), || w.try_wait().is_ok_and(|s| s.is_some()));
        if !exited {
            w.kill().ok();
        }
        w.wait().ok();
        assert!(answered, "test premise: the bare worker did not answer the Heavy job");
        rss
    })
}

#[test]
fn ag_pf_03_worker_above_recycle_mb_is_replaced_before_the_next_job() {
    // Premise: Heavy leaves the worker well above recycle_mb = 100.
    let after_heavy = heavy_rss_mb();
    assert!(
        after_heavy > 150.0,
        "test premise: the worker holds only {after_heavy:.0} MB after Heavy, need > 150"
    );
    let p = Project::new("recycle", "[workers]\ncount = 1\nrecycle_mb = 100\n");
    p.write("parts/s1.rhai", &box_part("S1", 10))
        .write("parts/s2.rhai", &box_part("S2", 20))
        .write("parts/s3.rhai", &box_part("S3", 30))
        .write("parts/heavy.rhai", HEAVY);
    let s = Service::start(&p);
    assert_eq!(s.recycled(), 0, "nothing recycled at start");

    // A small part (idle worker: 35 MB, §5) stays below 100 MB: the worker
    // stays warm (§2.1) and is not recycled.
    s.build_one("S1", "built", 1000.0);
    let before = s.workers();
    assert_eq!(before.len(), 1, "one warm worker: {before:?}");
    assert_eq!(s.recycled(), 0, "a small part does not recycle the worker");

    // The heavy job itself succeeds; recycling is no error.
    s.build_one("Heavy", "built", 1000.0);
    // "Before the next job": after the next job the replacement has happened
    // for certain, whether the service replaces eagerly or on demand.
    s.build_one("S2", "built", 2000.0);
    assert_eq!(s.recycled(), 1, "exactly the worker that built Heavy was recycled");
    let after = s.workers();
    assert_eq!(after.len(), 1, "one worker again: {after:?}");
    assert_ne!(after, before, "the worker process was replaced");
    let old = before[0];
    assert!(wait_until(Duration::from_secs(5), || !alive(old)), "old worker {old} still runs");
    let st = s.status();
    assert_eq!(st["workers"]["count"], json!(1), "{st}");

    // The fresh worker is small again: no further recycling.
    s.build_one("S3", "built", 3000.0);
    assert_eq!(s.recycled(), 1, "the fresh worker is not recycled for a small part");
    assert_eq!(s.workers(), after, "the fresh worker stays warm");
}

#[test]
fn ag_pf_03_default_recycle_limit_is_512_mb() {
    // Premise: Heavy leaves the worker between the 100 MB of the test above
    // and the default of 512 MB (measured 325 MB), so the same part that is
    // recycled there must not be recycled here.
    let after_heavy = heavy_rss_mb();
    assert!(
        after_heavy > 150.0 && after_heavy < 450.0,
        "test premise: the worker holds {after_heavy:.0} MB after Heavy, need 150..450"
    );
    let p = Project::new("recycle-default", "[workers]\ncount = 1\n");
    p.write("parts/s1.rhai", &box_part("S1", 10))
        .write("parts/s2.rhai", &box_part("S2", 20))
        .write("parts/heavy.rhai", HEAVY);
    let s = Service::start(&p);
    assert_eq!(s.recycled(), 0, "nothing recycled at start");
    s.build_one("S1", "built", 1000.0);
    let before = s.workers();
    assert_eq!(before.len(), 1, "one warm worker: {before:?}");
    s.build_one("Heavy", "built", 1000.0);
    s.build_one("S2", "built", 2000.0);
    assert_eq!(s.recycled(), 0, "below the default of 512 MB nothing is recycled");
    assert_eq!(s.workers(), before, "the same worker served all jobs");
}

// ===========================================================================
// AG-PF-04: cache eviction
// ===========================================================================

/// Keys in `.qoob3d/cache/<part>/` → the extensions present for each key.
fn cache_entries(p: &Project, part: &str) -> BTreeMap<String, BTreeSet<String>> {
    let dir = p.path(&format!(".qoob3d/cache/{part}"));
    let mut entries: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let Ok(read) = std::fs::read_dir(&dir) else {
        return entries;
    };
    for file in read.map_while(Result::ok) {
        let path = file.path();
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
        if ["brep", "qmsh", "json"].contains(&ext.as_str()) {
            entries.entry(stem).or_default().insert(ext);
        }
    }
    entries
}

/// The cache of `part` holds exactly the complete entries `keys`.
fn assert_cache(p: &Project, part: &str, keys: &[String], what: &str) {
    let entries = cache_entries(p, part);
    let all: BTreeSet<String> = ["brep", "json", "qmsh"].iter().map(|e| (*e).to_owned()).collect();
    for (key, exts) in &entries {
        assert_eq!(exts, &all, "{what}: entry {key} of {part} is incomplete");
    }
    let have: BTreeSet<&String> = entries.keys().collect();
    let want: BTreeSet<&String> = keys.iter().collect();
    assert_eq!(have, want, "{what}: cache entries of {part}");
}

/// Builds part P in `builds` different sizes with `[cache] keep` as in
/// `config` and checks after every build that exactly the newest
/// `min(n, keep)` entries of P remain, complete, the current one among them,
/// and that the cache of the unchanged part Q is left alone. Returns the
/// project, the service and the keys of P in build order.
fn build_versions(
    name: &str,
    config: &str,
    keep: usize,
    builds: u32,
) -> (Project, Service, Vec<String>) {
    let p = Project::new(name, &format!("[workers]\ncount = 1\n{config}"));
    p.write("parts/q.rhai", &box_part("Q", 7));
    p.write("parts/p.rhai", &box_part("P", 10));
    let s = Service::start(&p);
    s.build_one("Q", "built", 700.0);
    let q_key = s.key("Q");
    let mut keys: Vec<String> = Vec::new();
    for n in 0..builds {
        // P = box(10 + n, 10, 10) → 100 (10 + n) mm³; every size is a new key.
        p.write("parts/p.rhai", &box_part("P", 10 + n));
        s.build_one("P", "built", 100.0 * f64::from(10 + n));
        let key = s.key("P");
        assert!(!keys.contains(&key), "build {n}: a changed part file gives a new key");
        keys.push(key);
        // Newest by modification time = the last `keep` builds.
        let newest = &keys[keys.len().saturating_sub(keep)..];
        assert_cache(&p, "P", newest, &format!("{name}, after build {n}"));
        assert_cache(&p, "Q", std::slice::from_ref(&q_key), &format!("{name}, after build {n}"));
    }
    (p, s, keys)
}

#[test]
fn ag_pf_04_keep_2_leaves_the_current_and_the_previous_entry() {
    // The part of the task: built, then changed 4 times → 5 keys, 2 remain.
    let (p, s, keys) = build_versions("keep2", "[cache]\nkeep = 2\n", 2, 5);
    assert_eq!(cache_entries(&p, "P").len(), 2);
    // The survivors are usable: going back to the previous size is a cache
    // hit (its key is its file content again), not a rebuild.
    p.write("parts/p.rhai", &box_part("P", 13));
    s.build_one("P", "cached", 1300.0);
    assert_eq!(s.key("P"), keys[3]);
    assert_cache(&p, "P", &keys[3..], "after a cache hit");
}

#[test]
fn ag_pf_04_default_keeps_5_entries() {
    let (p, _s, _) = build_versions("keep-default", "", 5, 7);
    assert_eq!(cache_entries(&p, "P").len(), 5);
}

#[test]
fn ag_pf_04_keep_1_leaves_only_the_current_entry_and_a_failed_build_evicts_nothing() {
    let (p, s, keys) = build_versions("keep1", "[cache]\nkeep = 1\n", 1, 3);
    // Eviction runs after a *successful* build: a script error (box with a
    // zero size, AG-SC-04) leaves the last good entry in place.
    p.write("parts/p.rhai", &box_part("P", 0));
    let r = s.build(Some(&["P"]));
    assert_eq!(r["parts"]["P"]["state"], json!("failed"), "{r}");
    assert_cache(&p, "P", &keys[2..], "after a failed build");
}

// ===========================================================================
// AG-PF-05: tessellation quality
// ===========================================================================

/// Triangles of a qmesh, parsed here from the layout of WA-QM-01
/// (little endian): `b"QMSH"`, u32 version = 1, u32 V, T, E, P, positions
/// f32×3×V, normals f32×3×V, indices u32×3×T, edge points f32×3×E, starts
/// u32×(P+1).
struct Mesh {
    positions: Vec<[f64; 3]>,
    triangles: Vec<[usize; 3]>,
}

fn read_qmesh(bytes: &[u8]) -> Mesh {
    assert!(bytes.len() >= 24, "qmesh shorter than its header: {} bytes", bytes.len());
    assert_eq!(&bytes[..4], b"QMSH", "qmesh magic");
    let word = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().expect("4 bytes")) as usize;
    assert_eq!(word(4), 1, "qmesh version");
    let (v, t, e, p) = (word(8), word(12), word(16), word(20));
    let size = 24 + 24 * v + 12 * t + 12 * e + 4 * (p + 1);
    assert_eq!(bytes.len(), size, "qmesh size for V = {v}, T = {t}, E = {e}, P = {p}");
    let float =
        |o: usize| f64::from(f32::from_le_bytes(bytes[o..o + 4].try_into().expect("4 bytes")));
    let positions =
        (0..v).map(|i| [0, 1, 2].map(|c| float(24 + 12 * i + 4 * c))).collect::<Vec<_>>();
    let base = 24 + 24 * v;
    let triangles =
        (0..t).map(|i| [0, 1, 2].map(|c| word(base + 12 * i + 4 * c))).collect::<Vec<_>>();
    assert!(triangles.iter().flatten().all(|i| *i < v), "qmesh index out of range");
    Mesh { positions, triangles }
}

/// The cached mesh of `part` under its current key (§2.1).
fn cached_mesh(p: &Project, s: &Service, part: &str) -> Mesh {
    let path = p.path(&format!(".qoob3d/cache/{part}/{}.qmsh", s.key(part)));
    read_qmesh(&std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

/// Largest distance (mm) between the mesh of [`CYL`] and its cylinder wall.
/// Vertices lie on the surface (r = 20), so the mesh is farthest inside at
/// the middle of an edge: an edge spanning the angle φ has its midpoint at
/// radius r cos(φ/2), whatever its direction along Z. Cap triangles (all
/// three corners at the same z) are planar and exact; their edges may cross
/// the disc and are skipped.
fn wall_sag(mesh: &Mesh) -> f64 {
    let mut sag: f64 = 0.0;
    let mut wall_triangles = 0;
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| mesh.positions[i]);
        for q in [a, b, c] {
            let r = q[0].hypot(q[1]);
            assert!(r <= CYL_R + F32_POS, "vertex outside the cylinder: r = {r}");
            assert!(q[2] > -F32_POS && q[2] < 10.0 + F32_POS, "vertex outside z 0..10: {q:?}");
        }
        if (a[2] - b[2]).abs() < F32_POS && (b[2] - c[2]).abs() < F32_POS {
            continue;
        }
        wall_triangles += 1;
        for (m, n) in [(a, b), (b, c), (c, a)] {
            let on_wall = |q: [f64; 3]| (q[0].hypot(q[1]) - CYL_R).abs() < F32_POS;
            assert!(on_wall(m) && on_wall(n), "wall triangle with a corner off the wall");
            let mid = f64::midpoint(m[0], n[0]).hypot(f64::midpoint(m[1], n[1]));
            sag = sag.max(CYL_R - mid);
        }
    }
    assert!(wall_triangles > 0, "no wall triangles");
    sag
}

/// Restarts the service of `p` with `[mesh]` settings `mesh`, builds Cyl and
/// returns the build state, the key and the cached mesh. The service is
/// restarted because the requirement does not say that a running service
/// follows changes of `qoob3d.toml`.
fn build_cyl(p: &Project, mesh: &str) -> (String, String, Mesh) {
    p.config(&format!("[workers]\ncount = 1\n{mesh}"));
    let s = Service::start(p);
    let r = s.build(Some(&["Cyl"]));
    let part = &r["parts"]["Cyl"];
    let state = part["state"].as_str().unwrap_or_else(|| panic!("no state: {r}")).to_owned();
    // The mesh settings never change the exact shape.
    let v = part["volume"].as_f64().unwrap_or(f64::NAN);
    assert!((v - CYL_VOLUME).abs() / CYL_VOLUME < VOL, "[mesh] {mesh:?}: volume {v}");
    let key = s.key("Cyl");
    let mesh = cached_mesh(p, &s, "Cyl");
    // `/mesh/Cyl` serves that mesh.
    let served = s.request("GET", "/mesh/Cyl", None);
    assert_eq!(served.code, 200, "{}", served.text());
    assert_eq!(served.header("x-qoob3d-key"), Some(key.clone()));
    assert_eq!(read_qmesh(&served.body).triangles.len(), mesh.triangles.len());
    (state, key, mesh)
}

#[test]
fn ag_pf_05_deflection_is_configurable_and_part_of_the_key() {
    let p = Project::new("deflection", "");
    p.write("parts/cyl.rhai", CYL);

    // Defaults: deflection 0.01 mm, angle 20°. A chord spanning φ sags
    // r (1 − cos(φ/2)) ≤ 0.01 → φ ≤ 2 acos(1 − 0.01/20) = 0.0632 rad: at
    // least 100 segments around, so at least 200 wall triangles.
    let (state, default_key, fine) = build_cyl(&p, "");
    assert_eq!(state, "built");
    let fine_sag = wall_sag(&fine);
    assert!(fine_sag <= 0.01 + F32_POS, "default deflection 0.01: sag {fine_sag}");
    assert!(fine.triangles.len() >= 200, "default mesh has {} triangles", fine.triangles.len());

    // The defaults written out are the same tessellation.
    let (_, _, explicit) = build_cyl(&p, "[mesh]\ndeflection_mm = 0.01\nangle_deg = 20\n");
    assert_eq!(explicit.triangles.len(), fine.triangles.len(), "explicit defaults");

    // Deflection 0.2: φ ≤ 2 acos(1 − 0.2/20) = 0.283 rad = 16.2° (finer than
    // the 20° of the angle), 23 segments instead of 100: about a quarter of
    // the triangles. "Clearly fewer" is asserted as less than half.
    let (state, coarse_key, coarse) = build_cyl(&p, "[mesh]\ndeflection_mm = 0.2\n");
    assert_eq!(state, "built", "a changed deflection rebuilds the part");
    assert_ne!(coarse_key, default_key, "deflection_mm is part of the cache key");
    let coarse_sag = wall_sag(&coarse);
    assert!(coarse_sag <= 0.2 + F32_POS, "deflection 0.2: sag {coarse_sag}");
    assert!(
        2 * coarse.triangles.len() < fine.triangles.len(),
        "deflection 0.2 gives {} triangles, 0.01 gives {}",
        coarse.triangles.len(),
        fine.triangles.len()
    );
    // The coarse mesh uses its allowance: it is not the 0.01 mesh again.
    assert!(coarse_sag > 0.01 + F32_POS, "deflection 0.2 was not applied: sag {coarse_sag}");

    // Unchanged settings: same key, served from the cache.
    let (state, key, again) = build_cyl(&p, "[mesh]\ndeflection_mm = 0.2\n");
    assert_eq!((state.as_str(), &key), ("cached", &coarse_key));
    assert_eq!(again.triangles.len(), coarse.triangles.len());

    // Back to the defaults: the first key again (the key is a function of
    // the settings, not of their history).
    let (_, key, _) = build_cyl(&p, "");
    assert_eq!(key, default_key);
}

#[test]
fn ag_pf_05_angle_is_configurable_and_part_of_the_key() {
    let p = Project::new("angle", "");
    p.write("parts/cyl.rhai", CYL);
    // Deflection 1 mm alone would allow φ = 2 acos(1 − 1/20) = 36.4°
    // (10 segments), so the angle decides: 20° → at least 18 segments,
    // 5° → at least 72. Four times the segments; asserted as at least twice
    // the triangles.
    let (state, key_20, mesh_20) = build_cyl(&p, "[mesh]\ndeflection_mm = 1.0\nangle_deg = 20\n");
    assert_eq!(state, "built");
    let (state, key_5, mesh_5) = build_cyl(&p, "[mesh]\ndeflection_mm = 1.0\nangle_deg = 5\n");
    assert_eq!(state, "built", "a changed angle rebuilds the part");
    assert_ne!(key_5, key_20, "angle_deg is part of the cache key");
    assert!(
        mesh_5.triangles.len() >= 2 * mesh_20.triangles.len(),
        "angle 5° gives {} triangles, 20° gives {}",
        mesh_5.triangles.len(),
        mesh_20.triangles.len()
    );
    for (mesh, what) in [(&mesh_20, "20°"), (&mesh_5, "5°")] {
        let sag = wall_sag(mesh);
        assert!(sag <= 1.0 + F32_POS, "deflection 1.0, angle {what}: sag {sag}");
    }
    // The default angle is 20°: leaving it out is the same tessellation.
    let (_, _, default_angle) = build_cyl(&p, "[mesh]\ndeflection_mm = 1.0\n");
    assert_eq!(default_angle.triangles.len(), mesh_20.triangles.len(), "default angle_deg");
}

// ===========================================================================
// AG-PF-06: bench
// ===========================================================================

struct CliOut {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl CliOut {
    fn envelope(&self) -> Value {
        let v = serde_json::from_str(self.stdout.trim()).unwrap_or_else(|e| {
            panic!("stdout is no JSON envelope ({e}):\n{}\nstderr:\n{}", self.stdout, self.stderr)
        });
        assert_envelope(&v);
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

/// `qoob3d <args>` with working directory `cwd`, killed after `timeout`.
fn cli(cwd: &Path, args: &[&str], timeout: Duration) -> CliOut {
    let mut child = Command::new(BIN)
        .args(args)
        .current_dir(cwd)
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
    // Whatever bench starts must not hold the CLI's pipes open.
    let pipe = Duration::from_secs(3);
    let stdout = out.recv_timeout(pipe).expect("stdout closed: nothing may inherit it");
    let stderr = err.recv_timeout(pipe).expect("stderr closed: nothing may inherit it");
    CliOut { code, stdout, stderr }
}

/// Stops a service that a CLI call may have started for `dir` (the
/// requirement does not say whether bench uses one), with its workers.
struct Leftover(PathBuf);

impl Drop for Leftover {
    fn drop(&mut self) {
        let Some((port, pid)) = service_json(&self.0) else {
            return;
        };
        let workers = workers_of(pid);
        let (token, wait) = (service_token(&self.0), Duration::from_secs(2));
        http(port, token.as_deref(), "POST", "/shutdown", Some(&json!({})), wait).ok();
        if !wait_until(Duration::from_secs(10), || !alive(pid)) {
            kill(pid, "KILL");
        }
        for w in workers {
            if alive(w) {
                kill(w, "KILL");
            }
        }
    }
}

/// `result.parts.<name>` of a bench envelope with the four measures checked:
/// returns `(triangles, mesh_bytes)`.
fn bench_part(result: &Value, name: &str) -> (u64, u64) {
    let part = &result["parts"][name];
    assert!(part.is_object(), "bench result without parts.{name}: {result}");
    // One build of a primitive: more than nothing, far below the 60 s limit.
    let build_s = part["build_s"].as_f64().unwrap_or_else(|| panic!("{name}.build_s: {part}"));
    assert!(build_s > 0.0 && build_s < 60.0, "{name}.build_s = {build_s}");
    // A process with OCCT loaded holds some MB (§5: 35 MB idle in a debug
    // build) and cannot exceed the default limit of 4096 MB (AG-PJ-01).
    let peak =
        part["peak_memory_mb"].as_f64().unwrap_or_else(|| panic!("{name}.peak_memory_mb: {part}"));
    assert!((5.0..=4096.0).contains(&peak), "{name}.peak_memory_mb = {peak}");
    let triangles =
        part["triangles"].as_u64().unwrap_or_else(|| panic!("{name}.triangles: {part}"));
    let bytes = part["mesh_bytes"].as_u64().unwrap_or_else(|| panic!("{name}.mesh_bytes: {part}"));
    // WA-QM-01: 24 bytes header, 24 per vertex, 12 per triangle, plus edges.
    // A closed triangle mesh has V ≥ T/2 + 2 (Euler), so the mesh takes at
    // least 24 + 12 T + 24 (T/2 + 2) = 72 + 24 T bytes.
    assert!(bytes >= 72 + 24 * triangles, "{name}: {bytes} mesh bytes for {triangles} triangles");
    (triangles, bytes)
}

fn bench_project(name: &str) -> Project {
    let p = Project::new(name, "[workers]\ncount = 2\n");
    // A = box(20, 10, 10), B = A moved (dependency), Cyl = cylinder(40, 10).
    p.write("parts/a.rhai", &box_part("A", 20))
        .write(
            "parts/b.rhai",
            "part(\"B\", #{ depends: [\"A\"] }, |deps| deps[\"A\"].move(30, 0, 0));\n",
        )
        .write("parts/cyl.rhai", CYL);
    p
}

#[test]
fn ag_pf_06_bench_prints_the_measures_per_part_as_envelope() {
    let p = bench_project("bench");
    let _leftover = Leftover(p.dir.clone());
    let dir = p.dir.display().to_string();
    // The working directory is not the project: `<DIR>` decides.
    let run = || {
        let out = cli(&p.root, &["bench", &dir], Duration::from_secs(120));
        assert_eq!(out.code, Some(0), "stdout:\n{}\nstderr:\n{}", out.stdout, out.stderr);
        let env = out.envelope();
        assert_eq!(env["ok"], json!(true), "{env}");
        env["result"].clone()
    };
    let result = run();
    let names: BTreeSet<&str> = result["parts"]
        .as_object()
        .unwrap_or_else(|| panic!("bench result without the map parts: {result}"))
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, BTreeSet::from(["A", "B", "Cyl"]), "all parts of the project");

    // A box has six planar rectangles: 12 triangles, and its mesh stays
    // small (flat shaded 24 vertices, 12 straight edges: about 1 kB).
    for name in ["A", "B"] {
        let (triangles, bytes) = bench_part(&result, name);
        assert_eq!(triangles, 12, "triangles of the box {name}");
        assert!(bytes <= 4096, "mesh of the box {name}: {bytes} bytes");
    }
    // Cylinder r = 20 at the default deflection 0.01 mm (AG-PF-05): at least
    // N = 100 segments around (see ag_pf_05_deflection_…), so at least 2 N
    // wall triangles and N − 2 per cap: 4 N − 4 = 396.
    let (cyl_triangles, cyl_bytes) = bench_part(&result, "Cyl");
    assert!(cyl_triangles >= 396, "triangles of the cylinder: {cyl_triangles}");

    // If bench leaves its builds in the project cache, the numbers describe
    // those files.
    let cache = cache_entries(&p, "Cyl");
    if let Some(key) = cache.keys().next().filter(|_| cache.len() == 1) {
        let bytes = std::fs::read(p.path(&format!(".qoob3d/cache/Cyl/{key}.qmsh")))
            .expect("cached mesh of Cyl");
        assert_eq!(bytes.len() as u64, cyl_bytes, "mesh_bytes vs the cached qmesh");
        assert_eq!(read_qmesh(&bytes).triangles.len() as u64, cyl_triangles);
    }

    // Bench builds uncached: with the cache of the first run in place, a
    // second run measures real builds again and the same meshes.
    let second = run();
    for name in ["A", "B", "Cyl"] {
        assert_eq!(bench_part(&second, name), bench_part(&result, name), "second bench of {name}");
    }
}

#[test]
fn ag_pf_06_bench_of_a_folder_without_a_project_fails_with_an_envelope() {
    let p = Project::new("bench-empty", "");
    let empty = p.root.join("no-project");
    std::fs::create_dir_all(&empty).expect("create folder");
    let _leftover = Leftover(empty.clone());
    let out = cli(&p.root, &["bench", &empty.display().to_string()], Duration::from_secs(60));
    assert_eq!(out.code, Some(1), "stdout:\n{}\nstderr:\n{}", out.stdout, out.stderr);
    assert_eq!(out.envelope()["ok"], json!(false), "{}", out.stdout);
}
