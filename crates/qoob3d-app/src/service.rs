//! `qoob3d serve <DIR>`: the project service (AG-SV-01, agent.md §3.1,
//! docs/requirements/project-app.md §1, ADR-0053). HTTP on 127.0.0.1 with a
//! thread per connection, so `/status` never waits for a build; one job
//! runner thread; builds run in `qoob3d worker` processes (`pool.rs`), never
//! in this process. One service per folder: `.qoob3d/service.lock` is held
//! with an OS file lock, which also makes a stale `service.json` harmless.
//! Every request needs the token of this start from `service.json`, a local
//! `Host` and no `Origin` (AG-HD-03, ADR-0055).

mod bench;
mod checks;
mod discovery;
mod http;
mod jobs;
mod pool;
mod probe;
mod save;
pub(crate) mod snapshot;
mod tasks;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{BuildHasher as _, RandomState};
use std::io::Read as _;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use qoob3d_core::export::{self, ExportObject};
use qoob3d_core::fs::write_atomic;
use qoob3d_core::project::{Config, Limits, Mesh, ValueStatus, Values, Workers};
use qoob3d_core::qmesh;
use serde::Deserialize;
use serde_json::{Value, json};

use self::checks::CheckRecord;
use self::discovery::{Discovery, discovering};
use self::http::Reply;
use self::jobs::Job;
use self::pool::Pool;
use self::snapshot::{Record, Snapshot, cache_dir, views};
use crate::live::ProjectDir;
use crate::worker::{ErrorKind, PartError, Request};

/// How long shutdown waits for busy workers to be killed.
const STOP_WAIT: Duration = Duration::from_secs(3);
/// Read timeout of a client connection.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Jobs listed by `/status`, the newest ones (AG-HD-10).
const STATUS_JOBS: usize = 20;

pub(crate) struct Service {
    dir: PathBuf,
    port: u16,
    /// Random per start; required in `X-Qoob3d-Token` (AG-HD-03).
    token: String,
    pool: Arc<Pool>,
    /// Declarations of the part and check files (ADR-0056).
    discovery: Arc<Discovery>,
    state: Mutex<State>,
    /// Held while a task file is created or changed, so two requests never
    /// pick the same number or lose each other's log line (AG-HD-07).
    task_writes: Mutex<()>,
    /// Signalled on every job change.
    changed: Condvar,
    stopping: AtomicBool,
    /// Ids of worker requests.
    requests: AtomicU64,
}

#[derive(Default)]
struct State {
    jobs: BTreeMap<u64, Job>,
    queue: VecDeque<u64>,
    next_job: u64,
    records: HashMap<String, Record>,
    /// Last run of every check, by name.
    checks: HashMap<String, CheckRecord>,
}

/// 32 hex characters (128 bits) nobody outside can guess. The source is the
/// OS random device; where it cannot be read, two `RandomState` hashers,
/// which std keys from OS randomness (SipHash with 128 secret key bits each).
fn random_token() -> String {
    let mut bytes = [0u8; 16];
    let device = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if device.is_err() {
        let half = || RandomState::new().hash_one(std::process::id()).to_le_bytes();
        bytes[..8].copy_from_slice(&half());
        bytes[8..].copy_from_slice(&half());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs the service for the project in `dir` until `/shutdown`.
///
/// # Errors
/// No project, another service for the folder, or no port.
pub fn serve(dir: &Path) -> anyhow::Result<()> {
    let project = ProjectDir::open(dir)?;
    let dir = project.dir().to_path_buf();
    let data = dir.join(".qoob3d");
    std::fs::create_dir_all(&data).with_context(|| format!("creating {}", data.display()))?;
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data.join("service.lock"))
        .context("opening service.lock")?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            bail!("a service for {} is already running", dir.display())
        }
        Err(std::fs::TryLockError::Error(e)) => return Err(e).context("locking service.lock"),
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).context("binding a port")?;
    let port = listener.local_addr()?.port();
    // Read once at the start (AG-PF-06); a broken file is reported by `/status`.
    let workers = Config::load(&dir).map_or_else(|_| Workers::default(), |c| c.workers);
    let exe = std::env::current_exe().context("locating the qoob3d executable")?;
    let service = Arc::new(Service {
        pool: Arc::new(Pool::start(exe, workers.count, workers.recycle_mb)),
        dir,
        port,
        token: random_token(),
        task_writes: Mutex::new(()),
        discovery: Arc::default(),
        state: Mutex::new(State::default()),
        changed: Condvar::new(),
        stopping: AtomicBool::new(false),
        requests: AtomicU64::new(0),
    });
    let service_json = data.join("service.json");
    let info = json!({"port": port, "pid": std::process::id(), "token": service.token}).to_string();
    write_atomic(&service_json, info.as_bytes()).context("writing service.json")?;
    tracing::info!(port, dir = %service.dir.display(), "project service listening");

    // Discovery starts at once, so the first `/status` likely knows the parts.
    let _started = service.declarations(Some(Duration::ZERO));
    let runner = Arc::clone(&service);
    std::thread::Builder::new().name("qoob3d-jobs".into()).spawn(move || runner.run_jobs())?;
    for stream in listener.incoming() {
        if service.stopping.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let s = Arc::clone(&service);
        std::thread::spawn(move || s.connection(stream));
    }
    service.cancel_all();
    service.changed.notify_all();
    service.pool.close(STOP_WAIT);
    std::fs::remove_file(&service_json).ok();
    drop(lock);
    tracing::info!("project service stopped");
    Ok(())
}

