# Requirements: agent-first modelling with Rhai scripts

As of: 2026-11-22 · Status: **R1 specified** · Decision: ADR-0053 · Background: Werft concept
`werft/docs/konzept.md` (German), M0 measurements `werft/docs/m0_bericht.md`

Goal: an AI agent builds a model completely as code, checks, renders and exports it for FDM
printing without a UI; no run can block the environment or lose work; a human sees the state
live in the app and can model with the mouse later. IDs `AG-<area>-<no>` are stable.

Phases: **R1** script kernel · **R2** project, cache, worker processes · **R3** service, CLI,
MCP · then W1 (`project-app.md`), mouse modelling, photo measurement, print analysis.

## 1. Script kernel (R1) – `qoob3d-kernel`, module `script`

Rhai engine with our CAD API. Units mm, +Z up (ADR-0006). Every geometric operation calls
OCCT through exception-safe wrappers (ADR-0004, ADR-0014); no geometry math in the script
layer where OCCT has it.

| ID | Requirement |
| --- | --- |
| AG-SC-01 | `script::run(source: &str, options: &RunOptions) -> Result<RunOutput, ScriptError>` evaluates a script. `RunOutput` holds the shapes registered with `show(shape, "name")` in call order, and the printed lines (`print`). |
| AG-SC-02 | Sandbox: no file, network or process access from scripts (Rhai without such modules); `RunOptions.max_operations` (default 50 000 000) and a call depth limit stop runaway scripts with `ScriptError::Limit`. |
| AG-SC-03 | Errors name line and column of the script position (`ScriptError::{Syntax, Runtime, Geometry, Limit}` with `line`, `column`, `message`). A failed OCCT operation is `Geometry` with the name of the API function. |
| AG-SC-04 | Solids: `box(x, y, z)` (centered in X/Y, bottom at z = 0), `cylinder(d, h)` (axis Z, bottom at z = 0, centered), `sphere(d)` (center at origin), `cone(d1, d2, h)`, `torus(d_ring, d_tube)`. Invalid dimensions (≤ 0, NaN) are `Runtime` errors. |
| AG-SC-05 | Booleans as operators and functions: `a + b` (union), `a - b` (cut), `a & b` (common); `union(array)`, `cut(a, array)` run as **one** OCCT operation (M0: batched booleans are 20× faster). |
| AG-SC-06 | Transforms return new shapes: `s.move(x, y, z)`, `s.rotate(axis, deg)` with axis `"x"|"y"|"z"` or `[x, y, z]`, `s.mirror("xy"|"xz"|"yz")`, `s.scale(f)`; patterns `linear(s, n, [dx,dy,dz])`, `polar(s, n, deg_total)` (about Z). |
| AG-SC-07 | 2D profiles in the XY plane: `rect(w, h)` (centered), `circle(d)`, `polygon([[x,y],…])`, `slot(l, w)`; `extrude(profile, h)`, `revolve(profile, deg)` about the Y axis of the profile mapped to Z (profile drawn in XZ: x = radius, y = height), `loft([profile_at_z…])` via `profile.at_z(z)`. Profiles can be placed on planes: `on("xz", profile)`. |
| AG-SC-08 | Wire/pipe: `pipe(points, d, bend_radius)` sweeps a circle along a polyline whose corners are rounded with `bend_radius` by OCCT (M0 railing). `bend_radius <= d/2` is a `Runtime` error (M0: the kernel does not detect self-intersection). |
| AG-SC-09 | Edge features: `fillet(s, edges, r)`, `chamfer(s, edges, d)`; edge selectors `edges(s)` with filters `.parallel("z")`, `.at_z(z, tol)`, `.circles()`, `.lines()`, `.top()`, `.bottom()`; `all_edges(s)`. A failing fillet names the first edge that fails when tried one by one (M0 finding). |
| AG-SC-10 | Shell: `shell(s, thickness, open_faces)` with face selectors `faces(s).top()` etc. The result must pass the plausibility rule AG-SC-12, otherwise `Geometry` error (M0: OCCT returned "valid" shells with volume 0 or unchanged). |
| AG-SC-11 | Measure: `s.volume()`, `s.area()`, `s.bbox()` (map with `min`, `max`, `size`), `s.center()`, `distance(a, b)`, `ray(s, [x,y,z], [dx,dy,dz])` → hit points sorted by distance (from intersecting faces, not `Solid & Axis`), `s.is_valid()`, `s.solids()`. |
| AG-SC-12 | Plausibility after every boolean, shell, fillet and chamfer: result volume > 1e-6 mm³; cut and shell must shrink the volume, union must not shrink it below the largest input. A violation is a `Geometry` error naming the operation. |
| AG-SC-13 | Helpers for FDM connections: `pin(d, l)` (cylinder downward from z = 0), `hole(d, l, clearance)` (diameter `d + 2*clearance`, depth `l + clearance`). |
| AG-SC-14 | Import/export inside the worker: `import_step(path)` (path relative to the project, checked by the caller), and from Rust `export_stl`, `export_3mf` (existing writers, ADR-0034) and `export_step`. |
| AG-SC-15 | Performance: building the M0 probe parts as scripts (crane column, stair with 11 steps, railing) takes < 1 s each in release builds; script overhead < 5 % of the OCCT time for the stair. |
| AG-SC-16 | API reference: every function has a doc entry (signature, units, example) in `docs/script-api.md`; every example in it runs in a test. |

