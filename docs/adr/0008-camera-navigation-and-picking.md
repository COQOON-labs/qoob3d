---
id: ADR-0008
title: The viewport uses a quaternion orbit camera navigated like Fusion 360, orbits around the model point under the cursor found by a CPU raycast, and left drag orbits by default
status: accepted
date: 2026-11-23
crates: [qoob3d-core, qoob3d-render, qoob3d-app]
tags: [ux, navigation, camera, picking, macos]
supersedes: []
superseded_by: null
---

## Context

The user works with Fusion 360 and wants its controls 1:1. Mac users often work with a
trackpad without a middle mouse button. Fusion orbits around the model point under the cursor,
which needs a hit test against the displayed geometry. macOS does not pass three-finger
gestures to apps: with "three finger drag" enabled in the accessibility settings (the user
has it on), the system turns the gesture into an ordinary drag with the left mouse button,
and the app cannot tell the two apart.

The app shows script projects only; it has no selection yet. Later mouse tools that write
script lines will need picking of faces and edges.

This topic absorbs the former decisions 0017 and 0022.

## Decision

### Camera

`Camera` (`qoob3d-render`) has `target`, `distance`, `rotation: Quat` (camera-local axes:
x right, y up, z backwards), `fov_y` and `projection` (perspective or orthographic; both
share the visible height in the plane through `target`, so switching keeps the framing).
The orbit is a turntable around world Z plus a tilt around the camera's right axis, without a
limit (the camera can roll over the pole). Transitions slerp the rotation and interpolate the
distance logarithmically.

### Bindings

| Input | Action |
| --- | --- |
| Middle mouse button drag | Pan |
| Shift + middle mouse button drag | Orbit around the point under the cursor |
| Mouse wheel (forward = in) | Zoom towards the cursor |
| Double-click middle mouse button / F6 | Fit all (animated) |
| Trackpad: two fingers | Pan |
| Trackpad: Shift + two fingers | Orbit around the point under the cursor |
| Trackpad: pinch | Zoom towards the cursor |
| Left drag / macOS three-finger drag | Orbit around the point under the cursor (setting) |

Mouse wheel and trackpad are told apart by `MouseWheelUnit::Line`/`Page` versus `Point`.
Every input cancels a running camera animation. `docs/design/controls.md` is the full
specification, including the constants and the cursor rules.

### Left drag orbits, can be turned off

The setting `left_drag_orbit` ("Left drag orbits (incl. three-finger drag)") is **on by
default**. While dragging, the cursor is a grabbing hand.

### Picking by CPU raycast

`Camera::ray(ndc, aspect)` (perspective and ortho) plus `TriMesh::raycast` (Möller–Trumbore,
both sides, nearest hit) in `qoob3d-core`. The orbit pivot is the hit point at the start of
the orbit, otherwise the camera target, and is shown as a dot in the accent colour. A hit
also gives the triangle and thereby the B-rep face id of the mesh. A BVH or a GPU id buffer
comes only when measurements demand it; a linear search is enough for print-sized models.

## Consequences

- Mice with smooth scrolling (Magic Mouse) send `Point` and behave like a trackpad.
- Picking is testable without a GPU (unit tests in core); the smoke test checks that the
  pivot of a Shift + middle drag lies on the model.
- **Open:** when selection arrives with the mouse tools, Fusion's left-drag selection box
  collides with left-drag orbit. Then either the box gets a modifier or the setting is off
  by default; that is decided then. Edge picking will need a tolerance in pixels.

## Rejected alternatives

- **Left mouse button = orbit as the only scheme (Blender/Onshape style)** – contradicts the
  Fusion wish; here it is an additional, switchable binding.
- **Yaw/pitch angles instead of a quaternion** – ViewCube views and transitions are simpler
  with quaternions.
- **Real three-finger detection** – not possible, winit/eframe receive no multi-finger
  trackpad gestures on macOS.
- **GPU id buffer right away** – an extra pass and readback latency, unnecessary for the
  pivot.

## References

- `crates/qoob3d-render/src/camera.rs`, `crates/qoob3d-core/src/mesh.rs` (`raycast`),
  `crates/qoob3d-app/src/navigation.rs`, `crates/qoob3d-app/src/viewport.rs`,
  `crates/qoob3d-app/src/settings.rs`, `crates/qoob3d-app/src/smoke.rs`
- `docs/design/controls.md`
- Related: ADR-0006, ADR-0019
