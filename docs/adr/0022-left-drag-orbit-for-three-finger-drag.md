---
id: ADR-0022
title: Dragging with the left mouse button (macOS three-finger drag) orbits the view, can be turned off
status: accepted
date: 2026-10-13
crates: [qoob3d-app]
tags: [ux, navigation, macos]
supersedes: []
superseded_by: null
---

## Context

The user wants three-finger dragging on the trackpad to orbit like Shift + two fingers.
macOS does not pass three-finger gestures on to apps. If "Three finger drag" is enabled in the
accessibility settings (for the user: yes), the system turns it into an ordinary
**drag with the left mouse button**. The app cannot tell the two apart.

## Decision

- Setting `left_drag_orbit` (⚙ → "Left drag orbits (incl. three-finger drag)"),
  **on by default**: dragging with the left mouse button orbits around the point under the
  cursor (like Shift + middle mouse button, ADR-0017).
- While dragging, the cursor shows a grabbing hand.
- ADR-0008 continues to apply unchanged. This is an additional binding.

## Consequences

- **Open point for M3:** In Fusion, the left mouse button drags a selection box. When selection
  arrives, it must be decided whether the box gets a modifier key (e.g. Alt) or whether the
  setting is then off by default. That will be a new ADR.

## Rejected alternatives

- **Real three-finger detection** – not possible, winit/eframe do not receive
  multi-finger trackpad gestures on macOS.

## References

- `crates/qoob3d-app/src/navigation.rs`, `crates/qoob3d-app/src/settings.rs`
- ADR-0008, ADR-0017
