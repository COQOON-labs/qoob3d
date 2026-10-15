---
id: ADR-0028
title: Profiles are created in the kernel by splitting an auxiliary face with all sketch curves
status: accepted
date: 2026-10-13
crates: [qoob3d-kernel, qoob3d-core]
tags: [kernel, sketch, profiles]
supersedes: []
superseded_by: null
---

## Context

ADR-0011 specifies that the kernel computes the regions and that profiles are referenced by the
set of bounding curve IDs. The method was still open. It has to handle
intersections and islands (SK-PR-01) and deliver stable keys without guessing.

## Decision

- Computation happens in the sketch's **local 2D system** (plane z = 0). The mapping into
  space is done by Rust or, later, by the extrusion feature (`gp_Ax3`).
- Input: all non-construction curves (lines, circles) with their IDs. Rust filters out
  degenerate curves (length/radius ≈ 0) beforehand.
- Method (C++, `qoob3d_sketch_regions`, exception-safe per ADR-0014):
  1. Create an auxiliary face (rectangle around all curves, with a margin).
  2. `BOPAlgo_Builder` (General Fuse) with the face and all curve edges → face split into parts.
  3. Via the history (`Modified`/`IsSame`), assign each partial edge its curve ID.
     Edges of the auxiliary face mark the outside.
  4. Discard partial faces that touch an edge of the auxiliary face. Every remaining partial
     face is a profile.
  5. Per profile: key = sorted set of curve IDs, area (mm²) and a
     triangulation (2D) for display and hit testing.
- Profiles are recomputed on every rebuild (asynchronously, ADR-0005).

## Consequences

- Islands arise automatically: a circle inside a rectangle yields a ring (key
  {4 lines, circle}) and a disc ({circle}).
- Open contours split nothing and create no profile (SK-PR-04 shows the open ends).

## Rejected alternatives

- **Our own 2D arrangement computation in Rust** – circle intersections, tolerances and
  special cases are error-prone. OCCT solves exactly this robustly.
- **Faces directly from closed wires** – fails on self-intersecting shapes.

## References

- `crates/qoob3d-kernel/cpp/qoob3d_occt.cpp` (`qoob3d_sketch_regions`), `crates/qoob3d-kernel/src/profiles.rs`
- ADR-0011, ADR-0014