/// A failed request: HTTP code and the envelope's `error`.
struct Failure {
    code: u16,
    error: Box<PartError>,
    /// `result` of the envelope (e.g. the build result of a failed export).
    result: Value,
}

impl Failure {
    fn new(code: u16, kind: ErrorKind, message: impl ToString) -> Self {
        Self { code, error: Box::new(PartError::new(kind, message)), result: Value::Null }
    }

    fn not_found(message: impl ToString) -> Self {
        Self::new(404, ErrorKind::NotFound, message)
    }

    fn bad_request(message: impl ToString) -> Self {
        Self::new(400, ErrorKind::BadRequest, message)
    }
}

impl From<PartError> for Failure {
    fn from(error: PartError) -> Self {
        Self { code: 400, error: Box::new(error), result: Value::Null }
    }
}

type Answer = Result<Value, Failure>;

fn body<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, Failure> {
    serde_json::from_slice(bytes)
        .map_err(|e| Failure::bad_request(format!("invalid JSON body: {e}")))
}

#[derive(Deserialize)]
struct BuildBody {
    #[serde(default)]
    parts: Option<Vec<String>>,
    #[serde(default)]
    wait_s: f64,
}

#[derive(Deserialize)]
struct ExportBody {
    parts: Vec<String>,
    format: String,
}

impl Service {
    /// The state, also after a job thread panicked while holding the lock
    /// (AG-HD-01): every update of the state is complete in itself.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// AG-HD-03: the token of this start, sent as a header; a `Host` that
    /// is this service (not a name a web page resolved to 127.0.0.1); no
    /// `Origin`, which browsers add to requests of pages.
    fn allowed(&self, request: &http::Request) -> bool {
        let token = request.token.as_deref().unwrap_or_default().as_bytes();
        // Compared without an early exit, so timing tells nothing.
        let same = token.len() == self.token.len()
            && token.iter().zip(self.token.as_bytes()).fold(0, |d, (a, b)| d | (a ^ b)) == 0;
        let local = request.host.as_deref().is_some_and(|host| {
            host == format!("127.0.0.1:{}", self.port) || host == format!("localhost:{}", self.port)
        });
        same && local && !request.origin
    }

