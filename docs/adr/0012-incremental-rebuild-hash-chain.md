---
id: ADR-0012
title: The rebuild is incremental via a hash chain per feature
status: accepted
date: 2026-10-13
crates: [qoob3d-kernel]
tags: [kernel, performance, timeline]
supersedes: []
superseded_by: null
---

## Context

If a parameter changes, the timeline has to be recomputed from that feature on.
Maintaining dirty flags by hand is error-prone.

## Decision

- Cache key of feature *i*: `key_i = hash(key_{i-1}, Parameter_i)`.
- The kernel worker holds `key → OCCT result`. A rebuild walks the list and computes
  from the first key that is not in the cache.
- Suppressed features and the rollback marker are part of the key.
- Implementation in M4.

## Consequences

- Correct by construction: every change automatically invalidates everything after it.
- Undo/redo benefits without extra effort, because old keys are still in the cache.
  Memory must be bounded (LRU).

## Rejected alternatives

- **Dirty flags** – error-prone, especially with undo and reordering.
- **Always recompute everything** – too slow for large models.

## References

- ADR-0005
