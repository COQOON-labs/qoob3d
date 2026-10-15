---
id: ADR-0026
title: Cargo workspace with five crates – sketch logic in a separate, pure crate qoob3d-sketch
status: superseded
date: 2026-10-13
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-sketch, qoob3d-app, xtask]
tags: [architecture]
supersedes: [ADR-0003]
superseded_by: ADR-0036
---

## Context

ADR-0003 defined four crates. Sketches need tool state machines, snapping,
hit tests and later the solver. This logic should be testable without egui, GPU and OCCT
(SK-QA-01, E-07). In the app crate it would grow entangled with the UI.

## Decision

| Crate | Responsibility | may depend on |
| --- | --- | --- |
| `qoob3d-core` | Document model (incl. sketch data), expressions, mesh types, printers | – (no OCCT, GPU or UI crates, not even transitively) |
| `qoob3d-sketch` | Sketch logic: tools (state machines), snapping, hit testing, editing, later the solver | `core` |
| `qoob3d-kernel` | OCCT binding, worker thread, profiles, features, export | `core` |
| `qoob3d-render` | wgpu viewport, camera | `core` |
| `qoob3d-app` | eframe binary, UI, glue | `core`, `sketch`, `kernel`, `render` |
| `xtask` | Developer tools | – |

- Kernel, renderer and `sketch` do not know each other. Data flows through `core`.
- `qoob3d-sketch` must not pull in OCCT, wgpu, egui or eframe, not even transitively
  (checked by `cargo xtask arch`).
- Further crates or edges need a new ADR that supersedes this one.

## Consequences

- Sketch tools can be tested with pure unit and property tests.
- The app only translates input (pixels → sketch coordinates) and draws.

## Rejected alternatives

- **Sketch logic in the app crate** – only testable through UI tests, gets mixed up with egui.
- **Sketch logic in core** – core should remain the data model. The solver brings heavy mathematics with it.

## References

- `Cargo.toml`, `xtask/src/arch.rs`
- ADR-0003 (superseded)
