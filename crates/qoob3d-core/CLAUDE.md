# qoob3d-core

Shared pure data types: meshes, exports, imports, printers, script projects and their tasks. Used by
all other crates.

## Rules

- **No dependency on OCCT, wgpu, egui or eframe, not even transitively** (ADR-0036,
  checked by `cargo xtask check`). New dependencies only if they are small and pure
  (e.g. `serde`, `quick-xml`, `miniz_oxide`).
- Units: mm, Z up (ADR-0006). `TriMesh` uses `f32`.
- Everything here is testable without GPU and kernel, so thorough unit and
  property tests belong here.
- Readers of foreign bytes (`qmesh::read`, `export::zip_entries`/`unzip_stored`,
  `import::read_stl`/`read_3mf`, `font::lettering`) never panic and never allocate more than
  the input justifies.

## Files

- `src/mesh.rs`: `ShapeMesh` (faces + edges + `tints`, the colours of vertex runs, set by
  viewers from a part's `color`, never stored in the mesh file), `TriMesh` (with `face_ids`), `Polylines`,
  `Aabb`, `MeshReport` (watertight/manifold, volume; ADR-0013), `raycast` (ADR-0008).
  `welded` = grid weld (1e-4 mm) + `close_slivers` (vertices of open edges within 0.002 mm);
  `export_report` analyses that exported form (what slicers see, AG-SV-09).
- `src/export.rs`: STL, 3MF, ZIP writer and the one ZIP reader (`zip_entries`: end record,
  central directory, local headers; `unzip_stored` on top) with CRC-32 (ADR-0034).
  `export/bambu.rs`: a kit as a Bambu Studio project (AG-SV-10): plates (`plate_origin`),
  objects made of parts with a filament each, project settings that only name presets
  (`PRESETS`: a printer is listed only if the installed slicer has its three presets and
  marks them compatible; `tests/service.rs` of the app checks that against Bambu Studio).
- `src/import.rs` + `src/import/`: mesh import (AG-SC-23, ADR-0034): `ImportedMesh` (welded
  `f64` mesh in mm, `signed_volume`), `ImportError`, `MAX_TRIANGLES`; `stl.rs` binary and
  ASCII, `three_mf.rs` 3MF core + production extension (`quick-xml`, deflate via
  `miniz_oxide`): declared entry sizes at most 512 MiB together, each inflated at most to its
  declared size; triangles and component instances counted per object (iteratively, no
  recursion) before anything is expanded; mirror transforms flip the winding.
- `src/font.rs` + `src/font/`: lettering (AG-SC-38): `lettering(font bytes, text, Setting)` →
  `Lettering` (regions in mm: outline counter-clockwise, holes clockwise, `overlapping`).
  `skrifa` reads the glyphs (advance widths, `kern` pairs via `skrifa::raw`, no shaping); the size is the
  cap height (top of "H", else OS/2 capital height, else ascender); curves are flattened
  within `FLATNESS` × cap height. `regions.rs`: holes by nesting depth inside a glyph (not by
  winding), a sweep finds contours that cross (a contour crossing itself is an error, crossing
  or covered ink sets `overlapping`). Limits: `MAX_FONT_BYTES`, `MAX_CHARS`,
  `MAX_GLYPH_POINTS`, `MAX_POINTS`.
- `src/qmesh.rs`: indexed mesh transfer format service → app (WA-QM-*, ADR-0053); the reader
  checks sizes against the input before allocating.
- `src/fs.rs`: `write_atomic` (temp file + rename), the one copy for tasks and the service.
- `src/project/`: script project folder (AG-PJ-01..04, ADR-0053): `qoob3d.toml` (limits, workers
  with `recycle_mb`, `[cache] keep`, `[mesh]`; AG-PF-03..05; limits are finite and bounded,
  AG-HD-01), `values/*.toml` with mandatory source/status and optional `unit`, imports (never
  outside the project), build graph, cache key. `pose.rs` `PrintPose` (part option `pose`,
  AG-PJ-06: turns, then on the bed, centred; `apply_all` poses the meshes of one object together)
  and `arrange` (kits in rows, AG-SV-09). `plates.rs` `parse_color` (`#RRGGBB`, AG-PJ-07) and
  `plates` (footprints on plates in rows, groups never share a plate, an empty corner zone,
  AG-SV-10). `layout.rs` spells the names of the project
  (`qoob3d.toml`, the agent folders, `.qoob3d/` with `service.json`, the lock and the cache,
  `out/`); nobody else writes these literals.
- `src/tasks/`: task inbox (AG-TK-01..07, ADR-0054): `tasks/NNNN-slug.md` with TOML header, body
  and `## Log`; `create` (holds an OS lock on `tasks/` while it picks the number), `update`,
  `bind` and `apply` (`Change`: lists, state and note in one byte-preserving, atomic write;
  binding comes first, so the gate sees the new lists), role transitions, review gate
  (`gate_problems`: a task or requirement needs a part and a check bound, an idea a note).
  Log texts are fixed in the module comment. `text.rs` holds the file layout.
- `src/printer/mod.rs`: `PrinterProfile`, `BedGeometry` (ADR-0021).
- `src/printer/data.rs`: **generated** by `cargo xtask printers` from OrcaSlicer profiles –
  never edit by hand, regenerate instead.
- `tests/project.rs`, `tests/tasks.rs`, `tests/tasks_bind.rs`: project folder and task inbox.
- `tests/import.rs`: the import readers against hand-written files and the Bambu Studio
  fixture in `tests/fixtures/` (sizes and volumes in its README).

## Relevant ADRs

ADR-0006, ADR-0008, ADR-0013, ADR-0021, ADR-0034, ADR-0036, ADR-0053, ADR-0054