    fn next_request(&self) -> u64 {
        self.requests.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn connection(&self, mut stream: TcpStream) {
        stream.set_read_timeout(Some(CLIENT_TIMEOUT)).ok();
        let start = Instant::now();
        let reply = match http::read_request(&stream) {
            Ok(request) if self.allowed(&request) => self.route(&request, start),
            Ok(_) => {
                let message = "missing or wrong X-Qoob3d-Token (see .qoob3d/service.json), \
                               a foreign Host or an Origin header";
                envelope(Err(Failure::new(403, ErrorKind::Forbidden, message)), start)
            }
            Err(e) => envelope(Err(Failure::bad_request(e)), start),
        };
        if let Err(e) = http::write_reply(&mut stream, reply) {
            tracing::debug!("answering a client: {e}");
        }
        drop(stream);
        if self.stopping.load(Ordering::SeqCst) {
            // Wakes the accept loop so it sees the stop flag.
            TcpStream::connect((Ipv4Addr::LOCALHOST, self.port)).ok();
        }
    }

    fn route(&self, request: &http::Request, start: Instant) -> Reply {
        let path: Vec<&str> = request.path.iter().map(String::as_str).collect();
        let answer = match (request.method.as_str(), path.as_slice()) {
            ("GET", ["status"]) => Ok(self.status()),
            ("POST", ["build"]) => self.build(&request.body),
            ("POST", ["discover"]) => self.discover_request(&request.body),
            ("GET", ["job", id]) => self.job(id),
            ("POST", ["job", id, "cancel"]) => job_id(id).and_then(|n| {
                self.cancel(n).ok_or_else(|| unknown_job(n))?;
                self.job(id)
            }),
            ("GET", ["mesh", part]) => match self.mesh(part) {
                Ok(reply) => return reply,
                Err(e) => Err(e),
            },
            ("POST", ["export"]) => self.export(&request.body),
            ("GET", ["values"]) => self.values(),
            ("POST", ["check"]) => self.check(&request.body),
            ("POST", ["run"]) => self.run(&request.body),
            ("POST", ["measure"]) => self.measure(&request.body),
            ("POST", ["save"]) => self.save(&request.body),
            ("GET", ["tasks"]) => self.tasks(),
            ("GET", ["task", id]) => self.task(id),
            ("POST", ["task"]) => self.task_create(&request.body),
            ("POST", ["task", id]) => self.task_update(id, &request.body),
            ("POST", ["shutdown"]) => {
                self.stopping.store(true, Ordering::SeqCst);
                self.changed.notify_all();
                Ok(Value::Null)
            }
            (method, _) => {
                Err(Failure::not_found(format!("no endpoint {method} /{}", request.path.join("/"))))
            }
        };
        envelope(answer, start)
    }

    /// `GET /status` (WA-PR-01, §3.1). It never waits for discovery
    /// (ADR-0056) and answers from the last known declarations: parts of a
    /// file under rediscovery are `outdated`, those of a file that no longer
    /// loads `failed`; `discovering` lists the files not known yet. Every
    /// file that does not load is in `load_errors`, the first one also in
    /// `error`; a load error of the whole project gives an empty part list.
    fn status(&self) -> Value {
        let (count, busy) = self.pool.counts();
        let (part_files, check_files) = self.declarations(Some(Duration::ZERO));
        let pending: Vec<String> = [&part_files, &check_files]
            .into_iter()
            .flatten()
            .flat_map(|files| discovering(files))
            .map(|f| f.display().to_string())
            .collect();
        let snapshot = part_files.and_then(|files| Snapshot::load(&self.dir, files));
        let declared = check_files.and_then(checks::load);
        let mut st = self.lock();
        let first = st.jobs.len().saturating_sub(STATUS_JOBS);
        let jobs: Vec<Value> = st
            .jobs
            .iter()
            .skip(first)
            .map(|(id, j)| json!({"id": id, "state": j.state, "kind": j.kind()}))
            .collect();
        let mut result = json!({
            "project": Value::Null,
            "parts": [],
            "checks": [],
            "jobs": jobs,
            "workers": {"count": count, "busy": busy, "recycled": self.pool.recycled()},
        });
        let mut load_errors = Vec::new();
        match snapshot {
            Ok(s) => {
                result["project"] = json!(s.config.name);
                result["parts"] = json!(views(&self.dir, &s, &mut st.records));
                load_errors.extend(s.load_errors);
            }
            Err(message) => {
                result["project"] = json!(Config::load(&self.dir).map(|c| c.name).ok());
                result["error"] = json!(PartError::new(ErrorKind::Load, message));
            }
        }
        match declared {
            Ok(set) => {
                result["checks"] = json!(checks::status(&set.sources, &st));
                load_errors.extend(set.load_errors);
            }
            Err(message) if result.get("error").is_none() => {
                result["error"] = json!(PartError::new(ErrorKind::Load, message));
            }
            Err(_) => {}
        }
        if result.get("error").is_none()
            && let Some(first) = load_errors.first()
        {
            result["error"] = json!(PartError::new(ErrorKind::Load, &first.message));
        }
        result["load_errors"] = json!(load_errors);
        result["discovering"] = json!(pending);
        result
    }

    /// `POST /build` (§3.1).
    fn build(&self, bytes: &[u8]) -> Answer {
        let request: BuildBody = body(bytes)?;
        let id = self.submit(request.parts)?;
        self.answer_job(id, request.wait_s)
    }

    /// Waits up to `wait_s` for job `id`: its result or error if finished,
    /// else `{"job": id, "state": …}` (§3.1).
    fn answer_job(&self, id: u64, wait_s: f64) -> Answer {
        let wait = Duration::try_from_secs_f64(wait_s).unwrap_or(Duration::ZERO);
        self.wait(id, Some(wait));
        let st = self.lock();
        let job = st.jobs.get(&id).ok_or_else(|| unknown_job(id))?;
        if !job.state.finished() {
            return Ok(json!({"job": id, "state": job.state}));
        }
        match &job.error {
            Some(e) => Err(Failure::from(e.clone())),
            None => Ok(job.result()),
        }
    }

    fn job(&self, id: &str) -> Answer {
        let id = job_id(id)?;
        self.lock().jobs.get(&id).map(|j| j.json(id)).ok_or_else(|| unknown_job(id))
    }

    /// `GET /mesh/<part>` (WA-PR-02): the qmesh of the last successful build.
    fn mesh(&self, part: &str) -> Result<Reply, Failure> {
        let snapshot = self.snapshot(Some(Duration::ZERO)).map_err(Failure::not_found)?;
        let key = {
            let mut st = self.lock();
            let views = views(&self.dir, &snapshot, &mut st.records);
            let view = views
                .into_iter()
                .find(|v| v.name == part)
                .ok_or_else(|| Failure::not_found(format!("unknown part {part:?}")))?;
            view.key.ok_or_else(|| Failure::not_found(format!("part {part:?} was never built")))?
        };
        let path = cache_dir(&self.dir, part).join(&key).with_extension("qmsh");
        let file = std::fs::File::open(&path)
            .map_err(|e| Failure::not_found(format!("{}: {e}", path.display())))?;
        Ok(Reply::Mesh { key, file })
    }

    /// `POST /export` (§3.1): builds the parts if needed, then writes
    /// `out/<part>.<format>`; STL and 3MF from the cached mesh (`[mesh]`
    /// settings, AG-PF-05), STEP from the BREP in a worker.
    fn export(&self, bytes: &[u8]) -> Answer {
        let request: ExportBody = body(bytes)?;
        let format = request.format.as_str();
        if !matches!(format, "stl" | "3mf" | "step") {
            return Err(Failure::bad_request(format!(
                "unknown format {format:?}: use stl, 3mf or step"
            )));
        }
        if request.parts.is_empty() {
            return Err(Failure::bad_request("no parts to export"));
        }
        let id = self.submit(Some(request.parts.clone()))?;
        self.wait(id, None);
        let keys = {
            let st = self.lock();
            let job = st.jobs.get(&id).ok_or_else(|| unknown_job(id))?;
            if let Some(e) = &job.error {
                return Err(Failure::from(e.clone()));
            }
            let failures = job.failures();
            if !failures.is_empty() {
                let message = format!("not exported, parts failed: {}", failures.join("; "));
                let mut f = Failure::new(400, ErrorKind::Script, message);
                f.result = job.result();
                return Err(f);
            }
            request.parts.iter().map(|p| job.key(p).unwrap_or_default()).collect::<Vec<_>>()
        };
        let out = self.dir.join("out");
        std::fs::create_dir_all(&out)
            .map_err(|e| Failure::new(500, ErrorKind::Io, format!("{}: {e}", out.display())))?;
        let mut files = Vec::new();
        for (part, key) in request.parts.iter().zip(keys) {
            let file = format!("out/{part}.{format}");
            let base = cache_dir(&self.dir, part).join(key);
            if format == "step" {
                self.export_step(part, &base.with_extension("brep"), &file)?;
            } else {
                export_mesh(part, &base.with_extension("qmsh"), &self.dir.join(&file), format)
                    .map_err(|e| Failure::new(500, ErrorKind::Io, e))?;
            }
            files.push(file);
        }
        Ok(json!({"files": files}))
    }

    fn export_step(&self, part: &str, brep: &Path, file: &str) -> Result<(), Failure> {
        let limits = Config::load(&self.dir).map_or_else(|_| Limits::default(), |c| c.limits);
        let request = Request {
            job: self.next_request(),
            dir: self.dir.clone(),
            part: None,
            file: None,
            deps: BTreeMap::from([(part.to_owned(), brep.to_owned())]),
            step: Some(PathBuf::from(file)),
            mesh: Mesh::default(),
            cache: None,
            probe: None,
        };
        let answer = self.pool.run(&request, &limits, &AtomicBool::new(false))?;
        match answer.error {
            Some(e) => Err(Failure { code: 500, error: Box::new(e), result: Value::Null }),
            None => Ok(()),
        }
    }

    /// `GET /values`: every target value and the assumption report (AG-PJ-03).
    fn values(&self) -> Answer {
        let values = Values::load(&self.dir).map_err(|e| Failure::bad_request(e.to_string()))?;
        let list: Vec<Value> = values
            .iter()
            .map(|v| {
                json!({
                    "id": v.id,
                    "value": v.value,
                    "source": v.source,
                    "status": status_name(v.status),
                    "unit": v.unit,
                    "file": v.file.display().to_string(),
                })
            })
            .collect();
        let assumptions: Vec<&str> = values.assumptions().iter().map(|v| v.id.as_str()).collect();
        Ok(json!({"values": list, "assumptions": assumptions}))
    }
}

fn status_name(status: ValueStatus) -> &'static str {
    match status {
        ValueStatus::Measured => "measured",
        ValueStatus::OneSource => "one_source",
        ValueStatus::Assumption => "assumption",
    }
}

