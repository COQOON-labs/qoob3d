---
id: ADR-0056
title: Declaration discovery runs in a worker – the service never evaluates user scripts
status: accepted
date: 2026-11-22
crates: [qoob3d-kernel, qoob3d-app]
tags: [service, workers, security, performance]
supersedes: []
superseded_by: null
---

## Context

ADR-0055 let the service evaluate the top level of part and check files itself (declaration
discovery with `PartMode::Declare`, no CAD API) and protected it with script size limits
(AG-HD-02). Rhai's own limits miss `a += a` on arrays and assignment into containers, so the
kernel measures the variables in stages of 4096 operations. That costs quadratic time: a part
file whose top level runs one million operations took longer than the 20 s start-up budget of
the service (test `ag_hd_02_discovery_stops_after_a_million_operations`). Shrinking the cost
with larger stages would let more memory grow unseen inside the service process, which has no
memory limit of its own.

## Decision

- **The service never evaluates user scripts.** Declaration discovery of part and check files
  is a worker request (`declare`): the worker runs the file in `PartMode::Declare` under the
  project's time limit and the worker's resident-memory limit, and answers names, options,
  dependencies and check names, or the load error. The service caches the answer by the bytes
  of the file and its literally imported modules, as before.
- **Size limits stay in declare mode** (AG-HD-02: string 65536 bytes, array 10000 elements,
  map 1000 entries, at most 1000000 operations) so a bad file gives a clear `Limit` load error
  with its line. The measuring stages double (4096, 8192, …), so the total cost is linear in
  the operations of the file; memory that grows between two measurements is stopped by the
  worker's memory limit and reported as that file's load error (`memory_limit`).
- A worker crash or limit during discovery fails only that file; the service stays up and
  `/status` reports the load error.
- This replaces the bullet "Declarations without the CAD API" of ADR-0055 in so far as it
  places discovery in the service process; everything else of ADR-0055 stays in force.

## Consequences

- The promise "no run can block or kill the environment" holds for every script, including
  declarations.
- Discovery costs one worker round trip per changed file (milliseconds with warm workers);
  unchanged files come from the declaration cache.
- `/status` must not wait for discovery longer than its 100 ms budget: while discovery of a
  changed file runs, the previous declarations of that file are reported, marked `outdated`.

## Rejected alternatives

- **Quadratic staged measurement in the service** – too slow for legitimate long top levels.
- **`Engine::on_var`** – deprecated in Rhai and over the lint-suppression cap.
- **A memory limit for the service process itself** – no portable way without new
  dependencies, and the service would still die instead of reporting.

## References

- `docs/requirements/agent.md` AG-HD-02, `crates/qoob3d-kernel/src/script/limits.rs`,
  `crates/qoob3d-app/src/service/snapshot.rs`, `crates/qoob3d-app/src/worker.rs`
- Related: ADR-0053, ADR-0055
