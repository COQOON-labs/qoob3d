//! Milestone W1: QOOB3D opens a script project folder and shows its parts
//! live (docs/requirements/project-app.md, ADR-0053). Every test names its
//! requirement ID. All tests run against the fake `qoob3d serve` in `common/`.
//!
//! Sections: project and service start (WA-OP-*), client protocol (WA-PR-*),
//! polling (WA-LV-01/02/05), the view model `Live`, CLI (WA-OP-01), UI with
//! egui_kittest (WA-LV-04/05/06, WA-OP-03).
//!
//! R5 hardening of the client (docs/requirements/agent.md §6.1): the token
//! of `service.json` on every request (AG-HD-03), percent-encoded part names
//! (AG-HD-04), reconnecting through `service.json` (AG-HD-05).

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use common::{FakePart, FakeService, Mode, TempProject, cube_qmesh, same_dir, token_for};
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use qoob3d_app::client::{self, Client, Part, PartState, ProjectDir};
use qoob3d_app::live::{self, Live, Poller, Update};
use qoob3d_app::{QoobApp, StartOptions};

/// Generous upper bound for waiting on background work; the tests normally
/// finish in a fraction of it.
const PATIENCE: Duration = Duration::from_secs(10);

/// Short poll interval for the poller tests (the app uses 0.5 s, checked in
/// `poll_interval_is_half_a_second`).
const FAST: Duration = Duration::from_millis(50);

fn three_parts() -> Vec<FakePart> {
    vec![
        FakePart::built("shaft", "current", "k-shaft-1", cube_qmesh([0.0; 3], 10.0)),
        FakePart::built("lid", "outdated", "k-lid-1", cube_qmesh([20.0, 0.0, 0.0], 5.0)),
        FakePart::unbuilt("pin", "never_built"),
    ]
}

/// Project with `service.json` pointing to a running fake.
fn connected(tag: &str, parts: Vec<FakePart>) -> (TempProject, FakeService, Client) {
    let project = TempProject::new(tag);
    let fake = FakeService::start(parts);
    project.write_service_json(fake.port);
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    // A dud command: if connect wrongly starts a service, the test sees the call.
    let client =
        Client::connect(&opened, &project.dud_script(), Duration::from_secs(2)).expect("connect");
    (project, fake, client)
}

// ================================================================== WA-OP-01

/// WA-OP-01: a folder with `qoob3d.toml` is a project.
#[test]
fn project_opens_a_folder_with_qoob3d_toml() {
    let project = TempProject::new("open");
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    assert!(same_dir(opened.dir(), &project.dir));
}

/// WA-OP-01: without `qoob3d.toml` → error whose message names the folder.
#[test]
fn project_without_qoob3d_toml_is_an_error_naming_the_folder() {
    let project = TempProject::new("no-toml");
    let dir = project.not_a_project();
    let err = ProjectDir::open(&dir).expect_err("not a project");
    let msg = err.to_string();
    assert!(msg.contains(&dir.display().to_string()), "message does not name the folder: {msg}");
}

/// WA-OP-01: a folder that does not exist is an error naming it.
#[test]
fn project_in_missing_folder_is_an_error() {
    let project = TempProject::new("missing");
    let dir = project.root.join("does-not-exist");
    let err = ProjectDir::open(&dir).expect_err("missing folder");
    assert!(err.to_string().contains(&dir.display().to_string()), "{err}");
}

// ================================================================== WA-OP-02

/// WA-OP-02: `QOOB3D_SERVICE_CMD` gives the program, which may contain
/// arguments separated by spaces; unset, it is the app's own executable
/// (in this test process: the test binary).
#[test]
fn service_command_comes_from_the_environment_value() {
    let own = std::env::current_exe().expect("current exe").display().to_string();
    assert_eq!(client::service_command(None), std::slice::from_ref(&own));
    assert_eq!(
        client::service_command(Some("/opt/q/qoob3d --log debug")),
        ["/opt/q/qoob3d", "--log", "debug"]
    );
    // Repeated and surrounding spaces do not produce empty arguments.
    assert_eq!(client::service_command(Some("  sh  /x/start.sh ")), ["sh", "/x/start.sh"]);
    // An empty value is treated like an unset one.
    assert_eq!(client::service_command(Some("")), std::slice::from_ref(&own));
    assert_eq!(client::service_command(Some("   ")), [own]);
}

/// WA-OP-02 / WA-LV-01: the timing constants of the specification.
#[test]
fn timing_constants_match_the_specification() {
    assert_eq!(client::STATUS_TIMEOUT, Duration::from_secs(1));
    assert_eq!(client::START_TIMEOUT, Duration::from_secs(20));
}

/// WA-LV-01: polling every 0.5 s.
#[test]
fn poll_interval_is_half_a_second() {
    assert_eq!(live::POLL_INTERVAL, Duration::from_millis(500));
}

/// WA-OP-02: a running service from `service.json` is used, nothing started.
#[test]
fn connect_uses_the_running_service_without_starting_one() {
    let (project, fake, client) = connected("running", three_parts());
    assert!(fake.count("GET /status") >= 1, "connect checks GET /status");
    assert_eq!(project.script_call(), None, "service started although it was running");
    assert_eq!(client.status().expect("status").len(), 3);
}

/// Asserts the stand-in was called exactly as `<cmd> serve <DIR>`.
fn assert_started(project: &TempProject) {
    let call = project.script_call().expect("service start command was not run");
    assert_eq!(call.len(), 2, "arguments: {call:?}");
    assert_eq!(call[0], "serve");
    assert!(same_dir(Path::new(&call[1]), &project.dir), "serve {} is not the project", call[1]);
}

