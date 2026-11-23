//! Project service, worker processes, cache and CLI
//! against the real `qoob3d` binary (docs/requirements/agent.md §2, §2.1,
//! §3.1, §4; envelope and qmesh from docs/requirements/project-app.md;
//! ADR-0053), and the service hardening (§6.1: AG-HD-01, 02, 03, 06,
//! 10; ADR-0055). Every test names its requirement IDs.
//!
//! Test projects use only simple primitives (`box`, `cylinder`, `move`), so
//! every volume and bounding box is derived by hand next to the assertion.
//! Volumes of boxes and cylinders are exact in OCCT up to rounding; `VOL` =
//! 1e-6 relative is the same margin as the R1 kernel tests. Meshes are f32:
//! coordinates up to 40 mm round to < 4e-6 mm, so `F32_POS` = 1e-4 mm.
//!
//! Processes: every test owns its service through `support::Service`, whose
//! drop sends `/shutdown` and kills the service and its workers if they
//! linger; `support::Project` stops what is left and removes the folder
//! (helpers shared through `common/agent.rs`). The read-only checks share
//! one built project in `built_project_*`.

#[path = "common/builds.rs"]
mod builds;
#[path = "common/inspect.rs"]
mod inspect;
#[path = "common/agent.rs"]
mod support;

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use builds::WAIT_S;
use inspect::assert_vec;
use qoob3d_app::client;
use qoob3d_core::qmesh;
use serde_json::{Value, json};
use support::{
    BIN, Project, START, Service, VOL, alive, assert_rel, assert_rel_f64, cli, exchange, kill,
    service_json, service_token, wait_until,
};

const F32_POS: f64 = 1e-4;

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

impl Project {
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
}

// ===========================================================================
// Service
// ===========================================================================

