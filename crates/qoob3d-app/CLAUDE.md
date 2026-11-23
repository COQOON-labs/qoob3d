# qoob3d-app

Library + binary `qoob3d` (ADR-0013): the app that shows a script project live, the project
service, its workers, the CLI and the MCP server (ADR-0053, ADR-0054).

## Rules

- **Avoid the egui deadlock:** inside `ui.input(|i| …)`/`ctx.input(…)` **no**
  `Response` methods and no other `ctx` accesses. Fetch the state into local
  variables beforehand.
- Navigation exactly as in `docs/design/controls.md` (ADR-0008). When changing, adjust both.
- egui draws the overlays (ViewCube, navigation bar, pivot), wgpu the scene (ADR-0019).
- **No visible texts in code** (ADR-0020): always `tr("key")` / `tr_args(…)`.
  Create new keys in **all** `locales/*/main.ftl` (English = base). `cargo xtask i18n`
  checks this. Numbers via `i18n::decimal`.
- Translation keys always as literals in code, never assembled via `format!` – otherwise
  `cargo xtask i18n` does not find them.
- Colors from `theme.rs` / `docs/design/theme.md`.
- Cursor rule (docs/design/controls.md): arrow everywhere, pointing hand only on the ViewCube,
  grabbing hand while dragging. Labels are not selectable.
- Persistent settings belong in `Settings` (serde, `#[serde(default)]`).
- Every new interaction needs: a UI test in `tests/ui.rs` or `tests/project.rs`
  (egui_kittest) and, if it affects the camera, a step in `--smoke-test`.
- Textures displayed by egui must deliver gamma-encoded values (ADR-0019).
- HTTP never runs on the UI thread (WA-LV-01): the `Poller` thread talks to the service.

## Files

- `src/main.rs`: entry point, CLI switch. `src/lib.rs`: modules for tests.
- `src/cli.rs`: arguments (`--project`, `--smoke-test`) and the subcommands `serve`, `worker`,
  `build`, `status`, `export`, `values`, `bench`, `check`, `task list|show|new|set`, `init`,
  `render-project`, `mcp`.
- `src/service.rs` + `src/service/`: `qoob3d serve` (AG-SV-01, ADR-0053): routes and envelope,
  single instance via `.qoob3d/service.lock`; **every request needs the token of this start**
  (`service.json`, header `X-Qoob3d-Token`), a local `Host` and no `Origin`, else 403 before
  routing (`Service::allowed`, AG-HD-03, ADR-0055); `http.rs` minimal server side HTTP; `jobs.rs` job
  queue and parallel part scheduling, cache hits; `pool.rs` warm workers, time/memory/crash
  limits, replacement, recycling above `[workers] recycle_mb` (AG-PF-03); `discovery.rs`
  declaration discovery (ADR-0055): every changed part or check file goes to a worker
  (`Probe::Declare`) on a thread of its own, answers and load errors cached by file and
  literal-import bytes; `/status` and `/mesh` never wait for discovery and show the last
  known declarations (parts of a file under rediscovery `outdated`, of a broken file `failed`,
  a never discovered file no parts yet, `discovering` lists files not known yet), builds,
  checks and the gate wait for all files, `POST /discover` waits for chosen files (MCP
  `file_write` answers `declared`/`load_error` with it, `qoob3d status` settles up to 10 s);
  the cache lock is never held while a worker runs; `snapshot.rs` the project from those
  declarations (per-file `load_errors`, a broken file never hides the others), keys, states,
  cache eviction (`evict`, AG-PF-04);
  `bench.rs` `qoob3d bench <DIR>` (AG-PF-06): a fresh worker per build, entries in
  `.qoob3d/bench-<pid>/` (removed afterwards), never the project cache; it discovers in its own
  process. The service never runs OCCT and never evaluates a script. The `[mesh]` settings
  enter the cache key through `worker::key_version` (AG-PF-05).
  R4 (ADR-0054): `jobs.rs` `Work` (build, check, run) so `/check` and `/run` answer with a job
  like `/build`; `checks.rs` checks of the discovered `checks/*.rhai`, `/check`
  (one worker request per check), states passed/failed/stale/never_run (stale = check file or
  the build key of a part the check read changed), `gate_input` for the review gate; `probe.rs`
  `/run` and `/measure`; `save.rs` `/save` (`git` executable, never adds `.qoob3d/`, `out/`);
  `tasks.rs` `/tasks`, `/task/<id>` on `qoob3d_core::tasks` (`role` is required). `/status`
  stays lock-short: check states come from records, never from a run.
  R5 hardening (agent.md §6.1): job threads run inside `jobs::caught` (a panic is a `crash`
  of that job or part) and hold workers through `pool::Lease` (dropped without `give`, the
  worker is replaced); locks recover from poisoning; `/status` lists the 20 newest jobs with
  `kind`; task create/update/bind run under `Service::task_writes`; `POST /task/<id>` takes
  `parts`/`checks` (binding) and reports load errors of part and check files as the gate
  error; `/save` needs the project folder to be the Git top level and commits with the
  pathspec of the add. The cache key hashes the modules the engine resolved in the last
  build (`modules_read` in the cache `.json`, `snapshot::Reads`), the literal scan only
  before the first build (AG-HD-06). Declaration results are cached per file **including
  load errors**, so a file that hits the discovery limits is not evaluated on every poll.
  `/status` reports `load_errors: [{kind, message, file, line?, column?}]` (kind `load`,
  `time_limit`, `memory_limit`, `crash`) and the first one also as `error`; a build of all
  parts builds what loads and then fails with those errors.