/// WA-OP-02: no `service.json` → start the service with the command, wait
/// for `service.json` and a successful `GET /status`.
#[test]
fn connect_starts_the_service_when_service_json_is_missing() {
    let project = TempProject::new("start-missing");
    let fake = FakeService::start(three_parts());
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    let client = Client::connect(&opened, &project.start_script(fake.port), PATIENCE)
        .expect("service started and reachable");
    assert_started(&project);
    assert_eq!(client.status().expect("status").len(), 3);
}

/// WA-OP-02: a stale `service.json` (service died, port closed) → start.
#[test]
fn connect_starts_the_service_when_the_port_is_dead() {
    let project = TempProject::new("start-dead");
    project.write_service_json(TempProject::dead_port());
    let fake = FakeService::start(three_parts());
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    let client = Client::connect(&opened, &project.start_script(fake.port), PATIENCE)
        .expect("service started and reachable");
    assert_started(&project);
    assert_eq!(client.status().expect("status").len(), 3);
}

/// WA-OP-02: a service that accepts but does not answer `GET /status`
/// within 1 s counts as not running → start. Upper bound 1 s timeout +
/// start + generous slack (8 s) proves the timeout is not much longer than
/// the specified 1 s.
#[test]
fn connect_starts_the_service_when_status_does_not_answer_within_one_second() {
    let project = TempProject::new("start-hang");
    let hanging = FakeService::start(Vec::new());
    hanging.set_mode(Mode::Hang);
    project.write_service_json(hanging.port);
    let fake = FakeService::start(three_parts());
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    let start = Instant::now();
    let client = Client::connect(&opened, &project.start_script(fake.port), PATIENCE)
        .expect("service started and reachable");
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(900), "gave up before 1 s: {elapsed:?}");
    assert!(elapsed < Duration::from_secs(8), "status timeout too long: {elapsed:?}");
    assert_started(&project);
    assert_eq!(client.status().expect("status").len(), 3);
}

/// WA-OP-02: if the service never comes up, connect fails after the given
/// wait (here 1 s instead of 20 s) instead of hanging.
#[test]
fn connect_fails_when_the_service_never_comes_up() {
    let project = TempProject::new("never");
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    let start = Instant::now();
    let result = Client::connect(&opened, &project.dud_script(), Duration::from_secs(1));
    assert!(result.is_err(), "connected to nothing");
    assert!(project.script_call().is_some(), "start command was not even tried");
    assert!(start.elapsed() < Duration::from_secs(6), "waited {:?}", start.elapsed());
}

/// WA-OP-02: a start command that does not exist is an error, not a panic.
#[test]
fn connect_with_a_missing_program_is_an_error() {
    let project = TempProject::new("no-program");
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    let cmd = vec!["qoob3d-no-such-service-program".to_owned()];
    assert!(Client::connect(&opened, &cmd, Duration::from_secs(1)).is_err());
}

// ================================================================== WA-PR-01..03

/// WA-PR-01: parts with name, file, state and key; extra fields ignored
/// (the fake adds `volumen`, `projekt`, `auftraege`, `arbeiter`).
#[test]
fn status_lists_parts_with_state_and_key() {
    let mut parts = three_parts();
    parts.push(FakePart::unbuilt("bracket", "failed"));
    let (_project, _fake, client) = connected("status", parts);
    let parts = client.status().expect("status");
    let expected = [
        ("shaft", PartState::Current, Some("k-shaft-1")),
        ("lid", PartState::Outdated, Some("k-lid-1")),
        ("pin", PartState::NeverBuilt, None),
        ("bracket", PartState::Failed, None),
    ];
    assert_eq!(parts.len(), expected.len());
    for (part, (name, state, key)) in parts.iter().zip(expected) {
        assert_eq!(part.name, name);
        assert_eq!(part.file, format!("parts/{name}.rhai"));
        assert_eq!(part.state, state, "{name}");
        assert_eq!(part.key.as_deref(), key, "{name}");
    }
}

/// WA-PR-01: `ok: false` in the envelope is an error, not an empty list.
#[test]
fn status_with_an_error_envelope_is_an_error() {
    let (_project, fake, client) = connected("status-error", three_parts());
    fake.set_status_error(true);
    assert!(client.status().is_err());
}

/// WA-PR-02 + WA-QM-01: `/mesh/<name>` → the part's qmesh as `ShapeMesh`
/// (10 mm cube written by the test helper: 8 shared corners, 12 triangles,
/// 12 edge polylines, bounds 0..10) with the key from `X-Qoob3d-Key`.
#[test]
fn mesh_is_read_from_qmesh_with_the_header_key() {
    let (_project, fake, client) = connected("mesh", three_parts());
    let mesh = client.mesh("shaft").expect("mesh of a built part");
    assert_eq!(mesh.key, "k-shaft-1");
    assert_eq!(mesh.mesh.faces.positions.len(), 8, "indexed transfer keeps shared corners");
    assert_eq!(mesh.mesh.faces.triangle_count(), 12);
    assert_eq!(mesh.mesh.edges.len(), 12);
    let b = mesh.mesh.faces.bounds().expect("not empty");
    assert_eq!((b.min, b.max), ([0.0; 3], [10.0; 3]));
    assert_eq!(fake.count("GET /mesh/shaft"), 1);
}

/// WA-PR-02 + WA-QM-03: a corrupt mesh body (truncated qmesh) is an
/// error, not a panic and not an empty mesh.
#[test]
fn corrupt_mesh_bytes_are_an_error() {
    let mut bytes = cube_qmesh([0.0; 3], 10.0);
    bytes.truncate(bytes.len() - 1);
    let parts = vec![FakePart::built("shaft", "current", "k-shaft-1", bytes)];
    let (_project, _fake, client) = connected("mesh-corrupt", parts);
    assert!(client.mesh("shaft").is_err());
}

