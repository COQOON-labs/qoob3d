//! Pool of warm `qoob3d worker` processes (AG-PJ-05, agent.md §2.1,
//! ADR-0053). A job takes an idle worker, sends one request line and waits
//! for the answer while it watches the limits every [`POLL`]: wall-clock
//! time, resident memory and cancellation. A worker that exceeds a limit is
//! killed, one that dies on its own is a crash; both are replaced, so the
//! pool keeps its size. A worker that is too large after its job is replaced
//! as well (recycling, AG-PF-03).

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use qoob3d_core::project::Limits;

use crate::worker::{ANSWER_MARK, Answer, ErrorKind, PartError, Request};

/// Interval of the limit checks (§2.1: memory polled every 0.2 s).
const POLL: Duration = Duration::from_millis(200);

struct Worker {
    child: Child,
    stdin: ChildStdin,
    answers: Receiver<Answer>,
}

impl Worker {
    fn spawn(exe: &PathBuf) -> std::io::Result<Self> {
        let mut child = Command::new(exe)
            .arg("worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no worker stdin"))?;
        let stdout =
            child.stdout.take().ok_or_else(|| std::io::Error::other("no worker stdout"))?;
        let (tx, answers) = mpsc::channel();
        let pid = child.id();
        std::thread::Builder::new().name(format!("qoob3d-worker-{pid}")).spawn(move || {
            let mut out = BufReader::new(stdout);
            let mut line = Vec::new();
            // Bytes, not `lines()`: OCCT may print something that is not UTF-8.
            while out.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
                let text = String::from_utf8_lossy(&line);
                let answer =
                    text.find(ANSWER_MARK).and_then(|i| serde_json::from_str(&text[i..]).ok());
                if let Some(answer) = answer {
                    if tx.send(answer).is_err() {
                        break;
                    }
                } else {
                    tracing::debug!(pid, "worker output: {}", text.trim_end());
                }
                line.clear();
            }
        })?;
        Ok(Self { child, stdin, answers })
    }

    /// Kills (if still running) and reaps the process.
    fn end(mut self) -> Option<ExitStatus> {
        self.child.kill().ok();
        self.child.wait().ok()
    }
}

pub(crate) struct Pool {
    exe: PathBuf,
    idle: Mutex<Vec<Worker>>,
    available: Condvar,
    /// Live workers, idle or busy.
    total: AtomicUsize,
    busy: AtomicUsize,
    closed: AtomicBool,
    /// Resident memory (MB) above which a worker is replaced after its job.
    recycle_mb: u64,
    /// Workers recycled since the start.
    recycled: AtomicUsize,
    /// Largest resident memory (MB) seen at a busy worker.
    peak_mb: AtomicU64,
}

impl Pool {
    /// Starts `count` workers of the executable `exe`.
    pub(crate) fn start(exe: PathBuf, count: usize, recycle_mb: u64) -> Self {
        let mut idle = Vec::new();
        for _ in 0..count {
            match Worker::spawn(&exe) {
                Ok(w) => idle.push(w),
                Err(e) => tracing::error!("starting a worker: {e}"),
            }
        }
        Self {
            exe,
            total: AtomicUsize::new(idle.len()),
            idle: Mutex::new(idle),
            available: Condvar::new(),
            busy: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            recycle_mb,
            recycled: AtomicUsize::new(0),
            peak_mb: AtomicU64::new(0),
        }
    }

    /// Workers recycled since the start, for `/status` (AG-PF-03).
    pub(crate) fn recycled(&self) -> usize {
        self.recycled.load(Ordering::SeqCst)
    }

    /// Largest resident memory (MB) polled at a busy worker (AG-PF-06).
    pub(crate) fn peak_mb(&self) -> u64 {
        self.peak_mb.load(Ordering::SeqCst)
    }

