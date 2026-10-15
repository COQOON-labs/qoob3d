---
id: ADR-0003
title: Cargo workspace with four crates and fixed dependency directions
status: superseded
date: 2026-10-13
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-app]
tags: [architecture]
supersedes: []
superseded_by: ADR-0026
---

## Context

Document model, geometry kernel and rendering have very different dependencies
(OCCT with several minutes of build time, wgpu) and testability. Without hard boundaries,
kernel or GPU types seep into the model, and everything depends on everything.

## Decision

| Crate | Responsibility | may depend on |
| --- | --- | --- |
| `qoob3d-core` | Document model, shared pure data types (`TriMesh`, `Aabb`) | – (no OCCT, GPU or UI crates, not even transitively) |
| `qoob3d-kernel` | OCCT binding, worker thread, export | `core` |
| `qoob3d-render` | wgpu viewport, camera | `core` |
| `qoob3d-app` | eframe binary, UI, tools, glue | `core`, `kernel`, `render` |
| `xtask` | Developer tools (checks, docs lint) | – (no product crate) |

**Kernel and renderer do not know each other.** Data between them flows only via types from
`core`, mediated by the app. `cargo xtask check` verifies these rules using
`cargo metadata` – transitively as well (e.g. `core` must never pull in `wgpu` or
`opencascade-sys`).

## Consequences

- `core` compiles in seconds and is testable without GPU and OCCT.
- The renderer is testable without an OCCT build (headless render test).
- New crates or edges in the dependency graph require a new ADR.

## Rejected alternatives

- **A single crate** – build times, no enforced boundaries.
- **Renderer reads kernel types directly** – couples GPU code to OCCT, non-Send types.

## References

- `Cargo.toml`, `xtask/src/arch.rs`