impl Service {
    /// As [`Service::start`] with the service limited to `kib` KiB of
    /// address space (`ulimit -v`, inherited by its workers): a script that
    /// allocates without bound makes the service fail instead of exhausting
    /// the machine. `exec` keeps the pid, so `service.json` names the child.
    /// Where the shell cannot set the limit (macOS) the service runs without.
    fn start_capped(p: &Project, kib: u64) -> Self {
        let mut command = Command::new("sh");
        let script = format!("ulimit -v {kib} 2>/dev/null; exec \"$0\" serve \"$1\"");
        command.arg("-c").arg(script).arg(BIN).arg(&p.dir);
        Self::start_with(p, &mut command)
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

/// A built part in a build result: state, volume, bbox, valid, duration (§3.1).
fn assert_built(result: &Value, part: &str, state: &str, volume: f64, bbox: ([f64; 3], [f64; 3])) {
    let p = &result["parts"][part];
    assert_eq!(p["state"], json!(state), "{part}: {p}");
    assert_rel(&p["volume"], volume, VOL, &format!("volume of {part}"));
    assert_vec(&p["bbox"]["min"], bbox.0, 1e-6, &format!("bbox.min of {part}"));
    assert_vec(&p["bbox"]["max"], bbox.1, 1e-6, &format!("bbox.max of {part}"));
    let size = [bbox.1[0] - bbox.0[0], bbox.1[1] - bbox.0[1], bbox.1[2] - bbox.0[2]];
    assert_vec(&p["bbox"]["size"], size, 1e-6, &format!("bbox.size of {part}"));
    assert_eq!(p["valid"], json!(true), "{part}: {p}");
    // The FDM gate: the cached mesh, which STL and 3MF are exported from, is closed.
    assert_eq!(p["watertight"], json!(true), "{part}: {p}");
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

/// Linear deflection of the cached/served meshes (§2.1: 0.01 mm).
const DEFLECTION: f64 = 0.01;
/// Axes along which a part's surface is curved; planar axes need no deflection allowance.
const FLAT: [bool; 3] = [false; 3];
/// Cylinder with axis Z: curved in x and y, planar caps in z.
const CYL_Z: [bool; 3] = [true, true, false];

/// Mesh bbox against the exact bbox. Vertices lie on the exact surface, so
/// the mesh bbox lies inside the exact one (up to f32 rounding, `F32_POS`).
/// On planar axes the extremes are vertices, so both sides match to
/// `F32_POS`; on curved axes the extreme may fall between vertices, and the
/// chord there is at most the deflection away from the surface.
fn assert_box(
    got: ([f64; 3], [f64; 3]),
    want: ([f64; 3], [f64; 3]),
    curved: [bool; 3],
    what: &str,
) {
    for (i, is_curved) in curved.into_iter().enumerate() {
        let inward = if is_curved { DEFLECTION + F32_POS } else { F32_POS };
        let (lo, hi) = (got.0[i] - want.0[i], want.1[i] - got.1[i]);
        assert!(
            lo > -F32_POS && lo < inward && hi > -F32_POS && hi < inward,
            "{what}: bbox {got:?}, expected {want:?} (axis {i}, inward tolerance {inward})"
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
    assert_rel(&p["volume"], 6000.0, VOL, "volume of Normal");
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
        let r = s
            .try_request("GET", "/status", None, Duration::from_secs(1))
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
    assert_rel(&r["parts"]["B"]["volume"], 2400.0, VOL, "B after A");
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
    // Discovery runs in a worker (ADR-0055): wait until M1 and M2 are known.
    s.wait_discovery();
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
    for (part, volume, bbox, curved) in [
        ("A", 2000.0, A_BOX, FLAT),
        ("B", 2000.0, B_BOX, FLAT),
        ("C", 64.0, C_BOX, FLAT),
        ("D", D_VOLUME, D_BOX, CYL_Z),
    ] {
        let key = key_of(st, part);
        let base = p.path(&format!(".qoob3d/cache/{part}/{key}"));
        let brep = std::fs::read(base.with_extension("brep")).expect("cache .brep");
        assert!(!brep.is_empty(), "{part}.brep is empty");
        let qmsh = std::fs::read(base.with_extension("qmsh")).expect("cache .qmsh");
        assert_box(qmesh_bbox(&qmsh), bbox, curved, &format!("cached mesh of {part}"));
        let text = std::fs::read_to_string(base.with_extension("json")).expect("cache .json");
        let j: Value = serde_json::from_str(&text).expect("cache json");
        assert_rel(&j["volume"], volume, VOL, &format!("cached volume {part}"));
        for k in ["min", "max", "size"] {
            assert!(j["bbox"][k].is_array(), "{part}: bbox.{k} in {j}");
        }
        assert_eq!(j["valid"], json!(true), "{part}: {j}");
        assert_eq!(j["solids"], json!(1), "{part}: one solid: {j}");
        assert_eq!(j["watertight"], json!(true), "{part}: {j}");
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
    for (part, bbox, curved) in
        [("A", A_BOX, FLAT), ("B", B_BOX, FLAT), ("C", C_BOX, FLAT), ("D", D_BOX, CYL_Z)]
    {
        let r = s.get(&format!("/mesh/{part}"));
        assert_eq!(r.code, 200, "/mesh/{part}: {}", r.text());
        let ct = r.header("content-type").unwrap_or_default();
        assert!(ct.starts_with("application/octet-stream"), "{part}: {ct}");
        assert_eq!(r.header("x-qoob3d-key"), Some(key_of(st, part)), "{part}");
        assert_box(qmesh_bbox(&r.body), bbox, curved, &format!("mesh of {part}"));
    }
    // The cylinder D is tessellated to 0.01 mm: no vertex is outside r = 5.
    let mesh = qmesh::read(&s.get("/mesh/D").body).expect("qmesh of D");
    for p in &mesh.faces.positions {
        let r = f64::from(p[0]).hypot(f64::from(p[1]));
        assert!(r <= 5.0 + F32_POS, "vertex outside the cylinder: r = {r}");
    }
}

/// The W1 client (client.rs) talks to the real service.
fn check_live_client(p: &Project, st: &Value) {
    let dir = client::ProjectDir::open(&p.dir).expect("project dir");
    let client = client::Client::connect(&dir, &[BIN.to_owned()], START).expect("connect");
    let parts = client.status().expect("status via the W1 client");
    assert!(parts.iter().all(|part| part.state == client::PartState::Current), "{parts:?}");
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
    assert_rel_f64(vol, 2000.0, 1e-5, "STL volume of A");
    assert_box((lo, hi), A_BOX, FLAT, "STL bbox of A");
    let (vol, ..) = stl_volume_bbox(&std::fs::read(p.path("out/C.stl")).expect("out/C.stl"));
    assert_rel_f64(vol, 64.0, 1e-5, "STL volume of C");

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
    let step = p.read("out/A.step");
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
    let r = s
        .try_request("POST", "/shutdown", Some(&json!({})), Duration::from_secs(5))
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
    assert_rel_f64(vol, 2000.0, 1e-5, "CLI STL volume of A");

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

// ===========================================================================
// R5 hardening (agent.md §6.1): AG-HD-01, 02, 03, 06, 10
// ===========================================================================

/// The kind of the load error in a `/status` answer, whichever way the
/// envelope carries it (`result.error` as in §3.1 or the envelope's `error`).
fn load_error_kind(v: &Value) -> Option<&str> {
    v["result"]["error"]["kind"].as_str().or_else(|| v["error"]["kind"].as_str())
}

#[test]
fn ag_hd_01_a_time_limit_out_of_range_never_hangs_the_job_queue() {
    // AG-HD-01: `time_s = inf` and `1e20` are config errors naming the field,
    // and "no config value can panic the service or a job thread". Before
    // R5 both values loaded and the job thread died converting them to a
    // duration; the queue then never answered again.
    //
    // A panic inside a job thread ("fails that job with kind `crash` and
    // returns the worker to the pool") cannot be provoked from outside once
    // the config is validated; that half of the requirement needs a unit
    // test next to the job runner (see the report of the test author).
    let p = Project::new("hd-01", "[limits]\ntime_s = inf\n[workers]\ncount = 1\n");
    p.write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    for value in ["inf", "1e20"] {
        p.config(&format!("[limits]\ntime_s = {value}\n[workers]\ncount = 1\n"));
        // The build must be refused or fail, at once: `wait_s` = 10 bounds
        // the answer, so a hanging job shows as a running job, not as a
        // hanging test.
        let build = json!({"parts": null, "wait_s": 10});
        let r = s
            .try_request("POST", "/build", Some(&build), Duration::from_secs(30))
            .unwrap_or_else(|e| panic!("time_s = {value}: /build did not answer: {e}"));
        let v = r.envelope();
        assert_ne!(v["result"]["state"], json!("running"), "time_s = {value}: the job hangs: {v}");
        let failed = v["result"]["parts"]
            .as_object()
            .is_some_and(|m| m.values().any(|part| part["state"] == json!("failed")));
        assert!(v["ok"] == json!(false) || failed, "time_s = {value} was accepted: {v}");
        assert!(r.text().contains("time_s"), "time_s = {value}: the error names the field: {v}");
        // The service answers and reports the config error.
        let st = s.get("/status");
        assert_eq!(st.code, 200);
        assert!(
            st.text().contains("time_s"),
            "time_s = {value}: /status names the field: {}",
            st.text()
        );
    }
    // With a valid file the same service builds: nothing is stuck.
    p.config("[limits]\ntime_s = 60\n[workers]\ncount = 1\n");
    assert_normal_builds(&s);
    assert_eq!(s.status()["workers"]["busy"], json!(0));
}

/// Address-space cap for services that are fed a memory bomb: 4 000 000 KiB
/// (about 3.8 GiB). With the limit of AG-HD-02 missing, the bomb fails the
/// service at the cap instead of exhausting the machine.
const BOMB_CAP_KIB: u64 = 4_000_000;

/// AG-HD-02: 16 B doubled 40 times = 16 TiB at the top level of a file.
const STRING_BOMB: &str = "\
let s = \"0123456789abcdef\";
for i in 0..40 { s += s; }
";
/// One element doubled 40 times = 2^40 elements.
const ARRAY_BOMB: &str = "\
let a = [0];
for i in 0..40 { a += a; }
";

/// Starts a (capped) service on a project whose file `rel` is `source` and
/// expects it alive, with `/status` reporting the load error of that file.
fn assert_bomb_is_a_load_error(tag: &str, rel: &str, source: &str) {
    let p = Project::new(tag, "[workers]\ncount = 1\n");
    p.write("parts/normal.rhai", NORMAL).write(rel, source);
    // `start_capped` already needs an answer to `/status`.
    let s = Service::start_capped(&p, BOMB_CAP_KIB);
    // Discovery runs in a worker (ADR-0055): the first answers may come
    // before it has finished. Every poll stays within the 100 ms budget;
    // the load error naming the file follows within 10 s.
    wait_for_load_error(&s, rel, &["load"], Duration::from_secs(10));
    // From then on every answer carries it.
    for round in 0..3 {
        let t = Instant::now();
        let r = s
            .try_request("GET", "/status", None, Duration::from_secs(10))
            .unwrap_or_else(|e| panic!("{rel}, round {round}: /status: {e}\n{}", s.log()));
        // Crossing a size limit costs a handful of operations.
        assert!(t.elapsed() < Duration::from_secs(5), "{rel}: /status took {:?}", t.elapsed());
        assert_eq!(r.code, 200, "{rel}: {}", r.text());
        let v = r.envelope();
        assert!(r.text().contains(rel), "{rel}: /status names the file: {v}");
        assert_eq!(load_error_kind(&v), Some("load"), "{rel}: a load error: {v}");
    }
    assert!(alive(s.pid), "{rel}: the service died");
    // Without the bomb the same service builds the project.
    std::fs::remove_file(p.path(rel)).expect("remove the bomb");
    assert_normal_builds(&s);
}

#[test]
fn ag_hd_02_a_string_bomb_at_the_top_level_of_a_part_file_is_a_load_error() {
    // "A part file whose top level doubles a string 40 times leaves the
    // service alive and `/status` answering with the load error naming the
    // file." The part declared after the loop is never reached.
    let source = format!("{STRING_BOMB}part(\"Bomb\", #{{}}, |deps| box(1, 1, 1));\n");
    assert_bomb_is_a_load_error("hd-02-string", "parts/bomb.rhai", &source);
}

#[test]
fn ag_hd_02_an_array_bomb_at_the_top_level_of_a_part_file_is_a_load_error() {
    // "array 10000 elements": the same bomb with an array, also when the
    // loop is the last statement of the file and nothing reads the array.
    assert_bomb_is_a_load_error("hd-02-array", "parts/bomb.rhai", ARRAY_BOMB);
}

#[test]
fn ag_hd_02_a_string_bomb_in_a_check_file_is_a_load_error() {
    // "Declaration discovery in the service (`PartMode::Declare` and check
    // discovery)": check files are evaluated by the service as well.
    let source = format!("{STRING_BOMB}check(\"late\", || {{ expect(true, \"never\"); }});\n");
    assert_bomb_is_a_load_error("hd-02-check", "checks/bomb.rhai", &source);
}

/// The errors anywhere in a `/status` answer: every JSON object with a
/// string `kind` and a `message`, as `(kind, text of the object)`. ADR-0055
/// reports load errors per file; where in the answer is not specified.
fn status_errors(v: &Value, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(map) => {
            if let (Some(kind), true) =
                (map.get("kind").and_then(Value::as_str), map.contains_key("message"))
            {
                out.push((kind.to_owned(), v.to_string()));
            }
            for child in map.values() {
                status_errors(child, out);
            }
        }
        Value::Array(list) => {
            for child in list {
                status_errors(child, out);
            }
        }
        _ => {}
    }
}

/// Polls `/status` every 50 ms until it reports an error of one of `kinds`
/// that names the file `rel`, for at most `within`. Every poll must be
/// answered within 100 ms (AG-SV-01; ADR-0055: "`/status` must not wait for
/// discovery longer than its 100 ms budget"). Returns the kind found.
fn wait_for_load_error(s: &Service, rel: &str, kinds: &[&str], within: Duration) -> String {
    load_error_within(s, rel, kinds, within, Duration::from_millis(100))
}

/// As [`wait_for_load_error`] with `budget` per `/status` answer, for tests
/// whose subject is not the latency of `/status`.
fn load_error_within(
    s: &Service,
    rel: &str,
    kinds: &[&str],
    within: Duration,
    budget: Duration,
) -> String {
    let deadline = Instant::now() + within;
    let mut polls = 0;
    loop {
        let t = Instant::now();
        let r = s
            .try_request("GET", "/status", None, Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("{rel}, poll {polls}: /status: {e}\n{}", s.log()));
        let took = t.elapsed();
        assert!(took < budget, "{rel}, poll {polls}: /status took {took:?}");
        assert_eq!(r.code, 200, "{rel}, poll {polls}: {}", r.text());
        let v = r.envelope();
        let mut errors = Vec::new();
        status_errors(&v, &mut errors);
        let found =
            errors.iter().find(|(kind, text)| kinds.contains(&kind.as_str()) && text.contains(rel));
        if let Some((kind, _)) = found {
            return kind.clone();
        }
        assert!(
            Instant::now() < deadline,
            "{rel}: no load error ({kinds:?}) after {within:?} and {polls} polls: {v}\n{}",
            s.log()
        );
        polls += 1;
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn ag_hd_02_discovery_stops_after_a_million_operations() {
    // "`max_operations` 1000000" for discovery: 2 000 000 loop iterations
    // (at least one operation each) at the top level never reach the part
    // declaration; a worker run (50 000 000, AG-SC-02) would.
    //
    // ADR-0055: discovery runs in a worker, so the service is up at once
    // (`Service::start` needs an answer to `/status` within a second) and
    // the load error follows; with linear measuring cost a million
    // operations take about a second in a debug build, 10 s is generous.
    let p = Project::new("hd-02-ops", "[workers]\ncount = 1\n");
    p.write("parts/normal.rhai", NORMAL).write(
        "parts/slow.rhai",
        "let n = 0;\nfor i in 0..2000000 { n += 1; }\npart(\"Late\", #{}, |deps| box(1, 1, 1));\n",
    );
    let s = Service::start(&p);
    wait_for_load_error(&s, "parts/slow.rhai", &["load", "limit"], Duration::from_secs(10));
    let r = s.get("/status");
    assert!(
        !r.text().contains("\"Late\""),
        "the part after the loop is not declared: {}",
        r.text()
    );
    assert!(alive(s.pid));
}

/// HTTP status of a hand-written request: `host` as `Host` header (none if
/// `None`), `headers` verbatim. POST carries the body `{}`.
fn status_code(
    port: u16,
    method: &str,
    path: &str,
    host: Option<&str>,
    headers: &[(&str, &str)],
) -> u16 {
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    if let Some(host) = host {
        request.push_str(&format!("Host: {host}\r\n"));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    let body = if method == "POST" { "{}" } else { "" };
    request.push_str(&format!(
        "Connection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    ));
    exchange(port, &request, Duration::from_secs(10))
        .unwrap_or_else(|e| panic!("{method} {path} (Host {host:?}, {headers:?}): {e}"))
        .code
}

#[test]
fn ag_hd_03_requests_need_the_token_and_a_local_host_and_no_origin() {
    // AG-HD-03 (ADR-0055): "The service writes a random token (32 hex
    // characters) into `service.json` and requires it in the header
    // `X-Qoob3d-Token` on every request; requests carrying an `Origin`
    // header, or whose `Host` is not `127.0.0.1:<port>` or
    // `localhost:<port>`, are refused with 403 before routing."
    let p = Project::abcd("hd-03");
    let s = Service::start(&p);
    let token = service_token(&p.dir).expect("service.json has a token");
    assert_eq!(token.len(), 32, "32 hex characters: {token:?}");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()), "32 hex characters: {token:?}");
    // The file still carries port and pid (project-app.md §1).
    assert_eq!(service_json(&p.dir), Some((s.port, s.pid)));

    let port = s.port;
    let local = format!("127.0.0.1:{port}");
    let name = format!("localhost:{port}");
    let good = [("X-Qoob3d-Token", token.as_str())];
    // The premise: with token and local host the service answers.
    assert_eq!(status_code(port, "GET", "/status", Some(&local), &good), 200);
    assert_eq!(status_code(port, "GET", "/status", Some(&name), &good), 200, "Host localhost");
    // Header names are case-insensitive (RFC 9110 §5.1).
    let lower = [("x-qoob3d-token", token.as_str())];
    assert_eq!(status_code(port, "GET", "/status", Some(&local), &lower), 200);

    // No token, an empty one, a wrong one (one character changed, a prefix,
    // 32 zeros): 403 on every endpoint, also on unknown ones ("before
    // routing": not 404, not 400).
    let flipped = if token.starts_with('0') { "1" } else { "0" };
    let wrong = format!("{flipped}{}", &token[1..]);
    let zeros = "0".repeat(32);
    let bad_tokens: [Option<&str>; 5] =
        [None, Some(""), Some(wrong.as_str()), Some(&token[..31]), Some(zeros.as_str())];
    let endpoints = [
        ("GET", "/status"),
        ("GET", "/mesh/A"),
        ("GET", "/tasks"),
        ("GET", "/no/such/endpoint"),
        ("POST", "/build"),
        ("POST", "/run"),
        ("POST", "/task"),
        ("POST", "/save"),
        ("POST", "/shutdown"),
    ];
    for bad in bad_tokens {
        let headers: Vec<(&str, &str)> = bad.iter().map(|t| ("X-Qoob3d-Token", *t)).collect();
        for (method, path) in endpoints {
            let code = status_code(port, method, path, Some(&local), &headers);
            assert_eq!(code, 403, "{method} {path} with token {bad:?}");
        }
    }
    // The token in the query or in another header is no token.
    let query = format!("/status?token={token}");
    assert_eq!(status_code(port, "GET", &query, Some(&local), &[]), 403);
    let other = [("Authorization", token.as_str())];
    assert_eq!(status_code(port, "GET", "/status", Some(&local), &other), 403);

    // An `Origin` header marks a request from a web page: refused although
    // the token is right, whatever the origin says.
    for origin in ["http://evil.example", "null", local.as_str(), "http://localhost:3000", ""] {
        let headers = [("X-Qoob3d-Token", token.as_str()), ("Origin", origin)];
        for (method, path) in [("GET", "/status"), ("POST", "/build"), ("POST", "/shutdown")] {
            let code = status_code(port, method, path, Some(&local), &headers);
            assert_eq!(code, 403, "{method} {path} with Origin {origin:?}");
        }
    }

    // A foreign `Host` (DNS rebinding): refused although the token is right.
    let other_port = format!("127.0.0.1:{}", if port == 65535 { 65534 } else { port + 1 });
    let rebound = format!("evil.example:{port}");
    let suffix = format!("127.0.0.1:{port}.evil.example");
    let hosts = [
        "evil.example",
        rebound.as_str(),
        other_port.as_str(),
        "127.0.0.1",
        "localhost",
        suffix.as_str(),
    ];
    for host in hosts {
        let code = status_code(port, "GET", "/status", Some(host), &good);
        assert_eq!(code, 403, "Host {host:?}");
    }
    // No `Host` at all is not a local host either.
    assert_eq!(status_code(port, "GET", "/status", None, &good), 403, "no Host header");

    // None of the refused requests reached a route: nothing was built, no
    // task was created, and the refused /shutdown did not stop the service.
    // Discovery runs in a worker (ADR-0055): wait for it, so `states` lists
    // A..D and the check below is not vacuously true on an empty list.
    s.wait_discovery();
    let st = s.status();
    assert_eq!(states(&st).len(), 4, "A, B, C and D are listed: {st}");
    assert!(states(&st).values().all(|v| v == "never_built"), "a refused /build ran: {st}");
    assert_eq!(st["jobs"].as_array().map(Vec::len), Some(0), "a refused request made a job: {st}");
    assert!(!p.path("tasks").exists(), "a refused POST /task created a task");
    assert!(alive(s.pid));
}

#[test]
fn ag_hd_03_the_token_is_random_per_start() {
    // "a random token": two services never share one, and a restart of the
    // same project gets a new one (ADR-0055: "generates a random token per
    // start"). 32 hex characters are 128 bits; equal tokens by chance are
    // impossible in practice.
    let (p1, p2) = (Project::new("hd-03-a", ""), Project::new("hd-03-b", ""));
    let first = Service::start(&p1);
    let second = Service::start(&p2);
    let t1 = service_token(&p1.dir).expect("token of the first service");
    let t2 = service_token(&p2.dir).expect("token of the second service");
    assert_ne!(t1, t2, "two services share a token");
    // A token opens only its own service.
    let host = format!("127.0.0.1:{}", second.port);
    let foreign = [("X-Qoob3d-Token", t1.as_str())];
    assert_eq!(status_code(second.port, "GET", "/status", Some(&host), &foreign), 403);

    // Drop shuts the first service down and waits for it.
    drop(first);
    assert!(service_json(&p1.dir).is_none(), "the first service stopped");
    let again = Service::start(&p1);
    let t3 = service_token(&p1.dir).expect("token after the restart");
    assert_ne!(t3, t1, "the restarted service reuses its token");
    again.status();
}

#[test]
fn ag_hd_03_cli_and_app_client_send_the_token() {
    // "The app client, CLI and MCP send the token." (MCP: every test of
    // tests/mcp.rs runs against a service that requires it.)
    let p = Project::abcd("hd-03-clients");
    let s = Service::start(&p);
    assert!(service_token(&p.dir).is_some(), "the service requires a token");
    let before = service_json(&p.dir);

    let out = cli(&p.dir, &["build", "--wait", "60"], Duration::from_secs(90));
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    assert_eq!(built_cached(&out.envelope()["result"]).0, set(&["A", "B", "C", "D"]));
    let out = cli(&p.dir, &["status"], Duration::from_secs(90));
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    assert_eq!(out.envelope()["ok"], json!(true));
    // A client that is refused must not "repair" the situation by starting
    // a second service.
    assert_eq!(service_json(&p.dir), before, "the CLI used the running service");

    check_live_client(&p, &s.status());
    assert_eq!(service_json(&p.dir), before, "the app client used the running service");
}

/// V imports its module through a variable: line 3 is
/// `    import m as form;`, `import` at column 5, its path `m` at column 12.
const PART_VAR_IMPORT: &str = "\
part(\"V\", #{}, |deps| {
    let m = \"lib/form\";
    import m as form;
    form::make()
});
";

#[test]
fn ag_hd_06_import_through_a_variable_is_a_script_error_never_a_stale_part() {
    // AG-HD-06: "`import` with a non-literal path is refused with a script
    // error at its position." The scan for the cache key reads literal
    // paths only; before R5 such a part built, its key ignored the module,
    // and after a change of the module `/status` still said `current` and
    // `/build` served the old geometry from the cache.
    let p = Project::new("hd-06", "[workers]\ncount = 1\n");
    p.write("parts/v.rhai", PART_VAR_IMPORT)
        .write("lib/form.rhai", LIB_FORM)
        .write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    s.build(None);
    // form::make() becomes box(4, 4, 5) = 80 mm³ instead of 64 mm³.
    p.write("lib/form.rhai", "fn make() { box(4, 4, 5) }\n");
    let st = s.status();
    let r = s.build(None);
    let v = &r["parts"]["V"];
    if let Some(volume) = v["volume"].as_f64() {
        assert_rel_f64(volume, 80.0, VOL, "V is served with stale geometry (the old module)");
    }
    if states(&st)["V"] == "current" {
        let measured = s.post("/measure", &json!({"a": "V"})).envelope();
        let volume = measured["result"]["volume"].as_f64().unwrap_or(f64::NAN);
        assert_rel_f64(volume, 80.0, VOL, "V is `current` with stale geometry (the old module)");
    }

    // The requirement itself: a script error at the import.
    assert_eq!(error_kind(&r, "V"), "script", "{r}");
    let e = &v["error"];
    assert_eq!(e["file"], json!("parts/v.rhai"), "{e}");
    assert_eq!(e["line"].as_u64(), Some(3), "{e}");
    // Column of the statement or of its path expression (7 further).
    assert!([Some(5), Some(12)].contains(&e["column"].as_u64()), "{e}");
    assert_eq!(states(&s.status())["V"], "failed");
    // Other parts are unaffected.
    assert_normal_builds(&s);
}

#[test]
fn ag_hd_06_import_through_a_variable_at_the_top_level_is_a_load_error_with_position() {
    // The same at the top level of a part file, which discovery evaluates:
    // the error names the file and the line of the import (line 2).
    let p = Project::new("hd-06-top", "[workers]\ncount = 1\n");
    p.write(
        "parts/top.rhai",
        "let m = \"lib/form\";\nimport m as form;\npart(\"T\", #{}, |deps| form::make());\n",
    )
    .write("lib/form.rhai", LIB_FORM);
    let s = Service::start(&p);
    // Discovery runs in a worker (ADR-0055): poll until its answer is in.
    wait_for_load_error(&s, "parts/top.rhai", &["load"], Duration::from_secs(10));
    let r = s.get("/status");
    let v = r.envelope();
    assert!(r.text().contains("parts/top.rhai"), "/status names the file: {v}");
    assert!(r.text().contains("line 2") || r.text().contains("\"line\":2"), "and the line: {v}");
    let parts = v["result"]["parts"].as_array().cloned().unwrap_or_default();
    assert!(!parts.iter().any(|part| part["state"] == json!("current")), "{v}");
    let r = s.post("/build", &json!({"parts": null, "wait_s": WAIT_S}));
    let v = r.envelope();
    let built = v["result"]["built"].as_array().is_some_and(|b| !b.is_empty());
    assert!(!built, "a part with a non-literal import was built: {v}");
}

#[test]
fn ag_hd_06_changing_a_module_resolved_inside_a_module_rebuilds_the_part() {
    // "Changing any resolved module rebuilds the part": P imports lib/shape,
    // whose function imports lib/dims when it runs. P = box(W, 4, 4) with
    // W = 4 → 64 mm³; W = 6 → 96 mm³.
    let p = Project::new("hd-06-nested", "[workers]\ncount = 1\n");
    p.write(
        "parts/p.rhai",
        "import \"lib/shape\" as shape;\npart(\"P\", #{}, |deps| shape::make());\n",
    )
    .write(
        "lib/shape.rhai",
        "fn make() {\n    import \"lib/dims\" as dims;\n    box(dims::W, 4, 4)\n}\n",
    )
    .write("lib/dims.rhai", "export const W = 4;\n")
    .write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    let r = s.build(None);
    assert_built(&r, "P", "built", 64.0, ([-2.0, -2.0, 0.0], [2.0, 2.0, 4.0]));
    assert_eq!(built_cached(&s.build(None)), (set(&[]), set(&["Normal", "P"])));
    let key = key_of(&s.status(), "P");

    p.write("lib/dims.rhai", "export const W = 6;\n");
    assert_eq!(states(&s.status())["P"], "outdated", "the module P resolved changed");
    let r = s.build(None);
    assert_eq!(built_cached(&r), (set(&["P"]), set(&["Normal"])));
    assert_built(&r, "P", "built", 96.0, ([-3.0, -2.0, 0.0], [3.0, 2.0, 4.0]));
    assert_ne!(key_of(&s.status(), "P"), key);
}

#[test]
fn ag_hd_10_status_lists_at_most_the_20_newest_jobs_with_their_kind() {
    // AG-HD-10: "`/status` lists at most the 20 newest jobs, each with
    // `kind` (`build`, `check`, `run`)". 25 builds, then one check run and
    // one throwaway script: 27 jobs at least.
    let p = Project::new("hd-10", "[workers]\ncount = 1\n");
    p.write("parts/normal.rhai", NORMAL).write(
        "checks/normal.rhai",
        "check(\"normal_volume\", || {\n    \
         expect_near(part(\"Normal\").volume(), 6000, 0.001, \"volume\");\n});\n",
    );
    let s = Service::start(&p);
    for _ in 0..25 {
        s.build(Some(&["Normal"]));
    }
    let jobs = s.status()["jobs"].as_array().cloned().expect("jobs");
    assert_eq!(jobs.len(), 20, "25 builds were made, the 20 newest are listed");
    assert!(jobs.iter().all(|j| j["kind"] == json!("build")), "{jobs:?}");

    let checked = s.post("/check", &json!({"names": null, "wait_s": WAIT_S})).ok();
    assert_eq!(checked["passed"].as_u64(), Some(1), "{checked}");
    s.post("/run", &json!({"source": "print(1);", "wait_s": WAIT_S})).ok();
    let st = s.status();
    let jobs = st["jobs"].as_array().cloned().expect("jobs");
    assert_eq!(jobs.len(), 20, "{st}");
    let mut kinds = std::collections::BTreeMap::new();
    for j in &jobs {
        assert!(j.get("id").is_some() && j["state"].is_string(), "job entry {j}");
        let kind = j["kind"].as_str().unwrap_or_else(|| panic!("job without kind: {j}"));
        assert!(["build", "check", "run"].contains(&kind), "kind of {j}");
        *kinds.entry(kind.to_owned()).or_insert(0) += 1;
    }
    // The newest jobs are the check and the script; they pushed old builds out.
    assert_eq!(kinds.get("run"), Some(&1), "the newest job is listed: {st}");
    assert!(kinds.get("check").is_some_and(|n| *n >= 1), "the check job is listed: {st}");
    assert!(kinds.get("build").is_some_and(|n| *n <= 18), "old builds were dropped: {st}");
    let ids: BTreeSet<String> = jobs.iter().map(|j| j["id"].to_string()).collect();
    assert_eq!(ids.len(), jobs.len(), "no job twice: {st}");
}

// ===========================================================================
// ADR-0055: declaration discovery runs in a worker
// ===========================================================================

/// A top level that never ends. Rhai counts operations, so discovery stops
/// it at 1 000 000 (AG-HD-02) unless the time limit comes first.
const ENDLESS_TOP_LEVEL: &str = "loop {}\n";

/// A project with one normal part whose service runs; the file `rel` with
/// `source` is written afterwards, as an agent would.
fn service_then_file(tag: &str, extra: &str, rel: &str, source: &str) -> (Project, Service) {
    let p = Project::new(tag, extra);
    p.write("parts/normal.rhai", NORMAL);
    let s = Service::start_capped(&p, BOMB_CAP_KIB);
    // Discovery runs in a worker (ADR-0055): the first `/status` after the
    // start may not list `Normal` yet.
    s.wait_discovery();
    assert_eq!(states(&s.status())["Normal"], "never_built");
    p.write(rel, source);
    (p, s)
}

#[test]
fn discovery_an_endless_top_level_never_blocks_the_service() {
    // ADR-0055: "The service never evaluates user scripts. Declaration
    // discovery of part and check files is a worker request … under the
    // project's time limit". `/status` answers within 100 ms on every poll
    // from the moment the file exists, and reports the file's load error
    // (operation limit or time limit, whichever comes first) within 10 s;
    // `time_s` = 2.
    let limits = "[limits]\ntime_s = 2\n[workers]\ncount = 2\n";
    let (p, s) =
        service_then_file("adr-56-endless", limits, "parts/endless.rhai", ENDLESS_TOP_LEVEL);
    let before = service_json(&p.dir);
    let kinds = ["load", "limit", "time_limit"];
    wait_for_load_error(&s, "parts/endless.rhai", &kinds, Duration::from_secs(10));
    // The same service process, still answering; its workers are usable.
    assert!(alive(s.pid), "the service died");
    assert_eq!(service_json(&p.dir), before, "the service was restarted");
    assert_eq!(s.status()["workers"]["count"], json!(2));
    // The same for a check file.
    p.write("checks/endless.rhai", ENDLESS_TOP_LEVEL);
    wait_for_load_error(&s, "checks/endless.rhai", &kinds, Duration::from_secs(10));
    assert!(alive(s.pid), "the service died");
}

#[test]
fn discovery_a_file_that_does_not_load_fails_only_itself() {
    // ADR-0055: "A worker crash or limit during discovery fails only that
    // file; the service stays up". The normal part next to the endless
    // file is listed and builds (box(10, 20, 30) = 6000 mm³). The latency
    // of `/status` is the subject of the test above, not of this one.
    let limits = "[limits]\ntime_s = 2\n[workers]\ncount = 2\n";
    let (_p, s) =
        service_then_file("adr-56-beside", limits, "parts/endless.rhai", ENDLESS_TOP_LEVEL);
    let kinds = ["load", "limit", "time_limit"];
    let (within, patient) = (Duration::from_secs(10), Duration::from_secs(5));
    load_error_within(&s, "parts/endless.rhai", &kinds, within, patient);
    let st = s.status();
    assert!(status_parts(&st).contains_key("Normal"), "the other file's part is listed: {st}");
    assert_normal_builds(&s);
    assert_eq!(states(&s.status())["Normal"], "current");
    // The broken file is still reported after the build.
    load_error_within(&s, "parts/endless.rhai", &kinds, within, patient);
}

/// Grows memory where the staged size measurement does not look: 150 000
/// cheap iterations first (the measuring stages double, so late stages are
/// hundreds of thousands of operations apart), then 80 000 map entries with
/// a fresh string of 32 769 bytes each (below the 65536 of AG-HD-02):
/// 80 000 * 32 KiB = 2.4 GiB if nothing stops it, far above `memory_mb` =
/// 300. Bounded by the operation limit, and the service runs capped.
const MEMORY_BETWEEN_MEASUREMENTS: &str = "\
let x = 0;
for i in 0..150000 { x += 1; }
let s = \"0123456789abcdef\";
for i in 0..11 { s += s; }
let m = #{};
for i in 0..80000 { m[`k${i}`] = s + \"x\"; }
part(\"Late\", #{}, |deps| box(1, 1, 1));
";

#[test]
fn discovery_memory_grown_during_discovery_is_a_load_error_of_that_file() {
    // ADR-0055: "memory that grows between two measurements is stopped by
    // the worker's memory limit and reported as that file's load error
    // (`memory_limit`)"; a measurement that still catches it gives `limit`.
    // Either way: the service process itself never holds that memory, it
    // answers `/status` within 100 ms throughout and stays alive.
    let limits = "[limits]\ntime_s = 30\nmemory_mb = 300\n[workers]\ncount = 2\n";
    let (p, s) = service_then_file(
        "adr-56-memory",
        limits,
        "parts/hungry.rhai",
        MEMORY_BETWEEN_MEASUREMENTS,
    );
    let before = service_json(&p.dir);
    let kinds = ["load", "limit", "memory_limit"];
    wait_for_load_error(&s, "parts/hungry.rhai", &kinds, Duration::from_secs(15));
    assert!(alive(s.pid), "the service died");
    assert_eq!(service_json(&p.dir), before, "the service was restarted");
    let st = s.status();
    assert!(!st.to_string().contains("\"Late\""), "the part after the loop is not declared: {st}");
    // The worker that was stopped is replaced: the other part builds.
    assert_normal_builds(&s);
}

// ===========================================================================
// §3.1: `discovering` in /status and `POST /discover` (ADR-0055)
// ===========================================================================

/// M1 and M2 (depends on M1) in one file.
const PART_MULTI: &str = "\
part(\"M1\", #{}, |deps| box(10, 10, 10));
part(\"M2\", #{ depends: [\"M1\"] }, |deps| deps[\"M1\"].move(0, 0, 10));
";
/// Line 2 lacks the closing parenthesis of `part(`: a syntax error.
const PART_BROKEN: &str = "// a broken part file\npart(\"Broken\", #{}, |deps| box(1, 1, 1);\n";
const CHECK_NORMAL: &str = "\
check(\"normal_volume\", || {
    expect_near(part(\"Normal\").volume(), 6000, 0.001, \"volume\");
});
";
/// A top level of 200 000 loop iterations before its part: about 800 000
/// operations in declare mode (4 per iteration, the limit of AG-HD-02 is
/// 1 000 000 at 250 000 iterations; `script_hardening.rs` measures this),
/// so discovery takes a noticeable fraction of a second.
fn slow_part(name: &str) -> String {
    format!(
        "let x = 0;\nfor i in 0..200000 {{ x += 1; }}\npart(\"{name}\", #{{}}, |deps| box(1, 1, 1));\n"
    )
}

/// `{file: entry}` of the `files` of a `/discover` answer.
fn discovered(result: &Value) -> std::collections::BTreeMap<String, Value> {
    result["files"]
        .as_array()
        .unwrap_or_else(|| panic!("/discover without files: {result}"))
        .iter()
        .map(|f| (f["file"].as_str().unwrap_or_default().to_owned(), f.clone()))
        .collect()
}

/// Names in a list of declarations: `[{name, …}]` or plain strings.
fn declared_names(list: &Value) -> BTreeSet<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["name"].as_str().or_else(|| e.as_str()).map(str::to_owned))
        .collect()
}

/// A project with a normal part, a file with two parts, a broken part
/// file, a check file and a module nobody imports.
fn discover_project(tag: &str) -> Project {
    let p = Project::new(tag, "[workers]\ncount = 2\n");
    p.write("parts/normal.rhai", NORMAL)
        .write("parts/multi.rhai", PART_MULTI)
        .write("parts/broken.rhai", PART_BROKEN)
        .write("checks/normal.rhai", CHECK_NORMAL)
        .write("lib/form.rhai", LIB_FORM);
    p
}

#[test]
fn discover_waits_for_chosen_files_and_reports_declarations_or_errors() {
    // §3.1: "`POST /discover {"files": [..]|null, "wait_s": f64}` waits up
    // to `wait_s` for the discovery of those part and check files →
    // `{"files": [{file, state: "declared"|"failed"|"discovering", parts?:
    // [{name, depends}], checks?: [..], error?}], "discovering": [..]}`".
    let p = discover_project("discover");
    let s = Service::start(&p);
    let chosen = json!({"files": ["parts/multi.rhai", "parts/broken.rhai"], "wait_s": 10});
    let r = s.post("/discover", &chosen).ok();
    assert_eq!(r["discovering"], json!([]), "nothing is left after the wait: {r}");
    let files = discovered(&r);
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["parts/broken.rhai", "parts/multi.rhai"],
        "exactly the chosen files: {r}"
    );
    let multi = &files["parts/multi.rhai"];
    assert_eq!(multi["state"], json!("declared"), "{multi}");
    assert_eq!(declared_names(&multi["parts"]), set(&["M1", "M2"]), "{multi}");
    let parts = multi["parts"].as_array().cloned().unwrap_or_default();
    let depends = |name: &str| {
        let part = parts.iter().find(|part| part["name"] == json!(name));
        part.map(|part| part["depends"].clone()).unwrap_or_default()
    };
    assert_eq!((depends("M1"), depends("M2")), (json!([]), json!(["M1"])), "{multi}");
    assert!(multi.get("error").is_none_or(Value::is_null), "{multi}");

    let broken = &files["parts/broken.rhai"];
    assert_eq!(broken["state"], json!("failed"), "{broken}");
    let error = &broken["error"];
    assert!(error["message"].as_str().is_some_and(|m| !m.is_empty()), "{broken}");
    // The syntax error is on line 2.
    let line_named = error["line"].as_u64() == Some(2) || error.to_string().contains("line 2");
    assert!(line_named, "the error names line 2: {broken}");
    assert!(
        declared_names(&broken["parts"]).is_empty(),
        "a failed file declares nothing: {broken}"
    );
}

#[test]
fn discover_null_covers_all_part_and_check_files() {
    // "`null` = all": every `parts/*.rhai` and `checks/*.rhai`, not the
    // modules in `lib/`.
    let p = discover_project("discover-all");
    let s = Service::start(&p);
    let r = s.post("/discover", &json!({"files": null, "wait_s": 10})).ok();
    assert_eq!(r["discovering"], json!([]), "{r}");
    let files = discovered(&r);
    let expected =
        ["checks/normal.rhai", "parts/broken.rhai", "parts/multi.rhai", "parts/normal.rhai"];
    assert_eq!(files.keys().map(String::as_str).collect::<Vec<_>>(), expected, "{r}");
    for (file, state) in [
        ("checks/normal.rhai", "declared"),
        ("parts/broken.rhai", "failed"),
        ("parts/multi.rhai", "declared"),
        ("parts/normal.rhai", "declared"),
    ] {
        assert_eq!(files[file]["state"], json!(state), "{file}: {r}");
    }
    assert_eq!(declared_names(&files["parts/normal.rhai"]["parts"]), set(&["Normal"]));
    let check = &files["checks/normal.rhai"];
    assert_eq!(declared_names(&check["checks"]), set(&["normal_volume"]), "{check}");
    // `/status` agrees: the parts of the files that load, one load error.
    let st = s.status();
    assert_eq!(st["discovering"], json!([]), "{st}");
    let listed: BTreeSet<String> = status_parts(&st).into_keys().collect();
    assert_eq!(listed, set(&["M1", "M2", "Normal"]), "{st}");
    let errors = st["load_errors"].as_array().cloned().unwrap_or_default();
    assert_eq!(errors.len(), 1, "{st}");
    assert_eq!(errors[0]["file"], json!("parts/broken.rhai"), "{st}");
}

#[test]
fn discover_of_files_that_are_no_part_or_check_files_answers_at_once() {
    // The requirement speaks of "those part and check files". What a filter
    // that names something else gets is not specified (an error or an
    // empty list); it must not wait, must not report such a file as
    // declared, and must leave nothing in `discovering`.
    let p = discover_project("discover-filter");
    let s = Service::start(&p);
    for files in [
        json!(["lib/form.rhai"]),
        json!(["qoob3d.toml"]),
        json!(["parts/no-such-file.rhai"]),
        json!(["../outside.rhai"]),
        json!([]),
    ] {
        let t = Instant::now();
        let r = s.post("/discover", &json!({"files": files, "wait_s": 10}));
        let took = t.elapsed();
        assert!(took < Duration::from_secs(5), "{files}: /discover waited {took:?}");
        let v = r.envelope();
        if v["ok"] == json!(true) {
            let result = &v["result"];
            assert_eq!(result["discovering"], json!([]), "{files}: {v}");
            for entry in discovered(result).values() {
                assert_ne!(entry["state"], json!("declared"), "{files}: {v}");
                assert_ne!(entry["state"], json!("discovering"), "{files}: {v}");
            }
        }
    }
    // Nothing chosen: nothing reported.
    let none = s.post("/discover", &json!({"files": [], "wait_s": 10})).ok();
    assert_eq!(none["files"], json!([]), "{none}");
    // The service is unimpressed.
    assert_normal_builds(&s);
}

#[test]
fn status_lists_files_under_discovery_and_drops_them_once_known() {
    // §3.1: "`/status` never waits for discovery: `"discovering": [files]`
    // lists the part and check files whose declarations are not known yet
    // (… a new file has no parts yet)."
    let p = Project::new("discovering", "[workers]\ncount = 2\n");
    p.write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    s.post("/discover", &json!({"files": null, "wait_s": 10})).ok();
    assert_eq!(s.status()["discovering"], json!([]), "everything is known");

    // A new file that takes a while: the very next answer cannot know it.
    p.write("parts/slow.rhai", &slow_part("Slow"));
    let st = s.status();
    assert_eq!(st["discovering"], json!(["parts/slow.rhai"]), "{st}");
    let listed: BTreeSet<String> = status_parts(&st).into_keys().collect();
    assert_eq!(listed, set(&["Normal"]), "a new file has no parts yet: {st}");

    // `wait_s` = 0 does not wait either: the file is still being discovered.
    let r = s.post("/discover", &json!({"files": ["parts/slow.rhai"], "wait_s": 0})).ok();
    let files = discovered(&r);
    if files["parts/slow.rhai"]["state"] != json!("declared") {
        assert_eq!(files["parts/slow.rhai"]["state"], json!("discovering"), "{r}");
        assert_eq!(r["discovering"], json!(["parts/slow.rhai"]), "{r}");
    }

    // Once known it leaves the list and its part appears.
    let mut last = Value::Null;
    let known = wait_until(Duration::from_secs(10), || {
        last = s.status();
        last["discovering"] == json!([])
    });
    assert!(known, "parts/slow.rhai is still being discovered: {last}");
    let parts = status_parts(&last);
    assert_eq!(parts.keys().cloned().collect::<BTreeSet<_>>(), set(&["Normal", "Slow"]), "{last}");
    assert_eq!(parts["Slow"]["state"], json!("never_built"), "{last}");
    assert_eq!(last["load_errors"], json!([]), "{last}");

    // A changed file is discovered again; its earlier part stays listed.
    p.write("parts/slow.rhai", &format!("{}// changed\n", slow_part("Slow")));
    let st = s.status();
    assert_eq!(st["discovering"], json!(["parts/slow.rhai"]), "{st}");
    assert_eq!(states(&st)["Slow"], "outdated", "earlier parts are `outdated`: {st}");
    let r = s.post("/discover", &json!({"files": ["parts/slow.rhai"], "wait_s": 10})).ok();
    assert_eq!(discovered(&r)["parts/slow.rhai"]["state"], json!("declared"), "{r}");
    assert_eq!(s.status()["discovering"], json!([]));
}

#[test]
fn cli_status_waits_for_discovery_and_lists_a_freshly_written_part() {
    // §3.1: "`qoob3d status` waits up to 10 s for it before asking
    // `/status`": right after writing a part file the CLI answer is settled.
    let p = Project::new("cli-settled", "[workers]\ncount = 2\n");
    p.write("parts/normal.rhai", NORMAL);
    let s = Service::start(&p);
    p.write("parts/slow.rhai", &slow_part("Fresh")).write("parts/broken.rhai", PART_BROKEN);
    let out = cli(&p.dir, &["status"], Duration::from_secs(60));
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    let st = out.envelope()["result"].clone();
    assert_eq!(st["discovering"], json!([]), "the CLI answer is settled: {st}");
    let listed: BTreeSet<String> = status_parts(&st).into_keys().collect();
    assert_eq!(listed, set(&["Fresh", "Normal"]), "{st}");
    let errors = st["load_errors"].as_array().cloned().unwrap_or_default();
    assert!(errors.iter().any(|e| e["file"] == json!("parts/broken.rhai")), "{st}");
    drop(s);
}