- `src/worker.rs`: `qoob3d worker` (AG-PJ-05): request/answer types of the JSON-line protocol
  (answers start with `{"job":`, foreign stdout lines from OCCT are skipped), builds one part and
  writes its cache entry, writes STEP, declares a file (`declaration`, `PartMode::Declare` with
  the AG-HD-02 limits). Exits when stdin closes. `worker/probe.rs`: `Probe` requests that write
  nothing: one check, a throwaway script, measures of one or two parts, declarations.
- `src/service_cli.rs`: CLI clients against the service of the working directory (AG-SV-02),
  `check` (exit 1 if a check failed) and `task …` as role human.
- `src/mcp.rs` + `src/mcp/`: `qoob3d mcp [DIR]` (AG-MC-01..06): JSON-RPC 2.0 over stdio, one
  line per message, synchronous; **stdout carries only protocol messages** (never `println!`
  there, logs go to stderr). `tools.rs` tool table (schemas) and dispatch: service tools hand
  the service envelope through (service started on demand, `task_update` always as role
  agent, also binds `parts`/`checks`; `task_get` adds `project` from `/status`; `render` adds
  the `bbox` from the build results); `files.rs` confined file tools (AG-MC-03: allowed folders and extensions, 1 MiB, any
  symlink on the path refused, task files read-only); `render.rs` runs `render-project` as a
  child process and returns the PNG as base64 (`views`: 2 to 6 views as one contact sheet;
  the view table there must match `guide.md`); `resources.rs` embeds `docs/script-api.md`,
  `mcp/guide.md` (at most 120 lines; its minimal project is built by `tests/mcp_guide.rs`)
  and the kernel example scripts.
- `src/init.rs`: `qoob3d init <DIR>` (AG-SV-08); writes `lib/examples/` from the master
  copies in `docs/script-api/lib/examples/`. `src/project_render.rs`: `qoob3d
  render-project` (AG-RD-01): build through the service, load meshes, render orthographic;
  `--views a,b` gives a contact sheet (`project_render/sheet.rs`, built-in 5x7 label font).
- `src/client.rs`: `ProjectDir` and the minimal std::net HTTP `Client` of the service (start via
  `QOOB3D_SERVICE_CMD`; port and token from `service.json`, read again when the service does not
  answer; `segment` percent-encodes names in paths). **Every client of the service goes through
  `client::Client`**, so it sends the token. Used by the app, the CLI, MCP and
  `render-project`.
- `src/live.rs`: script project shown live (W1, docs/requirements/project-app.md): `Poller`
  thread (retries a failed first connection every interval), view model `Live`.
- `src/app.rs`: `QoobApp` (eframe::App): open project (`--project`, folder dialog, Cmd/Ctrl+O),
  live meshes into the viewport, error dialog, status line, the demo body for tests and the
  smoke test.
- `src/panels.rs`: toolbar (open, settings), browser (project parts with state and eye), empty
  state (WA-OP-04). Returns `UiAction`s, `app.rs` executes them.
- `src/viewport.rs`: viewport widget, camera animation, overlays, navigation bar.
- `src/viewcube.rs`: ViewCube (pure geometry + drawing), with unit tests.
- `src/navigation.rs`: mouse/trackpad → `Camera`, orbit pivot via raycast (ADR-0008).
- `src/theme.rs`: colors and egui style.
- `src/i18n.rs` + `locales/`: translations (Fluent). `src/settings.rs`: saved settings.
- `src/smoke.rs`: smoke test script and watchdog. Discards real input (otherwise the
  test flickers when someone moves the mouse over the window that pops up).
- `src/headless_render.rs`: renderer without a window → PNG (used by `render-project`).
- `tests/ui.rs`: UI tests of the real app (demo body, empty state), screenshots on request.
- `tests/project.rs` (+ `tests/common/`): live project view against a fake service.
- `tests/service.rs`: service, workers, cache and CLI against the real binary (R2/R3).
- Shared test support in `tests/common/` (included with `#[path]`; every item in a file is used
  by every test that includes it, which keeps dead-code warnings away): `agent.rs` (project,
  processes, HTTP, envelope, `Service`, CLI), `inspect.rs`, `artifacts.rs` (PNG, Git),
  `builds.rs`, `mcp.rs` (MCP stdio client); `mod.rs` is the fake service of `tests/project.rs`.
- `tests/agent.rs`, `tests/mcp.rs`: the agent workflow against the real binary:
  checks, tasks, run, measure, save, init, render-project; the MCP server and the acceptance.
  `tests/mcp_guide.rs`: guide, served reference, contact sheet, `task_get` (R5).
- `tests/perf.rs`: recycling, cache eviction, tessellation settings, bench (P1, AG-PF-03..06).

## Relevant ADRs

ADR-0008, ADR-0013, ADR-0019, ADR-0020, ADR-0021, ADR-0034, ADR-0053, ADR-0054, ADR-0055
