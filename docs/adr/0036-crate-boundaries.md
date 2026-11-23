---
id: ADR-0036
title: Six crates with fixed dependency directions – a pure core, the kernel as the only place with OCCT, a renderer without OCCT, a headless project crate, the app on top, xtask on core only – enforced by cargo xtask arch
status: accepted
date: 2026-11-23
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-project, qoob3d-app, xtask]
tags: [architecture, quality]
supersedes: []
superseded_by: null
---

## Context

Geometry kernel, rendering and the shared data have very different dependencies (OCCT with
minutes of build time, wgpu and egui) and testability. Without hard boundaries, kernel or GPU
types seep into the data model and everything depends on everything. OCCT shapes are not
`Send` and must stay where they were made. The quality gates in `xtask` must check the shipped
binary from the outside and still build quickly.

This topic absorbs the former decisions 0003 and 0026.

## Decision

| Crate | Responsibility | may depend on |
| --- | --- | --- |
| `qoob3d-core` | Pure data shared by everyone: `ShapeMesh`/`TriMesh`, `MeshReport`, STL/3MF writers, `qmesh`, project folder (values, build graph, cache key), task inbox, printer database | – (no OCCT, GPU or UI crates, not even transitively) |
| `qoob3d-kernel` | The only place with OCCT: own cxx wrappers on the `opencascade-sys` fork (ADR-0004), the Rhai script API (ADR-0053), tessellation, STEP | `core` (no GPU or UI crates) |
| `qoob3d-render` | wgpu viewport renderer, camera, headless GPU (ADR-0019) | `core` (no OCCT) |
| `qoob3d-project` | Script projects without GPU or UI: the service `qoob3d serve`, the worker process `qoob3d worker`, the service client, the MCP server, `qoob3d init` (ADR-0053–0055) | `core`, `kernel` (no GPU or UI crates) |
| `qoob3d-app` | Binary `qoob3d`: app and UI, CLI and its dispatch to `project`, `render-project` | `core`, `kernel`, `render`, `project` |
| `xtask` | Developer tools and gates | `core` |

- **Kernel and renderer do not know each other.** Meshes flow from the kernel to the renderer
  only as `qoob3d-core` types, through the app (in practice through the worker's cache entry
  and the service's `/mesh`).
- **OCCT types never leave the kernel.** Its OCCT bridge is private; the public API offers an
  opaque `script::Shape`, plain-data errors and core types. OCCT runs only in processes that
  call the kernel: `qoob3d worker` for every project build, and the app for its demo body
  (UI tests and smoke test). The service never runs OCCT (ADR-0055).
- **xtask uses only `qoob3d-core`.** Whatever needs geometry it checks through the built binary
  (`qoob3d serve`, `build`, `check`, `export`, `render-project`), never through a kernel
  dependency.
- `cargo xtask arch` (part of `cargo xtask check`, ADR-0013) checks these rules against `cargo
  metadata`: allowed direct workspace edges, and forbidden packages in the transitive non-dev
  tree (`core`: `opencascade-sys`, `occt-sys`, `wgpu`, `eframe`, `egui`, `egui-wgpu`, `winit`;
  `kernel` and `project`: the GPU/UI ones; `render`: the OCCT ones). Errors print the
  offending path.
- **Errors:** library crates use `thiserror`; `anyhow` with context is for process code, the
  app and `qoob3d-project` (service, worker and MCP server are the main logic of their
  processes). What a caller matches on stays plain data (`worker::ErrorKind`, envelopes).
- A new crate or edge changes this topic in the same commit as `xtask/src/arch.rs`.

## Consequences

- `core` compiles in seconds and is testable without GPU and OCCT (unit and property tests);
  the renderer is testable without an OCCT build; xtask does not link OCCT a second time.
- Gates test the real application from the outside, the way a script or an agent uses it.
- The service side builds and tests without the GPU stack; only the app links wgpu and egui.
  Integration tests of `qoob3d-project` still live in `qoob3d-app/tests/`, because they run the
  real binary (`CARGO_BIN_EXE_qoob3d` exists only in the binary's package).
- The former sketch crate is gone with the interactive path; nothing replaced it.

## Rejected alternatives

- **A single crate** – build times, no enforced boundaries.
- **Renderer reads kernel types directly** – couples GPU code to OCCT and non-`Send` types.
- **xtask depends on the kernel** – OCCT would be linked a second time, and the gate would not
  check the shipped binary.

## References

- `Cargo.toml`, `xtask/src/arch.rs`, `crates/*/Cargo.toml`
- `crates/qoob3d-kernel/src/lib.rs`, `crates/qoob3d-kernel/src/occt.rs`
- `docs/architecture.md` (crate dependencies)
- ADR-0004, ADR-0013, ADR-0019, ADR-0037, ADR-0053, ADR-0055
