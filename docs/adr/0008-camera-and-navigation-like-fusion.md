---
id: ADR-0008
title: Camera as an orbit camera with quaternion, navigation 1:1 like Fusion 360
status: accepted
date: 2026-10-13
crates: [qoob3d-render, qoob3d-app]
tags: [ux, navigation, camera]
supersedes: []
superseded_by: null
---

## Context

The user works with Fusion 360 and wants its controls 1:1. Mac users
often work with a trackpad without a middle mouse button.

## Decision

- **Camera** (`Camera`): `target`, `distance`, `rotation: Quat` (camera-local: x right,
  y up, z backwards) and `fov_y`. The orbit is a turntable around world Z plus a
  tilt around the camera's right axis, without limit (you can rotate over the pole).
- **Bindings** (`qoob3d-app/src/navigation.rs`):

  | Input | Action |
| --- | --- |
  | Drag middle mouse button | Pan |
  | Shift + drag middle mouse button | Orbit |
  | Mouse wheel (forward = in) | Zoom to the point under the cursor |
  | Double-click middle mouse button / F6 | Fit |
  | Trackpad: two fingers | Pan |
  | Trackpad: Shift + two fingers | Orbit |
  | Trackpad: pinch | Zoom to the cursor |

- Mouse wheel and trackpad are distinguished via `MouseWheelUnit::Line`/`Page` and `Point`
  respectively.
- From M2 on: the orbit rotates around the point under the cursor if there is geometry there
  (`Camera::orbit_around`), as in Fusion.

## Consequences

- Mice with smooth scrolling (Magic Mouse) send `Point` and behave like a trackpad.
- The left mouse button stays free for selection and tools.

## Rejected alternatives

- **Left mouse button = orbit (Blender/Onshape style)** – contradicts the Fusion wish and
  collides with selection.
- **Yaw/pitch angles instead of quaternion** – ViewCube views and transitions are simpler with
  quaternions.

## References

- `crates/qoob3d-render/src/camera.rs`, `crates/qoob3d-app/src/navigation.rs`
- `docs/design/controls.md`
