---
id: ADR-0055
title: The project service runs once per folder under a file lock, talks to its workers in framed JSON lines, never evaluates user scripts itself and accepts only local requests with a per-start token
status: accepted
date: 2026-11-23
crates: [qoob3d-core, qoob3d-kernel, qoob3d-project]
tags: [service, workers, cache, security, performance, dependencies]
supersedes: []
superseded_by: null
---

## Context

ADR-0053 and ADR-0054 fix the architecture: parts are Rhai scripts, a service per project
builds them in worker processes, and the app, the CLI and the MCP server are its clients.
Building it (R2–R4, P1) required decisions those ADRs do not record: how only one service runs
per folder, how the service reads worker answers when OCCT writes to stdout as well, who
evaluates the top level of a part file to find its declarations, and how a service on
localhost refuses requests from web pages.

The service process has no memory limit of its own, and its promise is that no script can block
or kill it. Rhai's size limits miss growth by assignment into containers (`m[k] = v`,
`a += a` on arrays), so a top level has to be measured from outside while it runs.

This topic absorbs the former decision 0056.

## Decision

### One service per project folder

The service holds an OS file lock on `.qoob3d/service.lock` (`File::try_lock`). After binding
it writes `.qoob3d/service.json` (port, pid, token) atomically and removes it on a clean exit;
a stale file is harmless because the lock decides. The names live in
`qoob3d_core::project::layout`.

### Framed worker protocol

Service and `qoob3d worker` exchange JSON lines over stdin/stdout, one request per line, one
answer line per job; a worker stays warm between jobs. Answers are framed by `{"job":`: the
service parses from that mark and skips every line without it, because OCCT (e.g. the STEP
translator) prints to stdout too. A worker exits when its stdin closes, so no worker outlives
its service. Logs go to stderr in every mode of the binary. The service owns the time and
memory limits and replaces a worker that crashes or is killed.

### The service never evaluates user scripts

Declaration discovery (parts, their options and dependencies, check names) is a worker request
(`Probe::Declare`). The worker evaluates the file in `PartMode::Declare`: stub
`part()`/`check()` and no CAD functions registered, so geometry at the top level of a part file
is an error and belongs inside the part closure. It runs under the project's time limit and the
worker's resident-memory limit and answers the declarations or the load error. The service
caches the answer by the bytes of the file and its literally imported modules.

- **Size limits in declare mode** (AG-HD-02): strings 65536 bytes, arrays 10000 elements, maps
  1000 entries, at most 1000000 operations; exceeding one is `ScriptError::Limit`, a load error
  with its line. Build runs have no script size limits; the worker's memory limit protects them.
- **Measuring in doubling stages:** the top level runs with an operation budget of 4096, 8192,
  … from the start each time, and the variables are measured after each stage, so the total
  cost stays linear in the operations of the file. What grows between two measurements is
  stopped by the worker's memory limit and reported as that file's load error.
- A worker crash or limit during discovery fails only that file; the service stays up.
- `/status` never waits for discovery (100 ms budget): parts of a file under rediscovery are
  reported from the previous declarations, marked `outdated`. Builds and checks wait until
  every file is known; `POST /discover` waits for chosen files.

### Jobs, cache key, exports, bench

- Jobs run one after another; inside a job, parts build in parallel across the worker pool as
  their dependencies finish. A job with a failed part ends in state `failed`.
- The cache key is SHA-256 over length-prefixed inputs (ADR-0053). The program part is
  `<version> kernel <fingerprint> entry <entry version> mesh <deflection> <angle>`, so a new
  program, kernel, cache entry format or tessellation setting rebuilds; modules enter as the
  list the engine actually resolved (AG-HD-06).
