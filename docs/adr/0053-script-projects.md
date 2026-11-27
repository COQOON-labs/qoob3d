---
id: ADR-0053
title: A model is a project folder of Rhai scripts on our own OCCT kernel, built by a local service in worker processes with limits and an exact cache
status: accepted
date: 2026-11-23
crates: [qoob3d-core, qoob3d-kernel, qoob3d-project, qoob3d-app]
tags: [architecture, ai-first, scripting, performance, process-isolation]
supersedes: []
superseded_by: null
---

## Context

QOOB3D is agent first: an AI agent builds the model, the human gives tasks and accepts
results (ADR-0054). The concept "Werft" (`docs/concept/werft-concept.md`) proposed models as
Python code (build123d on OCCT) in a Git folder, with a service, warm workers, a cache, values
with sources and checks. Free code is the agent's strongest tool. The user then decided on
everything in Rust with an embedded script language, because speed and memory matter most
(many polygons, many 3D operations, memory must not blow up).

Facts from milestone 0 (`docs/concept/m0-report.md`): in build123d all geometry runs in OCCT
and Python is glue only; importing build123d costs 1.6–1.7 s in every new process; the stair
probe part took 0.675 s. The same stair built in 161 ms in our Rhai kernel (measured
2026-11-22). OCCT can fail silently (a "valid" shell with volume 0), and it can hang, eat
memory or segfault on hard input (ADR-0004); the only real protection is a separate process
with time and memory limits.

The original interactive path (`.qbd` documents with sketches and features) was removed on
2026-11-23; the app shows script projects only.

This topic absorbs the former decision 0052.

## Decision

### Models are scripts in a project folder

A model is a project folder in Git: `qoob3d.toml` (name, units, limits, workers, cache, mesh
settings), `parts/*.rhai`, `lib/*.rhai` (modules), `values/*.toml` (target values with
mandatory `source` and `status`), `checks/*.rhai`, `tasks/*.md`; `out/` (exports) and
`.qoob3d/` (cache and service data) are never saved. The names are spelled in one place,
`qoob3d_core::project::layout`. A part file declares parts with
`part(name, #{ depends: [...] }, |deps| shape)`; the dependencies form the build graph.

### Rhai with our CAD API

- The script language is **Rhai**: pure Rust, sandboxed (no file or network access; `import`
  and `import_step` resolve inside the project folder only), with an operation limit
  (default 50 million) and a call depth limit.
- The CAD API (`qoob3d_kernel::script`) is registered into the engine; every call goes
  straight to OCCT through our wrappers (ADR-0004). Geometry never runs in Rhai.
- Booleans are batched and checked by a plausibility rule on the volume, because OCCT can
  return "valid" results that are wrong; freeform bodies get the rules below.
- The API reference `docs/script-api.md` is product surface: every example in it runs in the
  kernel tests, and it is served to agents over MCP.

### Booleans on freeform bodies (AG-SC-25…26, measured on a real model)

An imported hull (1689 faces, mostly B-spline) showed four ways OCCT fails silently or
loudly; each rule below is the one the measurements supported:

- **Volume**: the fixed Gauss integration was 1.1 % off on the hull (2 091 902 against
  2 115 980 mm³; Fusion: 2 115.8 cm³), so a cut of 15 pin holes looked like "removed no
  material". Volumes are integrated adaptively to 1e-7 relative.
- **Fuzzy for cuts and commons**: an exact common of the hull with a slab whose top lay in a
  plane through edges of the hull returned 39 270 mm³ and was valid; the integral of 2D
  sections gave 78 493, the fuzzy common 78 516. Common and cut of the same exact run still
  added up to the body (the piece was misclassified on both sides), so a bookkeeping check
  does not find this. Cuts, commons and sections of freeform bodies therefore use twice the
  inputs' largest edge tolerance (at least 1e-5 mm, which was the smallest value that gave
  the right result) as fuzzy value.
- **Exact unions**: the union of the same hull with its deck lid was right exactly
  (2 615 471 mm³) and destroyed with any fuzzy value (negative or 2.5 % off volume). Unions
  stay exact; fuzzy is their last retry. A union must also keep the box of all its inputs: an
  earlier variant dropped the lid while its volume stayed above the largest input.
- **Repair**: cuts through freeform faces came back invalid (edges whose 3D and 2D curves
  disagree). ShapeFix plus same-parameter made every one of them valid without changing the
  volume; ShapeFix_Wireframe changed it by up to 3 % and is not used. Invalid results of
  booleans on freeform bodies are repaired at once; `heal` and `defects` expose it to scripts.
- **Retries**: a failed or implausible boolean is run again serially (parallel runs failed
  where serial ones succeeded); analytic shapes never take the fuzzy or repair path, so their
  results and costs are unchanged. The cold build of the kit project went from invalid pieces
  to all valid in 2 min 15 s.

### Lettering from a font file of the project (AG-SC-38)

A model of a real ship needed its name on the hull, and the design it came from had none.
Decided 2026-11-26:

- **`text(string, #{ font, height })` returns a profile** like `rect` and `polygon`: outlines
  with the counters as holes, cap height in mm, so everything that takes a profile (extrude,
  profile algebra, `on`) works on letters.
