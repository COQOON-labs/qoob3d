---
id: ADR-0034
title: Export as binary STL and 3MF with our own dependency-free writers
status: accepted
date: 2026-10-14
crates: [qoob3d-core, qoob3d-kernel, qoob3d-app]
tags: [export, fdm]
supersedes: []
superseded_by: null
---

## Context

The goal is the path to the printer (ADR-0002). Bambu Studio and OrcaSlicer read STL and
prefer 3MF (units, multiple objects). Kernel meshes have duplicate vertices at
face boundaries; slicers, however, check manifoldness via indices.

## Decision

- For export, the kernel tessellates more finely: `EXPORT_TESSELLATION_TOLERANCE_MM` = 0.01 mm
  (viewport 0.05 mm), request `Request::Export`.
- `qoob3d_core::export`: `stl` (binary, all bodies in one mesh) and `three_mf`
  (3MF core specification, unit millimeter, one object per body, all on the plate).
  Vertices are welded beforehand (`TriMesh::welded`), degenerate triangles removed.
- ZIP without compression with our own writer and CRC-32 (no new dependency, zero license and
  supply-chain effort). The reader `unzip_stored` checks local headers **and** the
  central directory.
- In the app: toolbar EXPORT → STL / 3MF, file dialog, hint in the status bar.
  In the CLI: `--export`.

## Consequences

- 3MF files are larger than compressed ones, irrelevant for typical FDM parts (a few MB).
- STEP follows later via OCCT (kernel), not via these writers.

## Rejected alternatives

- **`zip` crate** – additional dependencies (deflate, time), unnecessary for stored entries.
- **OCCT STL writer** – no 3MF, no control over welding and objects.

## References

- Code: `crates/qoob3d-core/src/export.rs`, `crates/qoob3d-app/src/{app,batch}.rs`
- Related: ADR-0002, ADR-0033
