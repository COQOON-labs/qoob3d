---
id: ADR-0032
title: Gear and coil are parametric features; their geometry is generated in pure Rust (core::generate)
status: accepted
date: 2026-10-14
crates: [qoob3d-core, qoob3d-kernel]
tags: [features, geometry]
supersedes: []
superseded_by: null
---

## Context

Gears and springs/spirals are typical FDM parts. In Fusion they are created via a
script (Spur Gear) or the Coil primitive. Drawn by hand (involutes without splines,
helix without 3D sketches) they could not be made cleanly with our sketches, and they should
remain editable later (number of teeth, turns).

## Decision

- `SpurGear` (plane, module, number of teeth, pressure angle, thickness, bore) and `Coil` (plane,
  type helix/flat spiral, diameter, turns, pitch, wire diameter) are
  features of their own with an operation (ADR-0031).
- `qoob3d_core::generate` computes the geometry without OCCT:
  - **Involute spur gear** with standard dimensions (addendum m, dedendum 1.25 m). The outline
    consists of pieces (flanks and tip/root arcs as splines, radial lines below the
    base circle), chained exactly.
  - **Coil path**: helix (pitch per turn upwards) or Archimedean spiral in the
    plane, 36 points per turn; `coil_length` returns the exact length for checks.
- From these, the kernel builds bodies generically: `qoob3d_outline_prism` (outline of line and
  spline pieces, holes, extrusion) and `qoob3d_sweep_circle` (spline through the path,
  round cross-section, Frenet for the helix, fixed binormal for the flat spiral).

## Consequences

- The mathematics can be tested with unit and property tests (closed outline for all
  valid gears, tooth thickness at the pitch circle, path length).
- Further generated parts (threads, racks) follow the same pattern.

## Rejected alternatives

- **Gear as a sketch generator** (writing curves into a sketch) – the parameters would be
  lost afterwards; without splines in sketches, only possible as a polygon.
- **Polygonal flanks** – every edge would be visible in the viewport, the faces faceted.

## References

- Code: `crates/qoob3d-core/src/generate.rs`, `crates/qoob3d-kernel/src/model.rs`
- Related: ADR-0031
