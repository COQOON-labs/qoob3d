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
mod hashes;
mod http;
mod jobs;
mod pool;
mod probe;
mod save;
mod slicer;
mod slots;
pub(crate) mod snapshot;
mod summary;
mod tasks;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{BuildHasher as _, RandomState};
use std::io::Read as _;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use qoob3d_core::export::{self, ExportObject};
use qoob3d_core::fs::write_atomic;
use qoob3d_core::project::layout;
use qoob3d_core::project::{Config, Limits, Mesh, ServiceSettings, ValueStatus, Values, Workers};
use qoob3d_core::qmesh;
use serde::Deserialize;
use serde_json::{Value, json};

use self::checks::CheckRecord;
use self::discovery::{Discovery, discovering};
use self::hashes::Hashes;
use self::http::Reply;
use self::jobs::Job;
use self::pool::Pool;
use self::slots::{Slot, Slots};
use self::snapshot::{Hashing, Record, Snapshot};
use crate::client::ProjectDir;
use crate::worker::{ErrorKind, PartError, Request};

/// How long shutdown waits for busy workers to be killed.
const STOP_WAIT: Duration = Duration::from_secs(3);
/// Read timeout of a client connection.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Jobs listed by `/status`, the newest ones (AG-HD-10).
const STATUS_JOBS: usize = 20;
/// How often the idle time is looked at (WS-PRE-05).
const IDLE_TICK: Duration = Duration::from_secs(1);

pub(crate) struct Service {
    dir: PathBuf,
    port: u16,
    /// Random per start; required in `X-Qoob3d-Token` (AG-HD-03).
    token: String,
    pool: Arc<Pool>,
    /// Declarations of the part and check files (ADR-0055).
    discovery: Arc<Discovery>,
    /// Digests of the files behind keys and declarations (WS-PRE-01).
    hashes: Arc<Hashes>,
    state: Mutex<State>,
    /// Held while a task file is created or changed, so two requests never
    /// pick the same number or lose each other's log line (AG-HD-07).
    task_writes: Mutex<()>,
    /// Signalled on every job change.
    changed: Condvar,
    stopping: AtomicBool,
    /// Ids of worker requests.
    requests: AtomicU64,
    /// When the last allowed request came or ended (WS-PRE-05).
    last_request: Mutex<Instant>,
    /// The machine-wide worker budget (WS-PRE-06).
    slots: Slots,
    /// Slots this service holds, and requests that wait for one.
    slots_held: AtomicUsize,
    slots_waiting: AtomicUsize,
}

/// The reason of a job that waits for the machine-wide budget (WS-PRE-06).
const WAITING_FOR_SLOT: &str = "waiting for a worker slot";
/// How often a request that waits for a slot tries again.
const SLOT_POLL: Duration = Duration::from_millis(50);

