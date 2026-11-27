# QOOB3D (pronounced "cubed")

Parametric 3D CAD in the style of Fusion 360, native desktop app for macOS and Linux.
**Main purpose: designing models for FDM printers (e.g. Bambu Lab)** (ADR-0002).
When in doubt, choose what improves the path "design → slicer → print".
**Agent first** (ADR-0053, ADR-0054): a model is a project folder in Git whose parts are Rhai
scripts; an AI agent works on it over MCP, the human gives tasks and accepts results.

## Orientation – read first

| What | Where |
| --- | --- |
| Big picture: layers, threads, data flow, "where do I find what" | `docs/architecture.md` |
| **All decisions with rationale** (index with status) | `docs/adr/README.md` |
| **Requirements** per area (IDs `AG-…`, `WA-…`, `WS-…`, phases, open decisions) | `docs/requirements/` |
| Theme, colors, layout | `docs/design/theme.md` |
| Mouse, trackpad and key bindings | `docs/design/controls.md` |
| **Agent workflow**: script API, project folder, service, tasks, checks, MCP (IDs `AG-…`) | `docs/requirements/agent.md`, `docs/script-api.md` |
| Live project view in the app (IDs `WA-…`) | `docs/requirements/project-app.md` |
| Workbench: overview of projects, several open, agent sessions, Claude accounts (IDs `WS-…`; S0 done, S1–S3 specified) | `docs/requirements/workspace.md`, ADR-0057 |
| Background: original concept and M0 measurements | `docs/concept/` |
| Rules of a crate | `crates/<crate>/CLAUDE.md` (loaded automatically) |
| **Quality harness for AI agents**: layers, rule → check, voluntary tools | `docs/harness.md` |
| Style register (binding taste decisions) | `docs/design/code-style.md` |

Stack: Rust 1.99 (Edition 2024, `rust-toolchain.toml`) · OpenCascade 7.8.1 via a fork of
`opencascade-sys` 0.3.0 in `third_party/` + own cxx wrappers (ADR-0004) ·
Rhai 1.27 (script language, ADR-0053) · eframe/egui 0.36.2 · wgpu 30 (only via `qoob3d_render::wgpu`) · glam 0.34 · egui_kittest (tests).

## Commands

```sh
cargo run                                   # start the app
cargo xtask check                           # DEFINITION OF "DONE" (fmt, clippy, test, hooks, hygiene, arch, docs, i18n, models, deny, smoke)
cargo xtask check --fast                    # without smoke + deny (runs automatically in the Stop hook,
                                            #   .claude/hooks/check-on-stop.sh)
cargo xtask setup                           # once: activate Git hooks from .githooks/ (ADR-0037)
cargo xtask selftest                        # harness must reject every sabotage in harness/sabotage/
cargo xtask commits main..HEAD              # commit policy: identity, no AI attribution
cargo xtask docs [--fix]                    # docs lint; --fix regenerates the ADR index
cargo xtask i18n                            # translations complete, no hard-coded UI texts
cargo xtask printers <orca>/resources/profiles  # regenerate the printer database (ADR-0021)
cargo xtask models                          # reference project models/: build, checks, 3MF/STL exports (ADR-0013)
cargo xtask screenshot                      # whole UI (egui_kittest) + reference project → target/screenshots/*.png
cargo run -- --smoke-test                   # real app with synthetic input, exit code 0/1
RUST_LOG=debug cargo run                    # with logging
# Script projects (ADR-0053/0054); the service starts on demand, answers are JSON envelopes
cargo run -- init <DIR>                     # new project: qoob3d.toml, parts/ lib/ values/ checks/ tasks/
cargo run -- build [PARTS..]                # in the project folder; also: status, check [NAMES..], values
cargo run -- task list                      # also: show ID, new TITLE [--body T], set ID STATE [--note T] (as the human)
cargo run -- export PARTS.. --format 3mf    # stl | 3mf | step, into out/
cargo run -- render-project <DIR> --out x.png --view iso   # headless picture of the built parts
cargo run -- bench <DIR>                    # build seconds, peak worker memory, triangles per part
cargo run -- mcp [DIR]                      # MCP server over stdio for AI agents
cargo run -- --project <DIR>                # app with the live view of a project
cargo run -- serve <DIR>                    # the project service itself (its workers: `qoob3d worker`)
```

In tool shells, run `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"` first if
needed (rustup is installed keg-only via Homebrew).

## Working rules (binding)

