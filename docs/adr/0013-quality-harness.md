---
id: ADR-0013
title: One command defines "done" – cargo xtask check with smoke test, render CLI and geometry checks
status: accepted
date: 2026-10-13
crates: [xtask, qoob3d-app, qoob3d-core, qoob3d-render]
tags: [quality, tests, ai, security]
supersedes: []
superseded_by: null
---

## Context

The AI agent cannot see the running app. In M1 it started the app for 9 s,
reported "running" and overlooked a deadlock that egui only reports after 10 s.
Quality must not depend on diligence but has to be checked mechanically.

## Decision

1. **`cargo xtask check` is the definition of "done".** It runs in order:
   `fmt --check`, `clippy -D warnings`, `test`, architecture rules (ADR-0003),
   docs lint (ADR-0001), `cargo deny` and the smoke test. `cargo xtask check --fast`
   leaves out smoke test and deny (for hooks).
2. **The agent gets eyes:**
   - `qoob3d --render out.png [--view iso|front|top|right] [--size WxH]` renders headless
     without a window.
   - `qoob3d --smoke-test` starts the real app, feeds synthetic input (orbit, pan,
     zoom) through the regular input path, checks the effect and exits with
     exit code 0 or 1. A watchdog aborts with an error after 15 s without frame progress.
3. **Geometry is checked, not just counted:** `qoob3d_core::mesh::MeshReport` provides
   watertight/manifold, open and non-manifold edges as well as the signed volume.
   Kernel tests check an exact volume and that the mesh is watertight.
4. **Supply chain:** `cargo deny` checks advisories, licenses (LGPL only explicitly allowed
   for the OCCT crates) and sources (crates.io only).
5. **Rules for the agent** (stated in the CLAUDE.md):
   - Bugfixes start with a failing test.
   - No `#[ignore]` and no `#[allow(…)]` without a justifying comment.
   - Never accept golden files and snapshots without asking the user.
6. **Claude Code stop hook** runs `cargo xtask check --fast` if Rust code has changed.

## Consequences

- "Done" is verifiable. The agent can check results visually
  (`--render` → read the PNG).
- The smoke test needs a graphical session (fine locally, later `xvfb` on Linux CI).
- Later additions: `egui_kittest` for UI flows (M2/M3), `cargo-fuzz` for loading
  files (M3), `cargo-mutants` occasionally for test quality, CI for macOS and Linux.

## Rejected alternatives

- **Only manual tests by the user** – too slow, and bugs are found by the user.
- **Screenshots of the real window** – needs screen recording permission and is
  not deterministic.

## References

- `xtask/`, `crates/qoob3d-app/src/cli.rs`, `crates/qoob3d-app/src/smoke.rs`,
  `crates/qoob3d-core/src/mesh.rs`, `deny.toml`, `.claude/settings.json`
- ADR-0001, ADR-0003
