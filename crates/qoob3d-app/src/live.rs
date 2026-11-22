//! Live view of a script project (W1, docs/requirements/project-app.md,
//! ADR-0053): open the folder, find or start `qoob3d serve`, poll its status
//! in a background thread and load the meshes of parts whose cache key
//! changed. Named `live` because `crate::project` is the `.qbd` document.
//!
//! HTTP is a minimal HTTP/1.1 client over `std::net` (localhost only, one
//! request per connection, `Connection: close`, timeouts) and never runs on
//! the UI thread (WA-LV-01).

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use qoob3d_core::{ShapeMesh, qmesh};
use serde::Deserialize;

/// Status poll interval of the live view (WA-LV-01).
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// A running service answers `GET /status` within this time (WA-OP-02).
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(1);
/// How long to wait for a freshly started service (WA-OP-02).
pub const START_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest wait of `--project --render` for the build (WA-HL-01).
pub const BUILD_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout of requests to a connected service: a busy service may answer
/// slower than the 1 s start probe without counting as gone.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest body buffer reserved up front from a `Content-Length`.
const MAX_PREALLOC: usize = 64 << 20;

/// A project folder (with `qoob3d.toml`, WA-OP-01).
#[derive(Debug, Clone)]
pub struct ProjectDir {
    dir: PathBuf,
}

impl ProjectDir {
    /// Checks that `dir` exists and holds `qoob3d.toml`; errors name the folder.
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        if !dir.is_dir() {
            bail!("project folder {} does not exist", dir.display());
        }
        if !dir.join("qoob3d.toml").is_file() {
            bail!("{} is not a QOOB3D project (no qoob3d.toml)", dir.display());
        }
        Ok(Self { dir: dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()) })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn service_json(&self) -> PathBuf {
        self.dir.join(".qoob3d").join("service.json")
    }

    /// Port from `.qoob3d/service.json`, `None` if missing or invalid.
    fn port(&self) -> Option<u16> {
        #[derive(Deserialize)]
        struct ServiceJson {
            port: u16,
        }
        let text = std::fs::read_to_string(self.service_json()).ok()?;
        serde_json::from_str::<ServiceJson>(&text).ok().map(|s| s.port)
    }
}

/// The program that starts the service (WA-OP-02): the words of
/// `QOOB3D_SERVICE_CMD`, or this executable if unset or blank.
pub fn service_command(env_value: Option<&str>) -> Vec<String> {
    let words: Vec<String> =
        env_value.unwrap_or_default().split_whitespace().map(str::to_owned).collect();
    if !words.is_empty() {
        return words;
    }
    let own = std::env::current_exe().map_or_else(|_| "qoob3d".into(), |p| p.display().to_string());
    vec![own]
}

/// [`service_command`] for the current environment.
pub fn service_command_from_env() -> Vec<String> {
    service_command(std::env::var("QOOB3D_SERVICE_CMD").ok().as_deref())
}

/// Build state of a part (WA-PR-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartState {
    Current,
    Outdated,
    Failed,
    NeverBuilt,
}

/// One part as `GET /status` reports it; other fields are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Part {
    pub name: String,
    pub file: String,
    pub state: PartState,
    /// Cache key of the last successful build.
    pub key: Option<String>,
}

/// A part's mesh with the cache key it belongs to (`X-Qoob3d-Key`, WA-PR-02).
#[derive(Debug)]
pub struct PartMesh {
    pub key: String,
    pub mesh: ShapeMesh,
}

/// Connection to a running service on 127.0.0.1.
#[derive(Debug, Clone)]
pub struct Client {
    addr: SocketAddr,
}

