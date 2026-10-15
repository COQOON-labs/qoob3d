---
id: ADR-0029
title: Undo/redo via snapshots of the whole document, one sketch session as one timeline unit
status: accepted
date: 2026-10-13
crates: [qoob3d-app, qoob3d-core]
tags: [undo, document-model]
supersedes: []
superseded_by: null
---

## Context

SK-LC-08 requires fine-grained undo in sketch mode and one unit per sketch session in
the timeline. The document is small (parameters only, no geometry, ADR-0009).

## Decision

- The undo stack stores **immutable snapshots** of the document
  (`Arc<Document>`) with a translatable label. Every change creates a new
  snapshot; redo is discarded on a new change.
- When a sketch is finished, all snapshots of the session are merged into **one** entry
  ("Edit Sketch1").
- Limit: 200 entries.
- The saved state (dirty) is determined via the snapshot at the time of saving
  (`Arc::ptr_eq`), not via a counter.

## Consequences

- Undo is trivially correct and independent of the individual tools.
- The rebuild cache (ADR-0012) benefits, since old document states yield identical keys.

## Rejected alternatives

- **Command pattern with inverse operations** – every operation would need an error-free
  inverse. No benefit at this document size.

## References

- `crates/qoob3d-app/src/history.rs`
- ADR-0009, ADR-0012
