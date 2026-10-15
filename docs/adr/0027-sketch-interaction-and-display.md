---
id: ADR-0027
title: Sketch mode – left mouse button draws, look-at and snapping on, sketches as an egui overlay
status: accepted
date: 2026-10-13
crates: [qoob3d-app, qoob3d-sketch]
tags: [sketch, ux, rendering]
supersedes: []
superseded_by: null
---

## Context

From the sketch requirements, E-06, E-08 and E-09 need to be decided. It is also open
how sketch geometry is drawn: lines, points, dashed construction geometry, glyphs,
hover, input fields and profiles. Decided with the user on 2026-10-13.

## Decision

- **Left mouse button in sketch mode** draws, selects and drags. The left-drag orbit
  (ADR-0022) is off in sketch mode. Orbit there via Shift + two fingers or
  Shift + middle mouse button. Outside the mode, ADR-0022 applies unchanged.
- **Look-at** when entering a sketch: on (animated onto the plane normal).
- **Snapping** is on by default, Cmd/Ctrl suppresses it temporarily. A snap to an
  existing point uses that point's ID (ADR-0024).
- **Display as an egui overlay:** sketch curves, points, preview, snap glyphs,
  input fields and profile fills are projected through the camera and drawn with the
  egui painter. Profiles come triangulated from the kernel (ADR-0028) and are
  filled as an egui mesh.
  - Advantages: dashed lines, text and glyphs without new GPU pipelines, pixel-accurate
    hit tests in the same coordinates, visible in UI tests and screenshots.
  - Disadvantage: no occlusion by bodies. Sketches always lie on top, the way Fusion
    highlights active sketches. This will be re-evaluated with bodies (M4).
- Colors: under-constrained mint, construction dashed muted amber, preview
  dashed mint, profiles mint at 12 % (hover 25 %), inactive sketches gray
  (`docs/design/theme.md`).

## Consequences

- Tools work in sketch coordinates. Pixel → sketch via a raycast onto the plane
  (SK-UI-01), sketch → pixel via `Camera::project`.
- Snap tolerance and pick radius in pixels are converted into sketch units.

## Rejected alternatives

- **Sketches through the wgpu line pipeline** – dashing, glyphs, text and hover would be
  additional GPU code with no benefit for M3a.

## References

- `crates/qoob3d-app/src/sketch_ui.rs`, `crates/qoob3d-sketch/`
- ADR-0022, ADR-0024, ADR-0028, `docs/requirements/sketches.md` (SK-UI-*, SK-VS-*, E-06/08/09)
