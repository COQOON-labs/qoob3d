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
- `src/script.rs` + `src/script/`: script kernel (ADR-0053): `run`, `ScriptError`, `Shape`,
  sandboxed imports, STL/3MF/STEP export; `api.rs` solids, booleans (batched, plausibility
  AG-SC-12), transforms, measures; `profile.rs` profiles, profile algebra (AG-PF-02),
  extrude/revolve/loft, pipe;
  `select.rs` edge/face selectors, fillet, chamfer, shell; `project.rs` `part()`/`value()` with
  `PartMode` (all, declare without the CAD API, one part from dependency BREPs); `check.rs`
  `check()`/`expect…` and `part(name)` (AG-CK-01/02, `RunOptions::check`, results in
  `RunOutput::checks`; closures run only in `PartMode::All`); `measure.rs` area, center,
  distance, common volume for callers in Rust; BREP
  read/write and `Shape::mesh` for the cache. Runs on the calling thread, not
  the worker. Every API change updates `docs/script-api.md` (its examples run in
  `tests/script.rs`).
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
- `qoob3d_boolean` flattens compounds into argument lists: a compound of overlapping solids
  must never be one BOP argument (self-interference).
- `qoob3d_boolean` runs the BOP in parallel mode (AG-PF-01, `OSD_ThreadPool`, no TBB). On
  four cores the 1000-hole solid cut gained only ~10 %; most of a big BOP is serial.
- Profile algebra (`qoob3d_profile_boolean`): a cut whose tools are hole-free faces strictly
  inside one base face, clear of its holes and of each other, adds them as inner wires
  (`punch`) instead of running a BOP; everything else is a planar BOP. `punch` stores the
  2D curves of the edges, otherwise `BRepCheck_Analyzer` projects every edge again.
- `qoob3d_outline_face` trusts its input (closed, not self-intersecting, counter-clockwise):
  only for `rect`/`circle`/`slot`. `polygon` goes through the region search, which finds
  self-intersections.
- `BRepCheck_Analyzer` and `BRepGProp::SurfaceProperties` are slow on faces with very many
  wires (plate with 1000 holes: validity seconds, area ~0.1 s), whichever way it was built.
- STEP read/write share static OCCT settings; the wrappers serialise them with a mutex.
  OCCT's STEP translator prints progress to stdout.
- OCCT normals follow the surface, not the face orientation → for `TopAbs_REVERSED`
  flip winding and normal (done by `qoob3d_tessellate`).

## Relevant ADRs

ADR-0004, ADR-0005, ADR-0011, ADR-0012, ADR-0014, ADR-0028, ADR-0031, ADR-0032, ADR-0034, ADR-0047
