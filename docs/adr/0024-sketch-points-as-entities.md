---
id: ADR-0024
title: Sketches consist of points as entities of their own and curves that reference points
status: accepted
date: 2026-10-13
crates: [qoob3d-core, qoob3d-sketch]
tags: [sketch, document-model]
supersedes: [ADR-0010]
superseded_by: null
---

## Context

ADR-0010 envisaged basic shapes (line with its own coordinates, rectangle, circle) without a
solver. The sketch requirements (`docs/requirements/sketches.md`, SK-DT-01/02, E-02) show that
coincidence as mere equality of coordinates does not hold up. A constraint solver comes in M3b
(ADR-0025); a later migration would be more expensive than a clean model now. Decided with
the user on 2026-10-13.

## Decision

- A `Sketch` contains **points** (`SketchPoint { id, x, y, construction }`, local
  coordinates in mm, `f64`) and **curves** that reference point IDs:
  `Line { start, end }`, `Circle { center, radius }`. Arcs, splines etc. follow the
  same pattern.
- Two lines that share an end point reference **the same point ID**. Snapping
  to an existing point reuses it instead of creating a new one. This makes
  connectedness a relation and not a matter of tolerance.
- A rectangle is not a type of its own but 4 points + 4 lines.
- IDs (`EntityId`) count up per sketch and are never reused.
- Until the solver (M3a) the coordinates are the truth. After that they are the solver's
  starting value. Constraints and dimensions are added as extra fields with `#[serde(default)]`,
  so old files load without a migration.

## Consequences

- Deleting a point deletes all curves that use it. Deleting a curve leaves
  points that are still in use and removes orphaned points.
- Profile keys (ADR-0011) consist of curve IDs.

## Rejected alternatives

- **Keep the basic shapes (ADR-0010)** – connections would be tolerance comparisons,
  constraints would later need a migration of all files.

## References

- `crates/qoob3d-core/src/document/sketch.rs`, `docs/requirements/sketches.md` (SK-DT-*)
- ADR-0010 (superseded), ADR-0011, ADR-0025
