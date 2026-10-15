---
id: ADR-0016
title: Print bed from printer profiles, centered around the origin, drawn in the grid shader
status: superseded
date: 2026-10-13
crates: [qoob3d-core, qoob3d-render, qoob3d-app]
tags: [fdm, viewport, grid]
supersedes: []
superseded_by: ADR-0021
---

## Context

For FDM printing (ADR-0002) it must be visible while designing whether a part fits on the
print bed. Print beds differ per printer (A1 mini 180×180, X1/P1/A1 256×256 …).

## Decision

- `qoob3d_core::printer::{PrinterProfile, PRINTERS}`: built-in profiles with
  manufacturer specifications (bed W×D, max. height). Default: Bambu Lab P1P/P1S.
- The bed is displayed **centered around the origin**. Slicers arrange parts themselves,
  so only the size matters.
- Grid, print bed (surface and border) and X/Y axes are drawn by **one analytic
  full-screen shader** (`fs_grid`): intersection with the plane z = 0, lines smoothed via
  `fwidth`, grid spacing in powers of ten adapted to the zoom, fade-out towards the horizon.
  It writes `frag_depth` and is depth-tested against the bodies.
- The selection happens in the navigation bar. It is not yet persisted (comes with the
  settings).

## Consequences

- The grid is infinite and needs no geometry buffer. The fade-out dampens moiré at grazing angles.
- A warning "part larger than print bed/height" is later a pure bounding-box check.

## Rejected alternatives

- **Grid as line geometry** – finite, aliasing, rebuild when zooming.
- **Bed origin in the corner as in the printer coordinate system** – irrelevant for design and
  confusing.

## References

- `crates/qoob3d-core/src/printer.rs`, `crates/qoob3d-render/src/shaders/scene.wgsl` (`fs_grid`)
- ADR-0002, ADR-0006
