---
id: ADR-0031
title: Body features (extrude, revolve, loft, plane) with a rebuild via hash chain and booleans against touching bodies
status: accepted
date: 2026-10-14
crates: [qoob3d-core, qoob3d-kernel]
tags: [document-model, kernel, features]
supersedes: []
superseded_by: null
---

## Context

After sketches (M3a), real printable parts should come about: box, cylinder, sphere,
pyramid. This needs features that turn profiles into bodies, a behavior for
multiple bodies (new, join, cut) and a rebuild that only recomputes what has changed
(ADR-0012).

## Decision

- **Format version 2** (`document::FORMAT_VERSION`): `FeatureKind` gets `Plane`
  (offset construction plane), `Extrude`, `Revolve`, `Loft`, `SpurGear`, `Coil`
  (ADR-0032). Sketches get arcs (`CurveKind::Arc`, center/start/end,
  counterclockwise). `PlaneRef::Construction(FeatureId)` allows sketches on
  construction planes; they are resolved via `Document::plane_frame`. v1 → v2 is an empty
  migration (only new variants).
- References: profiles via `ProfileRef { sketch, key }` (ADR-0011), axes as an origin axis
  or a sketch line, loft apexes as a sketch point. `validate` requires every reference
  to point to an **earlier** feature.
- **Operation** per feature: `NewBody` (every solid a body of its own), `Join` (unites
  with all bodies the result touches; none → new body), `Cut` (subtracts from all
  touched bodies; none → feature error). "Touches" = distance ≤ 1e-6 mm.
- Body ID = generating feature + index (`BodyId`), name "Body N" in order of creation.
- **Rebuild** in the worker (`model.rs`): chain `key_i = hash(key_{i-1}, feature JSON)`; the
  state (bodies as OCCT shapes) after each feature is kept in the cache. The keys of the current
  and the previous rebuild are kept (cheap undo/redo). Profiles are cached per
  sketch via its hash, meshes per body on first demand.
- Errors are **feature errors**: the feature is skipped, the bodies stay as
  before, the reason is shown in red in the timeline. If a profile is missing, nothing is guessed.
- Booleans are simplified with `ShapeUpgrade_UnifySameDomain` (no seams on planar
  faces). The mesher discards degenerate triangles (e.g. at sphere poles).

## Consequences

- Unchanged features before a change cost nothing on rebuild.
- Without a body selection, join/cut act like Fusion's default ("all touched bodies").
  An explicit participant selection can be added later as a field.
- The timeline cannot be rolled back yet (M5).

## Rejected alternatives

- **One body per document** – multi-part prints (plate + gear) would not be possible.
- **Join/cut only against the last body** – surprising as soon as there are several bodies.

## References

- Code: `crates/qoob3d-core/src/document/{feature,mod}.rs`, `crates/qoob3d-kernel/src/model.rs`,
  `crates/qoob3d-kernel/cpp/qoob3d_occt.cpp`
- Tests: `crates/qoob3d-kernel/tests/features.rs`
- Related: ADR-0011, ADR-0012, ADR-0030, ADR-0032, ADR-0033
