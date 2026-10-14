# qoob3d-sketch

Sketch logic without UI, GPU and kernel (ADR-0036): tools, snapping, hit testing,
editing, from M3b the solver. Requirements: `docs/requirements/sketches.md` (IDs `SK-…`).

## Rules

- **Pure:** only `qoob3d-core` and math crates. No egui, wgpu, eframe, OCCT, not even
  transitively (`cargo xtask arch`).
- Everything in **sketch coordinates** (mm, `f64`, local system of the plane). This crate does
  not know pixels: tolerances come in as sketch units.
- Points are entities (ADR-0024). Snapping to a point returns its ID, tools
  reuse it instead of creating new points.
- Tools are state machines (`tools::Tool`). New tools get tests for the flow,
  Escape, value input and degenerate input.
- Texts (field names) are only keys (`field-length` …). Translation happens in the app (ADR-0020).

## Files

- `src/tools.rs`: line/polyline, rectangle (2 points, center), circle, arc (center → start →
  end, fields one after another), point.
- `src/snap.rs`: snapping and alignment with priority.
- `src/pick.rs`: hit testing, window and crossing selection.
- `src/edit.rs`: dragging, merging points, toggling construction.
- `src/geom.rs`: intersections (also arcs), nearest points, circle/arc polyline.
- Arcs: the end point entity must lie on the circle (`Sketch::repair_arcs` after
  dragging), otherwise `validate` rejects the file.

## Relevant ADRs

ADR-0024, ADR-0025, ADR-0036, ADR-0027