## 2. Project, values, cache (R2)

| ID | Requirement |
| --- | --- |
| AG-PJ-01 | Project folder: `qoob3d.toml` (`[project] name`, `units = "mm"`, `[limits] time_s = 60, memory_mb = 4096`, `[workers] count = 2`), `parts/*.rhai`, `values/*.toml`, `checks/*.rhai`, `out/`, `.qoob3d/`. |
| AG-PJ-02 | A part file declares parts with `part("Name", #{ depends: ["Other"], material: "PLA", color: "white" }, |deps| { … shape })`; several parts per file; names unique; unknown dependency or cycle is a load error naming the parts. |
| AG-PJ-03 | Target values: `values/*.toml`, each entry `[id] value = … ; source = "…" ; status = "measured"|"one_source"|"assumption"`. Missing `source`/`status` is a load error naming file and id. `value("crane.column")` in scripts returns the value and records the read. A report lists all assumptions. |
| AG-PJ-04 | Build graph and cache: a part is rebuilt iff its file, an imported module (`import "lib/x"`), a value it read, a dependency key or the program version changed. Cache in `.qoob3d/cache/<part>/<key>.{brep,qmsh,json}`. |
| AG-PJ-05 | Worker processes `qoob3d worker`: every build runs in a child process with time limit (kill) and memory limit (RSS polled every 0.2 s); crash, limit or kill replaces the worker; the error kind is `time_limit`, `memory_limit`, `crash` or `script` (with file, line, column). |

## 3. Service, CLI, MCP (R3)

| ID | Requirement |
| --- | --- |
| AG-SV-01 | `qoob3d serve <DIR>`: HTTP on 127.0.0.1, `.qoob3d/service.json`; endpoints `/status`, `/build`, `/job/<id>`, `/job/<id>/cancel`, `/mesh/<part>`, `/export`; `/status` answers in < 100 ms while builds run. |
| AG-SV-02 | CLI: `qoob3d build|status|export|values` against the service (started on demand), JSON envelope on stdout, exit 0/1. |
| AG-SV-03 | MCP: `qoob3d mcp <DIR>` over stdio with the tools of concept section 6 except photo/print: `project_status`, `build`, `job`, `run_script` (throwaway worker, does not change the project), `measure`, `render`, `check`, `compare`, `export`, `save_state` (Git commit after green checks); resources: `docs/script-api.md` and examples. |

## 4. Acceptance

- R1: the three M0 probe parts written as scripts reproduce the analytic volumes from
  `werft/m0/teile.py` to < 1e-6 relative; build times as AG-SC-15.
- R2: an endless script is killed after the time limit while the service answers; changing
  one part rebuilds only it and its dependents.
- R3: a fresh agent builds a box with lid and pin connection via MCP only, with a check.
