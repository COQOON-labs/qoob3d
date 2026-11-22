//! R4 without MCP: checks, tasks, throwaway scripts, measure, save, init and
//! the project render against the real `qoob3d` binary
//! (docs/requirements/agent.md §4.2–4.4: AG-CK-01..03, AG-SV-04..08,
//! AG-RD-01; ADR-0054; envelope from docs/requirements/project-app.md §1),
//! and R5 (§6.1 AG-HD-04, 07, 08, 09, 11; §6.2 AG-TK-05..07 through the
//! service). Every test names its requirement IDs.
//!
//! Test projects use only boxes and cylinders, so every expected number is
//! derived by hand next to the assertion. Tolerances: `VOL` = 1e-6 relative
//! for volumes and areas (exact solids, as in the R1 kernel tests), `POS` =
//! 1e-6 mm for coordinates and distances of planar faces.
//!
//! Processes and folders: `support::Project` stops the service of its folder
//! on drop and removes the folder (see `common/agent.rs`).

#[path = "common/agent.rs"]
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use qoob3d_app::live;
use serde_json::{Value, json};
use support::{
    BIN, Project, Service, VOL, assert_rel, assert_vec, cli, git, git_init, git_tracked,
    png_colors, png_size, service_json, wait_until,
};

const POS: f64 = 1e-6;
/// `wait_s` for synchronous requests; boxes build in milliseconds.
const WAIT_S: f64 = 60.0;
const CLI: Duration = Duration::from_secs(90);
/// A render starts a wgpu device (software Vulkan in CI): seconds, not minutes.
const RENDER: Duration = Duration::from_secs(120);

// ===========================================================================
// Test projects
// ===========================================================================

/// A = box(a_size, 10, 10) with a_size = 20 → 20·10·10 = 2000 mm³,
/// x ∈ [−10, 10], y ∈ [−5, 5], z ∈ [0, 10]; area 2·(200 + 200 + 100) = 1000 mm².
const PART_A: &str = "part(\"A\", #{}, |deps| box(value(\"a_size\"), 10, 10));\n";
/// B = A moved by 30 in x → x ∈ [20, 40]; gap to A = 20 − 10 = 10 mm.
const PART_B: &str = "part(\"B\", #{ depends: [\"A\"] }, |deps| deps[\"A\"].move(30, 0, 0));\n";
/// D = cylinder(10, 5) → V = π·5²·5 = 125π mm³, area = 2·π·5² + 2π·5·5 = 100π mm².
const PART_D: &str = "part(\"D\", #{}, |deps| cylinder(10, 5));\n";
/// E = box(10, 6, 20) moved to x ∈ [3, 13], y ∈ [−3, 3], z ∈ [−5, 15]: it
/// sticks through A without sharing a face. Common with A: x ∈ [3, 10],
/// y ∈ [−3, 3], z ∈ [0, 10] → 7·6·10 = 420 mm³.
const PART_E: &str = "part(\"E\", #{}, |deps| box(10, 6, 20).move(8, 0, -5));\n";
const VALUES: &str = "\
[a_size]
value = 20
source = \"test drawing\"
status = \"measured\"
";
const D_VOLUME: f64 = 125.0 * std::f64::consts::PI;
const D_AREA: f64 = 100.0 * std::f64::consts::PI;

/// One passing check on A (2000 mm³, see [`PART_A`]).
const CHECK_A: &str = "\
check(\"a_volume\", || {
    expect_near(part(\"A\").volume(), 2000, 0.001, \"volume of A\");
});
";
/// One check that fails whatever is built.
const CHECK_RED: &str = "\
check(\"always_red\", || {
    expect(1 > 2, \"one is greater than two\");
});
";