- **Files enter the key as their SHA-256** (WS-PRE-01), not as their bytes: the part file,
  every module, every imported STEP/STL/3MF/font file. Worker and service compute the same
  digests; the service remembers them by path, size and modification time
  (`service/hashes.rs`) and so reads a file only after it changed. A digest is remembered
  only if the file was older than 1 s when its reading began: a second write within the
  resolution of a timestamp keeps size and time (Git's "racy" case). The literal import scan
  is remembered the same way, by the stamps of the file and of its modules.
- **The kernel fingerprint** (WS-PRE-02) is SHA-256 at build time, computed by
  `qoob3d-project/build.rs` (`rerun-if-changed` on every input), over the files of
  `qoob3d-kernel` (`Cargo.toml`, `build.rs`, `cpp/`, `src/`) and of the OCCT fork
  `third_party/opencascade-sys` (its `Cargo.toml` pins the OCCT version), over the files of
  `qoob3d-core` whose code the kernel runs while it builds (lettering, mesh import and its
  zip reader, mesh welding; not tasks or project folder code, which change often and would
  rebuild every part each time), and over `name version source` of every package the kernel
  depends on in `Cargo.lock`, so an OCCT, Rhai, cxx or font reader update rebuilds. Of a
  path package the walk follows only what its `Cargo.toml` lists as normal or build
  dependency: the lock file lists dev dependencies too. It also takes the `cxxbridge-cmd`
  tree that cxx pins for no platform; an update there rebuilds once. What the kernel takes
  from `qoob3d_core::project` (pose, colour, the `[mesh]` settings) only declares parts,
  which the service keeps in memory, or enters the key as values. The kernel's own build
  script would compile the C++ wrappers again after every change of a Rust file. A rebuild
  of the app alone keeps the cache; `worker/tests.rs` recomputes the fingerprint, derives
  the list of core files from what the kernel names (and those files name through
  `crate::`, `super::` and child modules), and checks the packages against `cargo metadata`.
- **`/status` never holds the service lock while it reads or hashes** (WS-PRE-01):
  `Service::views` copies the records out, computes the keys and states without the lock and
  writes back only cache entries it found. `/status` and `/mesh` never wait for a large file
  (above 1 MiB): it is hashed by one background thread, its parts are `outdated` meanwhile
  and the file is listed in `hashing`. Builds, checks and the review gate hash what they need.
  Rhai sources are hashed by the request (they are read for the import scan anyway).
- **Keys are remembered with their inputs** (WS-PRE-01): `Hashes::key` keeps the last key of
  every part next to its `KeyInputs` and hashes again only when an input changed, and
  `Service::views` asks for every file's digest once per call, since parts share modules and
  imported files. What is left per poll is a `stat` per distinct file.
- **Workers run at nice 10 below their service** (WS-PRE-01): the worker lowers its own
  priority first thing (`rustix::process::setpriority_process`, before any thread starts; on
  Linux the value is per thread and inherited). A build keeps every core busy with OCCT
  threads; the service then still gets the CPU for `/status`. Measured on the Damen 5009
  project (113 parts, debug build): no request waited for a lock in any run; the slow answers
  of a cold build were a service thread without CPU time, slowed evenly in every phase. In the
  acceptance run (cold build, checks, kit export) `/status` went from median 7.9 ms, p99
  34 ms, maximum 146 ms to 5.7 ms, 17 ms and 52 ms with both measures (one run each).
- STL and 3MF exports are written from the cached mesh, so their quality follows `[mesh]`;
  STEP is written from the BREP in a worker.
- `qoob3d bench` uses a throwaway cache folder and a fresh worker per build, so the memory
  figure belongs to one part and the project cache is untouched.

### Busy or dead, summary file, idle shutdown

- **A client tells a busy service from a dead one** (WS-PRE-03): when `/status` does not
  answer within 1 s, the client starts a service only if the process in `service.json` is
  gone (`kill -0`) or does not hold `service.lock`; otherwise it waits for it and, after its
  timeout, fails with `client::Busy` (envelope kind `busy`). The lock is tried only when that
  process is alive, so a service that is starting never finds its lock taken by the check.
- **`.qoob3d/summary.json`** (WS-PRE-04): a thread takes `/status` every 0.5 s and writes the
  counts (`{project, updated, parts: {…}, checks: {…}, jobs_running, load_errors}`, the last
  one a count) atomically when one changed, so file changes nobody asked about arrive too,
  and at most twice per second. The file stays after the service stopped, with the state
  after the jobs were cancelled; an overview reads it without starting anything.
- **Idle shutdown** (WS-PRE-05): the service stops after `[service] idle_min` (default 30,
  fractions allowed, read at start) without allowed requests and without unfinished jobs; a
  job that ends starts the idle time anew. Refused requests (403) do not keep it alive.

### Machine-wide worker budget

At most `max_busy_workers` workers of all services of the user run at once (WS-PRE-06). A
slot is an OS file lock on `slot-<i>.lock`, `i` below the budget, in the per-user runtime
folder (`QOOB3D_RUNTIME_DIR`, else `$XDG_RUNTIME_DIR/qoob3d`, else `qoob3d-<user>` in the
temporary folder, which is per user on macOS; made with mode 0700). The budget is
`budget.json` (`{"max_busy_workers": n}`) in that folder, read at every attempt; without it,
one less than the physical cores, at least 1. Every worker request takes a slot before it
takes a worker (`Service::work`), except declaration discovery, which is bounded and which
builds wait for. A job whose requests all wait is shown `queued` with `reason: "waiting for a
worker slot"`; `/status` reports `workers.slots {budget, held, waiting}`. A dead service frees
its slots because the OS releases its locks.

### Token on localhost

The service listens on 127.0.0.1, generates a random token (32 hex characters) per start,
stores it in `service.json` and requires it as header `X-Qoob3d-Token`. Requests with an
`Origin` header, or whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>`, are refused
with 403 before routing (AG-HD-03). This closes requests from web pages (CSRF, DNS
rebinding). Local processes of the same user are trusted: the role `human` over HTTP is a
convention between local tools, and the MCP server never offers it.

### Dependencies

`qoob3d-core` uses `toml` (project and value files, task headers) and `sha2` (cache key), both
pure Rust; a hand-written parser or hash would cost more to maintain. `qoob3d-project` uses
`rustix` (feature `process`, already in the dependency tree) for the worker priority, since
`unsafe_code` is denied and `std` has no `setpriority`. `rhai` in
`qoob3d-kernel` is justified in ADR-0053; its fixed dependency `smartstring` is MPL-2.0
(file-level copyleft, used unmodified) and has a licence exception in `deny.toml`.

## Consequences

- No script, including a top level, can block or kill the service.
- Clients (app, CLI, MCP) read port and token from `service.json` and read it again whenever
  the service does not answer (AG-HD-05), so they follow a restarted service.
- Discovery costs one worker round trip per changed file (milliseconds with warm workers);
  unchanged files come from the declaration cache.
- A project in a shared or world-readable folder exposes the token to whoever can read the
  folder; that is the same trust as the model files themselves.
- A coarse `[mesh]` setting gives coarse STL/3MF exports; print exports should use the default.
- Sequential jobs keep the model simple, but a long build delays a check request. Revisit when
  agents work in parallel on one project.
- Changing the key to file digests and adding the kernel fingerprint rebuilds every part once.
- `/status` costs a `stat` per distinct file instead of reading them (113 parts of the Damen
  project: 4.5 ms median in the debug build, 6.5 ms before keys were remembered); right after a large file changed its parts read `outdated` until the
  background thread has hashed it.
- A file rewritten with the same size and modification time (and older than 1 s) is not seen;
  that is the price of not reading every file per poll.
- The summary thread computes `/status` twice per second as long as the service runs; the idle
  shutdown bounds that.
- Builds lose a little against other work of the user at the same time (nice 10), never
  against an idle machine.
- Tests give every project its own runtime folder (`QOOB3D_RUNTIME_DIR`), so they never take
  slots of the user's services.

## Rejected alternatives

- **Discovery in the service process** with staged measurement: with stages of a fixed size
  the cost was quadratic, and a top level of one million operations missed the 20 s start-up
  budget; larger stages would let more memory grow unseen in a process without a memory limit.
- **A memory limit for the service process itself:** no portable way without new
  dependencies, and the service would die instead of reporting.
- **`Engine::on_var`:** deprecated in Rhai and over the lint-suppression cap (ADR-0037).
- **Blanket script size limits for builds:** would turn legitimate large builds into script
  errors.
- **Unix domain socket instead of TCP:** no browser exposure at all, but the app and other
  tools would need platform-specific code; kept as an option.
- **Pid file instead of a lock:** stale after a crash and racy.
- **Hashing the bytes of every file per key** (as before WS-PRE-01): with 66 MB of imports
  and 108 parts `/status` hashed gigabytes inside the service lock.
- **Starting workers through `/usr/bin/nice`:** depends on an outside program and its path;
  the worker setting its own priority covers the pool, discovery and `bench` alike.
- **A file-change watcher** (FSEvents, inotify) instead of stamps: a new dependency and
  platform code for what a `stat` per file already gives.
- **All of `Cargo.lock` in the fingerprint:** every update of a UI crate would rebuild every
  part of every project (the Damen 5009 project: about five minutes).
- **All of `qoob3d-core/src` in the fingerprint:** simpler, but every change of the task
  inbox or the project folder code would rebuild every part.
- **`cargo metadata` in the build script** for the exact dependency kinds: a cargo run inside
  a build, for the sake of a few tool crates.
- **The kernel fingerprint from the binary's own hash:** every rebuild of the app would
  rebuild every part.
- **One worker pool for all projects:** would give up the per-folder isolation; slot locks
  give the same bound without a shared process.
- **Reading the app's settings in the service:** eframe keeps them in its own storage, and
  `qoob3d-project` must not know the app (ADR-0036); the app writes `budget.json` instead.

## References

- Code: `crates/qoob3d-project/src/service.rs`, `crates/qoob3d-project/src/service/`
  (`pool.rs`, `jobs.rs`, `discovery.rs`, `snapshot.rs`, `hashes.rs`, `summary.rs`,
  `slots.rs`, `http.rs`, `bench.rs`), `crates/qoob3d-project/build.rs`,
  `crates/qoob3d-project/src/worker.rs`, `crates/qoob3d-project/src/worker/probe.rs`,
  `crates/qoob3d-project/src/client.rs`, `crates/qoob3d-kernel/src/script/project.rs`,
  `crates/qoob3d-kernel/src/script/limits.rs`, `crates/qoob3d-core/src/project/layout.rs`
- Requirements: `docs/requirements/agent.md` (AG-HD-01 … AG-HD-09, AG-PJ-05, sections 2–6),
  `docs/requirements/workspace.md` (WS-PRE-01 … WS-PRE-06)
- Related: ADR-0053, ADR-0054, ADR-0057
