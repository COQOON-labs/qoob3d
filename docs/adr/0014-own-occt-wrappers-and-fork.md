---
id: ADR-0014
title: Own exception-safe OCCT wrappers in the kernel crate, opencascade as a minimal fork
status: accepted
date: 2026-10-13
crates: [qoob3d-kernel]
tags: [kernel, ffi, dependencies, tessellation]
supersedes: []
superseded_by: null
---

## Context

For M2 we need a tessellation with face IDs per triangle, correctly oriented normals
and feature edges that lie exactly on the faces. The `Mesher` of opencascade-rs does not provide
this (wrong normals, no IDs, no edges). In addition, the high-level crate
`opencascade` encapsulates its OCCT handles (`pub(crate) inner`), so that own cxx wrappers cannot
reach them. ADR-0004 planned a fork for this.

## Decision

- **Own cxx bridge in the kernel crate:** `crates/qoob3d-kernel/src/occt.rs` (bridge),
  `cpp/qoob3d_occt.{hxx,cpp}` (C++). `build.rs` compiles them against the OCCT that
  `opencascade-sys` (feature `builtin`) has already built. `occt-sys` provides the include path
  as a build dependency.
- The bridge reuses the OCCT types from `opencascade-sys`
  (`type TopoDS_Shape = opencascade_sys::topo_ds::TopoDS_Shape;`). opencascade-sys stays
  **unchanged from crates.io**.
- **Every C++ function** catches `Standard_Failure` (does not derive from `std::exception`!),
  rethrows it as `std::runtime_error`, and cxx turns that into an `Err`
  (`KernelError::Occt`). Results come as flat buffers (one FFI crossing per operation).
- **`opencascade` (high-level) is forked minimally** into `third_party/opencascade`
  (`[workspace] exclude`): only `Shape::as_ffi()`/`from_ffi()` added, KiCad removed.
  The changes are listed in `third_party/opencascade/README.md` (LGPL obligation).
  This refines ADR-0004: we do not fork the whole repo, only this one pure
  Rust crate.
- `qoob3d_tessellate` meshes with `BRepMesh_IncrementalMesh`, flips winding and normals of
  reversed faces and skips seams (`BRep_Tool::IsClosed`) as well as degenerate edges.
  Edges come from `PolygonOnTriangulation`, so the points are identical to the
  mesh vertices.

## Consequences

- New kernel operations (sketch faces, extrusion, Boolean operations, export) are built
  following the same pattern and are exception-safe from the start.
- The C++ code is part of our code. It runs through `cargo build` but is not (yet) covered by
  clippy, hence: keep it short, logic in Rust.
- On Linux the link order of static libs may become relevant (not yet tested).

## Rejected alternatives

- **Fork the whole opencascade-rs repo** – more maintenance effort, we would have to maintain the
  C++ bindings as well.
- **Drop the high-level crate, use only opencascade-sys** – contradicts the desired
  stack and gives away the convenient API for sketches and Boolean operations.
- **Mesh faces one by one with the existing mesher** – boundaries are not guaranteed to fit
  together, the mesh would not be watertight.

## References

- `crates/qoob3d-kernel/{build.rs,src/occt.rs,cpp/}`, `third_party/opencascade/README.md`
- ADR-0004, ADR-0005