/// STL or 3MF (one object) of the cached mesh `qmsh` to `out`.
fn export_mesh(part: &str, qmsh: &Path, out: &Path, format: &str) -> Result<(), String> {
    let bytes = std::fs::read(qmsh).map_err(|e| format!("{}: {e}", qmsh.display()))?;
    let mesh = qmesh::read(&bytes).map_err(|e| format!("{}: {e:?}", qmsh.display()))?;
    drop(bytes);
    let objects = [ExportObject { name: part.to_owned(), mesh: &mesh.faces }];
    let data = if format == "3mf" { export::three_mf(&objects) } else { export::stl(&objects) };
    write_atomic(out, &data).map_err(|e| format!("{}: {e}", out.display()))
}

fn job_id(id: &str) -> Result<u64, Failure> {
    id.parse().map_err(|_| Failure::not_found(format!("unknown job {id:?}")))
}

fn unknown_job(id: u64) -> Failure {
    Failure::not_found(format!("unknown job {id}"))
}

/// `qoob3d bench <DIR>` (AG-PF-06): prints the envelope with the measures
/// per part; the process exit code.
pub fn bench(dir: &Path) -> i32 {
    let start = Instant::now();
    let (code, body) = envelope_json(bench::run(dir), start);
    println!("{body}");
    i32::from(code != 200)
}

fn envelope(answer: Answer, start: Instant) -> Reply {
    let (code, body) = envelope_json(answer, start);
    Reply::Json { code, body: body.to_string() }
}

/// HTTP code and the JSON envelope of every answer (project-app.md §1).
fn envelope_json(answer: Answer, start: Instant) -> (u16, Value) {
    let duration_s = start.elapsed().as_secs_f64();
    match answer {
        Ok(result) => {
            (200, json!({"ok": true, "duration_s": duration_s, "warnings": [], "result": result}))
        }
        Err(f) => (
            f.code,
            json!({
                "ok": false,
                "duration_s": duration_s,
                "warnings": [],
                "result": f.result,
                "error": f.error,
            }),
        ),
    }
}