/// A slot of the budget held by a request of this service.
struct Held<'a> {
    service: &'a Service,
    _slot: Option<Slot>,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.service.slots_held.fetch_sub(1, Ordering::SeqCst);
    }
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
    let data = layout::data_dir(&dir);
    std::fs::create_dir_all(&data).with_context(|| format!("creating {}", data.display()))?;
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data.join(layout::SERVICE_LOCK))
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
    let config = Config::load(&dir).ok();
    let workers = config.as_ref().map_or_else(Workers::default, |c| c.workers.clone());
    let idle = config.map_or_else(ServiceSettings::default, |c| c.service).idle_min;
    let exe = std::env::current_exe().context("locating the qoob3d executable")?;
    let service = Arc::new(Service {
        pool: Arc::new(Pool::start(exe, workers.count, workers.recycle_mb)),
        dir,
        port,
        token: random_token(),
        task_writes: Mutex::new(()),
        discovery: Arc::default(),
        hashes: Arc::default(),
        state: Mutex::new(State::default()),
        changed: Condvar::new(),
        stopping: AtomicBool::new(false),
        requests: AtomicU64::new(0),
        last_request: Mutex::new(Instant::now()),
        slots: Slots::new(),
        slots_held: AtomicUsize::new(0),
        slots_waiting: AtomicUsize::new(0),
    });
    let service_json = data.join(layout::SERVICE_JSON);
    let info = json!({"port": port, "pid": std::process::id(), "token": service.token}).to_string();
    write_atomic(&service_json, info.as_bytes()).context("writing service.json")?;
    tracing::info!(port, dir = %service.dir.display(), "project service listening");

    // Discovery starts at once, so the first `/status` likely knows the parts.
    let _started = service.declarations(Some(Duration::ZERO));
    let runner = Arc::clone(&service);
    std::thread::Builder::new().name("qoob3d-jobs".into()).spawn(move || runner.run_jobs())?;
    let summariser = Arc::clone(&service);
    let summary = std::thread::Builder::new()
        .name("qoob3d-summary".into())
        .spawn(move || summariser.summarise())?;
    let watcher = Arc::clone(&service);
    let idle = Duration::from_secs_f64(idle * 60.0);
    std::thread::Builder::new()
        .name("qoob3d-idle".into())
        .spawn(move || watcher.watch_idle(idle))?;
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
    // WS-PRE-04: the last state, with the jobs cancelled, stays for the overview.
    summary.join().ok();
    service.wait_idle(STOP_WAIT);
    service.write_summary(None);
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
    /// One file `out/<kit>.<format>` with all parts, each `count` times in
    /// its print pose, arranged on the bed (AG-SV-09).
    #[serde(default)]
    kit: Option<String>,
    /// Bed width for the arrangement of a kit, mm.
    #[serde(default)]
    bed_mm: Option<f32>,
    /// One Bambu Studio project `out/<project>.3mf` with all parts on
    /// plates, each in the filament of its colour (AG-SV-10).
    #[serde(default)]
    project: Option<String>,
    /// For a project: parts that declare `print_with` print inside their
    /// host as one multi-colour object; otherwise every part is an object
    /// of its own and a plate holds one colour.
    #[serde(default)]
    multicolor: bool,
    /// For a project: the printer whose bed the plates are laid out for
    /// (an id of the printer database).
    #[serde(default)]
    printer: Option<String>,
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

    /// Stops the service from any thread: the accept loop is woken by a
    /// connection of its own and sees the flag.
    fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.changed.notify_all();
        TcpStream::connect((Ipv4Addr::LOCALHOST, self.port)).ok();
    }

    /// Stops the service after `idle` without requests and without jobs
    /// (WS-PRE-05); a job that ends starts the idle time anew.
    fn watch_idle(&self, idle: Duration) {
        let mut quiet_since = Instant::now();
        while !self.stopping.load(Ordering::SeqCst) {
            std::thread::sleep(IDLE_TICK.min(idle));
            if self.lock().jobs.values().any(|j| !j.state.finished()) {
                quiet_since = Instant::now();
                continue;
            }
            let last = *self.last_request.lock().unwrap_or_else(PoisonError::into_inner);
            if last.max(quiet_since).elapsed() >= idle {
                tracing::info!(idle_s = idle.as_secs_f64(), "project service idle, stopping");
                self.stop();
            }
        }
    }

    /// Runs `request` on a worker once a slot of the machine-wide budget is
    /// free (WS-PRE-06): every worker request but declaration discovery.
    ///
    /// # Errors
    /// As [`Pool::run`]; cancelled while it waits for a slot.
    pub(crate) fn work(
        &self,
        request: &Request,
        limits: &Limits,
        cancel: &AtomicBool,
    ) -> Result<crate::worker::Answer, PartError> {
        let held = self.slot(cancel)?;
        let answer = self.pool.run(request, limits, cancel);
        drop(held);
        answer
    }

    fn slot(&self, cancel: &AtomicBool) -> Result<Held<'_>, PartError> {
        let mut waiting = false;
        loop {
            if let Some(slot) = self.slots.try_take() {
                self.slots_held.fetch_add(1, Ordering::SeqCst);
                if waiting {
                    self.slots_waiting.fetch_sub(1, Ordering::SeqCst);
                }
                return Ok(Held { service: self, _slot: slot });
            }
            if !waiting {
                waiting = true;
                self.slots_waiting.fetch_add(1, Ordering::SeqCst);
            }
            if cancel.load(Ordering::SeqCst) || self.stopping.load(Ordering::SeqCst) {
                self.slots_waiting.fetch_sub(1, Ordering::SeqCst);
                return Err(PartError::new(ErrorKind::Cancelled, "cancelled"));
            }
            std::thread::sleep(SLOT_POLL);
        }
    }

    /// The state of `job` as clients see it, with its reason: a running
    /// job that holds no slot while one is waited for is `queued`
    /// (WS-PRE-06; jobs of one service run one after another).
    fn shown(&self, job: &Job) -> (jobs::JobState, Option<&'static str>) {
        let waits = self.slots_held.load(Ordering::SeqCst) == 0
            && self.slots_waiting.load(Ordering::SeqCst) > 0;
        if job.state == jobs::JobState::Running && waits {
            (jobs::JobState::Queued, Some(WAITING_FOR_SLOT))
        } else {
            (job.state, None)
        }
    }

    /// `{id, state, kind, reason?}` of a job in `/status`.
    fn job_entry(&self, id: u64, job: &Job) -> Value {
        let (state, reason) = self.shown(job);
        let mut entry = json!({"id": id, "state": state, "kind": job.kind()});
        if let Some(reason) = reason {
            entry["reason"] = json!(reason);
        }
        entry
    }

    fn touch(&self) {
        *self.last_request.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn next_request(&self) -> u64 {
        self.requests.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn connection(&self, mut stream: TcpStream) {
        stream.set_read_timeout(Some(CLIENT_TIMEOUT)).ok();
        let start = Instant::now();
        let reply = match http::read_request(&stream) {
            Ok(request) if self.allowed(&request) => {
                self.touch();
                let reply = self.route(&request, start);
                self.touch();
                reply
            }
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
    /// (ADR-0055) and answers from the last known declarations: parts of a
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
        let parts = snapshot.as_ref().map(|s| self.views(s, Hashing::Background)).ok();
        let st = self.lock();
        let first = st.jobs.len().saturating_sub(STATUS_JOBS);
        let jobs: Vec<Value> =
            st.jobs.iter().skip(first).map(|(id, j)| self.job_entry(*id, j)).collect();
        let mut result = json!({
            "project": Value::Null,
            "parts": [],
            "checks": [],
            "jobs": jobs,
            "workers": {
                "count": count,
                "busy": busy,
                "recycled": self.pool.recycled(),
                "slots": {
                    "budget": self.slots.budget(),
                    "held": self.slots_held.load(Ordering::SeqCst),
                    "waiting": self.slots_waiting.load(Ordering::SeqCst),
                },
            },
        });
        let mut load_errors = Vec::new();
        match snapshot {
            Ok(s) => {
                result["project"] = json!(s.config.name);
                result["parts"] = json!(parts.unwrap_or_default());
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
        // WS-PRE-01: large files hashed in the background; their parts are
        // `outdated` until the digests are known.
        let hashing: Vec<String> = (self.hashes.pending().iter())
            .map(|f| f.strip_prefix(&self.dir).unwrap_or(f).display().to_string())
            .collect();
        result["hashing"] = json!(hashing);
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
            let (state, reason) = self.shown(job);
            let mut entry = json!({"job": id, "state": state});
            if let Some(reason) = reason {
                entry["reason"] = json!(reason);
            }
            return Ok(entry);
        }
        match &job.error {
            Some(e) => Err(Failure::from(e.clone())),
            None => Ok(job.result()),
        }
    }

    fn job(&self, id: &str) -> Answer {
        let id = job_id(id)?;
        let st = self.lock();
        let job = st.jobs.get(&id).ok_or_else(|| unknown_job(id))?;
        let mut v = job.json(id);
        let (state, reason) = self.shown(job);
        v["state"] = json!(state);
        if let Some(reason) = reason {
            v["reason"] = json!(reason);
        }
        Ok(v)
    }

    /// `GET /mesh/<part>` (WA-PR-02): the qmesh of the last successful build.
    fn mesh(&self, part: &str) -> Result<Reply, Failure> {
        let snapshot = self.snapshot(Some(Duration::ZERO)).map_err(Failure::not_found)?;
        let view = self
            .views(&snapshot, Hashing::Background)
            .into_iter()
            .find(|v| v.name == part)
            .ok_or_else(|| Failure::not_found(format!("unknown part {part:?}")))?;
        let key =
            view.key.ok_or_else(|| Failure::not_found(format!("part {part:?} was never built")))?;
        let path = layout::cache_dir(&self.dir, part).join(&key).with_extension("qmsh");
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
        let out = self.dir.join(layout::OUT_DIR);
        std::fs::create_dir_all(&out)
            .map_err(|e| Failure::new(500, ErrorKind::Io, format!("{}: {e}", out.display())))?;
        let poses = self.poses();
        let io = |e: String| Failure::new(500, ErrorKind::Io, e);
        if let Some(name) = &request.project {
            return self.export_project(name, &request, &keys);
        }
        if let Some(kit) = &request.kit {
            if format == "step" || !plain_file_name(kit) {
                return Err(Failure::bad_request(
                    "kit takes a plain file name and the format stl or 3mf",
                ));
            }
            let objects = request
                .parts
                .iter()
                .zip(&keys)
                .map(|(part, key)| {
                    let qmsh = layout::cache_dir(&self.dir, part).join(key).with_extension("qmsh");
                    let pose = poses.get(part).cloned().flatten().unwrap_or_default();
                    Ok((part.clone(), pose.apply(&read_mesh(&qmsh)?), pose.count))
                })
                .collect::<Result<Vec<_>, String>>()
                .map_err(io)?;
            let file = format!("out/{kit}.{format}");
            let bed = request.bed_mm.unwrap_or(DEFAULT_BED_MM);
            let detail = export_kit(&objects, &self.dir.join(&file), format, bed).map_err(io)?;
            return Ok(json!({"files": [file], "details": [detail]}));
        }
        let mut files = Vec::new();
        let mut details = Vec::new();
        for (part, key) in request.parts.iter().zip(keys) {
            let file = format!("out/{part}.{format}");
            let base = layout::cache_dir(&self.dir, part).join(key);
            if format == "step" {
                self.export_step(part, &base.with_extension("brep"), &file)?;
                details.push(json!({"file": file, "parts": [part]}));
            } else {
                let pose = poses.get(part).cloned().flatten();
                let detail = export_mesh(
                    part,
                    &base.with_extension("qmsh"),
                    &self.dir.join(&file),
                    format,
                    pose.as_ref(),
                )
                .map_err(io)?;
                details.push(
                    json!({"file": file})
                        .as_object()
                        .cloned()
                        .map(|mut m| {
                            m.extend(detail.as_object().cloned().unwrap_or_default());
                            Value::Object(m)
                        })
                        .unwrap_or(detail),
                );
            }
            files.push(file);
        }
        Ok(json!({"files": files, "details": details}))
    }

    /// The print pose of every part (AG-PJ-06), from the last declarations.
    fn poses(&self) -> std::collections::HashMap<String, Option<qoob3d_core::project::PrintPose>> {
        self.snapshot(None)
            .map(|s| s.parts.into_iter().map(|p| (p.name, p.print)).collect())
            .unwrap_or_default()
    }

    /// The built parts as a slicer project `out/<name>.3mf` (AG-SV-10), with
    /// pose, colour and host from the last declarations.
    fn export_project(&self, name: &str, request: &ExportBody, keys: &[String]) -> Answer {
        let alone = request.kit.is_none() && request.bed_mm.is_none();
        if request.format != "3mf" || !plain_file_name(name) || !alone {
            return Err(Failure::bad_request(
                "project takes a plain file name and the format 3mf, without kit or bed_mm \
                 (the bed is the printer's)",
            ));
        }
        let io = |e: String| Failure::new(500, ErrorKind::Io, e);
        let printer = slicer::printer(request.printer.as_deref()).map_err(Failure::bad_request)?;
        let declared = self.snapshot(None).map(|s| s.parts).unwrap_or_default();
        let mut parts = Vec::new();
        for (part, key) in request.parts.iter().zip(keys) {
            let qmsh = layout::cache_dir(&self.dir, part).join(key).with_extension("qmsh");
            let source = declared.iter().find(|p| p.name == *part);
            parts.push(slicer::Part {
                name: part.clone(),
                mesh: read_mesh(&qmsh).map_err(io)?,
                pose: source.and_then(|p| p.print.clone()),
                color: source
                    .and_then(|p| p.color.as_deref())
                    .and_then(qoob3d_core::project::parse_color),
                print_with: source.and_then(|p| p.print_with.clone()),
            });
        }
        let file = format!("out/{name}.3mf");
        let detail = slicer::export(&parts, request.multicolor, &printer, &self.dir.join(&file))
            .map_err(|e| match e {
                slicer::Refused::Request(m) => Failure::bad_request(m),
                slicer::Refused::Io(m) => io(m),
            })?;
        Ok(json!({"files": [file], "details": [detail]}))
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
        let answer = self.work(&request, &limits, &AtomicBool::new(false))?;
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

/// Bed width for arranging a kit when the request names none, mm (the
/// 256 mm bed of the Bambu Lab printers, ADR-0002).
const DEFAULT_BED_MM: f32 = 256.0;

/// A plain file name for a kit: no path, no dot at the start.
fn plain_file_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\']) && name.len() <= 100
}

fn read_mesh(qmsh: &Path) -> Result<qoob3d_core::TriMesh, String> {
    let bytes = std::fs::read(qmsh).map_err(|e| format!("{}: {e}", qmsh.display()))?;
    let mesh = qmesh::read(&bytes).map_err(|e| format!("{}: {e:?}", qmsh.display()))?;
    Ok(mesh.faces)
}

/// What a slicer will find in an exported mesh: size, triangles, whether
/// it is closed, and the warnings (findings 1.5, 3.7).
fn mesh_detail(mesh: &qoob3d_core::TriMesh) -> Value {
    let report = mesh.export_report();
    let size = mesh.bounds().map(|b| std::array::from_fn::<f32, 3, _>(|i| b.max[i] - b.min[i]));
    let mut warnings = Vec::new();
    if !report.is_watertight() {
        warnings.push(format!(
            "not watertight: {} open, {} non-manifold, {} flipped edges; a slicer may reject it",
            report.open_edges, report.non_manifold_edges, report.inconsistent_edges
        ));
    }
    json!({
        "triangles": report.triangles,
        "size": size,
        "watertight": report.is_watertight(),
        "warnings": warnings,
    })
}

/// STL or 3MF (one object) of the cached mesh `qmsh` to `out`, in the
/// print pose if the part has one; what the file holds.
fn export_mesh(
    part: &str,
    qmsh: &Path,
    out: &Path,
    format: &str,
    pose: Option<&qoob3d_core::project::PrintPose>,
) -> Result<Value, String> {
    let mut mesh = read_mesh(qmsh)?;
    if let Some(pose) = pose {
        mesh = pose.apply(&mesh);
    }
    let objects = [ExportObject { name: part.to_owned(), mesh: &mesh }];
    let data = if format == "3mf" { export::three_mf(&objects) } else { export::stl(&objects) };
    write_atomic(out, &data).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut detail = mesh_detail(&mesh);
    detail["parts"] = json!([part]);
    if let Some(pose) = pose.filter(|p| p.count > 1) {
        detail["count"] = json!(pose.count);
    }
    Ok(detail)
}

/// One file of all `objects` (name, posed mesh, count), each copy its own
/// object, arranged on a bed `bed` mm wide (AG-SV-09).
fn export_kit(
    objects: &[(String, qoob3d_core::TriMesh, u32)],
    out: &Path,
    format: &str,
    bed: f32,
) -> Result<Value, String> {
    let copies: Vec<(String, &qoob3d_core::TriMesh)> = objects
        .iter()
        .flat_map(|(name, mesh, count)| {
            (1..=*count).map(move |i| {
                let label = if *count > 1 { format!("{name} {i}") } else { name.clone() };
                (label, mesh)
            })
        })
        .collect();
    let sizes: Vec<[f32; 2]> = copies
        .iter()
        .map(|(_, m)| m.bounds().map_or([0.0; 2], |b| [b.max[0] - b.min[0], b.max[1] - b.min[1]]))
        .collect();
    let centres = qoob3d_core::project::arrange(&sizes, bed);
    let placed: Vec<(String, qoob3d_core::TriMesh)> = copies
        .iter()
        .zip(&centres)
        .map(|((name, mesh), c)| {
            let mut m = (*mesh).clone();
            for p in &mut m.positions {
                p[0] += c[0];
                p[1] += c[1];
            }
            (name.clone(), m)
        })
        .collect();
    let exported: Vec<ExportObject<'_>> =
        placed.iter().map(|(name, mesh)| ExportObject { name: name.clone(), mesh }).collect();
    let data = if format == "3mf" { export::three_mf(&exported) } else { export::stl(&exported) };
    write_atomic(out, &data).map_err(|e| format!("{}: {e}", out.display()))?;
    let details: Vec<Value> = placed
        .iter()
        .map(|(name, mesh)| {
            let mut d = mesh_detail(mesh);
            d["object"] = json!(name);
            d
        })
        .collect();
    let footprint = centres.iter().zip(&sizes).fold([0.0f32; 2], |acc, (c, s)| {
        [acc[0].max(c[0] + s[0] / 2.0), acc[1].max(c[1] + s[1] / 2.0)]
    });
    Ok(json!({
        "file": out.file_name().map(|f| f.to_string_lossy().into_owned()),
        "parts": objects.iter().map(|(n, _, _)| n).collect::<Vec<_>>(),
        "objects": details,
        "footprint": footprint,
        "fits_one_bed": footprint[0] <= bed && footprint[1] <= bed,
    }))
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