impl Client {
    /// Uses the service of `project` if it answers `GET /status` within
    /// [`STATUS_TIMEOUT`]; otherwise starts `<cmd> serve <DIR>` detached and
    /// waits up to `wait` for `service.json` and a successful status (WA-OP-02).
    pub fn connect(project: &ProjectDir, cmd: &[String], wait: Duration) -> anyhow::Result<Self> {
        if let Some(client) = Self::probe(project) {
            return Ok(client);
        }
        start_service(project, cmd)?;
        let deadline = Instant::now() + wait;
        loop {
            if let Some(client) = Self::probe(project) {
                return Ok(client);
            }
            if Instant::now() >= deadline {
                bail!(
                    "the service for {} did not come up within {} s",
                    project.dir().display(),
                    wait.as_secs_f32()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The service from `service.json`, if it answers in time.
    fn probe(project: &ProjectDir) -> Option<Self> {
        let client = Self { addr: SocketAddr::from((Ipv4Addr::LOCALHOST, project.port()?)) };
        let response = client.request("GET", "/status", None, STATUS_TIMEOUT).ok()?;
        envelope::<StatusResult>(&response).ok().map(|_| client)
    }

    /// `GET /status` (WA-PR-01).
    pub fn status(&self) -> anyhow::Result<Vec<Part>> {
        let response = self.request("GET", "/status", None, REQUEST_TIMEOUT)?;
        Ok(envelope::<StatusResult>(&response)?.parts)
    }

    /// `GET /mesh/<name>` (WA-PR-02).
    pub fn mesh(&self, name: &str) -> anyhow::Result<PartMesh> {
        let response = self.request("GET", &format!("/mesh/{name}"), None, REQUEST_TIMEOUT)?;
        if response.code != 200 {
            envelope::<serde_json::Value>(&response)?;
            bail!("mesh of {name}: HTTP {}", response.code);
        }
        let mesh = qmesh::read(&response.body).with_context(|| format!("mesh of {name}"))?;
        Ok(PartMesh { key: response.key.unwrap_or_default(), mesh })
    }

    /// `POST /build` of all parts without waiting (WA-PR-03).
    pub fn build_all(&self) -> anyhow::Result<()> {
        let body = br#"{"parts": null, "wait_s": 0}"#;
        let response = self.request("POST", "/build", Some(body), REQUEST_TIMEOUT)?;
        envelope::<serde_json::Value>(&response).map(drop)
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> anyhow::Result<Response> {
        let mut stream = TcpStream::connect_timeout(&self.addr, timeout)
            .with_context(|| format!("connecting to the service at {}", self.addr))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let body = body.unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            self.addr,
            body.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(body)?;
        read_response(&mut stream).with_context(|| format!("{method} {path}"))
    }
}

/// Starts `<cmd> serve <DIR>` detached: own process group, no inherited
/// stdio, so it outlives the app (WA-OP-02).
fn start_service(project: &ProjectDir, cmd: &[String]) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt as _;
    let (program, args) = cmd.split_first().context("empty service command")?;
    let mut child = Command::new(program)
        .args(args)
        .arg("serve")
        .arg(project.dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| format!("starting the service with {program}"))?;
    tracing::info!(program, pid = child.id(), "started project service");
    // Reap the child when it exits, so it never lingers as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

struct Response {
    code: u16,
    /// `X-Qoob3d-Key` header.
    key: Option<String>,
    body: Vec<u8>,
}

fn read_response(stream: &mut TcpStream) -> anyhow::Result<Response> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 64 * 1024 {
            bail!("HTTP header too long");
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            bail!("connection closed before the HTTP header ended");
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&buf[..head_end]).context("HTTP header is not UTF-8")?;
    let mut lines = head.lines();
    let code = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .context("malformed HTTP status line")?;
    let (mut length, mut key) = (None, None);
    for (name, value) in lines.filter_map(|l| l.split_once(':')) {
        let (name, value) = (name.trim(), value.trim());
        if name.eq_ignore_ascii_case("Content-Length") {
            length = Some(value.parse::<usize>().context("malformed Content-Length")?);
        } else if name.eq_ignore_ascii_case("X-Qoob3d-Key") {
            key = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("Transfer-Encoding") {
            bail!("unsupported Transfer-Encoding {value}");
        }
    }
    let mut body = buf.split_off(head_end);
    match length {
        Some(length) => {
            body.reserve(length.saturating_sub(body.len()).min(MAX_PREALLOC));
            let missing = length.saturating_sub(body.len()) as u64;
            stream.take(missing).read_to_end(&mut body)?;
            if body.len() < length {
                bail!("HTTP body truncated: {} of {length} bytes", body.len());
            }
            body.truncate(length);
        }
        None => {
            stream.read_to_end(&mut body)?;
        }
    }
    Ok(Response { code, key, body })
}

#[derive(Deserialize)]
struct StatusResult {
    parts: Vec<Part>,
}

/// The JSON envelope `{ok, result, error}` of every service answer.
fn envelope<T: serde::de::DeserializeOwned>(response: &Response) -> anyhow::Result<T> {
    #[derive(Deserialize)]
    struct Envelope<T> {
        ok: bool,
        result: Option<T>,
        error: Option<ServiceError>,
    }
    #[derive(Deserialize)]
    struct ServiceError {
        kind: String,
        message: String,
    }
    let parsed: Envelope<T> = serde_json::from_slice(&response.body)
        .with_context(|| format!("invalid service answer (HTTP {})", response.code))?;
    if !parsed.ok {
        let (kind, message) = parsed.error.map_or_else(Default::default, |e| (e.kind, e.message));
        bail!("service error {kind} (HTTP {}): {message}", response.code);
    }
    parsed.result.context("service answer without result")
}

// ------------------------------------------------------------------ polling

/// What the poller hands to the UI.
#[derive(Debug)]
pub enum Update {
    /// The status changed (or answered again after being unreachable).
    Parts(Vec<Part>),
    /// A part's mesh with a key different from the one loaded before.
    Mesh { name: String, key: String, mesh: ShapeMesh },
    /// The service does not answer (WA-LV-05).
    Unreachable,
}

/// Background thread polling the service (WA-LV-01/02/05). Dropping it
/// stops the thread after its current request.
pub struct Poller {
    updates: Receiver<Update>,
    /// Dropped with the poller; the thread sees the disconnect and ends.
    _stop: Sender<()>,
}

impl Poller {
    /// Polls `client` every `interval`.
    pub fn spawn(client: Client, interval: Duration) -> Self {
        Self::start(move || Some(client), interval, || {})
    }

    /// For the app (WA-OP-02/03): connects (starting the service if needed)
    /// and requests one build of all parts in the background, then polls
    /// every [`POLL_INTERVAL`]. `notify` runs after every update (repaint).
    pub fn open(project: ProjectDir, cmd: Vec<String>, notify: impl Fn() + Send + 'static) -> Self {
        let connect = move || match Client::connect(&project, &cmd, START_TIMEOUT) {
            Ok(client) => {
                if let Err(err) = client.build_all() {
                    tracing::warn!("build request failed: {err:#}");
                }
                Some(client)
            }
            Err(err) => {
                tracing::error!("project service: {err:#}");
                None
            }
        };
        Self::start(connect, POLL_INTERVAL, notify)
    }

    fn start(
        connect: impl FnOnce() -> Option<Client> + Send + 'static,
        interval: Duration,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, updates) = mpsc::channel();
        let (stop, stopped) = mpsc::channel();
        std::thread::Builder::new()
            .name("qoob3d-live".into())
            .spawn(move || {
                let send = |u: Update| {
                    let sent = tx.send(u).is_ok();
                    notify();
                    sent
                };
                let Some(client) = connect() else {
                    send(Update::Unreachable);
                    return;
                };
                poll_loop(&client, interval, &stopped, send);
            })
            .expect("spawning the live poller thread");
        Self { updates, _stop: stop }
    }

    pub fn updates(&self) -> &Receiver<Update> {
        &self.updates
    }
}

/// One status request per `interval`; meshes only for parts whose key
/// differs from the key of the mesh last loaded (WA-LV-02).
fn poll_loop(
    client: &Client,
    interval: Duration,
    stopped: &Receiver<()>,
    send: impl Fn(Update) -> bool,
) {
    let mut loaded: HashMap<String, String> = HashMap::new();
    // Last parts sent; `None` after an outage, so recovery is always reported.
    let mut last: Option<Vec<Part>> = None;
    let mut reachable = true;
    let stop_requested = || !matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty));
    loop {
        match client.status() {
            Ok(parts) => {
                reachable = true;
                // A vanished part is forgotten, so it is fetched again on return.
                loaded.retain(|name, _| parts.iter().any(|p| p.name == *name));
                if last.as_ref() != Some(&parts) {
                    if !send(Update::Parts(parts.clone())) {
                        return;
                    }
                    last = Some(parts.clone());
                }
                for part in &parts {
                    let Some(key) = &part.key else { continue };
                    if loaded.get(&part.name) == Some(key) || stop_requested() {
                        continue;
                    }
                    match client.mesh(&part.name) {
                        Ok(m) => {
                            loaded.insert(part.name.clone(), m.key.clone());
                            let update =
                                Update::Mesh { name: part.name.clone(), key: m.key, mesh: m.mesh };
                            if !send(update) {
                                return;
                            }
                        }
                        Err(err) => tracing::warn!("loading {}: {err:#}", part.name),
                    }
                }
            }
            Err(err) => {
                if reachable {
                    tracing::warn!("project service not reachable: {err:#}");
                    reachable = false;
                    last = None;
                    if !send(Update::Unreachable) {
                        return;
                    }
                }
            }
        }
        if stopped.recv_timeout(interval) != Err(RecvTimeoutError::Timeout) {
            return;
        }
    }
}

// ------------------------------------------------------------------ view model

/// A part as the app shows it.
#[derive(Debug)]
pub struct LivePart {
    pub name: String,
    pub file: String,
    pub state: PartState,
    /// Cache key from the latest status.
    pub key: Option<String>,
    /// Mesh of the last successful build; stays while a rebuild fails.
    pub mesh: Option<ShapeMesh>,
    pub visible: bool,
}

/// What the app shows of a project: parts in status order (WA-LV-02/04/05).
#[derive(Debug, Default)]
pub struct Live {
    parts: Vec<LivePart>,
    unreachable: bool,
    /// Bumped whenever the drawn geometry changes (mesh, visibility, parts).
    version: u64,
}

impl Live {
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Parts(parts) => {
                self.unreachable = false;
                let mut old: HashMap<String, LivePart> =
                    self.parts.drain(..).map(|p| (p.name.clone(), p)).collect();
                self.parts = parts
                    .into_iter()
                    .map(|p| {
                        let prev = old.remove(&p.name);
                        LivePart {
                            visible: prev.as_ref().is_none_or(|q| q.visible),
                            mesh: prev.and_then(|q| q.mesh),
                            name: p.name,
                            file: p.file,
                            state: p.state,
                            key: p.key,
                        }
                    })
                    .collect();
                if old.values().any(|p| p.mesh.is_some()) {
                    self.version += 1;
                }
            }
            Update::Mesh { name, mesh, .. } => {
                if let Some(part) = self.parts.iter_mut().find(|p| p.name == name) {
                    part.mesh = Some(mesh);
                    self.version += 1;
                }
            }
            Update::Unreachable => self.unreachable = true,
        }
    }

