# qoob3d-app

Library + binary `qoob3d` (ADR-0018). This is where UI, tools, navigation and the
kernel connection come together.

## Rules

- **Avoid the egui deadlock:** inside `ui.input(|i| …)`/`ctx.input(…)` **no**
  `Response` methods and no other `ctx` accesses. Fetch the state into local
  variables beforehand.
- Always check kernel responses against the expected `RequestId` and discard stale ones (ADR-0005).
- Navigation exactly as in `docs/design/controls.md` (ADR-0008). When changing, adjust both.
- egui draws the overlays (ViewCube, navigation bar, pivot), wgpu the scene (ADR-0015).
- **No visible texts in code** (ADR-0020): always `tr("key")` / `tr_args(…)`.
  Create new keys in **all** `locales/*/main.ftl` (English = base). `cargo xtask i18n`
  checks this. Numbers via `i18n::decimal`.
- Colors from `theme.rs` / `docs/design/theme.md`. Functions of later milestones appear
  disabled with a hint (`panels::planned`).
- Cursor rule (docs/design/controls.md): arrow everywhere, pointing hand only on the ViewCube,
  grabbing hand while dragging. Labels are not selectable.
- Persistent settings belong in `Settings` (serde, `#[serde(default)]`).
- Every new interaction needs: a UI test in `tests/ui.rs` (egui_kittest) and, if it
  affects the camera, a step in `--smoke-test`.
- Textures displayed by egui must deliver gamma-encoded values (ADR-0019).
- **Change the document only via `Project`** (`edit`, `replace_sketch`, `preview_sketch` +
  `commit_preview`). Every change is an undo step with a translated label; a
  sketch session is merged into one step when it ends (ADR-0029).
- Sketch logic belongs in `qoob3d-sketch`. `sketch_ui.rs` only translates pixels ↔ plane, keys
  and drawing (ADR-0027). Tolerances in pixels, converted via `View::units_per_px`.
- Tool shortcuts (L/R/C/A/X) never fire while the HUD is accepting a value.
- Feature dialogs (`commands.rs`, ADR-0035) change the document only via
  `Project::preview_doc` / `commit_preview` / `cancel_preview`. There is no separate preview
  geometry: the preview is the normal rebuild.
- Translation keys always as literals in code (e.g. `CommandKind::keys`), never assembled via
  `format!` – otherwise `cargo xtask i18n` does not find them.

## Files

- `src/main.rs`: entry point, CLI switch. `src/lib.rs`: modules for tests.
- `src/cli.rs`: arguments (file, `--project`, `--demo`, `--render`, `--view`, `--size`, `--ortho`,
  `--zoom`, `--printer`, `--volume`, `--smoke-test`) and the subcommands `serve`, `worker`,
  `build`, `status`, `export`, `values`, `bench`, `check`, `task list|show|new|set`, `init`,
  `render-project`, `mcp`.
- `src/service.rs` + `src/service/`: `qoob3d serve` (AG-SV-01, ADR-0053): routes and envelope,
  single instance via `.qoob3d/service.lock`; `http.rs` minimal server side HTTP; `jobs.rs` job
  queue and parallel part scheduling, cache hits; `pool.rs` warm workers, time/memory/crash
  limits, replacement, recycling above `[workers] recycle_mb` (AG-PF-03); `snapshot.rs` part
  discovery (`PartMode::Declare`, no OCCT), keys, states, cache eviction (`evict`, AG-PF-04);
  `bench.rs` `qoob3d bench <DIR>` (AG-PF-06): a fresh worker per build, entries in
  `.qoob3d/bench-<pid>/` (removed afterwards), never the project cache. The service never runs
  OCCT. The `[mesh]` settings enter the cache key through `worker::key_version` (AG-PF-05).
  R4 (ADR-0054): `jobs.rs` `Work` (build, check, run) so `/check` and `/run` answer with a job
  like `/build`; `checks.rs` check discovery (`checks/*.rhai`, declared without OCCT), `/check`
  (one worker request per check), states passed/failed/stale/never_run (stale = check file or
  the build key of a part the check read changed), `gate_input` for the review gate; `probe.rs`
  `/run` and `/measure`; `save.rs` `/save` (`git` executable, never adds `.qoob3d/`, `out/`);
  `tasks.rs` `/tasks`, `/task/<id>` on `qoob3d_core::tasks` (`role` is required). `/status`
  stays lock-short: check states come from records, never from a run.
