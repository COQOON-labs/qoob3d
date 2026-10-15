---
id: ADR-0046
title: Rebuilds report what they computed, and body analysis is cached with the body
status: accepted
date: 2026-10-14
crates: [qoob3d-kernel]
tags: [kernel, performance, tests]
supersedes: []
superseded_by: null
---

## Context

ADR-0012 caches feature results along a hash chain. Whether the cache works was only visible
through timing: a kernel test required the second rebuild of an unchanged document to take less
than half the time of the first. It failed on most runs, because every rebuild still recomputed
the volume and the validity check (BRepCheck) of every body, even when the body came unchanged
from the cache. Timing assertions depend on machine load and cannot tell which work was repeated.

## Decision

1. **Statistics:** every `RebuildResult` carries `RebuildStats`:
   - `computed_features`: sketches and solid features evaluated in this rebuild,
   - `cached_features`: sketches and solid features taken from the cache,
   - `analyzed_bodies`: bodies whose analysis was computed in this rebuild.

   Suppressed features and construction planes count in neither feature field.
2. **Body analysis** (view mesh, volume, validity) is computed once per body state and stored with
   it, only for the bodies in the rebuild result (intermediate states between features are not
   analyzed). A body that comes from the cache is not analyzed again; its volume is bit-identical.
3. **Cache keys:**
   - Solid features use the hash chain of ADR-0012: changing feature *i* recomputes *i* and every
     later solid feature.
   - Sketch profiles depend only on the sketch itself and are keyed by its content, so an unchanged
     sketch after a changed feature stays cached.
   - A body created or modified by a recomputed step gets a new analysis, even if its geometry
     happens to be unchanged (the kernel does not compare geometry); bodies a recomputed step only
     carries over keep their analysis.
4. **Retention:** the kernel keeps the results of the current and the previous rebuild (cheap
   undo/redo) and drops the rest. Exports read the cache without changing what is retained.
5. **Tests** check the cache through these statistics, never through timing
   (`crates/qoob3d-kernel/tests/features.rs`).

## Consequences

- Rebuilding an unchanged document does no OCCT work; the UI and logs can show what a rebuild cost.
- Refines ADR-0012: the retention rule replaces the "bounded by LRU" consequence there.
- `RebuildStats` is public API of the kernel crate.

## Rejected alternatives

- **Keeping a timing test with a looser threshold:** still load-dependent and blind to which work
  was repeated.
- **Comparing geometry to skip analysis of unchanged bodies behind a recomputed step:** costs about
  as much as the analysis itself.

## References

- Code: `crates/qoob3d-kernel/src/model.rs` (`Model::rebuild`, `Model::run`, `RebuildStats`)
- Tests: `crates/qoob3d-kernel/tests/features.rs`
- Related: ADR-0005, ADR-0012, ADR-0031
