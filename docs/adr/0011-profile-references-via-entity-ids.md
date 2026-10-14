---
id: ADR-0011
title: Profiles are referenced via the set of their bounding entity IDs
status: accepted
date: 2026-10-13
crates: [qoob3d-core, qoob3d-kernel]
tags: [topological-naming, sketch, extrusion]
supersedes: []
superseded_by: null
---

## Context

An extrusion has to remember which region of a sketch it extrudes, even after
the sketch has been edited. This is a small form of the topological naming problem.

## Decision

- **The kernel** (OCCT) computes regions from all curves of a sketch. This includes
  intersections and nesting, e.g. a circle inside a rectangle yields a ring
  and a disc.
- `ProfileRef = { sketch: FeatureId, region: RegionKey }`, where `RegionKey` is the **sorted
  set of entity IDs** that bound the region.
- If a referenced region no longer exists after a change, the feature is marked as an
  error. **There is no guessing.**
- Bodies are referenced via the ID of the feature that created them (`BodyId`).
- "Sketch on body face" stays out until after the MVP.

## Consequences

- Robust against dimension changes, because the IDs stay the same.
- If two regions have the same boundary set, this is ambiguous (rare). Then there is
  an error instead of a silent wrong choice.

## Rejected alternatives

- **Region via index** – changes with every sketch change.
- **Region via a point in its interior** – fragile under size changes.

## References

- ADR-0010, ADR-0012
