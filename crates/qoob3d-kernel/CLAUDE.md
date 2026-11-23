# qoob3d-kernel

OpenCascade connection: the script kernel (ADR-0053). A script runs, builds and measures its
shapes on the thread that called `script::run`; the app runs scripts in worker processes.

## Rules

- **OCCT types never leave this crate.** The public API consists only of types from
  `qoob3d-core`, `script::Shape` and plain data (ADR-0036).
- **Our own OCCT operations** belong in `cpp/qoob3d_occt.cpp` and the bridge `src/occt.rs`
  (ADR-0004). Wrap every function with `guarded(...)` so that `Standard_Failure` →
  `std::runtime_error` → `Err`. Results as flat buffers (`rust::Vec`), logic in Rust as far
  as possible.
- `opencascade-sys` is a fork in `third_party/` (ADR-0004), used with the feature `builtin`
  (OCCT built from source into `target/OCCT`). Every change there belongs in the fork's
  `README.md` (LGPL).
- Tests check real geometry: volume, `MeshReport::is_watertight()`, normal direction,
  number of edges (seams must not appear as edges).

## Files

- `src/occt.rs`: the cxx bridge `ffi`, `Shape` (an owned, never null `TopoDS_Shape`), `occ()`
  and `shape()` turn caught OCCT exceptions and null shapes into `KernelError`.
- `src/tessellate.rs`: call of the C++ mesher → `ShapeMesh`, structure check.
- `src/script.rs` + `src/script/`: script kernel (ADR-0053): `run`, `ScriptError`, `Shape`,
  sandboxed imports, STEP export (STL/3MF are written by the service from the cached meshes,
  tessellated with the project's `[mesh]` settings, ADR-0034);
  `api.rs` solids (named kinds `BOX`…`TORUS`), booleans (batched, plausibility AG-SC-12),
  transforms, measures, `countersink`/`counterbore`, `mass`, `overhangs`/`bed_area`;
  `profile.rs` profiles, profile algebra (AG-PF-02), `rounded_rect`, `rotate`, `offset`,
  `area`, `section`, extrude/revolve/loft, pipe; `select.rs` edge/face selectors incl.
  `in_box`/`near`/`length`/`area`, fillet, chamfer, shell; `project.rs` `part()`/`value()` with
  `PartMode` (all, declare without the CAD API, one part from dependency BREPs); `check.rs`
  `check()`/`expect…` (incl. `expect_eq`) and `part(name)`; `limits.rs` the size limits of
  declaration discovery (AG-HD-02: runs in doubling stages that measure the variables, then a search for
  the statement that crossed a limit) (AG-CK-01/02, `RunOptions::check`, results in
  `RunOutput::checks`; closures run only in `PartMode::All`); `measure.rs` area, center,
  distance, common volume for callers in Rust; BREP
  read/write and `Shape::mesh` for the cache. Runs on the calling thread, not
  the worker. Every API change updates `docs/script-api.md` (its examples run in
  `tests/script.rs` and `tests/script_helpers.rs` with the project folder `docs/script-api`; the
  imported example assets live in `docs/script-api/lib/examples/`, and `qoob3d init` copies
  them). The reference is served to agents over MCP: no repository paths, requirement IDs,
  ADR numbers or Rust notes in it – those belong here.
- `src/occt.rs` + `cpp/`: own cxx bridge. `build.rs` compiles it against the OCCT headers
  from `DEP_OCCT_INCLUDE` (set by the `opencascade-sys` fork in `third_party/opencascade-sys`
  via `links = "occt"`, ADR-0004).

## Pitfalls

- First build ≈ 4 min (OCCT). CMake ≥ 4 needs `CMAKE_POLICY_VERSION_MINIMUM` (set in
  `.cargo/config.toml`).
- `Standard_Failure` does not derive from `std::exception`. cxx does not catch it without
  `guarded`, and the process aborts.
- `BOPAlgo_Builder::Modified` is not `const`; helper functions need a mutable
  reference.
- The helper face for General Fuse is larger than all curves. Regions that touch its boundary
  (ID −1) are the outside and are discarded.
- New OCCT operations get an **analytic** volume check in `tests/script*.rs`.
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
- `BRepOffsetAPI_MakeOffset` **segfaults** (no exception, `guarded` does not help) on a face
  with concentric circular wires and on a single circle of radius ≤ 1 mm.
  `qoob3d_profile_offset` therefore offsets every wire on its own, handles full circles itself
  and combines the regions with `punch` or a planar BOP. Only worker isolation protects
  against untried outlines.
- Rhai's own size limits miss `a += a` on arrays and assignment into containers (`m[k] = v`).
  Declare mode therefore turns fast operators off and measures the variables after stages
  that double (4096, 8192, … operations, each run from the start because Rhai cannot resume):
  the total cost stays below twice the last stage (ADR-0055). Memory grown between two
  measurements is not seen there; discovery runs in a worker whose memory limit stops it.
  `Engine::on_var` would avoid the reruns but is `#[deprecated]` in Rhai and would exceed the
  lint-suppression cap (`harness/ratchet.txt`).
- The `import` literal check is textual (Rhai's AST needs the `internals` feature): only
  double-quoted literals without escapes pass. `RunOutput::modules_read` lists the modules
  the resolver actually loaded; they feed the cache key.
- `Shape::bbox()` is quantised to 1e-9 mm (−0 → 0) so that the same part reports the same
  box before and after a BREP round trip (AG-HD-11); derive `size` from the quantised values.
- `qoob3d_overhang_area` meshes a copy of the curved faces (angle 0.1 rad; 0.025 rad took 5 s
  for a sphere, 118 s for a torus), so export and display meshes stay untouched.
- STEP read/write share static OCCT settings; the wrappers serialise them with a mutex.
  OCCT's STEP translator prints progress to stdout.
- OCCT normals follow the surface, not the face orientation → for `TopAbs_REVERSED`
  flip winding and normal (done by `qoob3d_tessellate`).

## Relevant ADRs

ADR-0004, ADR-0034, ADR-0036, ADR-0004, ADR-0053, ADR-0055
