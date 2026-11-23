# qoob3d-core

Shared pure data types: meshes, exports, printers, script projects and their tasks. Used by
all other crates.

## Rules

- **No dependency on OCCT, wgpu, egui or eframe, not even transitively** (ADR-0036,
  checked by `cargo xtask check`). New dependencies only if they are small and pure
  (e.g. `serde`).
- Units: mm, Z up (ADR-0006). `TriMesh` uses `f32`.
- Everything here is testable without GPU and kernel, so thorough unit and
  property tests belong here.
- Readers of foreign bytes (`qmesh::read`, `export::unzip_stored`) never panic and never
  allocate more than the input justifies.

## Files

- `src/mesh.rs`: `ShapeMesh` (faces + edges), `TriMesh` (with `face_ids`), `Polylines`,
  `Aabb`, `MeshReport` (watertight/manifold, volume; ADR-0013), `raycast` (ADR-0008).
- `src/export.rs`: STL, 3MF, ZIP writer/reader with CRC-32 (ADR-0034).
- `src/qmesh.rs`: indexed mesh transfer format service → app (WA-QM-*, ADR-0053); the reader
  checks sizes against the input before allocating.
- `src/fs.rs`: `write_atomic` (temp file + rename), the one copy for tasks and the service.
- `src/project/`: script project folder (AG-PJ-01..04, ADR-0053): `qoob3d.toml` (limits, workers
  with `recycle_mb`, `[cache] keep`, `[mesh]`; AG-PF-03..05; limits are finite and bounded,
  AG-HD-01), `values/*.toml` with mandatory source/status and optional `unit`, imports (never
  outside the project), build graph, cache key. `layout.rs` spells the names of the project
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

## Relevant ADRs

ADR-0036, ADR-0006, ADR-0013, ADR-0008, ADR-0021, ADR-0034, ADR-0053, ADR-0054
