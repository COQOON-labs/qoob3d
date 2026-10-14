# QOOB3D (pronounced "cubed")

Parametric 3D CAD in the style of Fusion 360, native desktop app for macOS and Linux.
**Main purpose: designing models for FDM printers (e.g. Bambu Lab)** (ADR-0002).
When in doubt, choose what improves the path "design → slicer → print".

## Orientation – read first

| What | Where |
| --- | --- |
| Big picture: layers, threads, data flow, "where do I find what" | `docs/architecture.md` |
| **All decisions with rationale** (index with status) | `docs/adr/README.md` |
| **Requirements** per area (IDs `SK-…` etc., phases, open decisions) | `docs/requirements/` (e.g. `sketches.md`) |
| Theme, colors, layout | `docs/design/theme.md` |
| Mouse, trackpad and key bindings | `docs/design/controls.md` |
| Rules of a crate | `crates/<crate>/CLAUDE.md` (loaded automatically) |
| **Quality harness for AI agents**: layers, roles, rule → check | `docs/harness.md`, `harness/rules.md` |
| Style register (binding taste decisions) | `docs/design/code-style.md` |

Stack: Rust 1.99 (Edition 2024, `rust-toolchain.toml`) · OpenCascade 7.8.1 via
forks of `opencascade-sys` 0.3.0 (ADR-0047) and `opencascade` in `third_party/` + own cxx wrappers (ADR-0014) ·
eframe/egui 0.36.2 · wgpu 30 (only via `qoob3d_render::wgpu`) · glam 0.34 · egui_kittest (tests).

## Commands

```sh
cargo run                                   # start the app
cargo xtask check                           # DEFINITION OF "DONE" (fmt, clippy, test, hooks, hygiene, rules, arch, docs, i18n, models, deny, smoke)
cargo xtask check --fast                    # without smoke + deny (runs automatically in the Stop hook,
                                            #   .claude/hooks/check-on-stop.sh)
cargo xtask setup                           # once: activate Git hooks from .githooks/ (ADR-0037)
cargo xtask selftest                        # harness must reject every sabotage in harness/sabotage/
cargo xtask commits main..HEAD              # commit policy: identity, no AI attribution, tests/code separate
cargo xtask docs [--fix]                    # docs lint; --fix regenerates the ADR index
cargo xtask i18n                            # translations complete, no hard-coded UI texts
cargo xtask printers <orca>/resources/profiles  # regenerate the printer database (ADR-0021)
cargo xtask models [--write]                # build, measure, export reference models (ADR-0033)
cargo run -- models/gear.qbd --report       # JSON: bodies, volume, watertight, errors (exit 1 = not printable)
cargo run -- models/gear.qbd --export x.3mf # STL/3MF headless; with --render x.png as an image
cargo run -- --render /tmp/x.png --view iso # headless scene render (+ --ortho --zoom 4 --printer bambu-lab-h2d --volume)
cargo xtask screenshot                      # whole UI (egui_kittest) + scenes → target/screenshots/*.png
cargo run -- --smoke-test                   # real app with synthetic input, exit code 0/1
RUST_LOG=debug cargo run                    # with logging
```

In tool shells, run `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"` first if
needed (rustup is installed keg-only via Homebrew).

## Working rules (binding)

1. **Done means: `cargo xtask check` is green.** Report nothing as finished before that.
   Additionally check visual changes with `cargo xtask screenshot` and look at the PNGs.
   Before the push: critic review (`/critic-review`, ADR-0041).
2. **Document decisions:** Every new architecture or design decision gets an ADR
   (copy `docs/adr/0000-template.md`, then `cargo xtask docs --fix`), in the same commit as the
   code. Do not rewrite accepted ADRs; supersede them with a new one.
   Reference the decision in code with `ADR-NNNN`.
3. **Bugfix = a failing test first**, then the fix.
4. **Lint exceptions only as `#[expect(lint, reason = "…")]`, `#[ignore = "reason"]`**, their
   number is capped (ADR-0040). Never weaken assertions to get tests green.
5. **Never accept golden files and snapshots without asking the user.**
6. **Crate boundaries** (ADR-0036): `core` without OCCT/GPU/UI; kernel and renderer do not know
   each other.
7. After every milestone: test list for the user, update the status below, adjust
   `docs/architecture.md` if the big picture has changed.
