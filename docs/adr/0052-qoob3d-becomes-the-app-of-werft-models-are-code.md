---
id: ADR-0052
title: QOOB3D becomes the app of Werft – models are build123d code, agents come first
status: superseded
date: 2026-11-22
crates: [qoob3d-app, qoob3d-core]
tags: [architecture, ai-first, werft]
supersedes: []
superseded_by: ADR-0053
---

## Context

The user brought in the concept "Werft" (`docs/concept/werft-concept.md`, English translation
of the German original): a CAD environment
in which a model is Python code (build123d on OCCT) in a Git folder, an AI agent is the main
user (MCP tools, background jobs, cache, values with sources, checks as pytest, photo
measurement), and a human can additionally model with the mouse. The concept planned a new
SwiftUI app from milestone 5.

Milestone 0 (`docs/concept/m0-report.md`) measured build123d: build time 0.009–0.675 s per
probe part, STEP import 0.42 s (1.15 MB), cache read 0.015 s, but 1.6–1.7 s for importing
build123d in a fresh process. QOOB3D already has what the Werft app needs on the human side:
Fusion navigation, ViewCube, sketches with value HUD and snapping, extrude/revolve/loft,
print bed with 441 printer profiles, 3MF export, headless render, CLI.

The user decided: "wir machen Rust" – the app is QOOB3D, not a new SwiftUI app – and asked to
combine the best concepts of both, AI first.

## Decision

- **One source of truth: model code.** A Werft project (`werft.toml`, `parts/*.py`,
  `soll/*.json`, `checks/`) is the model. Agents edit the files and use the Werft MCP tools.
- **The Werft core stays Python** (`werft/`): build123d for geometry, the service `werftd`
  with warm worker processes, time and memory limits, build graph and cache, MCP server and
  CLI. Reason: agents write build123d far more reliably than any custom API, and free code was
  the most powerful tool in practice (concept section 2.1).
- **QOOB3D is the app.** It opens a Werft project, connects to `werftd` over localhost HTTP,
  shows the built parts live (parts tree, section, measure, print bed) and later turns mouse
  actions into lines of build123d code (concept 7a) instead of `.qbd` features.
- `.qbd` documents keep working unchanged (ADR-0030) until the mouse tools write code; a
  converter `.qbd` → `parts/*.py` follows then.
- The Rust kernel (`qoob3d-kernel`) stays for sketch profiles and fast interactive preview;
  it is never the source of truth for a Werft project.

## Consequences

- Two languages: Rust for the app, Python for the model core. The boundary is a small JSON
  HTTP API of `werftd` plus mesh files; the app never imports Python.
- The app gains a live view of agent work early (concept milestone 5 moves forward).
- Werft documents in `werft/docs/` are German by the user's request; ADR-0045 needs an
  allowlist entry `path: werft/docs/` (human, harness file).
- OCCT versions differ (app 7.8.1, build123d 8.0.1); geometry crosses the boundary as meshes
  (STL/3MF/glb), not as BREP.

## Rejected alternatives

- **New SwiftUI app** – rejected by the user; would rebuild navigation and sketching that
  exist and are tested.
- **Everything in Rust, model as a feature document** – loses free code, the agents' strongest
  tool, and the build123d ecosystem.
- **Embedded scripting in Rust (Rhai, Lua)** – a custom CAD API that agents do not know.

## References

- `docs/concept/werft-concept.md`, `docs/concept/m0-report.md`, `werft/docs/m1_spec.md`
- Related: ADR-0030, ADR-0033, ADR-0045
