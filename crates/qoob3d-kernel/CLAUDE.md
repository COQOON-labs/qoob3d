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
  `in_box`/`near`/`length`/`area`, fillet, chamfer, shell; `project.rs` `part()`/`value()` (part
  options `depends`, `pose`, `color`, `print_with`; AG-PJ-06/07) with
  `PartMode` (all, declare without the CAD API, one part from dependency BREPs); `check.rs`
  `check()`/`expect…` (incl. `expect_eq`) and `part(name)`; `limits.rs` the size limits of
  declaration discovery (AG-HD-02: runs in doubling stages that measure the variables, then a search for
  the statement that crossed a limit) (AG-CK-01/02, `RunOptions::check`, results in
  `RunOutput::checks`; closures run only in `PartMode::All`); `measure.rs` area, center,
  distance, common volume for callers in Rust; BREP
  read/write and `Shape::mesh` for the cache; `polyhedron.rs` the topology for `import_mesh`
  (AG-SC-23): open/non-manifold edges counted and refused, one shell per edge-connected group;
  positive shells are solids, a negative shell is a cavity of the smallest positive shell that
  contains it (ray parity) or refused as inside out (AG-SC-37), coplanar
  neighbours grown into planar faces, their boundary chained into loops (outline first,
  holes after; at a point where a face touches itself the first side clockwise from the way
  back); `qoob3d_polyhedron` only builds it (shared vertices and edges, stored 2D curves, no
  sewing). `imports.rs` `import_step` (whole file, or solids by STEP name from the reader's
  transfer table, AG-SC-29; one read per file and run), `step_names`, `import_mesh`; both
  record what they read in `RunOutput::files_read` (AG-SC-24). `solid_ops.rs` `defects`,
  `heal`, `fill_voids`, `offset(s, d)`, `split`, `on_bed`, `heights`, `mesh_report`
  (AG-SC-26…34). `text.rs` `text` (AG-SC-38): options, the font file (confined and recorded
  like an import, read once per run), then `profile::regions` turns the polygons of
  `qoob3d_core::font` into faces: holes through the `punch` path of the profile cut (no
  boolean), one planar union only when the font module reports ink that overlaps.
  `run_keeping_prints` returns `RunFailure {error, printed}` (AG-SC-35).
  Runs on the calling thread, not
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
- **Freeform bodies (AG-SC-25, ADR-0053)**: `has_freeform` decides; cut/common/section use
  `freeform_fuzzy` (2 × largest edge tolerance, 1e-5…1e-3 mm), unions stay exact (fuzzy
  destroyed the hull + deck union), invalid results go through `repaired` (ShapeFix +
  `BRepLib::SameParameter`). **Never ShapeFix_Wireframe**: it changed slab volumes by 3 %.
  A common + cut bookkeeping check does not catch misclassified pieces: both sides agree.
  The retries (serial, then fuzzy for unions) are in `api::boolean`; `fuzzy < 0` means auto.
- **Operations must not change their operands.** Shapes share edges and faces with their copies
  (`move`), with the profile they were extruded from and with every result built from them. BOPs
  run with `SetNonDestructive` (by default OCCT adds pcurves and raises tolerances in place), and
  `repaired` (ShapeFix + SameParameter, also `heal`) works on a `BRepBuilderAPI_Copy`: in place it
  gave a face of the imported Damen hull a surface with poles near 1e100 mm, and every later boolean
  with that hull returned the whole tool. Test: `ag_sc_25_a_boolean_leaves_its_operands_as_they_were`;
  the hull case only in the opt-in acceptance test (`quinam_a_prism_cuts_the_hull_alike_twice`).
- `BRepGProp::VolumeProperties` with the fixed Gauss order was 1.1 % off on an imported hull;
  `qoob3d_volume` integrates adaptively (`VOLUME_RELATIVE_ERROR` 1e-7). With `OnlyClosed =
  true` it returns 0 for shells OCCT does not mark closed: keep it false.
- The bed and `top()`/`bottom()` tolerance is 1 µm: a planar cut face of a freeform body lay
  20 nm above the lowest point its B-spline faces reported.
- STEP names: `XSControl_TransferReader::EntityFromShapeResult` found no entity for the
  solids of our own STEP files; the names come from the `Transfer_TransientProcess` map
  (`StepRepr_RepresentationItem` → `TransferBRep::ShapeResult`, keyed by `TShape`).
- `print` is a reserved word in Rhai, also as a map key: the part option is `pose`.
- Rhai's expression depth differs between debug (32/16) and release (64/32) builds; we set
  128/64 so that nested calls in functions work in both (AG-SC-35).
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
- `qoob3d_polyhedron` runs no `BRepCheck_Analyzer`: on a 50 000-triangle sphere the check
  took 2.2 s of a 2.5 s import (and ~0.8 GB), and the build reports validity anyway. Its
  topology is checked in Rust instead.
- `BRepMesh_IncrementalMesh` costs ~60 kB per face even for faces it reuses (3 GB for an
  imported mesh of 50 000 faces). `qoob3d_polyhedron` therefore stores every face's own
  triangles as an exact triangulation (deflection 0) with the edge polygons, and
  `qoob3d_tessellate` meshes only the faces that are not planar, bounded by lines and
  exactly triangulated (`exactly_triangulated`); after a boolean only the new faces are
  meshed.
- OCCT normals follow the surface, not the face orientation → for `TopAbs_REVERSED`
  flip winding and normal (done by `qoob3d_tessellate`).

## Relevant ADRs

ADR-0004, ADR-0034, ADR-0036, ADR-0053, ADR-0055