/// WA-PR-02: the key belongs to the mesh, so it comes from the response
/// header, not from the (possibly newer) status.
#[test]
fn mesh_key_comes_from_the_response_header() {
    let (_project, fake, client) = connected("mesh-header", three_parts());
    fake.set_header_key_once("shaft", "k-from-header");
    assert_eq!(client.mesh("shaft").expect("mesh").key, "k-from-header");
}

/// WA-PR-02: 404 for a never-built part (and an unknown one) is an error.
#[test]
fn mesh_of_a_never_built_part_is_an_error() {
    let (_project, _fake, client) = connected("mesh-404", three_parts());
    assert!(client.mesh("pin").is_err());
    assert!(client.mesh("does-not-exist").is_err());
}

/// WA-PR-03: `POST /build` with `{"parts": null, "wait_s": 0}`.
#[test]
fn build_all_posts_null_parts_and_zero_wait() {
    let (_project, fake, client) = connected("build", three_parts());
    client.build_all().expect("build requested");
    let builds = fake.builds();
    assert_eq!(builds.len(), 1);
    let body: serde_json::Value = serde_json::from_str(&builds[0]).expect("JSON body");
    assert_eq!(body["parts"], serde_json::Value::Null, "{body}");
    assert!(body.get("parts").is_some(), "parts must be present as null: {body}");
    assert_eq!(body["wait_s"].as_f64(), Some(0.0), "{body}");
}

// ================================================================== AG-HD-03/04 client

/// AG-HD-03: "The app client, CLI and MCP send the token": every request of
/// the client carries the token of `service.json` in `X-Qoob3d-Token`, also
/// the probe of `connect` (the real service answers 403 without it).
#[test]
fn client_sends_the_token_of_service_json_with_every_request() {
    let (_project, fake, client) = connected("token", three_parts());
    client.status().expect("status");
    client.mesh("shaft").expect("mesh");
    client.build_all().expect("build requested");
    let sent = fake.tokens();
    let requests: Vec<&str> = sent.iter().map(|(r, _)| r.as_str()).collect();
    for request in ["GET /status", "GET /mesh/shaft", "POST /build"] {
        assert!(requests.contains(&request), "{request} was not sent: {requests:?}");
    }
    let expected = token_for(fake.port);
    for (request, token) in &sent {
        assert_eq!(token.as_deref(), Some(expected.as_str()), "token of `{request}`");
    }
}

/// AG-HD-03: the token belongs to the service that wrote `service.json`. A
/// service started by `connect` (WA-OP-02) is asked with the token it wrote,
/// not with one of a stale file.
#[test]
fn client_uses_the_token_of_the_service_it_started() {
    let project = TempProject::new("token-start");
    // Stale file: dead port, and with it another token.
    project.write_service_json(TempProject::dead_port());
    let fake = FakeService::start(three_parts());
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    let client = Client::connect(&opened, &project.start_script(fake.port), PATIENCE)
        .expect("service started and reachable");
    client.status().expect("status");
    let expected = token_for(fake.port);
    let sent = fake.tokens();
    assert!(!sent.is_empty());
    for (request, token) in &sent {
        assert_eq!(token.as_deref(), Some(expected.as_str()), "token of `{request}`");
    }
}

/// Part names that are no plain path segment: a space, the query and
/// fragment delimiters, a literal percent sequence (`%41` would decode to
/// `A`), a slash-free non-ASCII name. Cube sizes 1..5 tell them apart.
fn awkward_parts() -> Vec<FakePart> {
    let names = ["Crane column", "What?", "Hash#1", "Pct%41", "Geh\u{e4}use"];
    let mut parts: Vec<FakePart> = names
        .iter()
        .zip(1u8..)
        .map(|(name, n)| {
            FakePart::built(name, "current", &format!("k-{n}"), cube_qmesh([0.0; 3], f32::from(n)))
        })
        .collect();
    // What a client that sends `%41` unencoded would get instead of `Pct%41`.
    parts.push(FakePart::built("PctA", "current", "k-decoy", cube_qmesh([0.0; 3], 99.0)));
    parts
}

/// AG-HD-04: "Clients percent-encode every path segment they build from a
/// name (`/mesh/<part>` …); a part named `Crane column` … shows in the app
/// client". The fake decodes the segment like the real service; the request
/// line itself must be valid HTTP (no raw space, `?` or `#` in the segment).
#[test]
fn mesh_of_parts_with_awkward_names_is_fetched_with_an_encoded_path() {
    let (_project, fake, client) = connected("mesh-names", awkward_parts());
    for (part, n) in awkward_parts().iter().take(5).zip(1u8..) {
        let name = &part.name;
        let mesh = client.mesh(name).unwrap_or_else(|e| panic!("mesh of {name:?}: {e:#}"));
        assert_eq!(mesh.key, format!("k-{n}"), "{name:?}");
        let max = mesh.mesh.faces.bounds().expect("not empty").max;
        assert_eq!(max, [f32::from(n); 3], "{name:?} got the mesh of another part");
    }
    let log = fake.log();
    let segments: Vec<&str> = log.iter().filter_map(|r| r.strip_prefix("GET /mesh/")).collect();
    assert_eq!(segments.len(), 5, "one request per part, each a whole request line: {log:?}");
    for segment in segments {
        assert!(
            segment.is_ascii() && !segment.contains(['?', '#', ' ', '/']),
            "the name is not percent-encoded: {segment:?}"
        );
    }
    // RFC 3986 §2.1: a space is `%20` in a path (`+` is a form encoding).
    assert!(log.iter().any(|r| r == "GET /mesh/Crane%20column"), "{log:?}");
}

