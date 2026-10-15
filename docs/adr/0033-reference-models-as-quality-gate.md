---
id: ADR-0033
title: Reference models with analytical expectations are a quality gate; the CLI can build, check and export files
status: accepted
date: 2026-10-14
crates: [qoob3d-core, qoob3d-app, xtask]
tags: [quality, tests, ai]
supersedes: []
superseded_by: null
---

## Context

Geometry errors are often invisible to unit tests: a wrong sign, a face that is not
closed, an export that the slicer rejects. The AI co-developing here
needs hard, machine-checkable truths that do not come from the same code as the
result.

## Decision

- `qoob3d_core::document::examples` builds reference models the way a user builds them
  (sketch → feature). `cargo xtask models --write` writes them to `models/<name>.qbd`.
- Each model has a `models/<name>.expect.json` with **analytical** values (formula in the
  `why` field): number of bodies, volume with tolerance or range, bounding box.
- `cargo xtask models` (part of `cargo xtask check`) checks via the built binary:
  files up to date, every feature and curve kind in at least one model,
  `qoob3d <file> --report` without errors, valid and watertight bodies, volume, bounding box,
  `--export` as 3MF (our own reader **and** `unzip -t`) and STL.
- The CLI gets `--report` (JSON, exit code 1 on errors or non-printable bodies)
  and `--export <stl|3mf>`; `--render` renders files instead of the demo body.
- `crates/qoob3d-app/tests/modeling.rs` builds every shape end to end through the real UI
  (plane, tools with typed values, dialog, OK) and compares the volume.
- `cargo xtask screenshot` renders every model to `target/screenshots/model-<name>.png`.

## Consequences

- **New feature or curve kind = new reference model + expectation**, otherwise the check is red.
- Expectations are never adjusted to a result without checking the formula (like golden
  files, CLAUDE.md rule 5).
- On its first run, the independent verifier `unzip -t` found a bug in our own ZIP writer
  that our own reader had missed. Independent verifiers are therefore mandatory
  wherever they exist.

## Rejected alternatives

- **Snapshot comparison of meshes** – breaks on every tessellation change, says nothing
  about correctness.
- **Kernel tests only** – check neither the file format nor the CLI nor the export.

## References

- Code: `xtask/src/models.rs`, `crates/qoob3d-app/src/batch.rs`, `models/`
- Related: ADR-0013, ADR-0018, ADR-0031, ADR-0034, ADR-0036
