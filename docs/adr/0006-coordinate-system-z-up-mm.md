---
id: ADR-0006
title: World coordinates are right-handed with +Z up, unit millimeters
status: accepted
date: 2026-10-13
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render]
tags: [geometry, convention, fdm]
supersedes: []
superseded_by: null
---

## Context

CAD programs are inconsistent: Fusion uses Y up by default, but this can be changed.
Slicers (Bambu Studio, PrusaSlicer) and STEP files work with Z up and millimeters.

## Decision

- Right-handed, **+Z up**, the XY plane is the print bed.
- The internal unit is always **millimeters**: `f64` in the document and kernel, `f32` only
  on the GPU and in the `TriMesh`.
- Views: Front looks in the +Y direction (camera at −Y), Right looks in the −X direction
  (camera at +X). The home view is front-right-top.

## Consequences

- Export without swapping axes: a part lies in the slicer the way it was designed.
- Display units (e.g. inches) would later be a pure UI conversion.

## Rejected alternatives

- **Y up like the Fusion default** – does not fit slicers and STEP and forces conversions.

## References

- `crates/qoob3d-render/src/camera.rs`, `crates/qoob3d-core/src/mesh.rs`
- ADR-0002