    pub fn parts(&self) -> &[LivePart] {
        &self.parts
    }

    /// False while the service does not answer.
    pub fn reachable(&self) -> bool {
        !self.unreachable
    }

    pub fn set_visible(&mut self, name: &str, visible: bool) {
        if let Some(part) = self.parts.iter_mut().find(|p| p.name == name)
            && part.visible != visible
        {
            part.visible = visible;
            self.version += 1;
        }
    }

    /// Changes whenever [`Live::scene`] would change.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn has_meshes(&self) -> bool {
        self.parts.iter().any(|p| p.mesh.is_some())
    }

    /// The visible meshes merged for the renderer and picking.
    pub fn scene(&self) -> ShapeMesh {
        ShapeMesh::merge(self.parts.iter().filter(|p| p.visible).filter_map(|p| p.mesh.as_ref()))
    }
}

// ------------------------------------------------------------------ headless

/// `--project --render` (WA-HL-01): connect, build all, wait until no part is
/// outdated or never built (at most [`BUILD_TIMEOUT`]), load every mesh.
/// Returns the merged meshes and the names of failed parts.
pub fn build_and_load(
    project: &ProjectDir,
    cmd: &[String],
) -> anyhow::Result<(ShapeMesh, Vec<String>)> {
    let client = Client::connect(project, cmd, START_TIMEOUT)?;
    client.build_all()?;
    let deadline = Instant::now() + BUILD_TIMEOUT;
    let parts = loop {
        let parts = client.status()?;
        let pending = |p: &&Part| matches!(p.state, PartState::Outdated | PartState::NeverBuilt);
        if !parts.iter().any(|p| pending(&p)) {
            break parts;
        }
        if Instant::now() >= deadline {
            let names: Vec<&str> = parts.iter().filter(pending).map(|p| p.name.as_str()).collect();
            bail!("build not finished after {} s: {}", BUILD_TIMEOUT.as_secs(), names.join(", "));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    let mut meshes = Vec::new();
    for part in parts.iter().filter(|p| p.key.is_some()) {
        meshes.push(client.mesh(&part.name)?.mesh);
    }
    let failed =
        parts.into_iter().filter(|p| p.state == PartState::Failed).map(|p| p.name).collect();
    Ok((ShapeMesh::merge(&meshes), failed))
}