/// AG-HD-04 through the poller: the app shows such parts (WA-LV-02).
#[test]
fn poller_loads_parts_with_awkward_names() {
    let (_project, _fake, client) = connected("poll-names", awkward_parts());
    let poller = Poller::spawn(client, FAST);
    let mut missing: Vec<String> = awkward_parts().into_iter().map(|p| p.name).collect();
    let end = Instant::now() + PATIENCE;
    while !missing.is_empty() {
        let left = end.checked_duration_since(Instant::now()).unwrap_or_default();
        match poller.updates().recv_timeout(left) {
            Ok(Update::Mesh { name, .. }) => missing.retain(|m| *m != name),
            Ok(_) => {}
            Err(e) => panic!("no mesh for {missing:?} ({e:?})"),
        }
    }
}

// ================================================================== WA-LV-01/02/05 poller

/// Collects updates for `time`.
fn collect(poller: &Poller, time: Duration) -> Vec<Update> {
    let end = Instant::now() + time;
    let mut out = Vec::new();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Ok(u) = poller.updates().recv_timeout(left) {
            out.push(u);
        }
    }
    out
}

/// Receives until `pred` matches (panics after `PATIENCE`); returns all
/// updates seen, the matching one last.
fn until(poller: &Poller, what: &str, pred: impl Fn(&Update) -> bool) -> Vec<Update> {
    let end = Instant::now() + PATIENCE;
    let mut seen = Vec::new();
    loop {
        let left = end.checked_duration_since(Instant::now()).unwrap_or_default();
        match poller.updates().recv_timeout(left) {
            Ok(u) => {
                let done = pred(&u);
                seen.push(u);
                if done {
                    return seen;
                }
            }
            Err(e) => panic!("no update '{what}' ({e:?}); seen: {seen:?}"),
        }
    }
}

fn mesh_keys<'a>(updates: &'a [Update], part: &str) -> Vec<&'a str> {
    updates
        .iter()
        .filter_map(|u| match u {
            Update::Mesh { name, key, .. } if name == part => Some(key.as_str()),
            _ => None,
        })
        .collect()
}

fn part_names(parts: &[Part]) -> Vec<&str> {
    parts.iter().map(|p| p.name.as_str()).collect()
}

/// WA-LV-01/02: the poller polls repeatedly, fetches each built part once
/// and never fetches parts without mesh; unchanged keys are not fetched
/// again over ≥ 10 polls.
#[test]
fn poller_fetches_each_key_once() {
    let (_project, fake, client) = connected("poll-once", three_parts());
    let poller = Poller::spawn(client, FAST);
    let updates = collect(&poller, Duration::from_millis(700));
    assert!(fake.count("GET /status") >= 6, "not polling every 50 ms: {:?}", fake.log());
    assert_eq!(mesh_keys(&updates, "shaft"), ["k-shaft-1"]);
    assert_eq!(mesh_keys(&updates, "lid"), ["k-lid-1"]);
    assert!(mesh_keys(&updates, "pin").is_empty());
    assert_eq!(fake.count("GET /mesh/shaft"), 1);
    assert_eq!(fake.count("GET /mesh/lid"), 1);
    assert_eq!(fake.count("GET /mesh/pin"), 0, "never-built part fetched");
    let Some(Update::Parts(parts)) = updates.iter().rev().find(|u| matches!(u, Update::Parts(_)))
    else {
        panic!("no parts update: {updates:?}")
    };
    assert_eq!(part_names(parts), ["shaft", "lid", "pin"]);
}

/// WA-LV-02: a changed key → exactly that part is fetched again and its new
/// mesh delivered (new cube 0..30); the other part is not refetched.
#[test]
fn poller_refetches_only_parts_with_a_changed_key() {
    let (_project, fake, client) = connected("poll-change", three_parts());
    let poller = Poller::spawn(client, FAST);
    until(&poller, "both meshes", |_| fake.count("GET /mesh/lid") == 1);
    collect(&poller, Duration::from_millis(200));
    let mut parts = three_parts();
    parts[0] = FakePart::built("shaft", "current", "k-shaft-2", cube_qmesh([0.0; 3], 30.0));
    fake.set_parts(parts);
    let seen = until(
        &poller,
        "new shaft",
        |u| matches!(u, Update::Mesh { name, key, .. } if name == "shaft" && key == "k-shaft-2"),
    );
    let Some(Update::Mesh { mesh, .. }) = seen.last() else { unreachable!() };
    assert_eq!(mesh.faces.bounds().expect("not empty").max, [30.0; 3]);
    collect(&poller, Duration::from_millis(300));
    assert_eq!(fake.count("GET /mesh/shaft"), 2);
    assert_eq!(fake.count("GET /mesh/lid"), 1, "unchanged key fetched again");
}

/// WA-LV-02: a part that becomes built later (never_built → current with a
/// key) is fetched then.
#[test]
fn poller_fetches_a_part_once_it_gets_a_key() {
    let (_project, fake, client) = connected("poll-new", three_parts());
    let poller = Poller::spawn(client, FAST);
    collect(&poller, Duration::from_millis(200));
    let mut parts = three_parts();
    parts[2] = FakePart::built("pin", "current", "k-pin-1", cube_qmesh([0.0; 3], 1.0));
    fake.set_parts(parts);
    until(&poller, "pin", |u| matches!(u, Update::Mesh { name, .. } if name == "pin"));
    assert_eq!(fake.count("GET /mesh/pin"), 1);
}

