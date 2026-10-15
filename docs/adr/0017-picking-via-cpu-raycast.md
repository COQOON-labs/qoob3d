---
id: ADR-0017
title: Picking for now via CPU raycast against the triangle mesh
status: accepted
date: 2026-10-13
crates: [qoob3d-core, qoob3d-render, qoob3d-app]
tags: [picking, navigation]
supersedes: []
superseded_by: null
---

## Context

Fusion orbits around the model point under the cursor. From M3/M4 on, selection of faces,
profiles and edges is added. Options are a CPU raycast or a GPU ID buffer.

## Decision

- `Camera::ray(ndc, aspect)` (perspective and ortho) plus `TriMesh::raycast` (Möller–Trumbore,
  both sides, nearest hit) in `qoob3d-core`.
- The hit yields the triangle. Via `TriMesh::face_ids` (ADR-0014) the B-rep face is known.
- Orbit pivot = hit point at the start of the orbit, otherwise the camera target. The pivot is
  shown as a mint dot.
- Switch to a GPU ID buffer or a BVH only when measurements demand it
  (linear search is sufficient for MVP models).

## Consequences

- Picking is testable without a GPU (unit tests in core, smoke test checks the pivot).
- Edge picking will later need a tolerance in pixels. That comes with selection.

## Rejected alternatives

- **GPU ID buffer right away** – additional pass and readback latency, unnecessary for pivot and
  MVP selection.

## References

- `crates/qoob3d-core/src/mesh.rs` (`raycast`), `crates/qoob3d-render/src/camera.rs` (`ray`),
  `crates/qoob3d-app/src/navigation.rs`
