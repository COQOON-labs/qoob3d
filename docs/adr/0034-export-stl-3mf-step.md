---
id: ADR-0034
title: Parts are exported as binary STL and 3MF by the service from the cached mesh with our own dependency-free writers, and as STEP by OCCT in a worker; STL and 3MF are imported as polyhedral solids
status: accepted
date: 2026-11-25
crates: [qoob3d-core, qoob3d-kernel, qoob3d-project, qoob3d-app]
tags: [export, import, fdm, file-formats]
supersedes: []
superseded_by: null
---

## Context

The goal is the path to the printer (ADR-0002). Bambu Studio and OrcaSlicer read STL and
prefer 3MF (units, objects). OCCT meshes have duplicate vertices at face boundaries, while
slicers check manifoldness through indices. CAD tools that continue the work need STEP. Builds
run in worker processes and leave a cache entry `<key>.{brep,qmsh,json}` per part (ADR-0055);
the service itself never runs OCCT.

Import (2026-11-25): the second agent run stopped at a 3MF the human had of an existing part.
STL and 3MF are what slicers, MakerWorld and Bambu Studio hand out; STEP of such parts
rarely exists. Bambu Studio writes 3MF with deflated ZIP entries and the production
extension (meshes in `3D/Objects/*.model`, placed by component and build item transforms).
Agents need imported parts for measuring, fitting and collision checks, so they must be
solids the script API can cut, measure and ray-cast, not display-only meshes.

## Decision

- **Entry points:** `POST /export {"parts": [..], "format": "stl"|"3mf"|"step", "kit"?,
  "bed_mm"?, "project"?, "multicolor"?, "printer"?}`, the CLI
  `qoob3d export PARTS.. --format …` and the MCP tool `export` (ADR-0054). The service builds
  stale parts first, refuses the export if any part failed, and writes one file per part to
  `out/<part>.<format>`.
- **STL and 3MF from the cached mesh**: the service reads the part's `.qmsh`, encodes it with
  `qoob3d_core::export` and writes the file atomically; no worker, no new tessellation. The
  worker tessellated the mesh with the project's `[mesh]` settings, default
  `deflection_mm = 0.01` (well below an FDM nozzle) and `angle_deg = 20`; both are part of the
  cache key. Viewport and export therefore show the same triangles.
- **`qoob3d_core::export`**: `stl` (binary, all objects merged, header "QOOB3D binary STL,
  units mm") and `three_mf` (3MF core specification, unit millimetre, one object per mesh, all
  on the build plate). Vertices are welded first (`TriMesh::welded`, 1e-4 mm), collapsed
  triangles dropped.
- **ZIP without compression** by our own writer with CRC-32 (`zip_store`). The reader
  `unzip_stored` checks local headers **and** the central directory; it exists for tests and
  the reference gate.
- **STEP from the B-Rep**: the service sends a worker a request with the part's cached `.brep`;
  the worker reads it and writes STEP (AP214, one compound) with OCCT's `STEPControl_Writer`
  (`script::export_step`, guarded C++ wrapper, ADR-0004).

### Print pose, kits and what a slicer sees (AG-PJ-06, AG-SV-09, AG-SC-34)

- **Print pose as a part option**, not as a second part: `pose: #{ rotate, count }` is read
  at declaration time, carried by discovery, and applied by the service to the cached mesh
  when it exports STL or 3MF (`PrintPose::apply` in `qoob3d-core`: turns, then standing on
  z = 0, centred). The kit project needed 18 extra parts that only turned the first ones.
- **Kits**: `kit: name` writes all parts, each `count` times, as separate objects into one
  file, arranged in rows on a bed of `bed_mm` (default 256, 5 mm gaps). Rows beyond the bed's
  depth stay behind it; the slicer moves them to other plates. A kit carries no plates and no
  filaments; `project` (below) does.
- **The export weld closes slivers**: after the grid weld (1e-4 mm), vertices on edges that
  are still open are welded within 0.002 mm. One sliver of 0.7 µm made a 3MF non-manifold in
  Bambu Studio; vertices of a corner in neighbouring grid cells did the same.
- **Watertight means the exported form** (`TriMesh::export_report`): the build flag, the
  export details and `mesh_report` analyse the welded mesh. Counting triangles the export
  drops anyway flagged closed exports as open.

### Colours and slicer projects (AG-PJ-07, AG-SV-10)

A kit of 20 grey objects in one file that fits no bed left the real work to the human: spread
the parts over plates and assign filaments by hand. Decided 2026-11-26:

- **Colour is a part option**, `color: "#RRGGBB"`, like the pose: read at declaration time,
  reported by `/status`, drawn by the app and `render` (a tint per vertex run of the merged
  mesh, `ShapeMesh::tints`; not part of the mesh file), and one filament per distinct colour in
  a project. `print_with: "Host"` names the part it prints with as one multi-colour object.
- **`project: name` writes a Bambu Studio project**, `out/<name>.3mf`, for a `printer` of the
  printer database (default `bambu-lab-p1s`): objects in their print pose on plates of that
  bed, 5 mm apart, clear of the purge zone at the bed's front left corner.
- **Two ways to print in colour, chosen per export.** Default: every part is an object and a
  plate holds one colour, so nothing changes filaments during a print and the parts are
  joined afterwards. `multicolor`: a part with `print_with` is a part of its host's object in
  its own filament, posed with the host. Plates are grouped by the set of filaments their
  objects need, so only the plates of multi-colour objects change filaments.
- **The file is laid out as Bambu Studio writes it** (production extension, one model file per
  object, `Metadata/model_settings.config`, `Metadata/project_settings.config`) and names
  Bambu Studio as the application. What its source and its command line showed:
  - Plates, filaments and colours are read only from files whose `Application` starts with
    `BambuStudio-`; everything else is "geometry only". Version `02.00.00.00`: a newer one
    than the running slicer asks the user, an older one has settings converted.
  - Without project settings no plate is created and all objects are moved to the bed's
    centre. With them, a printer, process or filament preset is taken **from the slicer's own
    installed presets by name** as long as the file lists no differences to it. The file
    therefore names the printer preset, its default process and its default PLA
    (`export::bambu::PRESETS`) and carries none of their values. A name the slicer does not
    know would become a preset built from defaults, so only printers whose three presets
    exist and are compatible are listed (single nozzle; two nozzles need a filament map).
  - The number of filaments is the number of colours; `filament_settings_id`,
    `filament_diameter`, `filament_self_index` and `filament_extruder_variant` need one entry
    each, or the whole config is refused or part filaments fall back to 1.
  - Objects belong to the plate they lie on: plate `i` of `n` has its front left corner at
    column `i mod c`, row `i div c` with `c = ceil(sqrt(n))`, 1.2 beds apart, rows towards
    −y. The command line lays this out by the file's `printable_area`, so it is written.
  - An object whose outline reaches into the bed's exclusion zone is refused when slicing.
- **Bambu Studio is the test oracle.** Where it is installed, a service test loads a project
  of each kind with the presets of that installation (their `inherits` chains folded in the
  test), slices every plate and compares objects per plate and filaments per plate with the
  export's answer; it also checks that every listed preset exists and fits its printer. The
  command line needs no window. The first run of that test found three errors that our own
  reader had accepted: objects of the back row on no plate, an object in the exclusion zone,
  and a default filament name of the P1P that no installation has.