/// Checks that pass. `inclusive` pins the borders: |1.0 − 1.5| = 0.5 = tol
/// (exact in binary), 5 = min, 6 = max. `nan` fails: NaN compares false
/// with everything, so it is neither near nor between.
const CHECKS_GEOMETRY: &str = "\
check(\"a_volume\", || {
    let a = part(\"A\");
    expect_near(a.volume(), 2000, 0.001, \"volume of A\");
    expect_between(a.bbox().size[0], 19.9, 20.1, \"width of A\");
    expect(a.is_valid(), \"A is valid\");
    expect_near(a.bbox().size[0], value(\"a_size\"), 1e-6, \"width equals a_size\");
});
check(\"spacing\", || {
    expect_near(distance(part(\"A\"), part(\"B\")), 10.0, 1e-6, \"gap between A and B\");
});
check(\"inclusive\", || {
    expect_near(1.0, 1.5, 0.5, \"a difference equal to tol passes\");
    expect_between(5, 5, 6, \"min is inclusive\");
    expect_between(6.0, 5.0, 6.0, \"max is inclusive\");
});
check(\"nan\", || {
    expect_near(sqrt(-1.0), 0.0, 1.0, \"NaN is never near\");
    expect_between(sqrt(-1.0), -1.0, 1.0, \"NaN is never between\");
});
";

/// Checks that fail. `broken`: line 10 is `    let x = box(0, 1, 1);`, 4
/// spaces + `let x = ` = 12 characters before `box` → column 13 (1-based).
const CHECKS_FAILING: &str = "\
check(\"wrong\", || {
    let d = part(\"D\");
    expect_near(d.volume(), 100.0, 0.5, \"D has 100 mm3\");
    expect(true, \"still evaluated\");
    expect_between(d.bbox().size[2], 6, 7, \"D is 6 to 7 high\");
    expect(1 > 2, \"one is greater than two\");
});
check(\"broken\", || {
    expect(true, \"before the error\");
    let x = box(0, 1, 1);
    expect(true, \"never reached\");
});
check(\"ghost\", || {
    expect_near(part(\"Ghost\").volume(), 1.0, 0.1, \"ghost volume\");
});
";

/// Runs until the time limit: 16 B doubled 14 times = 256 KiB, each loop
/// iteration copies 512 KiB with a handful of Rhai operations, so the
/// operation limit (AG-SC-02, 50 M) is minutes away (same as tests/service.rs).
const ENDLESS_BODY: &str = "\
    let s = \"0123456789abcdef\";
    for i in 0..14 { s += s; }
    loop { let t = s + s; }
";

/// A, B, D, E with values: the parts of the read-only tests.
fn abde(name: &str) -> Project {
    let p = Project::new(name, "[workers]\ncount = 2\n");
    p.write("parts/a.rhai", PART_A)
        .write("parts/b.rhai", PART_B)
        .write("parts/d.rhai", PART_D)
        .write("parts/e.rhai", PART_E)
        .write("values/dims.toml", VALUES);
    p
}

/// A with its value and the passing check `a_volume`.
fn a_with_check(name: &str) -> Project {
    let p = Project::new(name, "");
    p.write("parts/a.rhai", PART_A)
        .write("values/dims.toml", VALUES)
        .write("checks/geometry.rhai", CHECK_A);
    p
}

// ===========================================================================
// Request helpers
// ===========================================================================

fn get(s: &Service, path: &str) -> Value {
    s.request("GET", path, None).envelope()
}

fn post(s: &Service, path: &str, body: &Value) -> Value {
    s.request("POST", path, Some(body)).envelope()
}

fn post_ok(s: &Service, path: &str, body: &Value) -> Value {
    s.request("POST", path, Some(body)).ok()
}

fn status(s: &Service) -> Value {
    s.request("GET", "/status", None).ok()
}

/// `POST /build` of all parts, waiting for the result (§3.1).
fn build_all(s: &Service) -> Value {
    let v = post(s, "/build", &json!({"parts": null, "wait_s": WAIT_S}));
    assert!(v["result"]["parts"].is_object(), "no build result: {v}");
    v["result"].clone()
}

/// `POST /check` (AG-CK-03), waiting for the result; `{checks, passed, failed}`.
fn run_checks(s: &Service, names: Option<&[&str]>) -> Value {
    let v = post(s, "/check", &json!({"names": names, "wait_s": WAIT_S}));
    assert!(v["result"]["checks"].is_array(), "no check result: {v}");
    v["result"].clone()
}

/// `{name: entry}` of a JSON list of objects with `name`.
fn by_name(list: &Value) -> BTreeMap<String, Value> {
    list.as_array()
        .unwrap_or_else(|| panic!("expected a list: {list}"))
        .iter()
        .map(|e| (e["name"].as_str().unwrap_or_default().to_owned(), e.clone()))
        .collect()
}

/// `{name: state}` of `/status` `result.<list>` (`parts` or `checks`).
fn states(status: &Value, list: &str) -> BTreeMap<String, String> {
    by_name(&status[list])
        .into_iter()
        .map(|(n, e)| (n, e["state"].as_str().unwrap_or_default().to_owned()))
        .collect()
}

fn check_state(s: &Service, name: &str) -> String {
    let st = status(s);
    states(&st, "checks").get(name).cloned().unwrap_or_else(|| panic!("no check {name}: {st}"))
}

/// The answer is an error envelope whose text names every word.
fn assert_refused(v: &Value, words: &[&str], what: &str) {
    assert_eq!(v["ok"], json!(false), "{what} must be refused: {v}");
    let text = v.to_string();
    for w in words {
        assert!(text.contains(w), "{what}: the error must name {w}: {v}");
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

// ===========================================================================
// AG-CK-01..03: checks
// ===========================================================================

#[test]
fn ag_ck_01_02_03_checks_run_record_expectations_and_report_states() {
    let p = abde("checks");
    p.write("checks/geometry.rhai", CHECKS_GEOMETRY).write("checks/failing.rhai", CHECKS_FAILING);
    let s = Service::start(&p);

    // AG-CK-03: declared checks are known before any run. Discovery runs
    // in a worker (ADR-0056): wait until it has seen every file.
    s.wait_discovery();
    let st = status(&s);
    let all = ["a_volume", "spacing", "inclusive", "nan", "wrong", "broken", "ghost"];
    let declared = states(&st, "checks");
    assert_eq!(declared.keys().map(String::as_str).collect::<BTreeSet<_>>(), BTreeSet::from(all));
    assert!(declared.values().all(|v| v == "never_run"), "{st}");
    assert_eq!(by_name(&st["checks"])["spacing"]["file"], json!("checks/geometry.rhai"), "{st}");
    assert_eq!(by_name(&st["checks"])["wrong"]["file"], json!("checks/failing.rhai"), "{st}");

    // AG-CK-03: nothing is built yet; /check builds the stale parts first.
    let r = run_checks(&s, None);
    assert!(states(&status(&s), "parts").values().all(|v| v == "current"), "parts built first");
    assert_eq!((r["passed"].as_u64(), r["failed"].as_u64()), (Some(3), Some(4)), "{r}");
    let checks = by_name(&r["checks"]);
    assert_eq!(checks.len(), 7, "{r}");
    check_passing(&checks);
    check_failing(&checks);
    check_script_errors(&checks);

    let st = states(&status(&s), "checks");
    for name in ["a_volume", "spacing", "inclusive"] {
        assert_eq!(st[name], "passed", "{st:?}");
    }
    for name in ["nan", "wrong", "broken", "ghost"] {
        assert_eq!(st[name], "failed", "{st:?}");
    }
    check_selection_and_cli(&s, &p);
    check_stale(&s, &p);
}

/// One recorded expectation `{text, passed, actual, expected, tol}` (AG-CK-02).
fn expectation<'a>(check: &'a Value, index: usize, text: &str, passed: bool) -> &'a Value {
    let e = &check["expectations"][index];
    assert_eq!(e["text"], json!(text), "expectation {index} of {check}");
    assert_eq!(e["passed"], json!(passed), "{text}: {e}");
    e
}

/// AG-CK-01/02: passing checks with `part`, `value` and the measure functions.
fn check_passing(checks: &BTreeMap<String, Value>) {
    let a = &checks["a_volume"];
    assert_eq!((&a["state"], &a["file"]), (&json!("passed"), &json!("checks/geometry.rhai")));
    assert!(a["duration_s"].as_f64().is_some_and(|d| d >= 0.0), "{a}");
    assert!(a.get("error").is_none_or(Value::is_null), "{a}");
    assert_eq!(a["expectations"].as_array().map(Vec::len), Some(4), "{a}");
    // A = box(20, 10, 10): 2000 mm³, 20 mm wide.
    let e = expectation(a, 0, "volume of A", true);
    assert_rel(&e["actual"], 2000.0, VOL, "actual volume of A");
    assert_eq!((f(&e["expected"]), f(&e["tol"])), (2000.0, 0.001), "{e}");
    // expect_between records its bounds as expected = [min, max], tol = null.
    let e = expectation(a, 1, "width of A", true);
    assert!((f(&e["actual"]) - 20.0).abs() < POS, "{e}");
    assert_eq!((f(&e["expected"][0]), f(&e["expected"][1])), (19.9, 20.1), "{e}");
    assert!(e["tol"].is_null(), "{e}");
    // expect records the condition: actual = cond, expected = true, tol = null.
    let e = expectation(a, 2, "A is valid", true);
    assert_eq!((&e["actual"], &e["expected"]), (&json!(true), &json!(true)), "{e}");
    // value("a_size") = 20 is available in checks.
    let e = expectation(a, 3, "width equals a_size", true);
    assert_eq!(f(&e["expected"]), 20.0, "{e}");

    // A x ≤ 10, B x ≥ 20 → 10 mm apart.
    let gap = expectation(&checks["spacing"], 0, "gap between A and B", true);
    assert!((f(&gap["actual"]) - 10.0).abs() < POS, "{gap}");

    let inc = &checks["inclusive"];
    assert_eq!(inc["state"], json!("passed"), "borders are inclusive: {inc}");
    expectation(inc, 0, "a difference equal to tol passes", true);
    expectation(inc, 1, "min is inclusive", true);
    expectation(inc, 2, "max is inclusive", true);
}

/// AG-CK-02: failed expectations are recorded and do not stop the check.
fn check_failing(checks: &BTreeMap<String, Value>) {
    let w = &checks["wrong"];
    assert_eq!(w["state"], json!("failed"), "{w}");
    assert!(w.get("error").is_none_or(Value::is_null), "failed expectations are no error: {w}");
    assert_eq!(w["expectations"].as_array().map(Vec::len), Some(4), "all four ran: {w}");
    // D = cylinder(10, 5): V = 125π ≈ 392.699, not 100 ± 0.5.
    let e = expectation(w, 0, "D has 100 mm3", false);
    assert_rel(&e["actual"], D_VOLUME, VOL, "actual volume of D");
    assert_eq!((f(&e["expected"]), f(&e["tol"])), (100.0, 0.5), "{e}");
    expectation(w, 1, "still evaluated", true);
    // D is 5 high, not 6..7.
    let e = expectation(w, 2, "D is 6 to 7 high", false);
    assert!((f(&e["actual"]) - 5.0).abs() < POS, "{e}");
    assert_eq!((f(&e["expected"][0]), f(&e["expected"][1])), (6.0, 7.0), "{e}");
    let e = expectation(w, 3, "one is greater than two", false);
    assert_eq!(e["actual"], json!(false), "{e}");

    let nan = &checks["nan"];
    assert_eq!(nan["state"], json!("failed"), "{nan}");
    expectation(nan, 0, "NaN is never near", false);
    expectation(nan, 1, "NaN is never between", false);
}

/// AG-CK-02: a script error fails the check with file, line, column;
/// AG-CK-01: `part` of an unknown part is an error.
fn check_script_errors(checks: &BTreeMap<String, Value>) {
    let b = &checks["broken"];
    assert_eq!(b["state"], json!("failed"), "{b}");
    let e = &b["error"];
    assert_eq!(e["kind"], json!("script"), "{b}");
    assert_eq!(e["file"], json!("checks/failing.rhai"), "{b}");
    assert_eq!((e["line"].as_u64(), e["column"].as_u64()), (Some(10), Some(13)), "{b}");
    assert!(e["message"].as_str().is_some_and(|m| !m.is_empty()), "{b}");
    let texts = b["expectations"].to_string();
    assert!(!texts.contains("never reached"), "the error stops the check: {b}");

    let g = &checks["ghost"];
    assert_eq!(g["state"], json!("failed"), "{g}");
    assert!(g["error"]["message"].as_str().is_some_and(|m| m.contains("Ghost")), "{g}");
    assert_eq!(g["error"]["file"], json!("checks/failing.rhai"), "{g}");
    assert_eq!(g["error"]["line"].as_u64(), Some(14), "{g}");
}

/// AG-CK-03: `names` selects checks; CLI `qoob3d check [NAMES..]` exits 1
/// iff a check failed.
fn check_selection_and_cli(s: &Service, p: &Project) {
    let r = run_checks(s, Some(&["spacing"]));
    assert_eq!(by_name(&r["checks"]).keys().collect::<Vec<_>>(), ["spacing"], "{r}");
    assert_eq!((r["passed"].as_u64(), r["failed"].as_u64()), (Some(1), Some(0)), "{r}");
    let v = post(s, "/check", &json!({"names": ["nope"], "wait_s": WAIT_S}));
    assert_refused(&v, &["nope"], "an unknown check name");

    let out = cli(&p.dir, &["check", "a_volume", "spacing"], CLI);
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    let r = &out.envelope()["result"];
    assert_eq!((r["passed"].as_u64(), r["failed"].as_u64()), (Some(2), Some(0)), "{r}");
    let out = cli(&p.dir, &["check"], CLI);
    assert_eq!(out.code, Some(1), "failed checks give exit 1: {}", out.stdout);
    let r = &out.envelope()["result"];
    assert_eq!((r["passed"].as_u64(), r["failed"].as_u64()), (Some(3), Some(4)), "{r}");
    let out = cli(&p.dir, &["check", "wrong"], CLI);
    assert_eq!(out.code, Some(1), "{}", out.stdout);
}

/// AG-CK-03: `stale` after a part was rebuilt or the check file changed.
fn check_stale(s: &Service, p: &Project) {
    assert_eq!(check_state(s, "wrong"), "failed");
    // D becomes cylinder(10, 6): rebuilt, so the check that measured D is stale.
    p.write("parts/d.rhai", &PART_D.replace("cylinder(10, 5)", "cylinder(10, 6)"));
    let r = build_all(s);
    assert!(r["built"].to_string().contains("\"D\""), "D was rebuilt: {r}");
    assert_eq!(check_state(s, "wrong"), "stale");
    let r = run_checks(s, Some(&["wrong"]));
    let w = &by_name(&r["checks"])["wrong"];
    // Height is now 6: inside [6, 7]. The volume 150π is still not 100.
    let e = expectation(w, 2, "D is 6 to 7 high", true);
    assert!((f(&e["actual"]) - 6.0).abs() < POS, "the check saw the rebuilt part: {e}");
    assert_eq!(check_state(s, "wrong"), "failed");

    // A changed check file makes its checks stale without a run.
    assert_eq!(check_state(s, "spacing"), "passed");
    p.write("checks/geometry.rhai", &format!("{CHECKS_GEOMETRY}// changed\n"));
    s.wait_discovery();
    assert_eq!(check_state(s, "spacing"), "stale");
    run_checks(s, Some(&["spacing"]));
    assert_eq!(check_state(s, "spacing"), "passed");
}

#[test]
fn ag_ck_01_duplicate_check_names_are_an_error_naming_the_check() {
    // "names are unique in the project": the same name in two files.
    let p = a_with_check("check-duplicate");
    p.write("checks/again.rhai", CHECK_A);
    let s = Service::start(&p);
    let v = post(&s, "/check", &json!({"names": null, "wait_s": WAIT_S}));
    let failed = v["result"]["failed"].as_u64().is_some_and(|n| n > 0);
    assert!(v["ok"] == json!(false) || failed, "duplicate reported as success: {v}");
    assert!(v.to_string().contains("a_volume"), "the error names the check: {v}");
    assert_ne!(v["result"]["passed"].as_u64(), Some(2), "both duplicates ran: {v}");
}

#[test]
fn ag_ck_01_a_check_on_a_failed_part_fails_naming_the_part() {
    // `part("Name")` is an error if the part is not built and current.
    let p = Project::new("check-failed-part", "");
    p.write("parts/bad.rhai", "part(\"Bad\", #{}, |deps| box(0, 1, 1));\n").write(
        "checks/bad.rhai",
        "check(\"uses_bad\", || {\n    expect(part(\"Bad\").is_valid(), \"bad is valid\");\n});\n",
    );
    let s = Service::start(&p);
    let r = run_checks(&s, None);
    assert_eq!((r["passed"].as_u64(), r["failed"].as_u64()), (Some(0), Some(1)), "{r}");
    let c = &by_name(&r["checks"])["uses_bad"];
    assert_eq!(c["state"], json!("failed"), "{c}");
    assert!(c["error"]["message"].as_str().is_some_and(|m| m.contains("Bad")), "{c}");
    assert_eq!(check_state(&s, "uses_bad"), "failed");
}

/// Polls `GET /job/<id>` until a final state (§3.1); the job.
fn wait_job(s: &Service, id: &Value, timeout: Duration) -> Value {
    let id = id.as_str().map_or_else(|| id.to_string(), str::to_owned);
    let mut job = Value::Null;
    let finished = wait_until(timeout, || {
        job = s.request("GET", &format!("/job/{id}"), None).ok();
        ["done", "failed", "cancelled"].contains(&job["state"].as_str().unwrap_or_default())
    });
    assert!(finished, "job {id} not finished after {timeout:?}: {job}");
    job
}

#[test]
fn ag_ck_03_ag_sv_05_checks_and_throwaway_scripts_obey_the_time_limit() {
    // "runs the checks in a worker under the project limits": time_s = 2.
    let p = Project::new("check-limit", "[limits]\ntime_s = 2\n[workers]\ncount = 1\n");
    p.write("parts/a.rhai", PART_A)
        .write("values/dims.toml", VALUES)
        .write("checks/geometry.rhai", CHECK_A)
        .write("checks/slow.rhai", &format!("check(\"endless\", || {{\n{ENDLESS_BODY}}});\n"));
    let s = Service::start(&p);
    build_all(&s);

    // wait_s = 0 answers at once with a job, as /build does (§3.1).
    let t0 = Instant::now();
    let r = post_ok(&s, "/check", &json!({"names": ["endless"], "wait_s": 0}));
    assert!(r["job"].is_u64() || r["job"].is_string(), "wait_s 0 returns a job: {r}");
    // The service answers while the check runs (AG-SV-01).
    assert!(status(&s)["checks"].is_array());
    let job = wait_job(&s, &r["job"], Duration::from_secs(15));
    let took = t0.elapsed();
    assert!(took >= Duration::from_secs(2), "stopped before the limit: {took:?}");
    assert!(took <= Duration::from_secs(6), "stopped too late: {took:?}");
    let c = &by_name(&job["result"]["checks"])["endless"];
    assert_eq!(c["state"], json!("failed"), "{job}");
    assert_eq!(c["error"]["kind"], json!("time_limit"), "{job}");
    assert_eq!(check_state(&s, "endless"), "failed");

    // The killed worker was replaced: other checks still run.
    let r = run_checks(&s, Some(&["a_volume"]));
    assert_eq!((r["passed"].as_u64(), r["failed"].as_u64()), (Some(1), Some(0)), "{r}");

    // AG-SV-05: a throwaway script runs in a worker under the same limit.
    let v = post(&s, "/run", &json!({"source": ENDLESS_BODY, "wait_s": 30}));
    assert_eq!(v["ok"], json!(false), "{v}");
    assert_eq!(v["error"]["kind"], json!("time_limit"), "{v}");
    let r = run_checks(&s, Some(&["a_volume"]));
    assert_eq!(r["passed"].as_u64(), Some(1), "{r}");
}

// ===========================================================================
// AG-SV-04: tasks over HTTP
// ===========================================================================

/// A task file as a human writes it (AG-TK-01), with an explicit state line.
const HAND_MADE: &str = "\
+++
title = \"Hand-made plate\"
kind = \"requirement\"
state = \"open\"
parts = [\"A\"]
checks = [\"a_volume\"]
created = \"2026-11-20\"
+++

The plate must be 20 mm wide.
Keep   this line – byte for byte.
";
const HAND_MADE_FILE: &str = "tasks/0001-hand-made-plate.md";

fn task(s: &Service, id: &str) -> Value {
    s.request("GET", &format!("/task/{id}"), None).ok()
}

fn task_state(s: &Service, id: &str) -> String {
    task(s, id)["state"].as_str().unwrap_or_default().to_owned()
}

/// `POST /task/<id>`; the envelope.
fn update(s: &Service, id: &str, body: &Value) -> Value {
    post(s, &format!("/task/{id}"), body)
}

/// An update that must succeed and leave the task in `state`.
fn update_ok(s: &Service, id: &str, body: &Value, state: &str) {
    let v = update(s, id, body);
    assert_eq!(v["ok"], json!(true), "{id} {body}: {v}");
    assert_eq!(v["result"]["state"], json!(state), "{id} {body}: {v}");
    assert_eq!(task_state(s, id), state);
}

/// An update that must be refused, name `words` and leave the state alone.
fn update_refused(s: &Service, id: &str, body: &Value, words: &[&str]) {
    let before = task_state(s, id);
    let v = update(s, id, body);
    assert_refused(&v, words, &format!("{id} {body}"));
    assert_eq!(task_state(s, id), before, "a refused update changed {id}");
}

/// `YYYY-MM-DD` without looking at the clock.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter()
            .enumerate()
            .all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

/// The task file has a log line `- <date> <who>: <text>` (AG-TK-01).
fn has_log_line(file: &str, who: &str, text: &str) -> bool {
    let log = file.split_once("\n## Log").map(|(_, l)| l).unwrap_or_default();
    log.lines().any(|l| {
        let Some((date, entry)) = l.strip_prefix("- ").and_then(|r| r.split_once(' ')) else {
            return false;
        };
        is_date(date) && entry == format!("{who}: {text}")
    })
}

#[test]
fn ag_sv_04_tasks_are_listed_created_and_shown() {
    let p = a_with_check("tasks");
    p.write(HAND_MADE_FILE, HAND_MADE);
    let s = Service::start(&p);

    let list = get(&s, "/tasks")["result"].clone();
    assert_eq!(list.as_array().map(Vec::len), Some(1), "{list}");
    let expected = json!({
        "id": "T-0001", "title": "Hand-made plate", "kind": "requirement", "state": "open",
        "parts": ["A"], "checks": ["a_volume"], "file": HAND_MADE_FILE,
    });
    for (key, value) in expected.as_object().expect("object") {
        assert_eq!(&list[0][key], value, "{key} of {list}");
    }
    let shown = task(&s, "T-0001");
    let body = shown["body"].as_str().unwrap_or_default();
    assert!(body.contains("Keep   this line – byte for byte."), "{shown}");
    assert!(!body.contains("+++") && !body.contains("title ="), "body without header: {shown}");
    assert_eq!(shown["log"].as_array().map(Vec::len), Some(0), "no log yet: {shown}");
    assert_eq!(shown["title"], json!("Hand-made plate"), "{shown}");

    // Create: next number 0002, slug from the title (ASCII lowercase, other → '-').
    let body = json!({"title": "Lid for the plate", "kind": "idea", "body": "Maybe later.",
        "parts": ["A"], "checks": ["a_volume"]});
    let t = post_ok(&s, "/task", &body);
    assert_eq!(t["id"], json!("T-0002"), "{t}");
    assert_eq!(t["file"], json!("tasks/0002-lid-for-the-plate.md"), "{t}");
    assert_eq!((&t["kind"], &t["state"]), (&json!("idea"), &json!("open")), "{t}");
    assert_eq!((&t["parts"], &t["checks"]), (&json!(["A"]), &json!(["a_volume"])), "{t}");
    let file = p.read("tasks/0002-lid-for-the-plate.md");
    assert!(file.starts_with("+++\n"), "TOML header between +++ lines:\n{file}");
    assert!(file.contains("title = \"Lid for the plate\""), "{file}");
    assert!(file.contains("Maybe later."), "{file}");
    assert!(task(&s, "T-0002")["body"].as_str().is_some_and(|b| b.contains("Maybe later.")));

    // Defaults: kind task, no parts, no checks.
    let t = post_ok(&s, "/task", &json!({"title": "Default kind"}));
    assert_eq!((&t["id"], &t["kind"]), (&json!("T-0003"), &json!("task")), "{t}");
    assert_eq!((&t["parts"], &t["checks"]), (&json!([]), &json!([])), "{t}");

    // Invalid input creates nothing.
    assert_refused(&post(&s, "/task", &json!({"title": ""})), &[], "an empty title");
    assert_refused(&post(&s, "/task", &json!({"title": "   "})), &[], "a blank title");
    let v = post(&s, "/task", &json!({"title": "Wish", "kind": "wish"}));
    assert_refused(&v, &["wish"], "an unknown kind");
    assert_refused(&post(&s, "/task", &json!({})), &[], "a task without title");
    let ids: Vec<Value> = get(&s, "/tasks")["result"]
        .as_array()
        .map(|l| l.iter().map(|t| t["id"].clone()).collect())
        .unwrap_or_default();
    assert_eq!(ids, [json!("T-0001"), json!("T-0002"), json!("T-0003")], "sorted by id");
    assert_eq!(std::fs::read_dir(p.path("tasks")).expect("tasks/").count(), 3);

    assert_refused(&get(&s, "/task/T-0099"), &["T-0099"], "an unknown task");
    let v = update(&s, "T-0099", &json!({"state": "in_progress", "role": "agent"}));
    assert_refused(&v, &["T-0099"], "updating an unknown task");

    // AG-TK-03: a file with a bad name is a load error naming the file.
    p.write("tasks/notes.md", "no task\n");
    assert_refused(&get(&s, "/tasks"), &["notes.md"], "a task file with a bad name");
}

#[test]
fn ag_sv_04_task_updates_follow_the_role_rules() {
    // AG-TK-02 through the service; the gate has its own test below.
    let p = a_with_check("task-roles");
    p.write(HAND_MADE_FILE, HAND_MADE);
    let s = Service::start(&p);
    let id = "T-0001";

    // `role` is required: without it nobody may be taken for the human.
    update_refused(&s, id, &json!({"state": "in_progress"}), &[]);
    update_refused(&s, id, &json!({"state": "in_progress", "role": "robot"}), &[]);
    update_refused(&s, id, &json!({"state": "finished", "role": "human"}), &["finished"]);

    // Agent, forbidden: the error names task, states and role.
    for state in ["done", "rejected", "review", "blocked"] {
        let body = json!({"state": state, "role": "agent"});
        update_refused(&s, id, &body, &[id, "open", state, "agent"]);
    }
    update_ok(
        &s,
        id,
        &json!({"state": "in_progress", "role": "agent", "note": "starting"}),
        "in_progress",
    );

    // AG-TK-03: only the state line changes, the log entry is appended.
    let file = p.read(HAND_MADE_FILE);
    let expected = HAND_MADE.replace("state = \"open\"", "state = \"in_progress\"");
    assert!(file.starts_with(expected.trim_end()), "header and body byte-identical:\n{file}");
    // AG-TK-07: `<old> -> <new>: <note>`.
    assert!(
        has_log_line(&file, "agent", "open -> in_progress: starting"),
        "log line `- <date> agent: open -> in_progress: starting`:\n{file}"
    );

    // blocked needs the question.
    update_refused(&s, id, &json!({"state": "blocked", "role": "agent"}), &[]);
    let ask = json!({"state": "blocked", "role": "agent", "note": "Which wall thickness?"});
    update_ok(&s, id, &ask, "blocked");
    assert!(task(&s, id)["log"].to_string().contains("Which wall thickness?"));
    for state in ["review", "done", "rejected", "open"] {
        let body = json!({"state": state, "role": "agent", "note": "n"});
        update_refused(&s, id, &body, &[id, "blocked", state, "agent"]);
    }
    update_ok(&s, id, &json!({"state": "in_progress", "role": "agent"}), "in_progress");
    for state in ["done", "rejected", "open"] {
        let body = json!({"state": state, "role": "agent", "note": "n"});
        update_refused(&s, id, &body, &[id, "in_progress", state, "agent"]);
    }

    // A note without a state only appends to the log.
    update_ok(&s, id, &json!({"note": "measured twice", "role": "agent"}), "in_progress");
    let file = p.read(HAND_MADE_FILE);
    assert!(has_log_line(&file, "agent", "measured twice"), "{file}");
    assert!(file.starts_with(expected.trim_end()), "a note changes only the log:\n{file}");
    let log = task(&s, id)["log"].clone();
    assert!(log.as_array().is_some_and(|l| l.len() >= 3), "starting, question, note: {log}");

    // Human: every transition, including done and rejected.
    update_ok(&s, id, &json!({"state": "done", "role": "human", "note": "accepted"}), "done");
    let file = p.read(HAND_MADE_FILE);
    assert!(has_log_line(&file, "human", "in_progress -> done: accepted"), "{file}");
    // Without a note the entry is the arrow alone.
    assert!(has_log_line(&file, "agent", "blocked -> in_progress"), "{file}");
    update_refused(
        &s,
        id,
        &json!({"state": "in_progress", "role": "agent"}),
        &[id, "done", "agent"],
    );
    update_ok(&s, id, &json!({"state": "rejected", "role": "human"}), "rejected");
    update_ok(&s, id, &json!({"state": "open", "role": "human"}), "open");
    let listed = get(&s, "/tasks")["result"][0]["state"].clone();
    assert_eq!(listed, json!("open"), "/tasks shows the state of the file");
}

/// Creates a task and moves it to `in_progress` as the agent; its id.
fn task_in_progress(s: &Service, title: &str, parts: &[&str], checks: &[&str]) -> String {
    let t = post_ok(s, "/task", &json!({"title": title, "parts": parts, "checks": checks}));
    let id = t["id"].as_str().unwrap_or_else(|| panic!("task id: {t}")).to_owned();
    update_ok(s, &id, &json!({"state": "in_progress", "role": "agent"}), "in_progress");
    id
}

/// A refused hand-over: kind `gate`, every unmet condition named.
fn gate_refused(s: &Service, id: &str, words: &[&str]) {
    let v = update(s, id, &json!({"state": "review", "role": "agent", "note": "please review"}));
    assert_refused(&v, words, &format!("review of {id}"));
    assert_eq!(v["error"]["kind"], json!("gate"), "{v}");
    assert_eq!(task_state(s, id), "in_progress", "a refused hand-over changed {id}");
}

#[test]
fn ag_sv_04_review_needs_current_parts_and_passed_checks() {
    // AG-TK-04, evaluated by the service from its own part and check states.
    // Plate = box(20, 10, 10) = 2000 mm³; names that no error text contains by chance.
    let plate = "part(\"Plate\", #{}, |deps| box(20, 10, 10));\n";
    let check = "check(\"plate_volume\", || {\n    \
                 expect_near(part(\"Plate\").volume(), 2000, 0.001, \"volume\");\n});\n";
    let p = Project::new("task-gate", "");
    p.write("parts/plate.rhai", plate)
        .write("checks/plate.rhai", check)
        .write("checks/red.rhai", CHECK_RED);
    let s = Service::start(&p);
    let review = json!({"state": "review", "role": "agent"});

    // Nothing built, nothing run: both conditions are reported together.
    let id = task_in_progress(&s, "First hand-over", &["Plate"], &["plate_volume"]);
    gate_refused(&s, &id, &["Plate", "plate_volume"]);
    build_all(&s);
    gate_refused(&s, &id, &["plate_volume"]);
    run_checks(&s, None);
    update_ok(&s, &id, &review, "review");

    // The agent may take it back; a changed part closes the gate again.
    update_ok(&s, &id, &json!({"state": "in_progress", "role": "agent"}), "in_progress");
    p.write("parts/plate.rhai", &format!("{plate}// changed\n"));
    s.wait_discovery();
    assert_eq!(states(&status(&s), "parts")["Plate"], "outdated");
    gate_refused(&s, &id, &["Plate"]);
    // Rebuilt, but the check has not run after the last build.
    build_all(&s);
    assert_eq!(check_state(&s, "plate_volume"), "stale");
    gate_refused(&s, &id, &["plate_volume"]);
    run_checks(&s, Some(&["plate_volume"]));
    update_ok(&s, &id, &review, "review");

    // A failed check, a check and a part that do not exist.
    let red = task_in_progress(&s, "Red", &["Plate"], &["always_red"]);
    gate_refused(&s, &red, &["always_red"]);
    let ghost = task_in_progress(&s, "Ghosts", &["Ghost"], &["no_such_check"]);
    gate_refused(&s, &ghost, &["Ghost", "no_such_check"]);

    // AG-TK-06: a task needs at least one part and one check bound; a note
    // does not replace them (`gate_refused` sends one). Both sides are green
    // where bound, so only the missing side closes the gate.
    let empty = task_in_progress(&s, "Just think", &[], &[]);
    gate_refused(&s, &empty, &["part", "check"]);
    let no_check = task_in_progress(&s, "Unchecked", &["Plate"], &[]);
    gate_refused(&s, &no_check, &["check"]);
    let no_part = task_in_progress(&s, "Partless", &[], &["plate_volume"]);
    gate_refused(&s, &no_part, &["part"]);

    // AG-TK-06: kind `idea` needs a note only.
    let t = post_ok(&s, "/task", &json!({"title": "Maybe a handle", "kind": "idea"}));
    let idea = t["id"].as_str().unwrap_or_else(|| panic!("task id: {t}")).to_owned();
    update_ok(&s, &idea, &json!({"state": "in_progress", "role": "agent"}), "in_progress");
    let v = update(&s, &idea, &review);
    assert_refused(&v, &["note"], "an idea handed over without a note");
    assert_eq!(v["error"]["kind"], json!("gate"), "{v}");
    let noted = json!({"state": "review", "role": "agent", "note": "thought about it"});
    update_ok(&s, &idea, &noted, "review");

    // The gate binds the agent; the human may make every transition (AG-TK-02).
    update_ok(&s, &red, &json!({"state": "review", "role": "human"}), "review");
}

#[test]
fn ag_sv_04_cli_task_commands_act_as_the_human() {
    let p = a_with_check("task-cli");
    assert!(service_json(&p.dir).is_none());
    let out = cli(
        &p.dir,
        &["task", "new", "Print a spacer", "--kind", "idea", "--body", "Ten millimetres."],
        CLI,
    );
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    let t = out.envelope()["result"].clone();
    assert_eq!((&t["id"], &t["kind"]), (&json!("T-0001"), &json!("idea")), "{t}");
    assert_eq!(t["file"], json!("tasks/0001-print-a-spacer.md"), "{t}");
    let s = Service::attach(&p);

    let out = cli(&p.dir, &["task", "new", "Second"], CLI);
    assert_eq!(out.envelope()["result"]["kind"], json!("task"), "default kind");
    let out = cli(&p.dir, &["task", "list"], CLI);
    assert_eq!(out.code, Some(0), "{}", out.stdout);
    let list = out.envelope()["result"].clone();
    assert_eq!(list.as_array().map(Vec::len), Some(2), "{list}");
    assert_eq!(list[0]["title"], json!("Print a spacer"), "{list}");

    let out = cli(&p.dir, &["task", "show", "T-0001"], CLI);
    assert_eq!(out.code, Some(0), "{}", out.stdout);
    let shown = out.envelope()["result"].clone();
    assert!(shown["body"].as_str().is_some_and(|b| b.contains("Ten millimetres.")), "{shown}");

    // The CLI is the human: open → done is allowed, with a log line by `human`.
    let out = cli(&p.dir, &["task", "set", "T-0001", "done", "--note", "printed and fits"], CLI);
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    assert_eq!(out.envelope()["result"]["state"], json!("done"));
    assert_eq!(task_state(&s, "T-0001"), "done");
    let file = p.read("tasks/0001-print-a-spacer.md");
    assert!(file.contains("state = \"done\""), "{file}");
    // AG-TK-07: `<old> -> <new>: <note>`.
    assert!(has_log_line(&file, "human", "open -> done: printed and fits"), "{file}");
    let out = cli(&p.dir, &["task", "set", "T-0002", "rejected"], CLI);
    assert_eq!(out.code, Some(0), "{}", out.stdout);

    // Failures: envelope with ok = false, exit 1.
    for args in [&["task", "set", "T-0001", "bogus"][..], &["task", "show", "T-0042"][..]] {
        let out = cli(&p.dir, args, CLI);
        assert_eq!(out.code, Some(1), "qoob3d {args:?}: {}", out.stdout);
        assert_eq!(out.envelope()["ok"], json!(false), "qoob3d {args:?}");
    }
    assert_eq!(task_state(&s, "T-0001"), "done");
}

// ===========================================================================
// AG-SV-05, AG-SV-06: throwaway scripts and measure on one built project
// ===========================================================================

/// Every file below `dir` with its bytes, relative paths.
fn tree(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            tree(&path, base, out);
        } else {
            let rel = path.strip_prefix(base).unwrap_or(&path).display().to_string();
            out.insert(rel, std::fs::read(&path).unwrap_or_default());
        }
    }
}

/// Project files and the cache; the rest of `.qoob3d/` (service.json, logs)
/// is the service's own business.
fn project_and_cache(p: &Project) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    tree(&p.dir, &p.dir, &mut files);
    files.retain(|rel, _| !rel.starts_with(".qoob3d") || rel.starts_with(".qoob3d/cache"));
    files
}

#[test]
fn ag_sv_05_ag_sv_06_run_and_measure_on_a_built_project() {
    let p = abde("probe");
    let s = Service::start(&p);
    let r = build_all(&s);
    assert_eq!(r["built"].as_array().map(Vec::len), Some(4), "{r}");
    check_run(&s, &p);
    check_run_errors(&s);
    check_measure(&s);
}

/// AG-SV-05: shapes with their measures, printed lines, nothing written.
fn check_run(s: &Service, p: &Project) {
    let before = project_and_cache(p);
    let source = "\
let a = part(\"A\");
print(\"first\");
show(a.move(0, 0, 5) + box(2, 2, 2).move(0, 0, 20), \"probe\");
show(cylinder(value(\"a_size\") / 2, 5), \"cyl\");
print(\"second\");
";
    let r = post_ok(s, "/run", &json!({"source": source, "wait_s": WAIT_S}));
    assert_eq!(r["printed"], json!(["first", "second"]), "{r}");
    let shapes = r["shapes"].as_array().unwrap_or_else(|| panic!("shapes: {r}"));
    assert_eq!(shapes.len(), 2, "{r}");

    // probe = A lifted to z ∈ [5, 15] (2000 mm³, area 1000 mm²) plus a
    // separate cube of 2 mm at z ∈ [20, 22] (8 mm³, area 24 mm²): two solids.
    let probe = &shapes[0];
    assert_eq!(probe["name"], json!("probe"), "shapes keep the call order: {r}");
    assert_rel(&probe["volume"], 2008.0, VOL, "volume of probe");
    assert_rel(&probe["area"], 1024.0, VOL, "area of probe");
    assert_vec(&probe["bbox"]["min"], [-10.0, -5.0, 5.0], POS, "probe bbox.min");
    assert_vec(&probe["bbox"]["max"], [10.0, 5.0, 22.0], POS, "probe bbox.max");
    assert_vec(&probe["bbox"]["size"], [20.0, 10.0, 17.0], POS, "probe bbox.size");
    // Centre of mass: z = (2000·10 + 8·21) / 2008.
    let cz = (2000.0 * 10.0 + 8.0 * 21.0) / 2008.0;
    assert_vec(&probe["center"], [0.0, 0.0, cz], POS, "probe center");
    assert_eq!((&probe["valid"], &probe["solids"]), (&json!(true), &json!(2)), "{probe}");

    // cyl = cylinder(20 / 2, 5) = D: 125π mm³, 100π mm², centre at z = 2.5.
    let cyl = &shapes[1];
    assert_eq!(cyl["name"], json!("cyl"), "{r}");
    assert_rel(&cyl["volume"], D_VOLUME, VOL, "volume of cyl");
    assert_rel(&cyl["area"], D_AREA, VOL, "area of cyl");
    assert_vec(&cyl["center"], [0.0, 0.0, 2.5], POS, "cyl center");
    assert_eq!((&cyl["valid"], &cyl["solids"]), (&json!(true), &json!(1)), "{cyl}");

    // A script without output is fine and empty.
    let r = post_ok(s, "/run", &json!({"source": "let x = 1 + 1;", "wait_s": WAIT_S}));
    assert_eq!((&r["shapes"], &r["printed"]), (&json!([]), &json!([])), "{r}");

    let after = project_and_cache(p);
    let (b, a): (Vec<_>, Vec<_>) = (before.keys().collect(), after.keys().collect());
    assert_eq!(b, a, "a throwaway script wrote or removed files");
    assert!(before == after, "a throwaway script changed project or cache files");
    assert!(states(&status(s), "parts").values().all(|v| v == "current"));
}

/// AG-SV-05: "or the script error", with the position in `source`.
fn check_run_errors(s: &Service) {
    // Line 2 is `let b = box(0, 1, 1);`: `box` at column 9.
    let source = "let a = 1;\nlet b = box(0, 1, 1);\nshow(b, \"b\");\n";
    let v = post(s, "/run", &json!({"source": source, "wait_s": WAIT_S}));
    assert_refused(&v, &[], "a script with a runtime error");
    assert_eq!(v["error"]["kind"], json!("script"), "{v}");
    let e = &v["error"];
    assert_eq!((e["line"].as_u64(), e["column"].as_u64()), (Some(2), Some(9)), "{v}");

    let v = post(s, "/run", &json!({"source": "let a = ;", "wait_s": WAIT_S}));
    assert_eq!(v["error"]["kind"], json!("script"), "syntax error: {v}");
    assert_eq!(v["error"]["line"].as_u64(), Some(1), "{v}");

    let v = post(s, "/run", &json!({"source": "show(part(\"Ghost\"), \"g\");", "wait_s": WAIT_S}));
    assert_refused(&v, &["Ghost"], "an unknown part in a throwaway script");
    let v = post(s, "/run", &json!({"source": "print(value(\"nope\"));", "wait_s": WAIT_S}));
    assert_refused(&v, &["nope"], "an unknown value in a throwaway script");
    assert_refused(&post(s, "/run", &json!({"wait_s": 1})), &[], "a run without source");
}

/// AG-SV-06: one part, two parts apart, two parts overlapping.
fn check_measure(s: &Service) {
    // A = box(20, 10, 10) (see PART_A).
    let a = post_ok(s, "/measure", &json!({"a": "A"}));
    assert_rel(&a["volume"], 2000.0, VOL, "volume of A");
    assert_rel(&a["area"], 1000.0, VOL, "area of A");
    assert_vec(&a["bbox"]["min"], [-10.0, -5.0, 0.0], POS, "A bbox.min");
    assert_vec(&a["bbox"]["max"], [10.0, 5.0, 10.0], POS, "A bbox.max");
    assert_vec(&a["bbox"]["size"], [20.0, 10.0, 10.0], POS, "A bbox.size");
    assert_vec(&a["center"], [0.0, 0.0, 5.0], POS, "A center");
    assert_eq!((&a["valid"], &a["solids"]), (&json!(true), &json!(1)), "{a}");
    assert!(a.get("distance").is_none_or(Value::is_null), "no distance without b: {a}");
    assert!(a.get("common_volume").is_none_or(Value::is_null), "{a}");

    let d = post_ok(s, "/measure", &json!({"a": "D"}));
    assert_rel(&d["volume"], D_VOLUME, VOL, "volume of D");
    assert_rel(&d["area"], D_AREA, VOL, "area of D");
    assert_vec(&d["center"], [0.0, 0.0, 2.5], POS, "D center");

    // A ends at x = 10, B starts at x = 20: 10 mm apart, nothing in common.
    let ab = post_ok(s, "/measure", &json!({"a": "A", "b": "B"}));
    assert_rel(&ab["volume"], 2000.0, VOL, "the measures of a stay");
    assert!((f(&ab["distance"]) - 10.0).abs() < POS, "{ab}");
    assert_eq!(ab["common_volume"].as_f64(), Some(0.0), "disjoint: {ab}");

    // E overlaps A with 7·6·10 = 420 mm³ (see PART_E): a finding, not an error.
    let ae = post_ok(s, "/measure", &json!({"a": "A", "b": "E"}));
    assert_eq!(ae["distance"].as_f64(), Some(0.0), "overlapping: {ae}");
    assert_rel(&ae["common_volume"], 420.0, VOL, "common volume of A and E");
    // Symmetric in a and b; the measures are those of a = E: 10·6·20 = 1200 mm³.
    let ea = post_ok(s, "/measure", &json!({"a": "E", "b": "A"}));
    assert_rel(&ea["volume"], 1200.0, VOL, "volume of E");
    assert_rel(&ea["common_volume"], 420.0, VOL, "common volume of E and A");

    let v = post(s, "/measure", &json!({"a": "Nope"}));
    assert_refused(&v, &["Nope"], "measuring an unknown part");
    let v = post(s, "/measure", &json!({"a": "A", "b": "Nope"}));
    assert_refused(&v, &["Nope"], "measuring against an unknown part");
    assert_refused(&post(s, "/measure", &json!({})), &[], "a measure without a");
}

// ===========================================================================
// AG-SV-07: save
// ===========================================================================

fn head(p: &Project) -> String {
    git(&p.dir, &["rev-parse", "HEAD"]).expect("HEAD exists")
}

/// A successful `/save`; checks hash and message, returns the full hash.
fn save_ok(s: &Service, p: &Project, message: &str) -> String {
    let r = post_ok(s, "/save", &json!({"message": message}));
    let commit = r["commit"].as_str().unwrap_or_else(|| panic!("result.commit: {r}"));
    let head = head(p);
    assert!(commit.len() >= 7 && head.starts_with(commit), "commit {commit} is HEAD {head}");
    assert_eq!(git(&p.dir, &["log", "-1", "--format=%s"]).as_deref(), Ok(message));
    head
}

#[test]
fn ag_sv_07_save_commits_only_a_green_project() {
    let p = a_with_check("save");
    p.write(HAND_MADE_FILE, HAND_MADE).write("lib/util.rhai", "fn one() { 1 }\n");
    let s = Service::start(&p);
    // out/A.stl and the cache exist, so they could be committed by mistake.
    post_ok(&s, "/export", &json!({"parts": ["A"], "format": "stl"}));
    assert!(p.path("out/A.stl").exists() && p.path(".qoob3d/cache").exists());

    // The temp folder is no repository (and has none above it).
    let v = post(&s, "/save", &json!({"message": "no repository"}));
    assert_refused(&v, &[], "save outside a Git repository");
    assert_eq!(v["error"]["kind"], json!("git"), "{v}");

    // Green: /save runs the checks itself (none was run before) and commits.
    git_init(&p.dir);
    let first = save_ok(&s, &p, "First state");
    assert_eq!(check_state(&s, "a_volume"), "passed", "save ran all checks");
    let tracked = git_tracked(&p.dir);
    for file in [
        "qoob3d.toml",
        "parts/a.rhai",
        "values/dims.toml",
        "checks/geometry.rhai",
        "lib/util.rhai",
        HAND_MADE_FILE,
    ] {
        assert!(tracked.iter().any(|t| t == file), "{file} not committed: {tracked:?}");
    }
    let leaked: Vec<_> =
        tracked.iter().filter(|t| t.starts_with(".qoob3d/") || t.starts_with("out/")).collect();
    assert!(leaked.is_empty(), ".qoob3d/ and out/ are never added: {leaked:?}");

    // A failed check: kind gate, the check is listed, nothing is committed.
    p.write("checks/red.rhai", CHECK_RED);
    let v = post(&s, "/save", &json!({"message": "red check"}));
    assert_refused(&v, &["always_red"], "save with a failed check");
    assert_eq!(v["error"]["kind"], json!("gate"), "{v}");
    assert_eq!(head(&p), first, "a refused save committed");
    assert!(!git_tracked(&p.dir).iter().any(|t| t == "checks/red.rhai"));

    // A failed part: the same.
    std::fs::remove_file(p.path("checks/red.rhai")).expect("remove red.rhai");
    p.write("parts/bad.rhai", "part(\"Bad\", #{}, |deps| box(0, 1, 1));\n");
    let v = post(&s, "/save", &json!({"message": "failed part"}));
    assert_refused(&v, &["Bad"], "save with a failed part");
    assert_eq!(v["error"]["kind"], json!("gate"), "{v}");
    assert_eq!(head(&p), first, "a refused save committed");

    // Green again with a change: a second commit on top of the first.
    std::fs::remove_file(p.path("parts/bad.rhai")).expect("remove bad.rhai");
    p.write("parts/a.rhai", &format!("{PART_A}// second state\n"));
    let second = save_ok(&s, &p, "Second state");
    assert_ne!(second, first);
    assert_eq!(git(&p.dir, &["rev-list", "--count", "HEAD"]).as_deref(), Ok("2"));
    assert_eq!(git(&p.dir, &["rev-parse", "HEAD~1"]).as_deref(), Ok(first.as_str()));
    let changed = git(&p.dir, &["show", "--name-only", "--format=", "HEAD"]).expect("git show");
    assert_eq!(changed.lines().collect::<Vec<_>>(), ["parts/a.rhai"], "only the change");
    assert_refused(&post(&s, "/save", &json!({})), &[], "a save without message");
}

// ===========================================================================
// AG-SV-08: init
// ===========================================================================

fn assert_skeleton(dir: &Path) {
    for sub in ["parts", "lib", "values", "checks", "tasks"] {
        assert!(dir.join(sub).is_dir(), "{sub}/ missing in {}", dir.display());
    }
    let toml = std::fs::read_to_string(dir.join("qoob3d.toml")).expect("qoob3d.toml");
    assert!(toml.contains("[project]") && toml.contains("name"), "AG-PJ-01:\n{toml}");
    assert!(toml.contains("units = \"mm\""), "AG-PJ-01:\n{toml}");
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).expect(".gitignore");
    let lines: Vec<&str> = ignore.lines().map(|l| l.trim().trim_start_matches('/')).collect();
    assert!(lines.contains(&".qoob3d/") && lines.contains(&"out/"), ".gitignore:\n{ignore}");
}

#[test]
fn ag_sv_08_init_creates_a_usable_skeleton_and_refuses_an_existing_project() {
    let p = Project::new("init", "");
    // A folder that does not exist yet.
    std::fs::remove_dir_all(&p.dir).expect("remove the prepared project");
    let dir = p.dir.to_str().expect("utf-8 temp path");
    let out = cli(&p.root, &["init", dir], CLI);
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    assert_skeleton(&p.dir);
    assert!(service_json(&p.dir).is_none(), "init needs no service");

    // The skeleton is a valid project for the real loader: status works.
    let out = cli(&p.dir, &["status"], CLI);
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    let st = out.envelope()["result"].clone();
    assert!(st["project"].is_string() && st["parts"].is_array(), "{st}");
    assert!(st.get("error").is_none_or(Value::is_null), "the skeleton loads cleanly: {st}");

    // A folder that already has qoob3d.toml is refused and left alone.
    p.write("parts/mine.rhai", PART_D);
    let toml = format!("{}# my own line\n", p.read("qoob3d.toml"));
    p.write("qoob3d.toml", &toml);
    let out = cli(&p.root, &["init", dir], CLI);
    assert_eq!(out.code, Some(1), "init over a project: {}{}", out.stdout, out.stderr);
    assert_eq!(p.read("qoob3d.toml"), toml, "a refused init changed qoob3d.toml");
    assert_eq!(p.read("parts/mine.rhai"), PART_D);

    // A folder with other content but no qoob3d.toml is not refused; what
    // is there stays.
    let other = p.root.join("existing");
    std::fs::create_dir_all(&other).expect("create folder");
    std::fs::write(other.join("README.md"), "my notes\n").expect("write README");
    std::fs::write(other.join(".gitignore"), "*.bak\n").expect("write .gitignore");
    let out = cli(&p.root, &["init", other.to_str().expect("utf-8")], CLI);
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    assert_skeleton(&other);
    assert_eq!(
        std::fs::read_to_string(other.join("README.md")).as_deref().ok(),
        Some("my notes\n")
    );
    let ignore = std::fs::read_to_string(other.join(".gitignore")).expect(".gitignore");
    assert!(ignore.contains("*.bak"), "an existing .gitignore keeps its lines:\n{ignore}");
}

// ===========================================================================
// AG-RD-01: render-project
// ===========================================================================

/// `qoob3d render-project <DIR> --out <root>/<name> <extra>`; exit code,
/// output and the PNG bytes if the file exists.
fn render_project(p: &Project, name: &str, extra: &[&str]) -> (support::CliOut, Option<Vec<u8>>) {
    let png = p.root.join(name);
    let dir = p.dir.to_str().expect("utf-8 temp path");
    let mut args = vec!["render-project", dir, "--out", png.to_str().expect("utf-8")];
    args.extend_from_slice(extra);
    // Working directory outside the project: DIR is the argument.
    let out = cli(&p.root, &args, RENDER);
    (out, std::fs::read(&png).ok())
}

/// A successful render: exit 0, a PNG of `size` that shows something.
fn rendered(p: &Project, name: &str, extra: &[&str], size: (u32, u32)) -> Vec<u8> {
    let (out, png) = render_project(p, name, extra);
    assert_eq!(out.code, Some(0), "render-project {extra:?}: {}{}", out.stdout, out.stderr);
    let png = png.unwrap_or_else(|| panic!("render-project {extra:?} wrote no PNG"));
    assert_eq!(png_size(&png), size, "render-project {extra:?}");
    assert!(png_colors(&png) > 1, "render-project {extra:?}: the image is one flat colour");
    png
}

#[test]
fn ag_rd_01_render_project_builds_and_renders_views_sizes_and_parts() {
    let p = abde("render");
    // Far away from the others: including it must change the fitted camera.
    p.write("parts/far.rhai", "part(\"Far\", #{}, |deps| box(5, 5, 5).move(500, 0, 0));\n");
    assert!(service_json(&p.dir).is_none());

    // No service, nothing built: started on demand, stale parts built first.
    let small = ["--size", "320x240"];
    let iso = rendered(&p, "iso.png", &small, (320, 240));
    let s = Service::attach(&p);
    let st = status(&s);
    assert_eq!(states(&st, "parts").len(), 5, "{st}");
    assert!(states(&st, "parts").values().all(|v| v == "current"), "built first: {st}");

    // Defaults: view iso, size 1024x768.
    let explicit = rendered(&p, "iso2.png", &["--size", "320x240", "--view", "iso"], (320, 240));
    assert!(iso == explicit, "the default view is iso (and renders are reproducible)");
    rendered(&p, "default.png", &["--parts", "A,B"], (1024, 768));

    // Views and part selections change the picture.
    let front = rendered(
        &p,
        "front.png",
        &[&small[..], &["--view", "front", "--parts", "A"]].concat(),
        (320, 240),
    );
    let top = rendered(
        &p,
        "top.png",
        &[&small[..], &["--view", "top", "--parts", "A"]].concat(),
        (320, 240),
    );
    assert!(front != top, "front and top view of a 20×10×10 box differ");
    assert!(front != iso, "a selection differs from all parts");
    // The camera fits the selected parts: with Far (480 mm away) in the
    // selection the same view of A cannot look the same.
    let wide = rendered(
        &p,
        "wide.png",
        &[&small[..], &["--view", "front", "--parts", "A,Far"]].concat(),
        (320, 240),
    );
    assert!(front != wide, "the camera is fitted to the selected parts");

    // Unknown part: exit 1 naming it.
    let (out, _) = render_project(&p, "nope.png", &["--parts", "A,Nope"]);
    assert_eq!(out.code, Some(1), "{}{}", out.stdout, out.stderr);
    assert!(format!("{}{}", out.stdout, out.stderr).contains("Nope"), "the error names the part");
    // Invalid view and size are rejected, not replaced by defaults.
    for extra in [&["--view", "sideways"][..], &["--size", "320"][..]] {
        let (out, png) = render_project(&p, "invalid.png", extra);
        assert!(out.code.is_some_and(|c| c != 0), "render-project {extra:?} must fail");
        assert!(png.is_none(), "render-project {extra:?} wrote a PNG");
    }
}

// ===========================================================================
// AG-TK-05: binding parts and checks through the service
// ===========================================================================

/// A free-text task as the human of the R5 run wrote it: nothing bound.
const FREE_TEXT: &str = "\
+++
# the comment of the human
title = \"Plate for the wall\"
state = \"open\"
created = \"2026-11-22\"
+++

A plate of 20 x 10 x 10 mm.
Keep   this line – byte for byte.
";
const FREE_TEXT_FILE: &str = "tasks/0001-plate-for-the-wall.md";

/// `text` without the `parts`/`checks` lines of its TOML header.
fn without_bindings(text: &str) -> String {
    let mut fences = 0;
    text.split_inclusive('\n')
        .filter(|line| {
            if line.trim_end() == "+++" {
                fences += 1;
            }
            let key = line.trim_start();
            !(fences == 1 && (key.starts_with("parts") || key.starts_with("checks")))
        })
        .collect()
}

#[test]
fn ag_tk_05_the_agent_binds_parts_and_checks_while_open_or_in_progress() {
    // AG-TK-05: "`POST /task/<id>` … accept[s] `parts` and `checks` (replace
    // the lists; the header lines are rewritten, the rest of the file stays
    // byte-identical). Unknown names are accepted at binding time and fail
    // the gate later."
    // Plate = box(20, 10, 10) = 2000 mm³.
    let p = Project::new("task-bind", "");
    p.write("parts/plate.rhai", "part(\"Plate\", #{}, |deps| box(20, 10, 10));\n")
        .write(
            "checks/plate.rhai",
            "check(\"plate_volume\", || {\n    \
             expect_near(part(\"Plate\").volume(), 2000, 0.001, \"volume\");\n});\n",
        )
        .write(FREE_TEXT_FILE, FREE_TEXT);
    let s = Service::start(&p);
    let id = "T-0001";
    build_all(&s);
    run_checks(&s, None);

    // `role` is required, as for every update (AG-SV-04).
    update_refused(&s, id, &json!({"parts": ["Plate"]}), &[]);
    assert_eq!(p.read(FREE_TEXT_FILE), FREE_TEXT, "a refused binding writes nothing");

    // Open: unknown names are accepted.
    let v =
        update(&s, id, &json!({"parts": ["Ghost"], "checks": ["no_such_check"], "role": "agent"}));
    assert_eq!(v["ok"], json!(true), "binding in open: {v}");
    let t = &v["result"];
    assert_eq!((&t["parts"], &t["checks"]), (&json!(["Ghost"]), &json!(["no_such_check"])), "{t}");
    assert_eq!(t["state"], json!("open"), "binding is no state change: {t}");
    let shown = task(&s, id);
    assert_eq!((&shown["parts"], &shown["checks"]), (&json!(["Ghost"]), &json!(["no_such_check"])));
    let listed = get(&s, "/tasks")["result"][0].clone();
    assert_eq!(listed["parts"], json!(["Ghost"]), "/tasks shows the binding: {listed}");
    // The rest of the file: byte-identical up to appended log lines.
    let file = p.read(FREE_TEXT_FILE);
    assert!(
        without_bindings(&file).starts_with(&without_bindings(FREE_TEXT)),
        "only the parts/checks lines of the header may change:\n{file}"
    );

    // …and fail the gate later, each by name.
    update_ok(&s, id, &json!({"state": "in_progress", "role": "agent"}), "in_progress");
    gate_refused(&s, id, &["Ghost", "no_such_check"]);

    // In progress: the lists are replaced, not merged. One list alone
    // leaves the other.
    let v = update(&s, id, &json!({"parts": ["Plate"], "role": "agent"}));
    assert_eq!(v["ok"], json!(true), "binding in in_progress: {v}");
    assert_eq!(v["result"]["parts"], json!(["Plate"]), "{v}");
    assert_eq!(v["result"]["checks"], json!(["no_such_check"]), "checks were not sent: {v}");
    gate_refused(&s, id, &["no_such_check"]);
    // An empty list unbinds: the gate then asks for a check (AG-TK-06).
    let v = update(&s, id, &json!({"checks": [], "role": "agent"}));
    assert_eq!(v["result"]["checks"], json!([]), "{v}");
    gate_refused(&s, id, &["check"]);

    // Binding and hand-over in one request: the gate sees the new lists.
    let hand_over = json!({"state": "review", "role": "agent", "note": "plate is ready",
        "parts": ["Plate"], "checks": ["plate_volume"]});
    update_ok(&s, id, &hand_over, "review");
    let shown = task(&s, id);
    assert_eq!((&shown["parts"], &shown["checks"]), (&json!(["Plate"]), &json!(["plate_volume"])));
    // AG-TK-06/07: the entry starts with the arrow, lists what was bound and
    // carries the note.
    let file = p.read(FREE_TEXT_FILE);
    let log = file.split_once("\n## Log").map(|(_, l)| l).unwrap_or_default();
    let entry = log.lines().find(|l| l.contains(" agent: in_progress -> review"));
    let entry = entry.unwrap_or_else(|| panic!("no hand-over entry:\n{file}"));
    for word in ["Plate", "plate_volume", "plate is ready"] {
        assert!(entry.contains(word), "the hand-over entry names {word:?}: {entry}");
    }
    assert!(
        without_bindings(&file)
            .replace("state = \"review\"", "state = \"open\"")
            .starts_with(&without_bindings(FREE_TEXT)),
        "header (but state, parts, checks) and body are byte-identical:\n{file}"
    );

    // In review the lists are frozen for the agent: the human looks at
    // exactly what the gate checked. The error names task and state.
    let before = p.read(FREE_TEXT_FILE);
    for body in [
        json!({"parts": [], "role": "agent"}),
        json!({"checks": ["plate_volume", "other"], "role": "agent"}),
        json!({"parts": ["Plate"], "checks": ["plate_volume"], "note": "n", "role": "agent"}),
    ] {
        update_refused(&s, id, &body, &[id, "review"]);
        assert_eq!(p.read(FREE_TEXT_FILE), before, "a refused binding writes nothing: {body}");
    }
    // Wrong types are refused.
    update_ok(&s, id, &json!({"state": "in_progress", "role": "agent"}), "in_progress");
    let before = p.read(FREE_TEXT_FILE);
    update_refused(&s, id, &json!({"parts": "Plate", "role": "agent"}), &[]);
    update_refused(&s, id, &json!({"checks": [1], "role": "agent"}), &[]);
    assert_eq!(p.read(FREE_TEXT_FILE), before);
    // Blocked, done, rejected: no binding by the agent either.
    for (state, by) in [("blocked", "agent"), ("done", "human"), ("rejected", "human")] {
        update_ok(&s, id, &json!({"state": state, "role": by, "note": "why?"}), state);
        update_refused(&s, id, &json!({"parts": ["Other"], "role": "agent"}), &[id, state]);
        if state == "blocked" {
            update_ok(&s, id, &json!({"state": "in_progress", "role": "agent"}), "in_progress");
        }
    }
    assert_eq!(task(&s, id)["parts"], json!(["Plate"]));
}

// ===========================================================================
// AG-HD-04: names that are no plain path segment
// ===========================================================================

/// `(name, height)`: each part is box(10, 10, height) → 100·height mm³,
/// z ∈ [0, height]. A space, the query and fragment delimiters, a literal
/// percent sequence. `PctA` is what `Pct%41` decodes to when a client sends
/// the name unencoded.
const AWKWARD: [(&str, f64); 5] =
    [("Crane column", 1.0), ("What?", 2.0), ("Hash#1", 3.0), ("Pct%41", 4.0), ("PctA", 9.0)];

#[test]
fn ag_hd_04_parts_with_awkward_names_build_load_in_the_app_client_and_render() {
    // AG-HD-04: "Clients percent-encode every path segment they build from
    // a name (`/mesh/<part>` …); a part named `Crane column` builds, shows
    // in the app client and renders."
    let p = Project::new("names", "[workers]\ncount = 2\n");
    let source: String = AWKWARD
        .iter()
        .map(|(name, h)| format!("part(\"{name}\", #{{}}, |deps| box(10, 10, {h}));\n"))
        .collect();
    p.write("parts/names.rhai", &source);
    let s = Service::start(&p);
    let r = build_all(&s);
    for (name, h) in AWKWARD {
        let part = &r["parts"][name];
        assert_eq!(part["state"], json!("built"), "{name:?}: {r}");
        assert_rel(&part["volume"], 100.0 * h, VOL, &format!("volume of {name:?}"));
    }
    let st = status(&s);
    assert!(states(&st, "parts").values().all(|v| v == "current"), "{st}");
    let keys = by_name(&st["parts"]);

    // The app client (W1): every mesh arrives and is the mesh of that part.
    let dir = live::ProjectDir::open(&p.dir).expect("project dir");
    let client = live::Client::connect(&dir, &[BIN.to_owned()], Duration::from_secs(20))
        .expect("connect to the running service");
    let listed: BTreeSet<String> =
        client.status().expect("status").into_iter().map(|part| part.name).collect();
    assert_eq!(listed, AWKWARD.iter().map(|(n, _)| (*n).to_owned()).collect());
    for (name, h) in AWKWARD {
        let mesh = client.mesh(name).unwrap_or_else(|e| panic!("mesh of {name:?}: {e:#}"));
        assert_eq!(Some(mesh.key.as_str()), keys[name]["key"].as_str(), "key of {name:?}");
        // A box is planar: its top vertices have exactly z = height (small
        // integers are exact in f32).
        let top = mesh.mesh.faces.bounds().expect("not empty").max[2];
        assert!((f64::from(top) - h).abs() < 1e-6, "{name:?} got the mesh of another part: {top}");
    }

    // Measure names the part in the body, not in the path: works already,
    // and stays the reference for the volume.
    let m = post_ok(&s, "/measure", &json!({"a": "Crane column"}));
    assert_rel(&m["volume"], 100.0, VOL, "volume of Crane column");

    // render-project loads the meshes with the same client.
    rendered(&p, "column.png", &["--size", "320x240", "--parts", "Crane column"], (320, 240));
    rendered(&p, "all.png", &["--size", "320x240"], (320, 240));
    let selection = "What?,Hash#1,Pct%41";
    rendered(&p, "three.png", &["--size", "320x240", "--parts", selection], (320, 240));
}

// ===========================================================================
// AG-HD-07: task create and update are serialised
// ===========================================================================

#[test]
fn ag_hd_07_concurrent_task_creates_and_updates_lose_nothing() {
    // AG-HD-07: "two simultaneous creates get two numbers; no lost log
    // line". 20 requests at once, each on its own connection.
    const N: usize = 20;
    let p = Project::new("task-race", "");
    let s = Service::start(&p);
    let created: Vec<Value> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..N)
            .map(|i| {
                let s = &s;
                scope.spawn(move || post(s, "/task", &json!({"title": format!("Task {i}")})))
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("request thread")).collect()
    });
    let mut ids = BTreeSet::new();
    for v in &created {
        assert_eq!(v["ok"], json!(true), "a concurrent create failed: {v}");
        ids.insert(v["result"]["id"].as_str().unwrap_or_else(|| panic!("id: {v}")).to_owned());
    }
    assert_eq!(ids.len(), N, "every create got its own number: {ids:?}");
    // 20 files that load (a duplicate number would be a load error, AG-TK-03).
    let list = get(&s, "/tasks");
    assert_eq!(list["ok"], json!(true), "the tasks load: {list}");
    let listed: BTreeSet<String> = list["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(listed, ids);
    let expected: BTreeSet<String> = (1..=N).map(|n| format!("T-{n:04}")).collect();
    assert_eq!(ids, expected, "the next free numbers, without gaps");
    assert_eq!(std::fs::read_dir(p.path("tasks")).expect("tasks/").count(), N, "no stray files");
    let titles: BTreeSet<String> = list["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["title"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(titles.len(), N, "every title arrived once: {titles:?}");

    // 20 notes on one task at once: 20 log lines.
    let noted: Vec<Value> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..N)
            .map(|i| {
                let s = &s;
                let body = json!({"note": format!("note number {i:02}"), "role": "agent"});
                scope.spawn(move || update(s, "T-0001", &body))
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("request thread")).collect()
    });
    for v in &noted {
        assert_eq!(v["ok"], json!(true), "a concurrent note failed: {v}");
    }
    let log = task(&s, "T-0001")["log"].clone();
    let texts: Vec<&str> =
        log.as_array().into_iter().flatten().filter_map(|l| l["text"].as_str()).collect();
    assert_eq!(texts.len(), N, "no lost and no doubled log line: {texts:?}");
    for i in 0..N {
        let note = format!("note number {i:02}");
        assert_eq!(texts.iter().filter(|t| **t == note).count(), 1, "{note:?} in {texts:?}");
    }
    // The file itself is intact: header, then one line per note.
    let file = p.read(by_name_file(&list, "T-0001"));
    assert_eq!(file.lines().filter(|l| l.contains(" agent: note number ")).count(), N, "{file}");
    assert_eq!(file.matches("## Log").count(), 1, "one log section:\n{file}");
}

/// `file` of task `id` in a `/tasks` answer.
fn by_name_file<'a>(list: &'a Value, id: &str) -> &'a str {
    let tasks = list["result"].as_array().into_iter().flatten();
    let task = tasks.into_iter().find(|t| t["id"] == json!(id));
    task.and_then(|t| t["file"].as_str()).unwrap_or_else(|| panic!("no file of {id}: {list}"))
}

// ===========================================================================
// AG-HD-08: save in a sub folder of a larger repository
// ===========================================================================

#[test]
fn ag_hd_08_save_refuses_a_project_that_is_a_sub_folder_of_a_repository() {
    // AG-HD-08: "`/save` commits only paths inside the project folder … and
    // refuses with kind `git` when the Git top level is not the project
    // folder." The outer repository has a commit and something else staged:
    // before R5 `/save` committed that along with the project.
    let p = a_with_check("save-outer");
    git_init(&p.root);
    std::fs::write(p.root.join("notes.txt"), "outer\n").expect("write notes.txt");
    git(&p.root, &["add", "notes.txt"]).expect("git add notes.txt");
    git(&p.root, &["commit", "--quiet", "--message", "Outer start"]).expect("outer commit");
    let base = git(&p.root, &["rev-parse", "HEAD"]).expect("outer HEAD");
    std::fs::write(p.root.join("staged.txt"), "somebody else's work in progress\n")
        .expect("write staged.txt");
    git(&p.root, &["add", "staged.txt"]).expect("git add staged.txt");

    let s = Service::start(&p);
    let v = post(&s, "/save", &json!({"message": "must not be committed"}));
    assert_refused(&v, &[], "save in a sub folder of a repository");
    assert_eq!(v["error"]["kind"], json!("git"), "{v}");
    // Nothing was committed, nothing was staged or unstaged.
    assert_eq!(git(&p.root, &["rev-parse", "HEAD"]).as_deref(), Ok(base.as_str()), "{v}");
    assert_eq!(
        git(&p.root, &["diff", "--cached", "--name-only"]).as_deref(),
        Ok("staged.txt"),
        "the index of the outer repository was changed"
    );
    assert_eq!(
        std::fs::read_to_string(p.root.join("staged.txt")).ok().as_deref(),
        Some("somebody else's work in progress\n")
    );

    // Control: once the project is its own repository, `/save` commits
    // there, only project paths, and the outer repository stays as it was.
    git_init(&p.dir);
    let head = save_ok(&s, &p, "Project state");
    assert_ne!(head, base);
    let tracked = git_tracked(&p.dir);
    assert!(tracked.iter().any(|t| t == "parts/a.rhai"), "{tracked:?}");
    assert!(
        !tracked.iter().any(|t| t.contains("staged.txt") || t.contains("notes.txt")),
        "{tracked:?}"
    );
    assert_eq!(git(&p.root, &["rev-parse", "HEAD"]).as_deref(), Ok(base.as_str()));
    assert_eq!(git(&p.root, &["diff", "--cached", "--name-only"]).as_deref(), Ok("staged.txt"));
}

#[test]
fn ag_hd_08_save_commits_only_project_paths_not_what_else_is_staged() {
    // "pathspec on the commit as on the add": something staged that `/save`
    // never adds (`out/` is excluded, AG-SV-07) must not ride along in the
    // commit.
    let p = a_with_check("save-pathspec");
    git_init(&p.dir);
    let s = Service::start(&p);
    post_ok(&s, "/export", &json!({"parts": ["A"], "format": "stl"}));
    git(&p.dir, &["add", "--force", "out/A.stl"]).expect("stage out/A.stl by hand");
    save_ok(&s, &p, "State without out/");
    let tracked = git_tracked(&p.dir);
    assert!(tracked.iter().any(|t| t == "parts/a.rhai"), "{tracked:?}");
    assert!(!tracked.iter().any(|t| t.starts_with("out/")), "out/ was committed: {tracked:?}");
}

// ===========================================================================
// AG-HD-09: the gate reports load errors as such
// ===========================================================================

#[test]
fn ag_hd_09_the_gate_reports_a_broken_file_instead_of_missing_parts() {
    // AG-HD-09: "The gate reports load errors of part and check files as
    // such instead of 'part does not exist'." One broken file makes the
    // whole project unloadable; the agent must learn which file, not that
    // its (existing, just built) part is gone.
    let plate = "part(\"Plate\", #{}, |deps| box(20, 10, 10));\n";
    let p = Project::new("gate-load", "");
    p.write("parts/plate.rhai", plate).write(
        "checks/plate.rhai",
        "check(\"plate_volume\", || {\n    \
         expect_near(part(\"Plate\").volume(), 2000, 0.001, \"volume\");\n});\n",
    );
    let s = Service::start(&p);
    let id = task_in_progress(&s, "Plate", &["Plate"], &["plate_volume"]);
    build_all(&s);
    run_checks(&s, None);
    let review = json!({"state": "review", "role": "agent", "note": "ready"});

    // Line 1 lacks the closing parenthesis of `part(`: a syntax error.
    p.write("parts/broken.rhai", "part(\"Broken\", #{}, |deps| box(1, 1, 1);\n");
    let v = update(&s, &id, &review);
    assert_refused(&v, &["parts/broken.rhai"], "review while a part file does not load");
    assert!(!v.to_string().contains("does not exist"), "Plate exists, the project is broken: {v}");
    assert_eq!(task_state(&s, &id), "in_progress");
    std::fs::remove_file(p.path("parts/broken.rhai")).expect("remove broken part file");

    // The same for a check file.
    p.write("checks/broken.rhai", "check(\"broken\", || { expect(true, \"x\") ;\n");
    let v = update(&s, &id, &review);
    assert_refused(&v, &["checks/broken.rhai"], "review while a check file does not load");
    assert!(!v.to_string().contains("does not exist"), "{v}");
    assert_eq!(task_state(&s, &id), "in_progress");
    std::fs::remove_file(p.path("checks/broken.rhai")).expect("remove broken check file");

    // Control: with the files gone the same hand-over passes (nothing was
    // rebuilt in between, so part and check are still current).
    update_ok(&s, &id, &review, "review");
}

// ===========================================================================
// AG-HD-11: one bounding box
// ===========================================================================

/// The nine numbers `min`, `max`, `size` of a bbox object as bit patterns.
fn bbox_bits(bbox: &Value, what: &str) -> Vec<u64> {
    let mut bits = Vec::new();
    for key in ["min", "max", "size"] {
        for i in 0..3 {
            let v =
                bbox[key][i].as_f64().unwrap_or_else(|| panic!("{what}: bbox.{key}[{i}]: {bbox}"));
            bits.push(v.to_bits());
        }
    }
    bits
}

/// A `/run` script that compares `part(name).bbox()` number by number with
/// `bbox` (from JSON; `parse_float` of the shortest round-trip text gives
/// the same f64 back) and prints one `true`/`false` per number, then shows
/// the part.
fn bbox_script(name: &str, bbox: &Value) -> String {
    let mut source = format!("let b = part(\"{name}\").bbox();\n");
    for key in ["min", "max", "size"] {
        for i in 0..3 {
            let text = bbox[key][i].as_f64().map(|v| format!("{v:?}")).unwrap_or_default();
            source.push_str(&format!("print(b.{key}[{i}] == parse_float(\"{text}\"));\n"));
        }
    }
    source.push_str(&format!("show(part(\"{name}\"), \"shown\");\n"));
    source
}

#[test]
fn ag_hd_11_bbox_is_bitwise_the_same_in_build_measure_cache_and_scripts() {
    // AG-HD-11: "`bbox` is the same in the build result, `/measure` and
    // scripts for the same key (one code path), tight to 1e-6 mm."
    //
    // Round = box(20, 10, 10) with all edges filleted (r = 2): the flat
    // faces keep their extent, so the box is still x ±10, y ±5, z 0..10.
    // D = cylinder(10, 5): x/y ±5, z 0..5. Both have curved faces, where a
    // bounding box from control points or with a tolerance gap is not tight.
    // Ball = sphere(10): ±5 in every axis. Ring = torus(20, 4): ring radius
    // 10, tube radius 2 → x/y ±12, z ±2.
    // Tilted = cylinder(10, 20) rotated by +30° about y (right-handed: the
    // axis tip (0, 0, 20) moves to (20·sin 30°, 0, 20·cos 30°) = (10, 0, 10√3)).
    // The end discs (r = 5) are tilted by 30°: they reach ±5·cos 30° in x
    // and ±5·sin 30° = ±2.5 in z around their centres, ±5 in y.
    let cos30 = 30f64.to_radians().cos();
    let tilted_min = [-5.0 * cos30, -5.0, -2.5];
    let tilted_max = [10.0 + 5.0 * cos30, 5.0, 20.0 * cos30 + 2.5];
    let p = Project::new("bbox", "[workers]\ncount = 2\n");
    p.write(
        "parts/round.rhai",
        "part(\"Round\", #{}, |deps| {\n    let b = box(20, 10, 10);\n    \
         fillet(b, all_edges(b), 2)\n});\n",
    )
    .write("parts/d.rhai", PART_D)
    .write(
        "parts/more.rhai",
        "part(\"Ball\", #{}, |deps| sphere(10));\n\
         part(\"Ring\", #{}, |deps| torus(20, 4));\n\
         part(\"Tilted\", #{}, |deps| cylinder(10, 20).rotate(\"y\", 30));\n",
    );
    let s = Service::start(&p);
    let built = build_all(&s);
    let cached = build_all(&s);
    let st = status(&s);
    let boxes = [
        ("Round", [-10.0, -5.0, 0.0], [10.0, 5.0, 10.0]),
        ("D", [-5.0, -5.0, 0.0], [5.0, 5.0, 5.0]),
        ("Ball", [-5.0; 3], [5.0; 3]),
        ("Ring", [-12.0, -12.0, -2.0], [12.0, 12.0, 2.0]),
        ("Tilted", tilted_min, tilted_max),
    ];
    for (name, min, max) in boxes {
        let reference = &built["parts"][name]["bbox"];
        assert_eq!(built["parts"][name]["state"], json!("built"), "{built}");
        // Tight: 1e-6 mm against the analytic box.
        let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        assert_vec(&reference["min"], min, POS, &format!("{name} bbox.min (build)"));
        assert_vec(&reference["max"], max, POS, &format!("{name} bbox.max (build)"));
        assert_vec(&reference["size"], size, POS, &format!("{name} bbox.size (build)"));
        let bits = bbox_bits(reference, name);

        // The cached build result and the cache entry of the key (§2.1).
        assert_eq!(cached["parts"][name]["state"], json!("cached"), "{cached}");
        let again = bbox_bits(&cached["parts"][name]["bbox"], name);
        assert_eq!(again, bits, "{name}: cached build result");
        let key = by_name(&st["parts"])[name]["key"].as_str().unwrap_or_default().to_owned();
        let entry: Value =
            serde_json::from_str(&p.read(&format!(".qoob3d/cache/{name}/{key}.json")))
                .expect("cache json");
        assert_eq!(bbox_bits(&entry["bbox"], name), bits, "{name}: cache entry");

        // /measure.
        let measured = post_ok(&s, "/measure", &json!({"a": name}));
        assert_vec(&measured["bbox"]["min"], min, POS, &format!("{name} bbox.min (measure)"));
        assert_vec(&measured["bbox"]["max"], max, POS, &format!("{name} bbox.max (measure)"));
        assert_eq!(bbox_bits(&measured["bbox"], name), bits, "{name}: /measure: {measured}");

        // A script: `part(name).bbox()` and the shape the script shows.
        let source = bbox_script(name, reference);
        let run = post_ok(&s, "/run", &json!({"source": source, "wait_s": WAIT_S}));
        assert_eq!(
            run["printed"],
            Value::from(vec!["true"; 9]),
            "{name}: bbox() in a script\n{source}"
        );
        let shown = &run["shapes"][0]["bbox"];
        assert_eq!(bbox_bits(shown, name), bits, "{name}: shape of /run: {shown}");
    }
}
