---
id: ADR-0036
title: Crate boundaries – as in ADR-0026, additionally xtask may use the pure qoob3d-core
status: accepted
date: 2026-10-14
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-sketch, qoob3d-app, xtask]
tags: [architecture, quality]
supersedes: [ADR-0026]
superseded_by: null
---

## Context

ADR-0026 allowed `xtask` no workspace dependencies. The reference model gate
(ADR-0033), however, has to write the example models from `qoob3d_core::document::examples`,
check that they cover all feature kinds and read exported 3MF files. That is only possible with
the document model. `qoob3d-core` is pure (no OCCT, no GPU), so xtask still builds
quickly.

## Decision

| Crate | Responsibility | may depend on |
| --- | --- | --- |
| `qoob3d-core` | Document model (incl. sketch data, features), expressions, mesh types, generators (gear, coil), export writers, printers | – (no OCCT, GPU or UI crates, not even transitively) |
| `qoob3d-sketch` | Sketch logic: tools (state machines), snapping, hit testing, editing, later the solver | `core` |
| `qoob3d-kernel` | OCCT binding, worker thread, profiles, features, export meshes | `core` |
| `qoob3d-render` | wgpu viewport, camera | `core` |
| `qoob3d-app` | eframe binary, UI, glue, CLI (`--report`, `--export`, `--render`) | `core`, `sketch`, `kernel`, `render` |
| `xtask` | Developer tools, quality gates | `core` |

- Kernel, renderer and `sketch` do not know each other. Data flows through `core`.
- `qoob3d-core` and `qoob3d-sketch` must not pull in OCCT, wgpu, egui or eframe,
  not even transitively (checked by `cargo xtask arch`).
- Whatever needs geometry, `xtask` checks via the built binary (`qoob3d … --report`), not
  via a dependency on the kernel.
- Further crates or edges need a new ADR that supersedes this one.

## Consequences

- Sketch tools can be tested with pure unit and property tests.
- Gates in xtask test the real application from the outside, the way a script or the AI
  uses it.

## Rejected alternatives

- **xtask depends on the kernel** – OCCT would be linked a second time, and the gate would not
  check the shipped binary.
- **Example models as hand-written JSON files** – drift away from the code; this way they are
  code, tested, and the files are generated from it.

## References

- `Cargo.toml`, `xtask/src/arch.rs`, `xtask/src/models.rs`
- ADR-0026 (superseded), ADR-0033
