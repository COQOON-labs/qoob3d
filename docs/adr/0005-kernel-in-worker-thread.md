---
id: ADR-0005
title: OCCT objects live exclusively in a dedicated kernel worker thread
status: accepted
date: 2026-10-13
crates: [qoob3d-kernel, qoob3d-app]
tags: [threading, kernel]
supersedes: []
superseded_by: null
---

## Context

Geometry operations (Boolean operations, tessellation) can take seconds, and the
UI must not freeze meanwhile. OCCT handles are not `Send`.

## Decision

- `qoob3d_kernel::Kernel::spawn(notify)` starts a thread (`qoob3d-kernel`) that owns **all**
  OCCT objects.
- Communication runs exclusively via messages containing plain data:
  `submit(Request) -> RequestId` and `try_recv() -> Option<Response>`.
- Every response carries the `RequestId` of its request. **The UI discards stale responses.**
- After every response the worker calls `notify()` (in the app: `ctx.request_repaint()`).
- From M4 on: if several rebuild requests are queued, only the newest one is
  processed. The old geometry stays visible until the new result arrives.
- When the `Kernel` handle is dropped, the worker terminates.

## Consequences

- The UI never blocks on geometry.
- The kernel cache (ADR-0012) lives in the worker thread.
- A later move into a **separate process** (crash isolation, untrusted STEP import)
  only changes the transport, not the protocol.

## Rejected alternatives

- **Rayon/tasks per request** – OCCT objects would have to move between threads (not `Send`).
- **Synchronous calls from the UI** – freezes with complex models.

## References

- `crates/qoob3d-kernel/src/worker.rs`, `crates/qoob3d-app/src/app.rs`