/// WA-LV-02: "differs from the one last loaded" – the loaded key is the one
/// of the mesh (header). If a fetch returned an older mesh than the status
/// announced, the part is fetched again until the keys agree.
#[test]
fn poller_refetches_when_the_loaded_mesh_is_older_than_the_status() {
    let project = TempProject::new("poll-race");
    let fake = FakeService::start(three_parts());
    project.write_service_json(fake.port);
    fake.set_header_key_once("shaft", "k-shaft-0");
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    let client = Client::connect(&opened, &project.dud_script(), PATIENCE).expect("connect");
    let poller = Poller::spawn(client, FAST);
    let seen = until(
        &poller,
        "current shaft",
        |u| matches!(u, Update::Mesh { name, key, .. } if name == "shaft" && key == "k-shaft-1"),
    );
    assert_eq!(mesh_keys(&seen, "shaft"), ["k-shaft-0", "k-shaft-1"]);
}

/// WA-LV-02: a vanished part is missing from the next parts update; if it
/// comes back with the same key, its mesh is fetched again (spec: "a part
/// that returns is fetched again").
#[test]
fn poller_reports_vanished_parts_and_refetches_returning_ones() {
    let (_project, fake, client) = connected("poll-vanish", three_parts());
    let poller = Poller::spawn(client, FAST);
    until(&poller, "lid", |_| fake.count("GET /mesh/lid") == 1);
    let mut parts = three_parts();
    let lid = parts.remove(1);
    fake.set_parts(parts.clone());
    until(
        &poller,
        "parts without lid",
        |u| matches!(u, Update::Parts(p) if part_names(p) == ["shaft", "pin"]),
    );
    parts.insert(1, lid);
    fake.set_parts(parts);
    until(&poller, "lid again", |u| matches!(u, Update::Mesh { name, .. } if name == "lid"));
    assert_eq!(fake.count("GET /mesh/lid"), 2);
}

/// WA-LV-05: the service stops → `Unreachable`; it answers again → polling
/// continues (new parts updates), no restart of the poller needed.
#[test]
fn poller_reports_unreachable_and_recovers() {
    let (_project, fake, client) = connected("poll-down", three_parts());
    let poller = Poller::spawn(client, FAST);
    until(&poller, "parts", |u| matches!(u, Update::Parts(_)));
    fake.set_mode(Mode::Refuse);
    until(&poller, "unreachable", |u| matches!(u, Update::Unreachable));
    fake.set_mode(Mode::Serve);
    until(&poller, "parts again", |u| matches!(u, Update::Parts(_)));
}

/// AG-HD-05: "The live poller re-reads `service.json` whenever the service
/// does not answer and reconnects to a service restarted on another port".
/// The old service is gone for good; the new one has another port, another
/// token (AG-HD-03) and a rebuilt part.
#[test]
fn poller_reconnects_to_a_service_restarted_on_another_port() {
    let (project, old, client) = connected("poll-restart", three_parts());
    let poller = Poller::spawn(client, FAST);
    until(&poller, "parts", |u| matches!(u, Update::Parts(_)));
    old.set_mode(Mode::Refuse);
    until(&poller, "unreachable", |u| matches!(u, Update::Unreachable));

    let mut parts = three_parts();
    parts[0] = FakePart::built("shaft", "current", "k-shaft-2", cube_qmesh([0.0; 3], 30.0));
    let new = FakeService::start(parts);
    assert_ne!(new.port, old.port, "test premise: another port");
    project.write_service_json(new.port);

    until(&poller, "parts again", |u| matches!(u, Update::Parts(_)));
    until(
        &poller,
        "the rebuilt shaft",
        |u| matches!(u, Update::Mesh { name, key, .. } if name == "shaft" && key == "k-shaft-2"),
    );
    // The poller talks to the new service with the new token, and goes on.
    let expected = token_for(new.port);
    let sent = new.tokens();
    for (request, token) in &sent {
        assert_eq!(token.as_deref(), Some(expected.as_str()), "token of `{request}`");
    }
    let polls = new.count("GET /status");
    collect(&poller, Duration::from_millis(300));
    assert!(new.count("GET /status") > polls, "polling the new service continues");
}

/// AG-HD-05: "a failed first connection is retried every poll interval
/// (WA-LV-05)". The app's poller (`Poller::open`) cannot start a service
/// (the program does not exist, so `connect` fails at once) and reports
/// `Unreachable`; when a service appears later, it connects without the app
/// being restarted. Bound: the service is found within `PATIENCE` (10 s =
/// 20 poll intervals of 0.5 s).
#[test]
fn poller_retries_a_failed_first_connection() {
    let project = TempProject::new("poll-late");
    let opened = ProjectDir::open(&project.dir).expect("valid project");
    let cmd = vec!["qoob3d-no-such-service-program".to_owned()];
    let poller = Poller::open(opened, cmd, || {});
    until(&poller, "unreachable", |u| matches!(u, Update::Unreachable));
    // Still nothing there for a few poll intervals.
    assert!(
        collect(&poller, Duration::from_millis(1200))
            .iter()
            .all(|u| matches!(u, Update::Unreachable)),
        "updates without a service"
    );

    let fake = FakeService::start(three_parts());
    project.write_service_json(fake.port);
    let seen = until(&poller, "parts", |u| matches!(u, Update::Parts(_)));
    let Some(Update::Parts(parts)) = seen.last() else { unreachable!() };
    assert_eq!(part_names(parts), ["shaft", "lid", "pin"]);
    until(&poller, "shaft", |u| matches!(u, Update::Mesh { name, .. } if name == "shaft"));
    // Polling goes on at the poll interval.
    let polls = fake.count("GET /status");
    collect(&poller, Duration::from_millis(1700));
    assert!(fake.count("GET /status") >= polls + 2, "polling stopped: {:?}", fake.log());
}