1. **Done means: `cargo xtask check` is green.** Report nothing as finished before that.
   Additionally check visual changes with `cargo xtask screenshot` and look at the PNGs.
2. **Document decisions that are costly to reverse** in an ADR, in the same commit as the code.
   ADRs are topic documents: extend the ADR of the topic, start a new one (copy
   `docs/adr/0000-template.md`, then `cargo xtask docs --fix`) only for a new topic; the history
   is in Git. Reference the decision in code with `ADR-NNNN`.
3. **Bugfix = a failing test first**, then the fix.
4. **Lint exceptions only as `#[expect(lint, reason = "…")]`, `#[ignore = "reason"]`**, their
   number is capped (ADR-0037). Never weaken assertions to get tests green.
5. **Never accept golden files and snapshots without asking the user.**
6. **Crate boundaries** (ADR-0036): `core` without OCCT/GPU/UI; kernel and renderer do not know
   each other; `qoob3d-project` (service, worker, MCP) without GPU/UI.
7. After every milestone: test list for the user, update the status below, adjust
   `docs/architecture.md` if the big picture has changed.
8. Commits only on the user's instruction. **No AI attribution** (`Co-Authored-By: Claude`,
   "Generated with …"), hooks and CI reject it. Author and committer are always the
   identity in `harness/identity.txt` (ADR-0037).
9. **The harness is read-only** (`xtask/`, `harness/`, `.claude/`, `.github/`,
   `.githooks/`, `.cargo/`, `Cargo.toml`, `clippy.toml` …, ADR-0037). Propose changes; the
   human unlocks by starting Claude Code with `QOOB3D_HARNESS_UNLOCK=1`. Do not add gates for
   merely conceivable problems, and no rework according to personal taste: questions of taste
   follow the style register.
10. **Separate roles are optional:** `/deliver` (subagent `test-author` writes the tests,
   `implementer` the code) and `/critic-review` are tools for changes where an independent
   oracle pays off (geometry, file formats, bugfixes). Nothing enforces them.

## Conventions

- **Everything in the repository is English:** code, comments, docs, CLAUDE.md files, harness,
  commit messages. The chat with the user may be German. German only in
  `crates/qoob3d-app/locales/de/` and in tests that assert German translations (ADR-0001).
- **UI texts: English as base language, German as language pack** (ADR-0020). Never literals
  in UI code, always `tr("key")`, keys in all `crates/qoob3d-app/locales/*/main.ftl`.
- Errors: `thiserror` in libraries, `anyhow` only in process code (`qoob3d-app`,
  `qoob3d-project`; ADR-0036). Logging with `tracing`.
- `unsafe_code = "deny"` in the whole workspace. FFI exclusively via cxx.
- Units: mm, +Z up (ADR-0006).

## Pitfalls (workspace-wide)

Crate-specific pitfalls are in the respective `crates/<crate>/CLAUDE.md`.

- **CMake ≥ 4** needs `CMAKE_POLICY_VERSION_MINIMUM=3.5` (`.cargo/config.toml`). Do not remove.
- **OCCT is built once into `target/OCCT`** and skipped while the pinned version is installed
  (ADR-0004). After compiler, flag or profile changes that should reach OCCT, delete
  `target/OCCT` (rebuild ~4 min).
- **eframe 0.36:** the entry point is `App::ui(&mut self, ui, frame)`. For synthetic
  input there is `App::raw_input_hook`.
- **egui deadlock:** no `Response`/`ctx` calls inside `ui.input(|i| …)`. In debug builds egui
  reports the deadlock only after 10 s, which is why `--smoke-test` (watchdog) catches it.
- **C++ exceptions from OCCT** abort the process if they are not caught. Our own
  wrappers use `guarded(...)` (ADR-0004).
- **Color space:** egui expects gamma-encoded texture samples. Render the viewport in sRGB, give
  egui a Unorm view (ADR-0019). Regression test in `crates/qoob3d-app/tests/ui.rs`.
- **References to superseded ADRs** outside `docs/adr/` are reported as errors by the docs lint.
  Always reference the valid ADR.
- **`cargo fmt` wraps lines:** scripts that change code by text replacement often fail
  afterwards. Read the current location first.
- **Language is per thread** (`i18n::set_language`). New threads that translate must set it.
- **`crates/qoob3d-core/src/printer/data.rs` is generated**, do not edit by hand.
- **Glyphs:** not every Unicode character is in the egui fonts, missing ones appear as
  "□". `every_ui_glyph_exists_in_the_fonts` checks all texts. `Fonts::has_glyph` reports
  false negatives, do not use it.
