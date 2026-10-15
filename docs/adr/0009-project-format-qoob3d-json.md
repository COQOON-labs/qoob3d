---
id: ADR-0009
title: Project files .qoob3d are versioned JSON and contain only the feature list
status: superseded
date: 2026-10-13
crates: [qoob3d-core]
tags: [file-format, document-model]
supersedes: []
superseded_by: ADR-0030
---

## Context

Parametric CAD can recompute geometry from the feature list at any time. If
geometry is stored, it becomes redundant and can become inconsistent. Project files
also come from foreign sources and are therefore not trustworthy.

## Decision

- Format: JSON via serde, envelope `{ "format": "qoob3d", "format_version": N, "document": {…} }`.
- **Only** the document is stored (feature list with parameters), no geometry.
- When loading: **validate first, then use.** Unknown `format_version`,
  NaN/∞, negative or absurd dimensions and size limits lead to a clear error.
  Invalid data never reaches the kernel.
- Older versions are read via explicit migration steps `vN → vN+1`. For each
  version there is a fixture file in the repo, and a test ensures that it loads.
- Implementation in M3.

## Consequences

- Files are small, diffable and readable by humans and AI.
- Opening requires recomputation. For large models this costs time; a
  geometry cache would later be a pure optimization.

## Rejected alternatives

- **Binary format / storing OCCT BRep as well** – not diffable, redundancy, attack surface.
- **RON/TOML** – less common in third-party tools, JSON is sufficient.

## References

- ADR-0010, ADR-0011
