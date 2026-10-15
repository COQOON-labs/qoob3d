---
id: ADR-0015
title: ViewCube and viewport overlays are drawn with egui, only the scene with wgpu
status: accepted
date: 2026-10-13
crates: [qoob3d-app]
tags: [ui, rendering, viewcube]
supersedes: []
superseded_by: null
---

## Context

Fusion controls in the viewport: ViewCube (clickable faces, edges and corners with
labels), home button, navigation bar, orbit pivot marker, status line. Text on
3D faces in wgpu would need a font atlas, and hit tests and hover logic twice.

## Decision

- wgpu renders the **scene** (bodies, edges, grid, axes) offscreen (ADR-0019).
- egui draws **overlays** on top:
  - ViewCube (`viewcube.rs`): cube projected orthographically with the camera rotation. Every
    visible face is divided into 3×3 regions (middle = face, borders = edge,
    corners = corner; border width `BAND`). A click sets the view direction to the sum of the
    involved normals. Labels in German, adapted to the visible face.
  - Home button, navigation bar (`egui::Area`, bottom center), pivot point, status line
    (`viewport.rs`).
- Geometry and hit test of the ViewCube are **pure functions** (`regions`, `hit`,
  `face_center`) with unit tests. Drawing happens in `show`.
- Camera changes (ViewCube, Home, Fit) are animated over 0.35 s
  (`Camera::interpolate`: slerp, logarithmic distance). Any user input aborts the
  animation.

## Consequences

- Overlays are testable via egui_kittest (labels, clicks) and appear in the screenshot (ADR-0018).
- Labels on the ViewCube are rotated but not perspectively sheared (unlike
  in Fusion). If needed, later as an egui mesh with a texture.

## Rejected alternatives

- **ViewCube as a 3D object in wgpu** – text, hover and hit test would be laborious and less
  testable.

## References

- `crates/qoob3d-app/src/{viewcube.rs,viewport.rs}`, `docs/design/controls.md`
- ADR-0008, ADR-0018
