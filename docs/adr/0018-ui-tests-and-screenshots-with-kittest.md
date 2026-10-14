---
id: ADR-0018
title: UI tests and screenshots of the real app headless with egui_kittest and wgpu
status: accepted
date: 2026-10-13
crates: [qoob3d-app, xtask]
tags: [quality, tests, ai, ui]
supersedes: []
superseded_by: null
---

## Context

ADR-0013 gives the agent eyes for the 3D scene with `--render`, but not for the UI
(panels, ViewCube, navigation bar). A color space bug in the display of the
viewport texture in egui (ADR-0019) was only visible in a UI screenshot.

## Decision

- `qoob3d-app` is a **library + thin binary**, so that integration tests can create `QoobApp`
  directly.
- `crates/qoob3d-app/tests/ui.rs` starts the **real app** with
  `Harness::builder().wgpu().build_eframe(...)` (egui_kittest with the features
  `eframe`, `wgpu`, `snapshot`). This way kernel thread, renderer and native texture run
  as in the program.
- Tests work via accessibility labels (`get_by_label(...).click()`) and
  app state (`state().viewport()`).
- **Pixel tests** check properties instead of golden images (e.g. brightness of the top face,
  regression for ADR-0019). Golden images come only once the looks are stable (ADR-0013:
  never accept without asking).
- `cargo xtask screenshot [directory]` writes `ui.png` (whole app) and scene renders
  (`iso`, `front`, `top`) for visual inspection by human and agent.

## Consequences

- UI changes are verifiable without a window and without screen recording permission.
- The tests need a GPU (available locally). CI will later need a software rasterizer.

## Rejected alternatives

- **Screenshots of the real window** – permissions, not deterministic, no access to the app state.

## References

- `crates/qoob3d-app/tests/ui.rs`, `crates/qoob3d-app/src/lib.rs`, `xtask/src/main.rs`
- ADR-0013, ADR-0019
