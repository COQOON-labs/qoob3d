---
id: ADR-0013
title: A green cargo xtask check defines done – smoke test, headless renders, kittest UI tests and a reference project with analytic expectations, verified by third-party tools
status: accepted
date: 2026-11-23
crates: [xtask, qoob3d-app, qoob3d-core, qoob3d-render]
tags: [quality, tests, ai, ui, fdm]
supersedes: []
superseded_by: null
---

## Context

The coding agent cannot see the running app. In M1 it started the app for 9 s, reported
"running" and missed a deadlock that egui reports only after 10 s. A color space bug in the
viewport (ADR-0019) was visible only in a UI screenshot. Geometry errors are often invisible to
unit tests: a wrong sign, an open face, an export the slicer rejects. On its first run, `unzip
-t` found a bug in our own ZIP writer that our own reader had missed. Quality therefore has to be
checked mechanically, against truths that do not come from the code under test.

This topic absorbs the former decisions 0018 and 0033.

## Decision

### One command defines "done"

`cargo xtask check` runs in order and stops at the first red step: `fmt --check`, `clippy -D
warnings` (all targets), `test`, `hooks`, `hygiene` (ADR-0037), `arch` (ADR-0036), `docs`
(ADR-0001), `i18n` (ADR-0020), `models` (below), then `cargo deny` (advisories, licences with
per-crate copyleft exceptions, crates.io only) and the smoke test. `--fast` leaves out `deny` and
the smoke test; the Claude Code Stop hook runs it. The check sets `QOOB3D_REQUIRE_GPU`, so tests
that need a GPU fail instead of skipping. CI runs the full check on macOS; Linux runs it under
`xvfb-run` with lavapipe, not yet required.

### The agent gets eyes

- `qoob3d --smoke-test` starts the real app with the demo body and feeds synthetic input through
  the regular egui input path (orbit around the cursor pivot, pan, wheel and pinch zoom,
  trackpad pan and orbit, F6 fit, ViewCube top, home). Each step checks its effect on the
  camera; the exit code is 0 or 1. A watchdog aborts after 15 s without a frame (120 s in
  total), so a deadlock fails in seconds. Real input is discarded.
- `qoob3d render-project <DIR> --out x.png [--view …|--views a,b] [--size WxH] [--parts …]`
  renders the built parts of a project headless and orthographic, several views as a labelled
  contact sheet. It is the only headless picture path; the MCP tool `render` runs it
  (ADR-0054).
- Geometry is measured, not counted: `MeshReport` (watertight, open and non-manifold edges,
  signed volume) in core; the script kernel measures the exact B-Rep, and kernel tests compare
  volumes with formulas.

### UI tests and screenshots with egui_kittest

`qoob3d-app` is a library plus a thin binary, so `tests/ui.rs` and `tests/project.rs` build the
real `QoobApp` with `Harness::builder().wgpu().build_eframe(…)`: renderer and native texture run
as in the program. Tests find widgets by accessibility label and read app state
(`state().viewport()`). Pixel tests check properties (e.g. brightness of the top face,
`viewport_is_not_gamma_darkened`), not golden images. `cargo xtask screenshot [dir]` writes
`ui.png`, `printer-list.png`, `empty.png` and `reference.png` (the reference project from iso,
front and top) to `target/screenshots/` for humans and agent to look at.

### The reference project is a gate

`models/` is a script project. Its checks (`models/checks/*.rhai`) compare every part with an
**analytic** formula written next to the expectation (`volume = 4/3 pi r^3`), with tolerance
1e-6. `cargo xtask models` builds the binary, copies the project to `target/models/project`
without cache, starts `qoob3d serve` on the copy and requires through the real CLI:

- every part builds, is valid and its mesh watertight;
- every check passes;
- `qoob3d export` writes 3MF and STL; the 3MF holds exactly one object for our own reader
  **and** for Info-ZIP (`unzip -t`); the binary STL has a valid size;
- if Bambu Studio is installed (or `QOOB3D_BAMBU_STUDIO` is set), `BambuStudio --info` loads
  the 3MF: one object, manifold, volume within 1 % of ours.

## Consequences

- "Done" is verifiable without a window or screen recording permission. The smoke test and the
  UI tests need a GPU and a graphical session (xvfb on Linux).
- Expectations of the reference project, like golden files and snapshots, are never adjusted
  to a result without checking the formula and asking the user.
- Wherever an independent verifier exists for a file format, it is used.

## Rejected alternatives

- **Only manual tests by the user** – too slow; the user finds the bugs.
- **Screenshots of the real window** – needs screen recording permission, not deterministic, no
  access to app state.
- **Snapshot comparison of meshes** – breaks on every tessellation change and says nothing
  about correctness.
- **Kernel tests only** – check neither the file formats nor the CLI nor the service.

## References

- `xtask/src/main.rs` (`check`, `screenshot`), `xtask/src/models.rs`, `models/`
- `crates/qoob3d-app/src/smoke.rs`, `crates/qoob3d-app/src/project_render.rs`,
  `crates/qoob3d-app/src/headless_render.rs`
- `crates/qoob3d-app/tests/ui.rs`, `crates/qoob3d-app/tests/project.rs`
- `crates/qoob3d-core/src/mesh.rs`, `deny.toml`, `.claude/hooks/check-on-stop.sh`,
  `.github/workflows/ci.yml`
- ADR-0001, ADR-0019, ADR-0034, ADR-0036, ADR-0037, ADR-0054
