# Requirements: agent-first modelling with Rhai scripts

As of: 2026-11-26 · Status: **R1–R4 and P1 implemented**, R5 specified, 6.6 implemented · Decision: ADR-0053 · Background: Werft concept
`docs/concept/werft-concept.md`, M0 measurements `docs/concept/m0-report.md`

Goal: an AI agent builds a model completely as code, checks, renders and exports it for FDM
printing without a UI; no run can block the environment or lose work; a human sees the state
live in the app and can model with the mouse later. IDs `AG-<area>-<no>` are stable.

Phases: **R1** script kernel · **R2** project, cache, worker processes · **R3** service, CLI ·
**R4** tasks, checks, MCP, render (ADR-0054) · then W1 (`project-app.md`), mouse modelling, photo measurement, print analysis.

## 1. Script kernel (R1) – `qoob3d-kernel`, module `script`

Rhai engine with our CAD API. Units mm, +Z up (ADR-0006). Every geometric operation calls
OCCT through exception-safe wrappers (ADR-0004); no geometry math in the script
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

### 2.1 Details for R2

- Crate placement (ADR-0053): parsing `qoob3d.toml`, `values/*.toml`, part declarations
  discovery, build graph and cache keys are pure logic in `qoob3d-core` (module
  `project`, must not depend on OCCT); evaluating parts uses `qoob3d_kernel::script` inside
  the worker; workers and the job runner live in `qoob3d-project`.
- Part declaration is found without running OCCT: the script is evaluated with a stub
  `part()` that only records name, options and the closure; `deps` passed to the closure is
  a map name → shape of the built dependencies.
- Cache key: SHA-256 (hex) over, in order: program version (with the kernel fingerprint,
  WS-PRE-02), the SHA-256 of the part file, the SHA-256 of every transitively imported module
  followed by that of every file `import_step`/`import_mesh` read in the last build (AG-SC-24;
  the service remembers these digests per file, WS-PRE-01), the sorted list of `(value id,
  canonical value)` read in the last build, and the keys of the dependencies. Before the
  first build the key is computed without values; after a build the recorded reads are part
  of the key, so changing a read value invalidates, changing an unread value does not.
- Cache entry files: `<key>.brep` (OCCT BREP), `<key>.qmsh` (tessellated, 0.01 mm linear
  deflection, edges included), `<key>.json` with `volume`, `bbox` `{min,max,size}`,
  `valid`, `watertight` (the mesh is closed and manifold, the print check), `solids`,
  `duration_s`, `values_read`, `modules_read`, `files_read`, `warnings`.
- Worker protocol: the parent starts `qoob3d worker` with stdin/stdout pipes, sends one JSON
  request per line (`{"job": id, "dir": …, "part": …, "deps": {name: brep_path}}`), the
  worker answers one JSON line per job. Workers stay warm between jobs.
- Limits: `time_s` per job (wall clock, kill + replace), `memory_mb` resident set size
  (polled every 0.2 s; Linux `/proc/<pid>/statm`, macOS via `ps -o rss=` or an equivalent
  without new heavy dependencies), kill + replace.

## 3. Service and CLI (R3)

### 3.1 HTTP details (envelope as in `project-app.md`)

- `GET /status` → `result`: `{"project": name, "parts": [{name, file, state, key, error?}],
  "jobs": [{id, state, kind, reason?}], "workers": {count, busy, recycled, slots: {budget,
  held, waiting}}, "hashing": [files],
  "load_errors": [{kind, message, file, line?, column?}], "error"?}` (superset of WA-PR-01;
  `recycled` as AG-PF-03). A part or check file that does not load is one entry of
  `load_errors` and never hides the parts of other files (ADR-0055); `error` repeats the
  first one, or is the load error of the whole project (configuration, values, dependency
  graph) with an empty part list. `/status` never waits for discovery: `"discovering":
  [files]` lists the part and check files whose declarations are not known yet (their earlier
  parts are `outdated`, a new file has no parts yet). `"hashing": [files]` lists large imported
  files whose digests are computed in the background (WS-PRE-01; their parts are `outdated`
  meanwhile, `qoob3d status` waits for them within its 10 s). A job that waits for a slot of the
  machine-wide worker budget is `queued` with `reason: "waiting for a worker slot"`
  (WS-PRE-06).
