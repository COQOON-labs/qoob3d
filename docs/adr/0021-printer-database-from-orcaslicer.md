---
id: ADR-0021
title: Printer database generated from OrcaSlicer profiles, with exclusion zones and nozzle areas
status: accepted
date: 2026-10-13
crates: [qoob3d-core, qoob3d-render, qoob3d-app, xtask]
tags: [fdm, printer, viewport, data]
supersedes: [ADR-0016]
superseded_by: null
---

## Context

ADR-0016 introduced five hand-maintained printers with a rectangular bed. The user wants
considerably more models and their particularities. Some printers have areas that must not or
cannot be printed on, such as Bambu X1/P1 (front left corner 18 × 28 mm),
Qidi Q1 Pro (cut-outs at the back) or dual-nozzle printers such as Bambu H2D, where strips are
reachable by only one nozzle. Delta printers have round beds. The slicer profiles of
OrcaSlicer (fork of Bambu Studio/PrusaSlicer) contain exactly this geometry for more than
400 models.

## Decision

- **Generated database** `crates/qoob3d-core/src/printer/data.rs`, produced by
  `cargo xtask printers <OrcaSlicer resources/profiles>`. The origin (commit) is in the
  file header. **Never edit the file by hand.**
- The generator processes `machine_model_list` per manufacturer, picks the 0.4 mm variant
  and resolves `inherits` (including subfolders). Only facts are taken over:
  `printable_area` (polygon), `bed_exclude_area` (even-odd polygon),
  `extruder_printable_area` (per nozzle) and `printable_height`. Pseudo manufacturers
  (`Custom`, `OrcaArena`) stay out. Polygons with more than 192 points in total
  are skipped and reported.
- `PrinterProfile` (core) is purely static. `bed()` returns the `BedGeometry` centered around
  the origin (as before: slicers arrange parts themselves).
- **Display in the grid shader** (`fs_grid`): polygons are stored in a uniform array.
  Per pixel there is an even-odd test (also works for holes and irregular
  shapes) as well as the distance to the edge for pixel-wide borders.
  - Printable area: light with a gray border.
  - **Exclusion zone: hatched yellow** with a yellow border.
  - **Single-nozzle zone: tinted yellow.**
  - Optionally the **build volume** as a wireframe up to the maximum height.
- Selection in the navigation bar with a search field, grouped by manufacturer. Markers:
  ▨ exclusion zone, ◐ single-nozzle zones. The tooltip shows the dimensions.

## Consequences

- New printer models arrive by regenerating from a newer OrcaSlicer version.
- OrcaSlicer is licensed under AGPL-3.0. Only dimensions and coordinates (facts) are taken over,
  no texts or images. This is added to the license review before publication
  (CLAUDE.md, open points).
- A warning "part lies in exclusion zone / exceeds build volume" is later a pure geometry
  check with the same polygons.

## Rejected alternatives

- **Hand-maintained list** – goes stale quickly, special zones get forgotten.
- **Triangulate polygons** – fails on the self-intersecting exclusion zones of some
  profiles (e.g. Qidi). The even-odd test in the shader is robust and exact.

## References

- `xtask/src/printers.rs`, `crates/qoob3d-core/src/printer/`,
  `crates/qoob3d-render/src/shaders/scene.wgsl` (`fs_grid`, `poly_inside`),
  `crates/qoob3d-render/src/renderer.rs` (`BedUniform`, `build_volume_segments`)
- ADR-0002, ADR-0016 (superseded)