/// WA-LV-01: dropping the poller stops its thread (no requests afterwards).
#[test]
fn dropping_the_poller_stops_polling() {
    let (_project, fake, client) = connected("poll-drop", three_parts());
    let poller = Poller::spawn(client, FAST);
    until(&poller, "parts", |u| matches!(u, Update::Parts(_)));
    drop(poller);
    // One request may still be in flight; then nothing for 5 intervals.
    std::thread::sleep(Duration::from_millis(150));
    let before = fake.count("GET /status");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(fake.count("GET /status"), before, "still polling after drop");
}

// ================================================================== Live (view model)

fn part(name: &str, state: PartState, key: Option<&str>) -> Part {
    Part { name: name.into(), file: format!("parts/{name}.rhai"), state, key: key.map(Into::into) }
}

fn cube_mesh(s: f32) -> qoob3d_core::ShapeMesh {
    qoob3d_core::qmesh::read(&cube_qmesh([0.0; 3], s)).expect("valid qmesh")
}

fn live_with_two_meshes() -> Live {
    let mut live = Live::default();
    live.apply(Update::Parts(vec![
        part("shaft", PartState::Current, Some("k1")),
        part("lid", PartState::Current, Some("k2")),
    ]));
    live.apply(Update::Mesh { name: "shaft".into(), key: "k1".into(), mesh: cube_mesh(1.0) });
    live.apply(Update::Mesh { name: "lid".into(), key: "k2".into(), mesh: cube_mesh(2.0) });
    live
}

/// WA-LV-02/04: parts in status order with state; meshes attached by name;
/// new parts are visible.
#[test]
fn live_lists_parts_and_attaches_meshes() {
    let live = live_with_two_meshes();
    let names: Vec<_> = live.parts().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["shaft", "lid"]);
    for p in live.parts() {
        assert!(p.mesh.is_some(), "{} has no mesh", p.name);
        assert!(p.visible, "{} hidden by default", p.name);
        assert_eq!(p.state, PartState::Current);
    }
    assert!(live.reachable());
}

/// WA-LV-02: a part that disappears from the status disappears from the view.
#[test]
fn live_drops_vanished_parts() {
    let mut live = live_with_two_meshes();
    live.apply(Update::Parts(vec![part("shaft", PartState::Current, Some("k1"))]));
    let names: Vec<_> = live.parts().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["shaft"]);
    assert!(live.parts()[0].mesh.is_some(), "the remaining mesh must stay");
}

/// WA-PR-02/WA-LV-02: a failed rebuild keeps the last successful mesh
/// visible; the state changes to `Failed`.
#[test]
fn live_keeps_the_last_mesh_of_a_failed_part() {
    let mut live = live_with_two_meshes();
    live.apply(Update::Parts(vec![
        part("shaft", PartState::Failed, Some("k1")),
        part("lid", PartState::Outdated, Some("k2")),
    ]));
    assert_eq!(live.parts()[0].state, PartState::Failed);
    assert_eq!(live.parts()[1].state, PartState::Outdated);
    assert!(live.parts().iter().all(|p| p.mesh.is_some()));
}

/// WA-LV-05: unreachable → flag set, meshes stay; answering again clears it.
#[test]
fn live_keeps_meshes_while_the_service_is_unreachable() {
    let mut live = live_with_two_meshes();
    live.apply(Update::Unreachable);
    assert!(!live.reachable());
    assert_eq!(live.parts().len(), 2, "parts must stay listed");
    assert!(live.parts().iter().all(|p| p.mesh.is_some()), "meshes must stay visible");
    live.apply(Update::Parts(vec![
        part("shaft", PartState::Current, Some("k1")),
        part("lid", PartState::Current, Some("k2")),
    ]));
    assert!(live.reachable());
}

/// WA-LV-04: the eye toggles visibility; a status update keeps the choice.
#[test]
fn live_visibility_survives_status_updates() {
    let mut live = live_with_two_meshes();
    live.set_visible("lid", false);
    assert!(!live.parts()[1].visible);
    live.apply(Update::Parts(vec![
        part("shaft", PartState::Current, Some("k1")),
        part("lid", PartState::Outdated, Some("k2")),
    ]));
    assert!(!live.parts()[1].visible, "update made a hidden part visible");
    assert!(live.parts()[0].visible);
}

// ================================================================== CLI (binary)

/// Runs the real binary; kills it after `limit` (a window opening by
/// mistake must not hang the suite).
fn run_qoob3d(args: &[&std::ffi::OsStr], env_cmd: Option<&str>, limit: Duration) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qoob3d"));
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    match env_cmd {
        Some(v) => cmd.env("QOOB3D_SERVICE_CMD", v),
        None => cmd.env_remove("QOOB3D_SERVICE_CMD"),
    };
    let mut child = cmd.spawn().expect("start qoob3d");
    let start = Instant::now();
    while child.try_wait().expect("wait").is_none() {
        if start.elapsed() > limit {
            child.kill().ok();
            let out = child.wait_with_output().expect("output");
            panic!(
                "qoob3d did not exit within {limit:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().expect("output")
}

fn os<S: AsRef<std::ffi::OsStr> + ?Sized>(s: &S) -> &std::ffi::OsStr {
    s.as_ref()
}

/// WA-OP-01: `qoob3d --project <DIR>` without `qoob3d.toml` exits with 2 and
/// names the folder (before any window opens).
#[test]
fn cli_without_qoob3d_toml_exits_with_code_2() {
    let project = TempProject::new("cli-no-toml");
    let dir = project.not_a_project();
    let out = run_qoob3d(&[os("--project"), dir.as_os_str()], None, Duration::from_secs(30));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains(&dir.display().to_string()), "folder not named: {stderr}");
}

/// WA-OP-01: a folder that does not exist → exit 2, folder named.
#[test]
fn cli_with_a_missing_folder_exits_with_code_2() {
    let project = TempProject::new("cli-missing");
    let dir = project.root.join("does-not-exist");
    let out = run_qoob3d(&[os("--project"), dir.as_os_str()], None, Duration::from_secs(30));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains(&dir.display().to_string()), "folder not named: {stderr}");
}

