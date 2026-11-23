---
id: ADR-0034
title: Parts are exported as binary STL and 3MF by the service from the cached mesh with our own dependency-free writers, and as STEP by OCCT in a worker
status: accepted
date: 2026-11-23
crates: [qoob3d-core, qoob3d-kernel, qoob3d-project, qoob3d-app]
tags: [export, fdm, file-formats]
supersedes: []
superseded_by: null
---

## Context

The goal is the path to the printer (ADR-0002). Bambu Studio and OrcaSlicer read STL and
prefer 3MF (units, objects). OCCT meshes have duplicate vertices at face boundaries, while
slicers check manifoldness through indices. CAD tools that continue the work need STEP. Builds
run in worker processes and leave a cache entry `<key>.{brep,qmsh,json}` per part (ADR-0055);
the service itself never runs OCCT.

## Decision

- **Entry points:** `POST /export {"parts": [..], "format": "stl"|"3mf"|"step"}`, the CLI
  `qoob3d export PARTS.. --format …` and the MCP tool `export` (ADR-0054). The service builds
  stale parts first, refuses the export if any part failed, and writes one file per part to
  `out/<part>.<format>`.
- **STL and 3MF from the cached mesh**: the service reads the part's `.qmsh`, encodes it with
  `qoob3d_core::export` and writes the file atomically; no worker, no new tessellation. The
  worker tessellated the mesh with the project's `[mesh]` settings, default
  `deflection_mm = 0.01` (well below an FDM nozzle) and `angle_deg = 20`; both are part of the
  cache key. Viewport and export therefore show the same triangles.
- **`qoob3d_core::export`**: `stl` (binary, all objects merged, header "QOOB3D binary STL,
  units mm") and `three_mf` (3MF core specification, unit millimetre, one object per mesh, all
  on the build plate). Vertices are welded first (`TriMesh::welded`, 1e-4 mm), collapsed
  triangles dropped.
- **ZIP without compression** by our own writer with CRC-32 (`zip_store`). The reader
  `unzip_stored` checks local headers **and** the central directory; it exists for tests and
  the reference gate.
- **STEP from the B-Rep**: the service sends a worker a request with the part's cached `.brep`;
  the worker reads it and writes STEP (AP214, one compound) with OCCT's `STEPControl_Writer`
  (`script::export_step`, guarded C++ wrapper, ADR-0004).

## Consequences

- Exports are checked by independent tools: `unzip -t` in kernel and service tests and in
  `cargo xtask models`, Bambu Studio's loader in the reference gate when installed (ADR-0013).
- 3MF files are larger than compressed ones; irrelevant for typical FDM parts (a few MB).
- A finer export than the viewport needs a finer `[mesh]` setting for the whole project; there
  is no separate export tolerance.
- The app itself has no export command; exporting is a project operation for agent and CLI.

## Rejected alternatives

- **`zip` crate** – more dependencies (deflate, time) for stored entries only.
- **OCCT STL writer** – no 3MF, no control over welding and objects.
- **Re-tessellating in a worker for export** – a second mesh that can differ from what was
  checked and shown, and a worker round trip for every export.

## References

- `crates/qoob3d-core/src/export.rs` (+ `export/tests.rs`), `crates/qoob3d-core/src/mesh.rs`
  (`welded`, `WELD_TOLERANCE_MM`), `crates/qoob3d-core/src/project/mod.rs` (`Mesh`)
- `crates/qoob3d-project/src/service.rs` (`export`, `export_mesh`, `export_step`),
  `crates/qoob3d-project/src/worker.rs` (`write_step`), `crates/qoob3d-app/src/service_cli.rs`
- `crates/qoob3d-kernel/src/script.rs` (`export_step`), `crates/qoob3d-kernel/cpp/`
- `docs/requirements/agent.md` (AG-SC-14, AG-PF-05)
- ADR-0002, ADR-0004, ADR-0013, ADR-0054, ADR-0055
