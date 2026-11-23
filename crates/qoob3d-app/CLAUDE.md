# qoob3d-app

Library + binary `qoob3d` (ADR-0013): the app that shows a script project live, the CLI and
`render-project`. The service, its workers, the service client, MCP and `init` live in
`qoob3d-project` (ADR-0036); `main.rs` dispatches to them.

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
  `render-project`, `mcp` (the project ones run code of `qoob3d-project`).
- `src/service_cli.rs`: CLI clients against the service of the working directory (AG-SV-02),
  `check` (exit 1 if a check failed) and `task …` as role human.
- `src/project_render.rs`: `qoob3d render-project` (AG-RD-01): build through the service, load
  meshes, render orthographic;
  `--views a,b` gives a contact sheet (`project_render/sheet.rs`, built-in 5x7 label font).
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
- Integration tests of `qoob3d-project` live here, because they run the real `qoob3d` binary
  (`CARGO_BIN_EXE_qoob3d` exists only in the binary's package).
- `tests/service.rs`: service, workers, cache and CLI against the real binary.
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
