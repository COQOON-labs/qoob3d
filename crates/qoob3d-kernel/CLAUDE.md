# qoob3d-kernel

OpenCascade connection. Runs entirely in its own worker thread.

## Rules

- **OCCT types never leave this crate.** The public API consists only of types from
  `qoob3d-core` and plain data (ADR-0005). This also applies to `occt_glam` (glam 0.24 from
  opencascade).
- New kernel functionality = new `Request`/`Response` variant, handled in
  `worker.rs::handle`.
- **Our own OCCT operations** belong in `cpp/qoob3d_occt.cpp` and the bridge `src/occt.rs`
  (ADR-0014). Wrap every function with `guarded(...)` so that `Standard_Failure` →
  `std::runtime_error` → `Err`. Results as flat buffers (`rust::Vec`), logic in Rust as far
  as possible.
- `opencascade` (high level) and `opencascade-sys` are forks in `third_party/`. Every change
  there belongs in the fork's `README.md` (LGPL).
- Tests check real geometry: volume, `MeshReport::is_watertight()`, normal direction,
  number of edges (seams must not appear as edges).

## Files

- `src/worker.rs`: `Kernel`, `Request` (`DemoPart`, `Rebuild`, `Export`), `Response`,
  worker loop, `rebuild_blocking`/`export_blocking` (CLI/tests only). Of several
  waiting `Rebuild`s only the newest is computed.
- `src/model.rs`: rebuild of the features into bodies, booleans (join/cut against touched bodies),
  hash chain cache, feature errors (ADR-0031). Shapes as `UniquePtr<TopoDS_Shape>` only here.
- `tests/features.rs`: every shape with analytic volume, join/cut, error case, cache,
  export accuracy, v2 fixture.
- `src/profiles.rs`: `sketch_profiles` → regions via General Fuse (`qoob3d_sketch_regions`
  in C++, ADR-0028). Construction and degenerate curves do not count.
- `src/tessellate.rs`: call of the C++ mesher → `ShapeMesh`, structure check, demo part.
- `src/occt.rs` + `cpp/`: own cxx bridge. `build.rs` compiles it against the OCCT headers
  from `DEP_OCCT_INCLUDE` (set by the `opencascade-sys` fork in `third_party/opencascade-sys`
  via `links = "occt"`, ADR-0047).

## Pitfalls

- First build ≈ 4 min (OCCT). CMake ≥ 4 needs `CMAKE_POLICY_VERSION_MINIMUM` (set in
  `.cargo/config.toml`).
- `Standard_Failure` does not derive from `std::exception`. cxx does not catch it without
  `guarded`, and the process aborts.
- `BOPAlgo_Builder::Modified` is not `const`; helper functions need a mutable
  reference.
- The helper face for General Fuse is larger than all curves. Regions that touch its boundary
  (ID −1) are the outside and are discarded.
- New OCCT operations get an **analytic** volume check in `tests/features.rs`.
- `BRepBuilderAPI_MakeEdge(gp_Circ, a0, a1)`: angles refer to the X direction of the
  `gp_Ax2` – always set it explicitly (`gp_Ax2(P, Z, X)`).
- Revolved bodies have degenerate edges at the poles; the mesher discards triangles with
  coincident corners, otherwise they do not count as watertight.
- The rotation for placing into a plane: `gp_Trsf::SetDisplacement(gp_Ax3(), target)`.
- OCCT normals follow the surface, not the face orientation → for `TopAbs_REVERSED`
  flip winding and normal (done by `qoob3d_tessellate`).

## Relevant ADRs

ADR-0004, ADR-0005, ADR-0011, ADR-0012, ADR-0014, ADR-0028, ADR-0031, ADR-0032, ADR-0034, ADR-0047