- `POST /discover {"files": [..]|null, "wait_s": f64}` (ADR-0055) waits up to `wait_s` for
  the discovery of those part and check files (`null` = all) → `{"files": [{file, state:
  "declared"|"failed"|"discovering", parts?: [{name, depends}], checks?: [..], error?}],
  "discovering": [..]}`. The MCP `file_write` of a part, check or `lib/` file uses it and
  answers `declared: {parts, checks}` or `load_error` (a `lib/` file: `load_errors`);
  `qoob3d status` waits up to 10 s for it before asking `/status`.
- `POST /build {"parts": [..]|null, "wait_s": f64}` → if finished within `wait_s`:
  `result` = build result; else `{"job": id, "state": "running"}`. Build result:
  `{"parts": {name: {state: "built"|"cached"|"failed", volume, bbox, valid, watertight, duration_s,
  error?: {kind, message, file?, line?, column?}}}, "built": [..], "cached": [..]}`.
- `GET /job/<id>` → `{id, state: "queued"|"running"|"done"|"failed"|"cancelled",
  progress: {done, total}, result?}`; `POST /job/<id>/cancel`.
- `GET /mesh/<part>` as WA-PR-02; `POST /export {"parts": [..], "format": "stl"|"3mf"|"step"}`
  writes `out/<part>.<ext>` (3MF: one object per part).
- `POST /shutdown` stops service and workers; `qoob3d serve` removes `service.json` on exit.

| ID | Requirement |
| --- | --- |
| AG-SV-01 | `qoob3d serve <DIR>`: HTTP on 127.0.0.1, `.qoob3d/service.json`; endpoints `/status`, `/build`, `/job/<id>`, `/job/<id>/cancel`, `/mesh/<part>`, `/export`; `/status` answers in < 100 ms while builds run. |
| AG-SV-02 | CLI: `qoob3d build|status|export|values` against the service (started on demand), JSON envelope on stdout, exit 0/1. |
| AG-SV-03 | MCP: `qoob3d mcp [DIR]` over stdio, specified in section 4.5 (AG-MC-*). |

## 4. Tasks, checks, MCP, render (R4) – ADR-0054

### 4.1 Task inbox (`qoob3d-core`, module `tasks`, pure)