- **Unit tests live in `<module>/tests.rs`** (`#[cfg(test)] mod tests;`), never inline
  (`xtask hygiene`).
- **kittest time:** the default is 0.25 s per step; double-clicks need
  `.with_step_dt(1.0 / 60.0)`. New egui areas (dialogs) are in place only after 1–2 frames;
  run a few steps before clicking.
- **Use independent verifiers:** our own ZIP reader missed a writer bug, `unzip -t`
  found it (ADR-0013). For file formats, always have a third-party tool check as well. If Bambu
  Studio is installed (or `QOOB3D_BAMBU_STUDIO` is set), `cargo xtask models` loads every 3MF
  with `BambuStudio --info` (objects, manifold, volume).

## Milestones

| #  | Content | Status |
| --- | --- | --- |
| M1 | Build OCCT, cube → triangles → wgpu window, rotatable with the mouse | ✅ done |
| M1.5 | Quality harness: `xtask check`, `--smoke-test`, `MeshReport`, cargo-deny, Stop hook, ADR structure | ✅ done |
| M2 | Viewport complete: orbit around cursor pivot, grid with print bed, axes, ViewCube, edges, perspective/ortho, own mesher. App frame in the theme. `egui_kittest` | ✅ done |
| M3a, M4 | Interactive modelling (`.qbd` documents, sketches, body features, timeline, undo) | ✅ done, removed in the 2026-11 cleanup: models are script projects |
| H1 | AI harness: Git hooks, CI, commit policy, style register, strict lints, sabotage self-test. `docs/harness.md` | ✅ done, made lean in the 2026-11 cleanup |
| R1 | Script kernel: sandboxed Rhai with a CAD API on OCCT (primitives, batched booleans with plausibility rule, profiles, pipes, fillet/chamfer, shell, measures, STEP). ADR-0053, `docs/script-api.md` | ✅ done |
| R2/R3 | Project folder, values with sources, build graph, cache; worker processes with time and memory limits; service (HTTP) and CLI | ✅ done |
| W1 | App shows a script project live (`--project`, folder dialog), indexed mesh format | ✅ done |
| R4 | Task inbox with roles and review gate, checks, MCP server with confined file tools and render, `init`, `save`. ADR-0054 | ✅ done |
| P1 | Performance and memory: parallel booleans, profile algebra, worker recycling, cache eviction, tessellation settings, `bench` | ✅ done |
| R5 | Findings of the first agent run and of the critic: task binding, guide, API helpers, service hardening | implemented, acceptance run open |
| C1 | Cleanup 2026-11: lean harness, old interactive path removed, topic ADRs | ✅ done (PR #7) |
| C2 | Own crate `qoob3d-project` for service, worker, client, MCP and `init` | in progress (branch `project-crate`) |
| S0 | Workbench service prerequisites WS-PRE-01..06: digest memo outside the lock, kernel fingerprint, `busy`, `summary.json`, idle shutdown, machine-wide worker budget (ADR-0055) | ✅ done |

## Open items

- **Typed service protocol** instead of indexing `serde_json::Value` on both sides: agreed as
  a separate PR after the cleanup.
- **App side of the agent workflow:** tasks and annotations in the app, mouse tools that write
  script lines (concept 7a), photo measurement (concept 6a), print analysis.
- **OS sandbox for the harness** (agreed 2026-10-15): the guard only blocks the edit tools on
  harness paths; write-protect `xtask/`, `harness/`, `.claude/`, `.github/`, `.githooks/`
  with Claude Code's sandbox (Seatbelt/bubblewrap) in an own ADR. Open: how the human unlock
  maps to it, whether cargo/OCCT build/git hooks work inside the sandbox.
- **Set up the GitHub wall** (ADR-0037, `docs/harness.md`): required checks `check (macOS)` and
  `policy` in the ruleset for `main`, CODEOWNERS, separate identity for agents without merge
  rights. Until then, CI is only as strong as the rights the agent pushes with.
- **Harness expansion:** deterministic checks for accessibility (AccessKit names,
  contrast, target sizes), fuzzing of the file formats (qmesh, task files).
- **License:** OCCT is LGPL-2.1 and is linked statically. The printer data comes from
  OrcaSlicer (AGPL-3.0); only dimensions/coordinates are taken over (ADR-0021). Check both
  before distributing binaries.
- **Linux build** is not tested yet. Requires CMake, a C++ compiler, Vulkan and the
  X11/Wayland/xkbcommon dev packages.
