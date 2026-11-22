---
id: ADR-0055
title: Project service internals – single instance by file lock, framed worker protocol, declarations without the CAD API, token on localhost
status: accepted
date: 2026-11-22
crates: [qoob3d-core, qoob3d-kernel, qoob3d-app]
tags: [service, workers, cache, security, dependencies]
supersedes: []
superseded_by: null
---

## Context

ADR-0053 and ADR-0054 fix the architecture (Rhai scripts, worker processes, service, MCP).
While building R2–R4 and P1 several decisions were made in code that those ADRs do not record;
the critic review of 2026-11-22 asked for them (K-10, K-12) and found gaps that need a decision
(no authentication on localhost, memory of declaration discovery).

## Decision

- **One service per project folder** through an OS file lock on `.qoob3d/service.lock`
  (`File::try_lock`). `service.json` (port, pid, token) is written atomically after binding and
  removed on a clean exit; a stale file is harmless because the lock decides.
- **Worker protocol:** JSON lines over stdin/stdout. Every answer line starts with `{"job":`;
  the service skips other stdout lines, because OCCT (e.g. the STEP translator) prints to
  stdout. A worker exits when its stdin closes, so no worker outlives its service. Logs go to
  stderr in every mode of the binary.
- **Declarations without the CAD API:** the service finds parts and checks by evaluating the
  file with a stub `part()`/`check()` and without the CAD functions registered
  (`PartMode::Declare`), so the service process can never reach OCCT. Geometry at the top level
  of a part file is therefore an error; it belongs inside the part closure. Declaration
  discovery runs with tight script size limits (AG-HD-02); worker runs are protected by the
  resident-memory limit instead.
- **Jobs run one after another**; inside a job, parts build in parallel across the worker pool
  as their dependencies finish. A job with a failed part ends in state `failed`.
- **Cache key composition:** SHA-256 over length-prefixed inputs (ADR-0053). Tessellation
  settings enter through the program-version string (`<version> mesh <deflection> <angle>`),
  the modules through the list the engine actually resolved (AG-HD-06).
- **Exports** (STL, 3MF) are written from the cached mesh, so their quality follows
  `[mesh]`; STEP is written from the BREP in a worker.
- **`qoob3d bench`** uses a throwaway cache folder and a fresh worker per build, so the memory
  figure belongs to one part and the project cache is untouched.
- **Authentication on localhost:** the service generates a random token per start, stores it
  in `service.json` (readable only through the file system) and requires it as header
  `X-Qoob3d-Token`; requests with an `Origin` header or a foreign `Host` are refused. This
  closes requests from web pages (CSRF, DNS rebinding). Local processes of the same user are
  trusted; the role `human` over HTTP is therefore a convention between local tools, while the
  MCP server never offers it.
- **New dependencies in `qoob3d-core`:** `toml` (project and value files, task headers) and
  `sha2` (cache key). Both are pure Rust without system libraries; a hand-written TOML parser
  or hash would be more code to maintain than the dependency costs. `rhai` in `qoob3d-kernel`
  is justified in ADR-0053; its fixed dependency `smartstring` is MPL-2.0 (file-level
  copyleft, unmodified use) and needs a licence exception.
- **The M0 Python probe** was removed from the working tree (ADR-0045); numbers and formulas
  live in `docs/concept/m0-report.md` and the kernel tests.

## Consequences

- Clients (app, CLI, MCP) must read `service.json` for port and token and re-read it when the
  service does not answer (AG-HD-05).
- A project in a shared or world-readable folder exposes the token to whoever can read the
  folder; that is the same trust as the model files themselves.
- A coarse `[mesh]` setting gives coarse STL/3MF exports; print exports should use the default.
- Sequential jobs keep the model simple; a long build delays a check request. Revisit when
  agents work in parallel on one project.

## Rejected alternatives

- **Unix domain socket instead of TCP** – no browser exposure at all, but the app and other
  tools would need platform-specific code; kept as an option.
- **Blanket script size limits** – would turn legitimate large worker runs into script errors
  and still allow gigabytes within the operation budget in the service.
- **Pid file instead of a lock** – stale after a crash and racy.

## References

- Code: `crates/qoob3d-app/src/service.rs`, `service/{pool,jobs,snapshot,http}.rs`,
  `worker.rs`, `crates/qoob3d-kernel/src/script/project.rs`
- Requirements: `docs/requirements/agent.md` sections 2–6
- Related: ADR-0045, ADR-0053, ADR-0054
