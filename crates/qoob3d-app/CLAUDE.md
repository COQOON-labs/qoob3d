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
- `src/cli.rs`: arguments (file, `--demo`, `--render`, `--view`, `--size`, `--ortho`, `--zoom`,
  `--printer`, `--volume`, `--smoke-test`).
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
- `tests/modeling.rs`: every shape end to end through the UI, export, editing; screenshots
  of dialog and model (60 Hz steps for double-clicks).

## Relevant ADRs

ADR-0005, ADR-0008, ADR-0013, ADR-0015, ADR-0017, ADR-0018, ADR-0019, ADR-0020, ADR-0021, ADR-0022, ADR-0023, ADR-0027, ADR-0029, ADR-0033, ADR-0034, ADR-0035
