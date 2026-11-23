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
  `<version> entry <entry version> mesh <deflection> <angle>`, so a new program, cache entry
  format or tessellation setting rebuilds; modules enter as the list the engine actually
  resolved (AG-HD-06).
- STL and 3MF exports are written from the cached mesh, so their quality follows `[mesh]`;
  STEP is written from the BREP in a worker.
- `qoob3d bench` uses a throwaway cache folder and a fresh worker per build, so the memory
  figure belongs to one part and the project cache is untouched.

### Token on localhost

The service listens on 127.0.0.1, generates a random token (32 hex characters) per start,
stores it in `service.json` and requires it as header `X-Qoob3d-Token`. Requests with an
`Origin` header, or whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>`, are refused
with 403 before routing (AG-HD-03). This closes requests from web pages (CSRF, DNS
rebinding). Local processes of the same user are trusted: the role `human` over HTTP is a
convention between local tools, and the MCP server never offers it.

### Dependencies

`qoob3d-core` uses `toml` (project and value files, task headers) and `sha2` (cache key), both
pure Rust; a hand-written parser or hash would cost more to maintain. `rhai` in
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

## References

- Code: `crates/qoob3d-project/src/service.rs`, `crates/qoob3d-project/src/service/`
  (`pool.rs`, `jobs.rs`, `discovery.rs`, `snapshot.rs`, `http.rs`, `bench.rs`),
  `crates/qoob3d-project/src/worker.rs`, `crates/qoob3d-project/src/worker/probe.rs`,
  `crates/qoob3d-project/src/client.rs`, `crates/qoob3d-kernel/src/script/project.rs`,
  `crates/qoob3d-kernel/src/script/limits.rs`, `crates/qoob3d-core/src/project/layout.rs`
- Requirements: `docs/requirements/agent.md` (AG-HD-01 … AG-HD-09, AG-PJ-05, sections 2–6)
- Related: ADR-0053, ADR-0054
