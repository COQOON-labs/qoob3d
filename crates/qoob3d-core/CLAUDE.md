# qoob3d-core

Document model and shared pure data types. Used by all other crates.

## Rules

- **No dependency on OCCT, wgpu, egui or eframe, not even transitively** (ADR-0036,
  checked by `cargo xtask check`). New dependencies only if they are small and pure
  (e.g. `serde`).
- Units: mm, Z up (ADR-0006). `TriMesh` uses `f32`, the document model `f64`.
- **File format (ADR-0030):** every change to serialized types needs a new
  `FORMAT_VERSION`, a migration in `document::migrate` and a new fixture. Never change existing
  fixtures in `tests/fixtures/`, they prove that old files load.
- IDs (`FeatureId`, `EntityId`) are stable and never reused (counter in the document
  or in the sketch, SK-DT-03). `validate` checks this on load.
- Plane convention: `OriginPlane::frame()` (XY: u=X, v=Y; XZ: u=X, v=Z, n=−Y; YZ: u=Y, v=Z)
  must match the ViewCube faces (top/front/right).
- Everything here is testable without GPU and kernel, so thorough unit and
  property tests belong here.

## Files

- `src/mesh.rs`: `ShapeMesh` (faces + edges), `TriMesh` (with `face_ids`), `Polylines`,
  `Aabb`, `MeshReport` (watertight/manifold, volume; ADR-0013), `raycast` (ADR-0017).
- `src/document/mod.rs`: `Document`, `Feature`, `FeatureKind`, envelope, `to_json`/`from_json`,
  `validate`, `migrate`. `src/document/sketch.rs`: `Sketch`, points, curves, `OriginPlane`,
  `PlaneFrame` (local ↔ world, ray intersection). ADR-0024.
- `src/expr.rs`: expressions in input fields (`+ − * /`, parentheses, units mm/cm/m/in/um,
  comma depending on language).
- `src/profile.rs`: `Profile` (key = sorted curve IDs, area, 2D triangles; ADR-0011,
  ADR-0028).
- `src/document/feature.rs`: body features (Extrude, Revolve, Loft, Plane, SpurGear, Coil),
  `ProfileRef`, `AxisRef`, `BodyOp`, limits (ADR-0031). New fields need
  `#[serde(default)]` or a new format version.
- `src/document/examples.rs`: reference models as built in the UI; source for `models/`
  (`cargo xtask models --write`) and `all_kinds()` (source of the v2 fixture, ADR-0033).
- `src/generate.rs`: involute spur gear (contour from pieces), coil path, path length (ADR-0032).
- `src/export.rs`: STL, 3MF, ZIP writer/reader with CRC-32 (ADR-0034).
- `tests/fixtures/`: `format_v1.qbd`, `format_v2.qbd` (all feature and curve kinds).
- `src/printer/mod.rs`: `PrinterProfile`, `BedGeometry` (ADR-0021).
- `src/printer/data.rs`: **generated** by `cargo xtask printers` from OrcaSlicer profiles –
  never edit by hand, regenerate instead.

## Relevant ADRs

ADR-0036, ADR-0006, ADR-0030, ADR-0011, ADR-0013, ADR-0021, ADR-0017, ADR-0024, ADR-0028
