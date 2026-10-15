---
id: ADR-0025
title: The constraint solver goes into the MVP, order M3a → M4 → M3b → M3c
status: accepted
date: 2026-10-13
crates: []
tags: [planning, sketch, solver]
supersedes: []
superseded_by: null
---

## Context

The original MVP excluded a constraint solver. The sketch requirements
(sections 16/17, E-03, E-10) show that parametric sketching without constraints
remains provisional. At the same time, a printable part should come about early. Decided with
the user on 2026-10-13.

## Decision

- **M3a:** Document model, save/load, sketch mode on XY/XZ/YZ, line/polyline,
  rectangle, circle, point, construction, value input at the cursor, snapping,
  selection/dragging/deletion, profiles with islands, undo/redo, timeline/browser.
- **M4:** Extrusion (new body/join/cut).
- **M3b:** Constraint solver, constraints, dimensions, user parameters, arc, slot,
  polygon, trim/extend, fillet, offset, mirror.
- **M3c:** Construction planes (offset, at an angle, 3 points, midplane), sketches on
  faces, project, slice.
- The choice of solver (E-01) is made in a short spike before M3b: our own solver in Rust
  versus planegcs (LGPL). The result becomes an ADR of its own.

## Consequences

- After M3a + M4, simple parts can be printed.
- M5/M6 (timeline editing, export) come after that. The milestone table in
  CLAUDE.md has been adjusted accordingly.

## Rejected alternatives

- **Solver after the MVP** – until then, sketches could only be changed to a limited extent.
- **All of M3 before M4** – the first printable part would come considerably later.

## References

- `docs/requirements/sketches.md` (sections 16, 17)
- ADR-0024