- `src/worker.rs`: `qoob3d worker` (AG-PJ-05): request/answer types of the JSON-line protocol
  (answers start with `{"job":`, foreign stdout lines from OCCT are skipped), builds one part and
  writes its cache entry, writes STEP. Exits when stdin closes. `worker/probe.rs`: `Probe`
  requests that write nothing: one check, a throwaway script, measures of one or two parts.
- `src/service_cli.rs`: CLI clients against the service of the working directory (AG-SV-02),
  `check` (exit 1 if a check failed) and `task …` as role human; `segment` percent-encodes ids.
- `src/mcp.rs` + `src/mcp/`: `qoob3d mcp [DIR]` (AG-MC-01..06): JSON-RPC 2.0 over stdio, one
  line per message, synchronous; **stdout carries only protocol messages** (never `println!`
  there, logs go to stderr). `tools.rs` tool table (schemas) and dispatch: service tools hand
  the service envelope through (service started on demand, `task_update` always as role
  agent); `files.rs` confined file tools (AG-MC-03: allowed folders and extensions, 1 MiB, any
  symlink on the path refused, task files read-only); `render.rs` runs `render-project` as a
  child process and returns the PNG as base64; `resources.rs` embeds `docs/script-api.md`,
  `mcp/guide.md` (at most 60 lines) and the kernel example scripts.
- `src/init.rs`: `qoob3d init <DIR>` (AG-SV-08). `src/project_render.rs`: `qoob3d
  render-project` (AG-RD-01): build through the service, load meshes, render orthographic.
- `src/live.rs`: script project shown live (W1, ADR-0053, docs/requirements/project-app.md):
  `ProjectDir`, minimal std::net HTTP `Client` (service start via `QOOB3D_SERVICE_CMD`), `Poller`
  thread (HTTP never on the UI thread), view model `Live`, headless `build_and_load`.
- `src/app.rs`: `QoobApp` (eframe::App), `Mode` (model/plane pick/sketch), dialogs
  (unsaved, error), keyboard shortcuts, kernel polling, status line.
- `src/panels.rs`: toolbar (file, undo/redo, sketch tools, CREATE, CONSTRUCT,
  EXPORT), browser (planes, bodies, sketches), timeline, sketch palette. Returns `UiAction`s,
  `app.rs` executes them.
- `src/commands.rs`: feature dialogs (Extrude, Revolve, Loft, Plane, Gear, Coil), selection
  in the viewport, icons, test helpers for screen positions.
- `src/batch.rs`: `--report` (JSON), `--export`, scene for `--render` (ADR-0033).
- `src/sketch_ui.rs`: sketch mode (`SketchSession`: tools, selection, dragging,
  box selection, HUD), plane pick (`plane_pick`), drawing of inactive sketches.
- `src/project.rs`: open document, undo/redo, rebuild connection, load/save (atomic).
  `src/history.rs`: snapshot history.
- `src/viewport.rs`: viewport widget, camera animation, overlays, navigation bar.
- `src/viewcube.rs`: ViewCube (pure geometry + drawing), with unit tests.
- `src/navigation.rs`: mouse/trackpad → `Camera`, orbit pivot via raycast (ADR-0017).
- `src/theme.rs`: colors and egui style.
- `src/i18n.rs` + `locales/`: translations (Fluent). `src/settings.rs`: saved settings.
- `src/smoke.rs`: smoke test script and watchdog. Discards real input (otherwise the
  test flickers when someone moves the mouse over the window that pops up).
- `src/headless_render.rs`: `--render`, kernel + renderer without a window → PNG.
- `tests/ui.rs`: UI tests of the real app, screenshot on request.
- `tests/project.rs` (+ `tests/common/`): live project view against a fake service.
- `tests/service.rs`: service, workers, cache and CLI against the real binary (R2/R3).
- `tests/agent.rs`, `tests/mcp.rs` (+ `tests/common/agent.rs`): R4 against the real binary:
  checks, tasks, run, measure, save, init, render-project; the MCP server and the acceptance.
- `tests/perf.rs`: recycling, cache eviction, tessellation settings, bench (P1, AG-PF-03..06).
- `tests/modeling.rs`: every shape end to end through the UI, export, editing; screenshots
  of dialog and model (60 Hz steps for double-clicks).

## Relevant ADRs

ADR-0005, ADR-0008, ADR-0013, ADR-0015, ADR-0017, ADR-0018, ADR-0019, ADR-0020, ADR-0021, ADR-0022, ADR-0023, ADR-0027, ADR-0029, ADR-0033, ADR-0034, ADR-0035, ADR-0053, ADR-0054
