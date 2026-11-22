//! R4 without MCP: checks, tasks, throwaway scripts, measure, save, init and
//! the project render against the real `qoob3d` binary
//! (docs/requirements/agent.md §4.2–4.4: AG-CK-01..03, AG-SV-04..08,
//! AG-RD-01; ADR-0054; envelope from docs/requirements/project-app.md §1).
//! Every test names its requirement IDs.
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

use serde_json::{Value, json};
use support::{
    Project, Service, VOL, assert_rel, assert_vec, cli, git, git_init, git_tracked, png_colors,
    png_size, service_json, wait_until,
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

    // AG-CK-03: declared checks are known before any run.
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
    assert!(
        has_log_line(&file, "agent", "starting"),
        "log line `- <date> agent: starting`:\n{file}"
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
    assert!(has_log_line(&p.read(HAND_MADE_FILE), "human", "accepted"));
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

    // Neither parts nor checks: the gate needs a note.
    let empty = task_in_progress(&s, "Just think", &[], &[]);
    let v = update(&s, &empty, &review);
    assert_refused(&v, &[], "a hand-over without parts, checks and note");
    assert_eq!(v["error"]["kind"], json!("gate"), "{v}");
    let noted = json!({"state": "review", "role": "agent", "note": "thought about it"});
    update_ok(&s, &empty, &noted, "review");

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
    assert!(has_log_line(&file, "human", "printed and fits"), "{file}");
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