8. Commits only on the user's instruction. **No AI attribution** (`Co-Authored-By: Claude`,
   "Generated with …"), hooks and CI reject it (ADR-0038). Author and committer are always the
   identity in `harness/identity.txt` (ADR-0044). Tests and production code in separate
   commits (exception with trailer `Mixed-Change: <reason>`).
9. **A new feature kind** (or curve kind) is only done with: data model + `validate` +
   format version/migration/fixture (ADR-0030), kernel test with **analytic** volume
   (`crates/qoob3d-kernel/tests/features.rs`), example in `document::examples` +
   `models/<name>.expect.json` with formula (ADR-0033), dialog in `commands.rs`, E2E test in
   `crates/qoob3d-app/tests/modeling.rs`, translations, docs. `cargo xtask models` reports
   missing coverage.
10. **The harness is read-only** (`xtask/`, `harness/`, `.claude/`, `.github/`,
   `.githooks/`, `Cargo.toml`, `clippy.toml` …, ADR-0037). Propose changes; the human
   unlocks with `QOOB3D_HARNESS_UNLOCK=1`. The unlock lifts only the harness-file protection;
   fixtures, roles and bypass rules stay, and agents never edit settings or shell startup
   files that could set the variable (ADR-0043). Questions of taste follow the style register,
   no rework according to personal taste (ADR-0040).
11. **Code is produced via `/deliver`:** the subagent `test-author` writes tests, the subagent
   `implementer` writes production code, never the same context (ADR-0039). The guard hook
   enforces this.

## Conventions

- **Everything in the repository is English:** code, comments, docs, CLAUDE.md files, harness,
  commit messages. The chat with the user may be German. German only in
  `crates/qoob3d-app/locales/de/` and in tests that assert German translations (ADR-0045).
- **UI texts: English as base language, German as language pack** (ADR-0020). Never literals
  in UI code, always `tr("key")`, keys in all `crates/qoob3d-app/locales/*/main.ftl`.
- Errors: `thiserror` in libraries, `anyhow` only in `qoob3d-app`. Logging with `tracing`.
- `unsafe_code = "deny"` in the whole workspace. FFI exclusively via cxx.
- Units: mm, +Z up (ADR-0006).

## Pitfalls (workspace-wide)

Crate-specific pitfalls are in the respective `crates/<crate>/CLAUDE.md`.

- **CMake ≥ 4** needs `CMAKE_POLICY_VERSION_MINIMUM=3.5` (`.cargo/config.toml`). Do not remove.
- **Changing `[profile.dev.package."*"]`** triggers a complete OCCT rebuild (~4 min).
- **eframe 0.36:** the entry point is `App::ui(&mut self, ui, frame)`. For synthetic
  input there is `App::raw_input_hook`.
- **egui deadlock:** no `Response`/`ctx` calls inside `ui.input(|i| …)`. In debug builds egui
  reports the deadlock only after 10 s, which is why `--smoke-test` (watchdog) catches it.
- **C++ exceptions from OCCT** abort the process if they are not caught. Our own
  wrappers use `guarded(...)` (ADR-0004, ADR-0014).
- **Color space:** egui expects gamma-encoded texture samples. Render the viewport in sRGB, give
  egui a Unorm view (ADR-0019). Regression test in `crates/qoob3d-app/tests/ui.rs`.
- **References to superseded ADRs** outside `docs/adr/` are reported as errors by the docs lint.
  Always reference the valid ADR.
- **`cargo fmt` wraps lines:** scripts that change code by text replacement often fail
  afterwards. Read the current location first.
- **Language is per thread** (`i18n::set_language`). New threads that translate must set it.
- **`crates/qoob3d-core/src/printer/data.rs` is generated**, do not edit by hand.
- **egui tab focus:** Tab and arrow keys move the keyboard focus in egui before our
  code runs. As long as the value HUD needs keys, `app.rs` resets the focus direction
  (`hud_has_keyboard`).
- **Glyphs:** not every Unicode character is in the egui fonts, missing ones appear as
  "□". `every_ui_glyph_exists_in_the_fonts` checks all texts. `Fonts::has_glyph` reports
  false negatives, do not use it.
- **Unit tests live in `<module>/tests.rs`** (`#[cfg(test)] mod tests;`), never inline
  (ADR-0039, `xtask hygiene`).
