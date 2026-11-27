//! Client side of the project service (ADR-0053, ADR-0055): the project
//! folder, finding or starting `qoob3d serve`, and requests to it. Used by the
//! live view, the CLI clients, the headless render and the MCP server.
//!
//! HTTP is a minimal HTTP/1.1 client over `std::net` (localhost only, one
//! request per connection, `Connection: close`, timeouts). Port and token of
//! the service come from `.qoob3d/service.json` and are read again whenever
//! the service does not answer (AG-HD-05). **Every client of the service goes
//! through [`Client`]**, so it sends the token.

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use qoob3d_core::project::layout;
use qoob3d_core::{ShapeMesh, qmesh};
use serde::Deserialize;

/// A running service answers `GET /status` within this time (WA-OP-02).
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(1);
/// How long to wait for a freshly started service (WA-OP-02).
pub const START_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest wait of a headless render for the build (AG-RD-01).
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
        if !dir.join(layout::CONFIG_FILE).is_file() {
            bail!("{} is not a QOOB3D project (no qoob3d.toml)", dir.display());
        }
        Ok(Self { dir: dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()) })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Name of the folder, as the app shows it.
    pub fn name(&self) -> String {
        self.dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }

    fn service_json(&self) -> PathBuf {
        layout::service_json(&self.dir)
    }

    /// Port and token from `.qoob3d/service.json`, `None` if missing or
    /// invalid.
    fn service(&self) -> Option<ServiceJson> {
        let text = std::fs::read_to_string(self.service_json()).ok()?;
        serde_json::from_str(&text).ok()
    }
}

/// What the service wrote into `.qoob3d/service.json` (ADR-0055).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ServiceJson {
    port: u16,
    /// Sent as `X-Qoob3d-Token` with every request (AG-HD-03).
    #[serde(default)]
    token: Option<String>,
}

/// `text` as one segment of a request path (AG-HD-04): everything but
/// unreserved characters is percent-encoded, so a name never changes the path.
pub fn segment(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
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
    /// `#RRGGBB`, if the part declares a colour (AG-PJ-07).
    #[serde(default)]
    pub color: Option<String>,
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
    project: ProjectDir,
    service: ServiceJson,
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
    pub fn probe(project: &ProjectDir) -> Option<Self> {
        let client = Self { project: project.clone(), service: project.service()? };
        let response = client.request("GET", "/status", None, STATUS_TIMEOUT).ok()?;
        envelope::<StatusResult>(&response).ok().map(|_| client)
    }

    /// Reads `service.json` again (AG-HD-05): a restarted service has
    /// another port and token.
    pub fn reload(&mut self) {
        if let Some(service) = self.project.service().filter(|s| *s != self.service) {
            tracing::info!(port = service.port, "project service restarted, reconnecting");
            self.service = service;
        }
    }

    /// `GET /status` (WA-PR-01).
    pub fn status(&self) -> anyhow::Result<Vec<Part>> {
        let response = self.request("GET", "/status", None, REQUEST_TIMEOUT)?;
        Ok(envelope::<StatusResult>(&response)?.parts)
    }

    /// `GET /mesh/<name>` (WA-PR-02).
    pub fn mesh(&self, name: &str) -> anyhow::Result<PartMesh> {
        let response =
            self.request("GET", &format!("/mesh/{}", segment(name)), None, REQUEST_TIMEOUT)?;
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

    /// Any request; the JSON envelope whatever the HTTP status (CLI, AG-SV-02).
    pub fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
        timeout: Duration,
    ) -> anyhow::Result<serde_json::Value> {
        let body = body.map(serde_json::Value::to_string);
        let response = self.request(method, path, body.as_ref().map(String::as_bytes), timeout)?;
        serde_json::from_slice(&response.body)
            .with_context(|| format!("invalid service answer (HTTP {})", response.code))
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        timeout: Duration,
    ) -> anyhow::Result<Response> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.service.port));
        let mut stream = TcpStream::connect_timeout(&addr, timeout)
            .with_context(|| format!("connecting to the service at {addr}"))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let body = body.unwrap_or_default();
        let token = self.service.token.as_deref().unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nX-Qoob3d-Token: {token}\r\n\
             Connection: close\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n",
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
