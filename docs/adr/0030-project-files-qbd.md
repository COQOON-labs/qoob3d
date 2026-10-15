---
id: ADR-0030
title: Project files are called .qbd, are versioned JSON and contain only the feature list
status: accepted
date: 2026-10-13
crates: [qoob3d-core, qoob3d-app]
tags: [file-format, document-model]
supersedes: [ADR-0009]
superseded_by: null
---

## Context

ADR-0009 defined the project format with the extension `.qoob3d`. The extension is long and
awkward to type. The user wants `.qbd` ("QooB3D"). There are no files at users yet, so a
switch costs nothing now.

Use of the extension: `.qbd` is only documented as an old Quicken keyboard layout file
(Windows 3), practically dead. `.q3d` was rejected because Quest3D (a 3D tool) and
Inspect3D use it.

## Decision

- Extension: **`.qbd`** (`document::FILE_EXTENSION`). File dialogs filter by it, "Save
  As" appends it if it is missing.
- Format: JSON via serde, envelope `{ "format": "qoob3d", "format_version": N, "document": {…} }`.
  The identifier `"format": "qoob3d"` stays: files are recognizable by their content, independent
  of the name.
- **Only** the document is saved (feature list with parameters), no geometry.
- When loading: **validate first, then use.** Unknown `format_version`,
  NaN/∞, negative or absurd dimensions and size limits lead to a clear error.
  Invalid data never reaches the kernel.
- Older versions are read via explicit migration steps `vN → vN+1`. For each
  version there is a fixture file `crates/qoob3d-core/tests/fixtures/format_vN.qbd` in the repo,
  and a test ensures that it loads. The content of a fixture is never changed.
- `.qoob3d` is no longer supported (no such files exist).

## Consequences

- Files are small, diffable and readable for humans and AI.
- Opening requires a recomputation. For large models this takes time; a
  geometry cache would later be a pure optimization.
- The v1 fixture was only renamed (`format_v1.qoob3d` → `format_v1.qbd`), its content is
  unchanged.

## Rejected alternatives

- **Keep `.qoob3d`** – long, the user's wish.
- **`.q3d`** – taken in the 3D field (Quest3D, Inspect3D).
- **Binary format / also storing OCCT BRep** – not diffable, redundancy, attack surface.
- **RON/TOML** – less common in third-party tools, JSON is sufficient.

## References

- Code: `crates/qoob3d-core/src/document/mod.rs`, `crates/qoob3d-app/src/{app,project}.rs`
- Related: ADR-0011, ADR-0029