Rejected: a vendor-neutral 3MF with core-spec colours (Bambu Studio offers a colour dialog
for it but creates no plates); embedding preset values (the profiles are AGPL data, ADR-0021,
and would go stale); `bed_mm` for projects (the bed must be the named printer's).

### Import (AG-SC-23, AG-SC-24, AG-MC-11)

- **Script function `import_mesh(path)`** next to `import_step`, same confinement to the
  project folder. The format follows the extension (`.stl`, `.3mf`).
- **Readers in `qoob3d_core::import`**, pure Rust: STL binary and ASCII, 3MF core plus the
  production extension, nested components, transforms, units. Output is an indexed `f64`
  mesh in mm, welded at `WELD_TOLERANCE_MM`, collapsed triangles dropped. Two small
  dependencies, both already in the lock file through other crates: `miniz_oxide` (inflate)
  and `quick-xml` (pull parser). The ZIP central directory reader is our own and shared with
  `unzip_stored`. Limits against hostile files: 200 000 triangles, 512 MiB inflated in total,
  sizes checked before allocating. The 512 MiB are checked on the declared sizes of all
  entries before anything is inflated, and each entry inflates at most to its declared size,
  so a lying header cannot make the reader allocate more. Triangles and component instances
  (at most four per allowed triangle) are counted per object, without recursion, before
  anything is expanded. The start part comes from `_rels/.rels`, else `3D/3dmodel.model`.
- **Kernel: indexed mesh → polyhedral B-Rep.** One planar face per group of coplanar
  adjacent triangles, edges shared through the vertex indices, one solid per closed
  edge-manifold shell. An open or non-manifold mesh is a geometry error, never a silent shell.
  The topology (faces, their loops with holes, solids) is computed in Rust; the guarded C++
  wrapper only builds it and runs no `BRepCheck_Analyzer`, which took 2.2 s of a 2.5 s
  import of 50 000 triangles; the build reports validity as for every part. Each face keeps
  its own triangles as an exact triangulation, and tessellation meshes only faces without
  one: OCCT's mesher needed 3 GB for 50 000 planar faces. A 191 000-triangle import builds
  in 18 s with a peak of 3 GB (debug build), inside the default worker limits. A shell with no
  positive volume is a cavity of the smallest positive shell that contains one of its points
  (ray parity in a skew direction) and becomes an inner shell of that solid; one inside no
  other shell is inside out and a geometry error. Fusion 360 wrote a fused boat with two
  closed inner shells, which the first rule (every negative shell refused) could not read. Imported parts are ordinary shapes: booleans, `distance`, `ray`, `section`,
  `overhangs`.
- **Imported files are build inputs.** The worker records every file `import_step` and
  `import_mesh` read (`files_read` in the cache `.json` and the build result), and their
  bytes enter the cache key after the imported modules (cache entry version 3); a file that
  is gone hashes as empty. This also fixes STEP imports, whose changes went unnoticed before.
- **Agent access:** `file_list` reports importable files as `imports` (path, size); the file
  tools stay text-only. Reference files are put into `lib/` by the human.

## Consequences

- Exports are checked by independent tools: `unzip -t` in kernel and service tests and in
  `cargo xtask models`, Bambu Studio's loader in the reference gate when installed (ADR-0013).
- 3MF files are larger than compressed ones; irrelevant for typical FDM parts (a few MB).
- A finer export than the viewport needs a finer `[mesh]` setting for the whole project; there
  is no separate export tolerance.
- The app itself has no export command; exporting is a project operation for agent and CLI.
- Imported meshes keep their facets: a cylinder from an STL is a prism of many planar faces,
  so `fillet` on its edges and analytic radii do not apply. Booleans on large meshes are
  slow; the triangle limit keeps a build inside the worker limits.
- The import is tested against a 3MF written by Bambu Studio itself
  (`crates/qoob3d-core/tests/fixtures/`), not only against files we write.

## Rejected alternatives

- **`zip` crate** – more dependencies (deflate, time) for stored entries only. For the
  import it would bring `flate2`, time and encryption support; we only need inflate.
- **OCCT mesh readers (`RWStl`)** – produce a `Poly_Triangulation` without B-Rep faces, so
  no booleans or selections, and OCCT 7.8 has no 3MF reader at all.
- **Sewing triangle faces (`BRepBuilderAPI_Sewing`)** – finds shared edges by tolerance
  search although the welded indices already know them; slow on large meshes.
- **Imported meshes as display-only references** – an agent needs to measure and cut them.
- **OCCT STL writer** – no 3MF, no control over welding and objects.
- **Re-tessellating in a worker for export** – a second mesh that can differ from what was
  checked and shown, and a worker round trip for every export.

## References

- `crates/qoob3d-core/src/export/bambu.rs`, `crates/qoob3d-core/src/project/plates.rs`,
  `crates/qoob3d-project/src/service/slicer.rs`, `crates/qoob3d-app/tests/service.rs`
  (`ag_sv_10_*`)
- `crates/qoob3d-core/src/export.rs` (+ `export/tests.rs`), `crates/qoob3d-core/src/mesh.rs`
  (`welded`, `WELD_TOLERANCE_MM`), `crates/qoob3d-core/src/project/mod.rs` (`Mesh`)
- `crates/qoob3d-project/src/service.rs` (`export`, `export_mesh`, `export_step`),
  `crates/qoob3d-project/src/worker.rs` (`write_step`), `crates/qoob3d-app/src/service_cli.rs`
- `crates/qoob3d-kernel/src/script.rs` (`export_step`), `crates/qoob3d-kernel/cpp/`
- `crates/qoob3d-core/src/import.rs` (readers), `crates/qoob3d-kernel/src/script/api.rs`
  (`import_mesh`), `crates/qoob3d-project/src/mcp/files.rs` (`imports`)
- `docs/requirements/agent.md` (AG-SC-14, AG-PF-05, AG-SC-23, AG-SC-24, AG-MC-11)
- ADR-0002, ADR-0004, ADR-0013, ADR-0054, ADR-0055
