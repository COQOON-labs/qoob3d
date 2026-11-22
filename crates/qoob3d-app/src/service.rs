//! `qoob3d serve <DIR>`: the project service (AG-SV-01, agent.md §3.1,
//! docs/requirements/project-app.md §1, ADR-0053). HTTP on 127.0.0.1 with a
//! thread per connection, so `/status` never waits for a build; one job
//! runner thread; builds run in `qoob3d worker` processes (`pool.rs`), never
//! in this process. One service per folder: `.qoob3d/service.lock` is held
//! with an OS file lock, which also makes a stale `service.json` harmless.

mod bench;
mod checks;
mod http;
mod jobs;
mod pool;
mod probe;
mod save;
pub(crate) mod snapshot;
mod tasks;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use qoob3d_core::export::{self, ExportObject};
use qoob3d_core::project::{Config, Limits, Mesh, ValueStatus, Values, Workers};
use qoob3d_core::qmesh;
use serde::Deserialize;
use serde_json::{Value, json};

use self::checks::{CheckFiles, CheckRecord};
use self::http::Reply;
use self::jobs::Job;
use self::pool::Pool;
use self::snapshot::{Declarations, Record, Snapshot, cache_dir, views};
use crate::live::ProjectDir;
use crate::worker::{ErrorKind, PartError, Request};

/// How long shutdown waits for busy workers to be killed.
const STOP_WAIT: Duration = Duration::from_secs(3);
/// Read timeout of a client connection.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct Service {
    dir: PathBuf,
    port: u16,
    pool: Pool,
    declarations: Declarations,
    check_files: CheckFiles,
    state: Mutex<State>,
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

/// Writes `bytes` to `path` through a temporary file and a rename, so
/// readers never see a half-written file.
///
/// # Errors
/// Writing or renaming failed.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
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
        pool: Pool::start(exe, workers.count, workers.recycle_mb),
        dir,
        port,
        declarations: Declarations::default(),
        check_files: CheckFiles::default(),
        state: Mutex::new(State::default()),
        changed: Condvar::new(),
        stopping: AtomicBool::new(false),
        requests: AtomicU64::new(0),
    });
    let service_json = data.join("service.json");
    let info = json!({"port": port, "pid": std::process::id()}).to_string();
    write_atomic(&service_json, info.as_bytes()).context("writing service.json")?;
    tracing::info!(port, dir = %service.dir.display(), "project service listening");

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
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("service lock poisoned")
    }

    fn next_request(&self) -> u64 {
        self.requests.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn connection(&self, mut stream: TcpStream) {
        stream.set_read_timeout(Some(CLIENT_TIMEOUT)).ok();
        let start = Instant::now();
        let reply = match http::read_request(&stream) {
            Ok(request) => self.route(&request, start),
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

    /// `GET /status` (WA-PR-01, §3.1); a load error is reported in `error`
    /// with an empty part list, so the service stays reachable.
    fn status(&self) -> Value {
        let (count, busy) = self.pool.counts();
        let snapshot = Snapshot::load(&self.dir, &self.declarations);
        let declared = checks::load(&self.dir, &self.check_files);
        let mut st = self.lock();
        let jobs: Vec<Value> =
            st.jobs.iter().map(|(id, j)| json!({"id": id, "state": j.state})).collect();
        let mut result = json!({
            "project": Value::Null,
            "parts": [],
            "checks": [],
            "jobs": jobs,
            "workers": {"count": count, "busy": busy, "recycled": self.pool.recycled()},
        });
        match snapshot {
            Ok(s) => {
                result["project"] = json!(s.config.name);
                result["parts"] = json!(views(&self.dir, &s, &mut st.records));
            }
            Err(message) => {
                result["project"] = json!(Config::load(&self.dir).map(|c| c.name).ok());
                result["error"] = json!(PartError::new(ErrorKind::Load, message));
            }
        }
        match declared {
            Ok(sources) => result["checks"] = json!(checks::status(&sources, &st)),
            Err(message) if result.get("error").is_none() => {
                result["error"] = json!(PartError::new(ErrorKind::Load, message));
            }
            Err(_) => {}
        }
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
        let snapshot = Snapshot::load(&self.dir, &self.declarations).map_err(Failure::not_found)?;
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