    /// `(count, busy)` for `/status`.
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.total.load(Ordering::SeqCst), self.busy.load(Ordering::SeqCst))
    }

    /// Runs `request` on a worker within `limits`.
    ///
    /// # Errors
    /// Limit, crash, cancellation or no worker; the worker's own failures
    /// come back as an [`Answer`] with `ok: false`.
    pub(crate) fn run(
        &self,
        request: &Request,
        limits: &Limits,
        cancel: &AtomicBool,
    ) -> Result<Answer, PartError> {
        let mut worker = self.take(cancel)?;
        let line = serde_json::to_string(request).expect("a request serialises");
        if writeln!(worker.stdin, "{line}").and_then(|()| worker.stdin.flush()).is_err() {
            let status = self.retire(worker);
            return Err(crash(status));
        }
        let start = Instant::now();
        let time_limit = Duration::from_secs_f64(limits.time_s);
        loop {
            match worker.answers.recv_timeout(POLL) {
                Ok(answer) if answer.job == request.job => {
                    let rss = resident_mb(worker.child.id()).unwrap_or(0);
                    self.peak_mb.fetch_max(rss, Ordering::SeqCst);
                    if rss > self.recycle_mb {
                        tracing::info!(rss, limit = self.recycle_mb, "recycling a worker");
                        self.recycled.fetch_add(1, Ordering::SeqCst);
                        self.retire(worker);
                    } else {
                        self.give(worker);
                    }
                    return Ok(answer);
                }
                Ok(_) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    let status = self.retire(worker);
                    return Err(crash(status));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            let rss = resident_mb(worker.child.id());
            self.peak_mb.fetch_max(rss.unwrap_or(0), Ordering::SeqCst);
            let error = if cancel.load(Ordering::SeqCst) {
                PartError::new(ErrorKind::Cancelled, "cancelled")
            } else if start.elapsed() >= time_limit {
                let message = format!("time limit of {} s exceeded, worker killed", limits.time_s);
                PartError::new(ErrorKind::TimeLimit, message)
            } else if let Some(rss) = rss
                && rss > limits.memory_mb
            {
                let message = format!(
                    "memory limit of {} MB exceeded ({rss} MB resident), worker killed",
                    limits.memory_mb
                );
                PartError::new(ErrorKind::MemoryLimit, message)
            } else {
                continue;
            };
            self.retire(worker);
            return Err(error);
        }
    }

    /// An idle, live worker; waits while all are busy.
    fn take(&self, cancel: &AtomicBool) -> Result<Worker, PartError> {
        let mut idle = self.idle.lock().expect("pool lock poisoned");
        loop {
            if cancel.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) {
                return Err(PartError::new(ErrorKind::Cancelled, "cancelled"));
            }
            if let Some(mut w) = idle.pop() {
                self.busy.fetch_add(1, Ordering::SeqCst);
                // Killed from outside while idle: replace it before use.
                if !matches!(w.child.try_wait(), Ok(None)) {
                    drop(idle);
                    self.retire(w);
                    idle = self.idle.lock().expect("pool lock poisoned");
                    continue;
                }
                return Ok(w);
            }
            if self.total.load(Ordering::SeqCst) == 0 {
                return Err(PartError::new(ErrorKind::Crash, "no worker process could be started"));
            }
            idle = self.available.wait_timeout(idle, POLL).expect("pool lock poisoned").0;
        }
    }

    fn give(&self, worker: Worker) {
        self.idle.lock().expect("pool lock poisoned").push(worker);
        self.busy.fetch_sub(1, Ordering::SeqCst);
        self.available.notify_one();
    }

    /// Ends a busy worker and puts a fresh one in its place (unless closed).
    fn retire(&self, worker: Worker) -> Option<ExitStatus> {
        let status = worker.end();
        let replacement = if self.closed.load(Ordering::SeqCst) {
            None
        } else {
            Worker::spawn(&self.exe).map_err(|e| tracing::error!("replacing a worker: {e}")).ok()
        };
        if let Some(w) = replacement {
            self.give(w);
        } else {
            self.total.fetch_sub(1, Ordering::SeqCst);
            self.busy.fetch_sub(1, Ordering::SeqCst);
            self.available.notify_all();
        }
        status
    }

    /// Stops the idle workers; busy ones end when their job sees the
    /// cancellation. Waits up to `wait` for them.
    pub(crate) fn close(&self, wait: Duration) {
        self.closed.store(true, Ordering::SeqCst);
        let idle = std::mem::take(&mut *self.idle.lock().expect("pool lock poisoned"));
        self.total.fetch_sub(idle.len(), Ordering::SeqCst);
        for w in idle {
            w.end();
        }
        self.available.notify_all();
        let deadline = Instant::now() + wait;
        while self.busy.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn crash(status: Option<ExitStatus>) -> PartError {
    let how = status.map_or_else(|| "unknown status".to_owned(), |s| s.to_string());
    PartError::new(ErrorKind::Crash, format!("the worker process died ({how})"))
}

/// Resident set size of `pid` in MB (Linux: `/proc/<pid>/status`).
#[cfg(target_os = "linux")]
fn resident_mb(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// Resident set size of `pid` in MB (`ps -o rss=`, kB).
#[cfg(not(target_os = "linux"))]
fn resident_mb(pid: u32) -> Option<u64> {
    let out = Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output().ok()?;
    let kb: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(kb / 1024)
}
