---
id: ADR-0002
title: QOOB3D is primarily a design tool for FDM printing; in trade-offs the path design → slicer → print wins
status: accepted
date: 2026-11-23
crates: [qoob3d-core, qoob3d-kernel, qoob3d-app]
tags: [product, fdm]
supersedes: []
superseded_by: null
---

## Context

Parametric CAD can serve many purposes: mechanical engineering, assemblies, drawing
derivation, CAM. The user mainly designs parts for FDM printers such as Bambu Lab. Without a
clear focus, features compete for priority. How models are made (scripts written by an AI
agent, the human accepting the result) is a separate decision, ADR-0053; this one says what
the models are for.

## Decision

In trade-offs, whatever improves the path **design → slicer → print** wins. Specifically:

- The XY plane is the print bed, +Z the build direction, the unit is mm (ADR-0006).
- Exported meshes must be **watertight and manifold**. Every project build checks its mesh
  and reports `watertight`; a part that is not watertight gets a warning that a slicer may
  reject it. The reference project gate (ADR-0013) checks it in CI.
- **3MF** is a first-class export target next to STL and STEP, because Bambu Studio works
  with it natively (ADR-0034). When Bambu Studio is installed, the gate loads every exported
  3MF with it as an independent verifier.
- Tessellation is fine enough for printing: a project part is meshed once per cache key at
  `[mesh] deflection_mm` (default 0.01 mm, well below a nozzle width, and 20° angle), and that
  one mesh serves the live view and the STL/3MF export, so the slicer gets what the user saw.
- The viewport shows the print bed and build volume of a chosen printer profile, from a
  database generated from OrcaSlicer (ADR-0021).
- The script API carries FDM-specific helpers and print analysis: pins and holes with
  clearance, countersinks and counterbores, fillets and chamfers (elephant's foot), the area
  on the bed and the overhang area.
- Still open and next in priority: tolerance and fit parameters as project values, wall
  thickness analysis, threads, embossed text.

## Consequences

- Drawing derivation, assemblies and CAM have low priority.
- Mesh quality is a test criterion, not just the looks.
- A feature that helps only outside FDM printing needs a reason beyond completeness.

## Rejected alternatives

- **A general Fusion clone as the goal** – too broad, gives no criterion for priorities.

## References

- `crates/qoob3d-core/src/project/mod.rs` (`Mesh`), `crates/qoob3d-project/src/worker.rs`
  (`watertight`), `xtask/src/models.rs` (reference gate, Bambu Studio),
  `crates/qoob3d-core/src/mesh.rs` (`MeshReport`), `crates/qoob3d-core/src/export.rs`
- `docs/script-api.md` (sections Print analysis, FDM helpers)
- Related: ADR-0006, ADR-0013, ADR-0021, ADR-0034, ADR-0053
