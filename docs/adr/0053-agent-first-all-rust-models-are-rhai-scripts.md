---
id: ADR-0053
title: Agent first, all Rust – models are Rhai scripts on our OCCT kernel, heavy work in worker processes
status: accepted
date: 2026-11-22
crates: [qoob3d-core, qoob3d-kernel, qoob3d-app]
tags: [architecture, ai-first, scripting, performance, process-isolation]
supersedes: [ADR-0052]
superseded_by: null
---

## Context

ADR-0052 kept the model core of the Werft concept (`docs/concept/werft-concept.md`) in Python
(build123d) and made QOOB3D its app. The user then decided: everything in Rust, with an
embedded script language for models, because performance and memory matter most ("many
polygons, many 3D computations, memory must not blow up").

Facts from M0 (`docs/concept/m0-report.md`): in build123d all geometry runs in OCCT (C++);
Python is only glue. Importing build123d costs 1.6–1.7 s per process. OCCT can fail
silently (a "valid" shell with volume 0) and hang or eat memory on hard operations, so the
only real protection is a separate process with time and memory limits. QOOB3D already
talks to OCCT 7.8.1 through its own cxx wrappers (ADR-0014).

Constraints: the workspace members and the architecture rules are harness files (ADR-0037),
so new crates need a human unlock; until then everything lives in the existing crates.

## Decision

- **A model is a project folder in Git** (concept section 5, English names):
  `qoob3d.toml` (units mm, printer, limits), `parts/*.rhai` (parts as scripts),
  `values/*.toml` (target values with mandatory `source` and `status`),
  `checks/*.rhai` (checks), `out/` (exports), `.qoob3d/` (cache and service data, not in Git).
- **Script language: Rhai** (pure Rust, sandboxed, no file or network access, operation
  and depth limits). Our CAD API is registered into the engine; every call goes straight to
  OCCT through `qoob3d-kernel`. The script is glue only, geometry never runs in Rhai.
- **Crate placement (until new crates are unlocked):** script engine and CAD API in
  `qoob3d-kernel` (module `script`); project folder, values, build graph and cache keys in
  `qoob3d-core` (module `project`, no OCCT); service, worker processes, CLI and MCP server in
  `qoob3d-app` as subcommands of the one binary (`qoob3d serve | worker | build | status |
  export | mcp`).
- **Worker processes:** every build, check and analysis runs in a `qoob3d worker` child
  process with a time limit (hard kill) and a memory limit (resident set size, polled); a
  crash, hang or memory blow-up kills only that worker, which is replaced. The service and
  the app never run OCCT for project builds. This also closes the open item "process
  isolation of the kernel" in CLAUDE.md.
- **Memory and speed:** meshes are flat buffers (`ShapeMesh`), tessellated once per cache
  key in the worker and stored as an indexed binary file next to the BREP in the cache;
  the app loads changed parts only. Cache key = hash of the script, its imported modules,
  the target values it read, the keys of its dependencies and the program version.
- **Agent first:** the MCP server (stdio, `qoob3d mcp <dir>`) exposes the tools of concept
  section 6 as thin wrappers over the service API, answers as JSON
  `{ok, duration_s, warnings, result}`, and ships the script API reference and examples as
  resources so a fresh agent can learn the API.
- **The app** opens a project folder and shows the parts live (requirements
  `docs/requirements/project-app.md`); later mouse tools write script lines (concept 7a).

## Consequences

- No Python at runtime. The M0 Python probe stays as measured evidence only.
- We build and maintain the CAD API ourselves (the main cost); each OCCT capability needs
  a small exception-safe C++ wrapper (ADR-0004, ADR-0014).
- Agents do not know the API from training; the API reference and examples must be complete
  and are part of the MCP resources and of the tests (every documented example runs).
- One binary serves app, service, worker, CLI and MCP; worker start costs milliseconds.
- The W1 protocol uses English names (`.qoob3d/service.json`, `/status`, `/mesh/<part>`).

## Rejected alternatives

- **Python/build123d core (ADR-0052)** – rejected by the user; Python overhead (1.7 s per
  process) and memory of Python objects.
- **Models as a pure feature document** – no free code, which the concept names the most
  powerful tool for agents.
- **Models as Rust source compiled per part** – compile times of seconds per change.
- **Lua/JavaScript engines** – need C code or a large runtime; Rhai is pure Rust and
  sandboxed by default.

## References

- `docs/concept/werft-concept.md`, `docs/concept/m0-report.md`
- `docs/requirements/agent.md`, `docs/requirements/project-app.md`
- Related: ADR-0004, ADR-0005, ADR-0014, ADR-0030, ADR-0036
