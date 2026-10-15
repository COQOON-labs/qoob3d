---
id: ADR-0010
title: In the MVP, sketches consist of primitives with their own dimensions, without a constraint solver
status: superseded
date: 2026-10-13
crates: [qoob3d-core]
tags: [sketch, document-model]
supersedes: []
superseded_by: ADR-0024
---

## Context

A constraint solver is explicitly not part of the MVP. Nevertheless, dimensions should be
editable afterwards, e.g. "rectangle width = 40".

## Decision

- `SketchEntity` ∈ { `Line {a, b}`, `Rect {corner, width, height}`, `Circle {center, radius}` },
  each with a stable `EntityId`. Dimensions are of type `Length` (newtype, mm).
- Lines connect via snapping to equal coordinates (tolerance), not via shared
  point IDs.
- In the MVP the sketch plane is one of XY, XZ and YZ.

## Consequences

- Rectangle and circle stay editable as a unit, simple and without a solver.
- With the solver comes `format_version` 2: a rectangle becomes 4 lines, 4 points and
  constraints (horizontal, vertical, dimension). The migration is unambiguous.
- `Length` as a newtype later allows expressions and user parameters without restructuring.

## Rejected alternatives

- **Points + constraints already now without a solver** – editing afterwards would be
  cumbersome without a solver (moving points one by one).

## References

- ADR-0009, ADR-0011