- **The guard blocks shell commands** that name a harness path of this repository (token
  start, `./`, repo root) and could write (`>`, `sed -i`, `cp` …; `2>/dev/null`, `>/dev/null`,
  `&>/dev/null` and `2>&1` do not count); also any command naming
  `QOOB3D_HARNESS_UNLOCK`/`QOOB3D_SKIP_REVIEW`. Then run the command with a different target,
  or edit with Edit/Write.
- **Never change fixtures** under `crates/qoob3d-core/tests/fixtures/`. New format version →
  new fixture + migration (ADR-0030).
- **kittest time:** the default is 0.25 s per step; double-clicks need
  `.with_step_dt(1.0 / 60.0)` (see `tests/modeling.rs`). New egui areas (dialogs) are in
  place only after 1–2 frames; run a few steps before clicking.
- **Use independent verifiers:** our own ZIP reader missed a writer bug, `unzip -t`
  found it (ADR-0033). For file formats, always have a third-party tool check as well. If Bambu
  Studio is installed (or `QOOB3D_BAMBU_STUDIO` is set), `cargo xtask models` loads every 3MF
  with `BambuStudio --info` (objects, manifold, volume); so does the E2E export test.

## Milestones

| #  | Content | Status |
| --- | --- | --- |
| M1 | Build OCCT, cube → triangles → wgpu window, rotatable with the mouse | ✅ done |
| M1.5 | Quality harness: `xtask check`, `--render`, `--smoke-test`, `MeshReport`, cargo-deny, Stop hook, ADR structure | ✅ done |
| M2 | Viewport complete: orbit around cursor pivot, grid with print bed, axes, ViewCube, edges, perspective/ortho, own mesher. App frame (toolbar, tree, timeline) in the theme. `egui_kittest` | ✅ done |
| M3a | Document model + `.qbd` (validation, migrations, fixture), sketches on XY/XZ/YZ, line/polyline, rectangle (2 points, center), circle, point, construction, value HUD with expressions, snapping, select/drag/delete, profiles via General Fuse, undo/redo, browser/timeline. Order ADR-0025, status per requirement: `docs/requirements/sketches.md` §18 | ✅ done |
| H1 | AI harness: guard, roles (test-author/implementer/critic/verifier), Git hooks, CI, commit policy, rule and style register, strict lints, sabotage self-test. ADR-0037…0042, `docs/harness.md` | ✅ done (GitHub settings open) |
| M4 | Body features: extrude, revolve, loft (also to a tip), offset plane, spur gear, coil/spring; new body/join/cut; dialogs with live preview; arcs in sketches; rebuild cache (hash chain); export STL/3MF; CLI `--report/--export`; reference model gate. ADR-0031…0036, status: `docs/requirements/bodies.md` | ✅ done |
| M3b | Solver (spike E-01 first), constraints, dimensions, parameters, arc, slot, polygon, trim, fillet, offset, mirror | open |
| M3c | Construction planes (also inclined), sketches on faces, project, slice | open |
| M5 | Editable timeline (parameters, recompute from a feature, suppress, rollback, error states) | open |
| M6 | STEP export, menu, recently opened files, body selection, chamfers/fillets | open |

## Open items

- **Set up the GitHub wall** (ADR-0037, `docs/harness.md`): ruleset for `main` with
  required checks, CODEOWNERS, separate identity for agents without merge rights. Until then,
  CI is only as strong as the rights the agent pushes with.
- **Harness expansion:** deterministic checks for accessibility (AccessKit names,
  contrast, target sizes) and usability (interaction budgets of the core flows), fuzzing of the
  file formats, threshold for the mutation score after the first baseline.

- **License:** OCCT is LGPL-2.1 and is linked statically. The printer data comes from
  OrcaSlicer (AGPL-3.0); only dimensions/coordinates are taken over (ADR-0021). Check both
  before distributing binaries.
- **Selection box in model mode** (ADR-0022): in sketch mode, left drag belongs to
  drawing and selecting (ADR-0027). In model mode it still orbits; feature dialogs
  select by click. Re-evaluate with body/face selection (M6).
- **Join/cut participants** are always all touched bodies (ADR-0031); an explicit selection
  is still missing.
- **Linux build** is not tested yet. Requires CMake, a C++ compiler, Vulkan and the
  X11/Wayland/xkbcommon dev packages.
- **Process isolation of the kernel** (crash protection, untrusted STEP import), at the latest
  with import.
