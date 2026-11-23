---
id: ADR-0006
title: World coordinates are right-handed with +Z up and the XY plane as print bed; the only unit is the millimetre
status: accepted
date: 2026-11-23
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-app]
tags: [geometry, convention, fdm]
supersedes: []
superseded_by: null
---

## Context

CAD programs disagree: Fusion uses Y up by default (configurable). Slicers (Bambu Studio,
PrusaSlicer, OrcaSlicer), STEP files and 3MF work with Z up and millimetres. QOOB3D designs
parts for FDM printing (ADR-0002), so a part should lie in the slicer exactly as it was
modelled.

## Decision

- World coordinates are right-handed, **+Z up**, and the XY plane at z = 0 is the print bed.
  The viewport grid and the printer bed are drawn in that plane (ADR-0021).
- The only unit is the **millimetre**. Scripts, the kernel and the project logic compute in
  `f64`; `f32` is used only for meshes (`TriMesh`, `ShapeMesh`, the `qmesh` transfer format)
  and on the GPU. `units` in `qoob3d.toml` defaults to `"mm"`, and any other unit is refused
  when the project loads.
- Views follow Fusion's ViewCube: Front looks along +Y (camera at −Y), Right looks along −X
  (camera at +X), Top looks down −Z. Home is the front-right-top corner. The same names
  (`iso`, `front`, `back`, `left`, `right`, `top`, `bottom`) are used by `render-project` and
  the MCP `render` tool.
- Angles in the script API are degrees; only Rhai's own trigonometry (`sin`, `cos`, …)
  takes radians, as the API reference states.

## Consequences

- STL, 3MF and STEP are exported without swapping axes.
- Display units such as inches would be a pure UI conversion; the project format stays mm.
- `f32` meshes lose precision far from the origin (about 0.01 mm at 100 m), which is
  irrelevant at print bed sizes.

## Rejected alternatives

- **Y up like the Fusion default** – does not fit slicers and STEP and forces conversions at
  every boundary.
- **A unit per project** – every value, check and export would carry a conversion; FDM
  printing is metric.

## References

- `crates/qoob3d-render/src/camera.rs` (`ViewPreset`), `crates/qoob3d-core/src/mesh.rs`,
  `crates/qoob3d-core/src/qmesh.rs`, `crates/qoob3d-core/src/project/mod.rs` (`Config::units`)
- `docs/script-api.md` (Conventions)
- Related: ADR-0002, ADR-0008, ADR-0021