- **The font is a file of the project**, read like an imported STEP or mesh file: confined to
  the project folder, and its bytes enter the cache key. No system fonts (a build would
  depend on the machine) and no font compiled into the kernel.
- **Outlines are parsed in `qoob3d-core`** with `skrifa` (fontations: pure Rust, maintained,
  egui's font reader too; the legacy `kern` table through its `read-fonts`; at least 0.50,
  whose `read-fonts` no longer panics on a glyph without points, which the damaged-font
  property test found in 0.44)
  and flattened to polygons within 0.002 × cap height; holes are found by nesting depth, not
  by winding. Font bytes are foreign input: sizes are capped (16 MiB, 200 characters, points
  per glyph). `ttf-parser`, used first, is unmaintained (RUSTSEC-2026-0192) and a parser
  of foreign bytes without fixes is not kept.
- **New projects get DejaVu Sans Bold** with its licence (`lib/examples/`, 0.7 MB): a free
  face with Latin diacritics, so the reference examples run everywhere.

Known gap: glyphs that touch in one point (kerned "R" and "D" of that font) give an edge
shared by four faces once extruded; the first model works around it with extra spacing.
Rejected: OCCT's own text builder (needs FreeType in the OCCT build, ADR-0004).

### Worker processes, the service never runs OCCT

- `qoob3d serve <DIR>` is the project service (HTTP on 127.0.0.1); its internals (token,
  jobs, declaration discovery) are ADR-0055.
- Every build, check, throwaway script, measure and declaration discovery runs in a
  `qoob3d worker` child process. A worker that exceeds the time limit (`[limits] time_s`,
  default 60 s, hard kill) or the memory limit (`memory_mb`, default 4096, resident set size
  polled) is killed; a crash kills only that worker. All three are replaced. A worker above
  `[workers] recycle_mb` (default 512) is replaced after its job. The service and the app
  never run OCCT for a project and never evaluate a script.

### Exact cache, meshes once

- Cache key = SHA-256 over the program version (including the cache entry version and the
  `[mesh]` settings), the part file, the modules the engine resolved, the target values the
  part read, and the keys of its dependencies.
- The worker writes `.qoob3d/cache/<part>/<key>.{brep,qmsh,json}`: the BREP for dependents,
  STEP and measures; the mesh tessellated once in the indexed `qmesh` format; the build
  results (volume, bounding box, valid, watertight, values and modules read, warnings),
  written last and atomically, so it marks the entry complete. `[cache] keep` (default 5)
  entries per part survive eviction.
- The app polls `/status` and loads only changed meshes; STL and 3MF are written from the
  cached mesh, STEP from the BREP.

### One binary

Service, worker, CLI clients, MCP server and app are subcommands of the one binary `qoob3d`
in `qoob3d-app` (`serve`, `worker`, `build`, `status`, `export`, `values`, `check`, `task`,
`bench`, `init`, `render-project`, `mcp`, `--project`). The project logic without OCCT lives
in `qoob3d-core::project`, the script engine in `qoob3d-kernel::script`.

## Consequences

- No Python at runtime; worker start costs milliseconds instead of 1.7 s.
- We build and maintain the CAD API ourselves (the main cost); each OCCT capability needs a
  small exception-safe wrapper and an analytic volume test.
- Agents do not know the API from training, so the reference, the guide and the examples must
  be complete; tests enforce that every documented function and example works.
- An OCCT segfault or hang costs one job, not the service or the app.
- Service, workers and MCP live in `qoob3d-project`, without GPU or UI (ADR-0036).

## Rejected alternatives

- **Python/build123d core with QOOB3D as its app** – the first plan; 1.7 s import per
  process, memory of Python objects, two languages and two OCCT versions (geometry would
  cross the boundary as meshes only).
- **Models as a feature document** – the removed `.qbd` path; no free code, which the agent
  needs.
- **Models as Rust source compiled per part** – seconds of compile time per change.
- **Lua or JavaScript engines** – need C code or a large runtime; Rhai is pure Rust and
  sandboxed by default.
- **OCCT in a thread of the service** – a segfault or memory blow-up would take down the
  service.
- **One fuzzy value for every boolean** – right for the cuts, destroyed the union (see above).
- **Bookkeeping of cuts (common + cut = body)** – costs a second result and three volumes per
  cut and did not find the misclassified slab.

## References

- Code: `crates/qoob3d-kernel/src/script.rs`, `crates/qoob3d-kernel/src/script/`,
  `crates/qoob3d-core/src/project/`, `crates/qoob3d-core/src/qmesh.rs`,
  `crates/qoob3d-project/src/{service.rs,worker.rs}`, `crates/qoob3d-app/src/{cli.rs,live.rs}`,
  `crates/qoob3d-project/src/service/{pool,jobs,snapshot}.rs`
- `docs/script-api.md`, `docs/requirements/agent.md` (§6.6), `docs/requirements/project-app.md`,
  `docs/concept/werft-concept.md`, `docs/concept/m0-report.md`
- Related: ADR-0004, ADR-0036, ADR-0054, ADR-0055