// ================================================================== UI (egui_kittest)

type H = Harness<'static, QoobApp>;

fn harness(dir: &Path) -> H {
    let dir = dir.to_path_buf();
    let mut h =
        Harness::builder().with_size(egui::vec2(1440.0, 860.0)).wgpu().build_eframe(move |cc| {
            let options = StartOptions { project_dir: Some(dir), ..Default::default() };
            QoobApp::new(cc, options).expect("app starts")
        });
    h.run_steps(3);
    h
}

fn wait_until(h: &mut H, what: &str, done: impl Fn(&QoobApp) -> bool) {
    let start = Instant::now();
    while !done(h.state()) {
        assert!(start.elapsed() < PATIENCE, "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn steps_for(h: &mut H, time: Duration) {
    let start = Instant::now();
    while start.elapsed() < time {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn meshes_loaded(app: &QoobApp, n: usize) -> bool {
    app.live().is_some_and(|l| l.parts().iter().filter(|p| p.mesh.is_some()).count() == n)
}

fn ui_parts() -> Vec<FakePart> {
    vec![
        FakePart::built("shaft", "current", "k-shaft-1", cube_qmesh([0.0; 3], 10.0)),
        FakePart::built("lid", "outdated", "k-lid-1", cube_qmesh([20.0, 0.0, 0.0], 5.0)),
        FakePart::unbuilt("bracket", "failed"),
        FakePart::unbuilt("pin", "never_built"),
    ]
}

/// App opened on a project served by a fake with `ui_parts`, meshes loaded.
fn opened(tag: &str) -> (TempProject, FakeService, H) {
    let project = TempProject::new(tag);
    let fake = FakeService::start(ui_parts());
    project.write_service_json(fake.port);
    let mut h = harness(&project.dir);
    wait_until(&mut h, "parts listed and meshes loaded", |a| {
        a.live().is_some_and(|l| l.parts().len() == 4) && meshes_loaded(a, 2)
    });
    h.run_steps(2);
    (project, fake, h)
}

fn text_of(label: Option<String>, value: Option<String>) -> String {
    format!("{} {}", label.unwrap_or_default(), value.unwrap_or_default())
}

/// Rect of the browser row showing `name` together with `state`
/// (case-insensitive, one label per row).
fn row(h: &H, name: &str, state: &str) -> Option<egui::Rect> {
    let (name, state) = (name.to_lowercase(), state.to_lowercase());
    h.query_all_by(move |n| {
        let t = text_of(n.label(), n.value()).to_lowercase();
        t.contains(&name) && t.contains(&state)
    })
    .next()
    .map(|n| n.rect())
}

fn has_text(h: &H, text: &str) -> bool {
    let text = text.to_lowercase();
    h.query_all_by(move |n| text_of(n.label(), n.value()).to_lowercase().contains(&text))
        .next()
        .is_some()
}

/// Pixels in the browser band of `row` whose colour is the error red
/// (theme ERROR #E5534B): clearly red-dominant. Neutral text (#D4D7DC) and
/// the grey panel never pass `r > g + 70 && r > b + 70`.
fn red_pixels(h: &mut H, row: egui::Rect) -> usize {
    let right = h.state().viewport().layout().expect("viewport laid out").rect.left();
    let image = h.render().expect("render UI");
    let scale = image.width() as f32 / 1440.0;
    let (y0, y1) = ((row.top() * scale) as u32, (row.bottom() * scale) as u32);
    let x1 = ((right * scale) as u32).min(image.width());
    let mut count = 0;
    for y in y0..y1.min(image.height()) {
        for x in 0..x1 {
            let [r, g, b, _] = image.get_pixel(x, y).0.map(i32::from);
            if r > 150 && r > g + 70 && r > b + 70 {
                count += 1;
            }
        }
    }
    count
}

/// WA-LV-04: every part listed with name and state (English texts:
/// up to date / outdated / error / never built); the `failed` part is
/// drawn in the error colour, a healthy one is not.
#[test]
fn ui_browser_lists_project_parts_with_state_and_marks_errors() {
    let (_project, _fake, mut h) = opened("ui-list");
    for (name, state) in
        [("shaft", "up to date"), ("lid", "outdated"), ("bracket", "error"), ("pin", "never built")]
    {
        assert!(row(&h, name, state).is_some(), "no browser row '{name} … {state}'");
    }
    let failed = row(&h, "bracket", "error").expect("listed");
    let healthy = row(&h, "shaft", "up to date").expect("listed");
    assert!(red_pixels(&mut h, failed) >= 5, "failed part not in the error colour");
    assert_eq!(red_pixels(&mut h, healthy), 0, "healthy part drawn in the error colour");
}

/// WA-LV-04: German language pack (states as in the specification).
#[test]
fn ui_part_states_are_translated_to_german() {
    let (_project, _fake, mut h) = opened("ui-de");
    h.get_by_label("⚙ Settings").click();
    h.run_steps(2);
    h.get_by_label("Deutsch").click();
    h.run_steps(3);
    for (name, state) in
        [("shaft", "aktuell"), ("lid", "veraltet"), ("bracket", "fehler"), ("pin", "nie gebaut")]
    {
        assert!(row(&h, name, state).is_some(), "no browser row '{name} … {state}'");
    }
}

/// WA-LV-04: the eye in a part's row toggles its visibility.
#[test]
fn ui_eye_toggles_part_visibility() {
    let (_project, _fake, mut h) = opened("ui-eye");
    let lid = row(&h, "lid", "outdated").expect("listed");
    let visible = |h: &H| {
        h.state()
            .live()
            .expect("project open")
            .parts()
            .iter()
            .find(|p| p.name == "lid")
            .map(|p| p.visible)
    };
    assert_eq!(visible(&h), Some(true));
    let eye = h
        .query_all_by_label("👁")
        .map(|n| n.rect())
        .find(|r| (r.center().y - lid.center().y).abs() < lid.height() / 2.0)
        .expect("eye in the lid row");
    h.event(egui::Event::PointerMoved(eye.center()));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: eye.center(),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(2);
    assert_eq!(visible(&h), Some(false));
}

/// WA-LV-05: service stops → "service not reachable", meshes stay; it
/// answers again → the hint goes away and polling continues.
#[test]
fn ui_shows_service_not_reachable_and_keeps_meshes() {
    let (_project, fake, mut h) = opened("ui-down");
    assert!(!has_text(&h, "service not reachable"));
    fake.set_mode(Mode::Refuse);
    let start = Instant::now();
    while !has_text(&h, "service not reachable") {
        assert!(start.elapsed() < PATIENCE, "no 'service not reachable' hint");
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(meshes_loaded(h.state(), 2), "meshes vanished while unreachable");
    let polls = fake.count("GET /status");
    fake.set_mode(Mode::Serve);
    let start = Instant::now();
    while has_text(&h, "service not reachable") {
        assert!(start.elapsed() < PATIENCE, "hint stays after the service is back");
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(fake.count("GET /status") > polls);
    assert!(meshes_loaded(h.state(), 2));
}

/// WA-OP-03: opening the project requests exactly one build of all parts
/// (also while polling goes on for a few intervals).
#[test]
fn ui_requests_one_build_on_opening() {
    let (_project, fake, mut h) = opened("ui-build");
    let polls = fake.count("GET /status");
    let start = Instant::now();
    while fake.count("GET /status") < polls + 3 {
        assert!(start.elapsed() < PATIENCE, "polling stopped: {:?}", fake.log());
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    let builds = fake.builds();
    assert_eq!(builds.len(), 1, "log: {:?}", fake.log());
    let body: serde_json::Value = serde_json::from_str(&builds[0]).expect("JSON body");
    assert_eq!(body["parts"], serde_json::Value::Null);
}

/// WA-LV-01: HTTP never runs on the UI thread. With every `/status` answer
/// delayed by 2 s (after connecting), no frame may take as long: a frame
/// that waited for the poll would take ≥ 2 s; 1 s leaves room for slow
/// software rendering.
#[test]
fn ui_frames_do_not_wait_for_http() {
    let (_project, fake, mut h) = opened("ui-thread");
    fake.set_status_delay(Duration::from_secs(2));
    let start = Instant::now();
    let mut slowest = Duration::ZERO;
    while start.elapsed() < Duration::from_millis(2500) {
        let t = Instant::now();
        h.step();
        slowest = slowest.max(t.elapsed());
        std::thread::sleep(Duration::from_millis(10));
    }
    fake.set_status_delay(Duration::ZERO);
    assert!(slowest < Duration::from_secs(1), "a frame took {slowest:?}");
}

/// WA-LV-06 (SHOULD): the first meshes fit the camera onto the parts (a
/// 20 mm cube at 100..120 → target at its centre (110, 110, 10), as F6 does;
/// 1 mm is far below the cube size); a later mesh update (a cube elsewhere)
/// does not move the camera.
#[test]
fn ui_first_meshes_fit_the_camera_later_updates_do_not() {
    let project = TempProject::new("ui-fit");
    let fake = FakeService::start(vec![
        FakePart::built("shaft", "current", "k1", cube_qmesh([100.0, 100.0, 0.0], 20.0)),
        FakePart::unbuilt("pin", "never_built"),
    ]);
    project.write_service_json(fake.port);
    let mut h = harness(&project.dir);
    wait_until(&mut h, "mesh", |a| meshes_loaded(a, 1));
    steps_for(&mut h, Duration::from_millis(300));
    wait_until(&mut h, "camera settled", |a| !a.viewport().is_animating());
    let fitted = h.state().viewport().camera;
    let centre = glam::Vec3::new(110.0, 110.0, 10.0);
    assert!(fitted.target.distance(centre) < 1.0, "camera target {} not fitted", fitted.target);

    fake.set_parts(vec![
        FakePart::built("shaft", "current", "k2", cube_qmesh([300.0, 300.0, 0.0], 50.0)),
        FakePart::unbuilt("pin", "never_built"),
    ]);
    wait_until(&mut h, "new mesh", |a| {
        a.live().is_some_and(|l| l.parts().iter().any(|p| p.key.as_deref() == Some("k2")))
    });
    steps_for(&mut h, Duration::from_millis(300));
    wait_until(&mut h, "camera settled", |a| !a.viewport().is_animating());
    assert_eq!(h.state().viewport().camera, fitted, "a later update moved the camera");
}