| ID | Requirement |
| --- | --- |
| AG-TK-01 | A task is a file `tasks/NNNN-slug.md` (4 digits, lowercase slug `[a-z0-9-]+`). Its id is `T-NNNN`. The file starts with a TOML header between two lines `+++`: `title` (required, non-empty), `kind` = `"task"` \| `"requirement"` \| `"idea"` (default `task`), `state` (default `open`), `parts = [..]`, `checks = [..]` (default empty), `created = "YYYY-MM-DD"` (optional). After the header: the body (Markdown, the human's words), optionally followed by a section `## Log` with one line per entry `- <YYYY-MM-DD> <who>: <text>`. |
| AG-TK-02 | States: `open`, `in_progress`, `blocked`, `review`, `done`, `rejected`. Allowed transitions for the role **agent**: `open → in_progress`, `in_progress → blocked` (note required: the question), `blocked → in_progress`, `in_progress → review` (gate AG-TK-04), `review → in_progress`. Role **human**: every transition, including `done` and `rejected`. A forbidden transition is an error naming task, states and role. |
| AG-TK-03 | `Tasks::load(dir)` reads all tasks sorted by id; errors name the file (bad name, missing header, missing title, unknown kind/state, duplicate number). `create(dir, title, kind, body, parts, checks, date)` picks the next free number and a slug from the title (ASCII lowercase, other characters → `-`, at most 40 characters) and returns the task. `update(dir, id, role, new_state, note, date, gate)` rewrites only the `state` line of the header and appends the log entry (who = `agent` or `human`); body and other header fields stay byte-identical. A note without state change only appends to the log. |
| AG-TK-04 | Gate for `in_progress → review`: every part in `parts` exists and is in state `current`, every check in `checks` exists and passed in a run made after the last build; if `parts` and `checks` are both empty the gate needs a note. The pure layer takes this as input (`GateInput { parts: map name → state, checks: map name → passed }`) and reports every unmet condition. |

### 4.2 Checks (`checks/*.rhai`, run in a worker)

| ID | Requirement |
| --- | --- |
| AG-CK-01 | A check file declares checks with `check("name", || { … })`; names are unique in the project. Inside: `part("Name")` returns the built shape of a part (error if not built and current), `value("id")` as in parts, and all measure functions of the script API. |
| AG-CK-02 | Expectations: `expect(cond, "text")`, `expect_near(actual, expected, tol, "text")`, `expect_between(actual, min, max, "text")`. Every expectation is recorded with text, actual, expected/tolerance and pass/fail; a failed expectation does not stop the check. A script error fails the check with file, line, column. |
| AG-CK-03 | `POST /check {"names": [..]|null, "wait_s": f64}` builds stale parts first, runs the checks in a worker under the project limits and returns `{"checks": [{name, file, state: "passed"|"failed", duration_s, expectations: [{text, passed, actual, expected, tol}], error?}], "passed": n, "failed": n}`. `/status` gains `"checks": [{name, file, state: "passed"|"failed"|"stale"|"never_run"}]`; a check is `stale` when a part was rebuilt or a check file changed after its last run. CLI: `qoob3d check [NAMES..]`, exit 1 if any failed. |

### 4.3 Service additions

| ID | Requirement |
| --- | --- |
| AG-SV-04 | `GET /tasks` → list `{id, title, kind, state, parts, checks, file}`; `GET /task/<id>` → plus `body` and `log`; `POST /task {title, kind?, body?, parts?, checks?}` creates; `POST /task/<id> {state?, note?, role}` updates with the gate evaluated by the service from its own part and check states. CLI: `qoob3d task list|show ID|new TITLE [--kind K] [--body TEXT]|set ID STATE [--note TEXT]` (CLI acts as role human). |
| AG-SV-05 | `POST /run {"source": str, "wait_s": f64}` runs a throwaway script in a worker: `part("Name")` and `value("id")` are available, nothing is written to the project or the cache; result `{shapes: [{name, volume, area, bbox, center, valid, solids}], printed: [..]}` or the script error. |
| AG-SV-06 | `POST /measure {"a": part, "b"?: part}` → for `a`: volume, area, bbox, center, valid, solids; with `b`: `distance` and `common_volume` (0 if disjoint; an overlap is a finding, not an error). |
| AG-SV-07 | `POST /save {"message": str}` → runs all checks; if any part failed or any check failed: error kind `gate` listing them; else `git add` of the project files (never `.qoob3d/`, `out/`) and `git commit` in the project folder, returns the commit hash. Not a Git repository → error kind `git`. |
| AG-SV-08 | `qoob3d init <DIR>` creates a project skeleton (`qoob3d.toml`, `parts/`, `lib/`, `values/`, `checks/`, `tasks/`, `.gitignore` with `.qoob3d/` and `out/`) and refuses a non-empty folder that already has `qoob3d.toml`. |

### 4.4 Render for the agent

| ID | Requirement |
| --- | --- |
| AG-RD-01 | `qoob3d render-project <DIR> --out <PNG> [--view iso|front|back|left|right|top|bottom] [--size WxH] [--parts A,B]` renders the built parts headless (service started on demand, builds stale parts first), default view `iso`, default size 1024x768, camera fitted to the selected parts, orthographic. Unknown part → exit 1 naming it. |

### 4.5 MCP server (`qoob3d mcp [DIR]`, stdio)

| ID | Requirement |
| --- | --- |
| AG-MC-01 | JSON-RPC 2.0 over stdio, one message per line (MCP stdio transport). Implements `initialize` (answers with the client's protocol version if it is one of `2024-11-05`, `2025-03-26`, `2025-06-18`, else `2025-06-18`; capabilities `tools`, `resources`; `serverInfo` name `qoob3d`; `instructions` = the workflow guide), `notifications/initialized`, `ping`, `tools/list`, `tools/call`, `resources/list`, `resources/read`. Unknown method → error −32601; malformed JSON → −32700; nothing but protocol messages on stdout. DIR defaults to the working directory; the service is started on demand. |
| AG-MC-02 | Every tool result has one text content holding the JSON envelope `{ok, duration_s, warnings, result|error}` and `isError = !ok`. Tools (input schemas with descriptions in `tools/list`): `project_status`, `guide` (workflow guide), `api_reference {search?}` (the whole `docs/script-api.md`, or only the sections whose heading or text contains the search string, case-insensitive), `task_list {state?}`, `task_get {id}`, `task_create {title, kind?, body?, parts?, checks?}`, `task_update {id, state?, note?}` (role agent: can never set `done`/`rejected`), `file_list {dir?}`, `file_read {path}`, `file_write {path, content}`, `build {parts?, wait_s?}`, `job {id, cancel?}`, `run_script {source, wait_s?}`, `measure {a, b?}`, `check {names?, wait_s?}`, `render {view?, parts?, width?, height?}`, `export {parts, format}`, `values {}` (all values plus the assumptions), `save_state {message}`. |
| AG-MC-03 | File tools are confined: paths relative to the project, only under `parts/`, `lib/`, `values/`, `checks/`, `tasks/` and the file `qoob3d.toml`, only extensions `.rhai`, `.toml`, `.md`; no `..`, no absolute paths, no symlink escape, never `.qoob3d/`; at most 1 MiB per file. Writing a task file through `file_write` is refused (tasks change only through the task tools, so the role rules hold). |
| AG-MC-04 | `render` returns two contents: the text envelope (with `path` of the PNG under `out/renders/`) and an image content (`type: "image"`, base64 PNG, `mimeType: "image/png"`). |
| AG-MC-05 | Resources: `qoob3d://script-api` (Markdown), `qoob3d://guide`, and one per example script `qoob3d://examples/<name>.rhai`. The API reference and examples are embedded in the binary, so they match its version. |
| AG-MC-06 | The workflow guide (also tool `guide`) tells an agent in at most 60 lines: project layout, the loop (task_list → task_update in_progress → read API reference → write part and check → build → check → render and look → task_update review), the rules (values need sources; never guess a dimension, ask via `blocked`; booleans batched; units mm), and how errors are reported. |

## 5. Performance and memory (P1)

Measured on 2026-11-22 (debug app, 4 cores, hands-on project): service 25 MB resident, idle
worker 35 MB; a plate with 1000 holes as one batched cut takes 12.5 s, leaves the worker at
196 MB and a 6.2 MB mesh; the cache never shrinks.

| ID | Requirement |
| --- | --- |
| AG-PF-01 | Batched booleans (`union(array)`, `cut(a, array)`, operators) run OCCT in parallel mode. Results stay identical to 1e-9 relative volume. |
| AG-PF-02 | Profile algebra: `p1 + p2`, `p1 - p2`, `p1 & p2` and `profile_cut(p, array)`, `profile_union(array)` on 2D profiles (holes allowed), then `extrude`/`revolve`. `profile.move(x, y)` places a profile in its plane. A cut may split a profile into several regions (several solids after extrude); `profile_union([])` is an error. A plate 164 x 104 x 3 with 1000 holes of diameter 2 built as `extrude(profile_cut(rect, circles), 3)` has the analytic volume to 1e-6 relative and builds at least 10 times faster than the same part as a batched solid cut (both measured in the same test run). |
| AG-PF-03 | Worker recycling: after a job, a worker whose resident memory exceeds `[workers] recycle_mb` (default 512) is replaced by a fresh one before the next job; `/status` reports `workers.recycled` (count since start). Replacement may be lazy (when the next job is assigned). |
| AG-PF-04 | Cache eviction: per part at most `[cache] keep` entries (default 5, newest by modification time, the current key always kept); eviction runs after each successful build of that part and removes `.brep`, `.qmsh` and `.json` together; failed builds and cache hits evict nothing; `keep = 0` is a config error. |
| AG-PF-05 | Tessellation quality is configurable: `[mesh] deflection_mm` (default 0.01) and `angle_deg` (default 20); both are part of the cache key. |
| AG-PF-06 | `qoob3d bench <DIR>` builds all parts uncached twice and prints per part: build seconds, peak worker memory (MB), triangles, mesh bytes – as JSON envelope; used to track performance over time. `result.parts` maps name → `{build_s, peak_memory_mb, triangles, mesh_bytes}` (the faster of the two runs); a folder without a project gives exit 1 with `ok: false`. Settings in `qoob3d.toml` are read at service start. |

## 6. Hardening and agent feedback (R5)

Sources: critic review of 2026-11-22 (verified findings) and the first run of a fresh agent
that worked a free-text task ("wall hook for a bicycle helmet") over MCP only: 29 tool
calls, build and four checks green at the first attempt, no server error – and the review
gate proved nothing, because the task had no parts and checks bound and the agent could not
bind them.

### 6.1 Service hardening (critic)

| ID | Requirement |
| --- | --- |
| AG-HD-01 | Config: `[limits] time_s` must be finite and at most 86400; `memory_mb`, `recycle_mb` at most 1048576; violations are config errors naming the field. No config value can panic the service or a job thread; a panic in a job thread fails that job with kind `crash` and returns the worker to the pool. |
| AG-HD-02 | Declaration discovery (a worker request, ADR-0055; the service never evaluates user scripts) (`PartMode::Declare` and check discovery) runs with tight script size limits: string 65536 bytes, array 10000 elements, map 1000 entries, `max_operations` 1000000; exceeding one is `ScriptError::Limit` and becomes that file's load error. Worker runs keep no script size limits (the resident-memory limit of the worker protects them, AG-PJ-05). A part file whose top level doubles a string 40 times leaves the service alive and `/status` answering with the load error naming the file. |
| AG-HD-03 | The service writes a random token (32 hex characters) into `service.json` and requires it in the header `X-Qoob3d-Token` on every request; requests carrying an `Origin` header, or whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>`, are refused with 403 before routing. The app client, CLI and MCP send the token. `project-app.md` WA-PR gains the same rule. |
| AG-HD-04 | Clients percent-encode every path segment they build from a name (`/mesh/<part>`, `/task/<id>`, `/job/<id>`); a part named `Crane column` builds, shows in the app client and renders. |
| AG-HD-05 | The live poller re-reads `service.json` whenever the service does not answer and reconnects to a service restarted on another port; a failed first connection is retried every poll interval (WA-LV-05). |
| AG-HD-06 | The modules that feed the cache key are the modules the engine actually resolved during the build (recorded like value reads), plus the literal imports found by the scan before the first build. `import` with a non-literal path is refused with a script error at its position. Changing any resolved module rebuilds the part. |
| AG-HD-07 | Task create and update are serialised in the service (two simultaneous creates get two numbers; no lost log line). |
| AG-HD-08 | `/save` commits only paths inside the project folder (pathspec on the commit as on the add) and refuses with kind `git` when the Git top level is not the project folder. |
| AG-HD-09 | The gate reports load errors of part and check files as such instead of "part does not exist". |
| AG-HD-10 | `/status` lists at most the 20 newest jobs, each with `kind` (`build`, `check`, `run`). |
| AG-HD-11 | `bbox` is the same in the build result, `/measure` and scripts for the same key (one code path), tight to 1e-6 mm. |

### 6.2 Tasks and gate (agent run)

| ID | Requirement |
| --- | --- |
| AG-TK-05 | The agent can bind parts and checks to a task while it is `open` or `in_progress`: `POST /task/<id>` and MCP `task_update` accept `parts` and `checks` (replace the lists; the header lines are rewritten, the rest of the file stays byte-identical). Unknown names are accepted at binding time and fail the gate later. |
| AG-TK-06 | Gate for kinds `task` and `requirement`: at least one part **and** at least one check bound; kind `idea` needs a note only. The log entry of a hand-over to review lists the bound parts and checks. |
| AG-TK-07 | A state change is logged as `<old> -> <new>` followed by `: <note>` when a note is given; a note-only update logs the note. |

### 6.3 Guide and reference (agent run)

| ID | Requirement |
| --- | --- |
| AG-MC-07 | The guide (now at most 120 lines) states the dimension policy: dimensions the task or a measurement fixes are values with status `measured` or `one_source`; free design dimensions are values with status `assumption` and are listed in the review note; a missing **interface** dimension (something the part must fit to) or contradictory text is a question via `blocked`. It contains one complete minimal project (a values file, a part, a check), the TOML rules for values (ids, optional `unit` key), a table of the render views (direction the camera looks along, which axis is up), and an FDM section (model in print orientation or say so, fillet on top and chamfer on the bed face, overhang rule 45 degrees, holes). |
| AG-MC-08 | `api_reference` and `qoob3d://script-api` serve an agent edition of the reference: no repository paths, requirement IDs, ADR numbers or Rust notes; every example runs in a fresh project as written (files it imports are created by `qoob3d init` under `lib/examples/` or the example is self-contained). |
| AG-MC-09 | `render` accepts `views: [..]` (2 to 6 named views) and returns one contact sheet image with a label per view; the JSON result carries `bbox` of the rendered selection and the view table entry of each view. |
| AG-MC-10 | `task_get` returns, besides the task, `project`: the part and check names with states, so a new task needs no separate status call. |

### 6.4 Script API additions (agent run)

| ID | Requirement |
| --- | --- |
| AG-SC-17 | Profiles: `rounded_rect(w, h, r)`, `profile.rotate(deg)` about the profile origin, `offset(profile, d)` (positive outward, round corners; a negative offset that removes everything is a Geometry error). |
| AG-SC-18 | `countersink(d, head_d, angle_deg, l)`: tool body for a countersunk screw hole (clearance cylinder of length `l` downward from z = 0 plus the cone at the top); `counterbore(d, head_d, head_h, l)`. |
| AG-SC-19 | Selection by position: `.in_box([x0,y0,z0], [x1,y1,z1])` for edges and faces (whole element inside), `.near([x,y,z], r)`; `faces(s)….area()` (sum) and `.len()`; `edges(s)….length()`. |
| AG-SC-20 | `expect_eq(actual, expected, text)` records both values (ints, floats exactly, strings, bools). |
| AG-SC-21 | `s.mass(density_g_cm3)` in grams; `section(s, plane, offset)` returns the cut profile (`"xy"`, `"xz"`, `"yz"` at the offset) with `.area()`. |
| AG-SC-22 | Print analysis (first step of concept milestone 4): `overhangs(s, max_angle_deg)` returns the total area in mm^2 of faces whose downward normal is steeper than the limit, excluding faces on the bed plane z = min; `bed_area(s)` returns the area in contact with z = min. Both are exact on planar faces and within 1 % on curved ones. |

### 6.5 Mesh import (second agent run)

Source: the second free-text task (hose guide for a Bambu Lab A1) stopped at the human's
3MF of the old clip: qoob3d read only STEP, and the agent could not even see the file. STL
and 3MF are what slicers, MakerWorld and Bambu Studio hand out. Decision: ADR-0034.

| ID | Requirement |
| --- | --- |
| AG-SC-23 | `import_mesh(path)` reads an STL (binary or ASCII) or a 3MF (chosen by the extension `.stl` / `.3mf`, case-insensitive; any other extension is a runtime error naming the path and both extensions) from the project folder (same confinement as `import_step`) and returns it as a shape in mm. 3MF: core specification and the production extension (`p:path` components in other model files of the archive, as Bambu Studio writes them), components nested to any depth, the `transform` of every component and build item applied (3MF row-vector convention: `x' = x·m00 + y·m10 + z·m20 + m30`, …), the model's `unit` (`micron`, `millimeter`, `centimeter`, `inch`, `foot`, `meter`) converted to mm, ZIP entries stored or deflated; every build item is imported, resources not referenced from the build are not. A transform with a negative determinant (mirror) flips the winding, so the volume stays positive. The triangle limit counts triangles as read, after expanding components and before dropping collapsed ones. Vertices closer than 1e-4 mm are welded, collapsed triangles dropped. Every closed, edge-manifold group of connected triangles becomes one solid (`s.solids()` counts them); coplanar adjacent triangles become one planar face, so a box mesh of 12 triangles has 6 faces and 12 edges. The volume of a closed mesh equals its signed triangle volume to 1e-6 relative. Errors (runtime, naming the file): unreadable or malformed file (with the reason, e.g. the ZIP entry or XML element; this includes a CRC-32 mismatch, an unknown `unit`, a file without triangles, and an ASCII STL that ends early), a required 3MF extension other than the production extension, more than 200 000 triangles (naming the count), and a mesh with boundary or non-manifold edges (geometry error naming their count). Readers of these bytes never panic and never allocate more than the input justifies; a 3MF whose entries inflate to more than 512 MiB is refused. |
| AG-SC-24 | Files read by `import_step` and `import_mesh` during a build are inputs of the part like imported modules: the build result lists them per part in `result.parts[<name>].files_read` (project-relative, `[]` when none), their bytes enter the cache key, and changing, replacing or removing one makes the part stale and rebuilds it (and its dependents) on the next build. |
| AG-MC-11 | `file_list` also reports the importable files (`.step`, `.stp`, `.stl`, `.3mf`, case-insensitive) under the allowed folders in a separate list `imports` of `{path, size}` (project-relative path, size in bytes; filtered by `dir` like `files`); `files` is unchanged. `file_read` on such a file answers an error that names `import_step` or `import_mesh`; `file_write` still refuses them. The guide tells the agent that the human puts reference files into `lib/` and how to import them. |

### 6.6 Findings of a real model (Damen 5009 display kit)

Source: an agent turned a Fusion 360 RC ship model (355 bodies, an imported freeform hull of
1689 faces, scale 1:40) into an FDM display kit in 1:87 over MCP and wrote down what did not
work (`FINDINGS.md` of that project). The measurements behind the decisions are in ADR-0053
(booleans on freeform bodies) and ADR-0034 (meshes, kits).

| ID | Requirement |
| --- | --- |
| AG-SC-25 | Booleans on freeform bodies (a face that is not plane, cylinder, cone, sphere or torus): without a `fuzzy` option, `cut`/`-`, `common`/`&` and `section` use twice the largest edge tolerance of the inputs (0.00001–0.001 mm) as fuzzy value, a union is exact; every boolean that fails or is implausible is computed again serially, a union finally with the fuzzy value. A result that BRepCheck calls invalid is repaired (ShapeFix, same-parameter) when the repair is valid. A union must keep the extent of its inputs (bbox within 1e-6 · size + 0.001 mm), else it is a geometry error. Volumes are integrated adaptively to 1e-7 relative. `union`, `cut` and `common` take `#{ fuzzy: mm }`. Analytic shapes stay exact and unchanged. |
| AG-SC-26 | `defects(s)` lists what BRepCheck finds (at most 50: `kind`, `index`, `problem`, `min`, `max`), empty for a valid shape; `heal(s[, tolerance])` repairs an invalid shape with ShapeFix and same-parameter (default 0.0001 mm), returns a valid one unchanged, and refuses a volume change beyond the tolerance times the area. |
| AG-SC-27 | `outline(profile)` drops the holes of a profile; `fill_voids(s)` drops the cavities of every solid. |
| AG-SC-28 | `on(plane, profile, offset)` places a profile on the parallel plane where `section(s, plane, offset)` cuts; `split(s, plane, offset)` returns `[below, above]` (a plane that does not cut is a geometry error); `on_bed(s)` stands `s` on z = 0, centred on the Z axis. |
| AG-SC-29 | `import_step(path, name)` returns the solids whose STEP name is `name` (or one of an array of names), `step_names(path)` the names in file order; a missing name is a runtime error listing the names; a STEP file is read once per run. |
| AG-SC-30 | `group(shapes)` is one shape of several without a boolean. |
| AG-SC-31 | `offset(s, d)` grows (round edges) or shrinks a solid; the volume must change in the direction of `d`. |
| AG-SC-32 | `heights(s, [x0, y0], [x1, y1], step)` returns the highest z per grid point (row per y, `()` where nothing is hit), at most 1 000 000 points, with one intersector for all lines. |
| AG-SC-33 | `loft(profiles, #{ smooth: true })` lofts a smooth surface; without the option the loft stays ruled. |
| AG-SC-34 | `mesh_report(s)` returns `watertight`, `triangles`, `open_edges`, `non_manifold_edges`, `flipped_edges`, `volume` of the exported form of `s`, tessellated with the project's `[mesh]` settings (also in checks). |
| AG-SC-35 | A geometry error caught with `catch (e)` reads as `"<function> failed: <message>"`; the lines a script printed before an error come back with it (`error.printed` of builds, checks and `run_script`), and a successful build lists them in `printed`. Expression nesting is 128 levels at the top level and 64 in functions in every build. |
| AG-SC-36 | A face OCCT leaves without triangles is meshed again finer and with relative deflection; if one still has none, a repaired copy of the whole shape (without its triangles) is meshed, and as the last resort the face gets a fan over its boundary points; only when that fails too the build fails with kind `script`, naming the part, the face and its box. Validity is judged before meshing. |
| AG-SC-37 | `bed_area`, `overhangs`, `top()` and `bottom()` count what lies within 0.001 mm of the lowest (highest) point; a cut face 20 nm above the lowest point of freeform faces is the bed face. Cavities of a mesh (closed inward surfaces inside another) become inner shells of their solid; an inward surface inside nothing is refused. |
| AG-SC-38 | `text(string, #{ font, height, spacing, align })` sets one line of lettering as a profile: the glyph outlines of a TrueType/OpenType file of the project (read like an import, an input of the cache key), scaled so that the cap height (height of "H", else the capital height metric, else the ascender) is `height` mm, baseline y = 0, running along +x, placed by advance widths and `kern` pairs; curves are polygons within 0.002 × `height` (at most 24 segments each), counters are holes by nesting, ink that touches or overlaps is united. A character without a glyph (named), an empty or blank string, a line break, more than 200 characters and a file that is no font (or larger than 16 MiB) are runtime errors. |
| AG-PJ-06 | The part option `pose: #{ rotate: [axis, deg] or a list of them, count: n }` declares the print pose (turns in order, then standing on the bed, centred) and the number of copies; a wrong `pose` is a load error naming the part. |
| AG-SV-09 | `/export` writes STL and 3MF in each part's print pose and answers `details` per file (`triangles`, `size`, `watertight`, `warnings`; `count` when > 1). With `kit: name` (stl or 3mf) all parts, each `count` times, go into one file `out/<name>.<format>`, arranged in rows on a bed `bed_mm` wide (default 256, gap 5 mm), with `objects`, `footprint` and `fits_one_bed`. Watertightness is judged on the exported form: vertices on open edges within 0.002 mm are welded, collapsed triangles do not count. |
| AG-PJ-07 | The part option `color: "#RRGGBB"` (or one of the sixteen basic web colour names, `orange`, `grey`) declares the part's colour, reported as `#RRGGBB`: `/status` reports it, the app and `render` draw the part in it, and a slicer project prints it in the filament of that colour. `print_with: "Host"` names the part it prints with as one multi-colour object. A wrong `color` or `print_with` is a load error naming the part. |
| AG-SV-10 | `/export` with `project: name` (format 3mf, not with `kit` or `bed_mm`) writes one Bambu Studio project `out/<name>.3mf` for `printer` (an id of the printer database whose presets are known, default `bambu-lab-p1s`): all parts in their print pose and count as objects on plates of that bed, 5 mm apart and from the edge, clear of the bed's exclusion zone; one filament per distinct colour in order of first use (parts without colour share a light grey one). Default: every part is an object and parts of different colours never share a plate. With `multicolor: true` a part with `print_with` is a part of its host's object, posed with the host; a plate holds objects that need the same filaments; a host that is not exported or prints with another part itself is refused. The answer lists `printer`, `presets`, `filaments` (`color`, `parts`), `plates` (`filaments`, `objects`) and `objects` (`plate`, `size`, `fits`, `watertight`, `parts` with `filament`); an object larger than the bed lies alone on a plate, `fits: false`, with a warning. The file names the slicer's printer, process and filament presets and carries none of their values. Where Bambu Studio is installed, a test has it load and slice the projects and compares plates and filaments. CLI: `--project NAME [--multicolor] [--printer ID]`; MCP `export {project, multicolor, printer}`. |
| AG-MC-12 | `render` takes `source` (a throwaway script whose shown shapes are rendered; nothing is written to the project, the meshes in `.qoob3d/run/` are removed) and `region` (`{min, max}`: the camera frames this box); orthographic images frame the projected box of what they show; a file name longer than 96 characters uses a hash of the part list. `file_write` of `qoob3d.toml` answers when its sections take effect. |

## 7. Acceptance

- R1: the three M0 probe parts written as scripts reproduce the analytic volumes from
  the M0 probe (`docs/concept/m0-report.md`, formulas in
  `crates/qoob3d-kernel/tests/script_examples.rs`) to < 1e-6 relative; build times as AG-SC-15.
- R2: an endless script is killed after the time limit while the service answers; changing
  one part rebuilds only it and its dependents.
- R3: CLI and service pass `crates/qoob3d-app/tests/service.rs`.
- R4: a fresh agent, given only the MCP server and a task file "box with lid and pin
  connection, 0.2 mm clearance", builds both parts with a check and hands the task over in
  state `review`; the human sets `done`.
- R5: the fresh-agent run is repeated with a new free-text task; the task ends in `review`
  with parts and checks bound, and the agent's friction list has no item of sections 6.2–6.3.
- Real model (6.6): the Damen 5009 kit project builds from an empty cache with every part
  valid and every printed part watertight, its checks pass, and every exported 3MF loads in
  Bambu Studio as manifold (opt-in test `crates/qoob3d-app/tests/acceptance.rs`).
- Mesh import (6.5): a 3MF written by Bambu Studio from two parts of known volume imports
  with both volumes and sizes; the human's 3MF of a real part, copied into `lib/`, imports in
  a fresh agent session and the agent can measure it.
