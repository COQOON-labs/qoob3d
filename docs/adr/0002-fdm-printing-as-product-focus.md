---
id: ADR-0002
title: QOOB3D is primarily a design tool for FDM printing
status: accepted
date: 2026-10-13
crates: []
tags: [product, fdm]
supersedes: []
superseded_by: null
---

## Context

Parametric CAD can serve many purposes (mechanical engineering, assemblies, drawing
derivation). The user mainly designs parts for FDM printers such as Bambu Lab. Without a
clear focus, features compete for priority.

## Decision

In trade-offs, whatever improves the path **design → slicer → print** wins.
Specifically:

- XY plane = print bed, +Z = build direction (see ADR-0006).
- Exported meshes must be **watertight and manifold**. This is checked in tests.
- **3MF** is a first-class export target alongside STL and STEP (Bambu Studio works with it
  natively).
- Export tessellation is separate from viewport tessellation and finer
  (default ~0.01 mm chordal deviation).
- The viewport optionally shows the print bed of a printer profile (e.g. A1 mini 180×180,
  X1/P1/A1 256×256 mm).
- After the MVP, priority goes to: chamfers and fillets (elephant's foot), tolerance and
  fit parameters, overhang and wall thickness analysis, threads, embossed text.

## Consequences

- Drawing derivation, assemblies and CAM have low priority.
- Mesh quality is a test criterion, not just the looks.

## Rejected alternatives

- **General Fusion clone goal** – too broad for an MVP, no criterion for priorities.

## References

- ADR-0006, ADR-0013
