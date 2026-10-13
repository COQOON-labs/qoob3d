# qoob3d-core

Dokumentmodell und gemeinsame reine Datentypen. Wird von allen anderen Crates benutzt.

## Regeln

- **Keine Abhängigkeit auf OCCT, wgpu, egui oder eframe, auch nicht transitiv** (ADR-0003,
  geprüft von `cargo xtask check`). Neue Abhängigkeiten nur, wenn sie klein und rein sind
  (z. B. `serde`).
- Einheiten: mm, Z oben (ADR-0006). `TriMesh` nutzt `f32`, das Dokumentmodell später `f64`.
- Alles hier ist ohne GPU und Kernel testbar, also gehören hierher gründliche Unit- und
  Property-Tests.

## Dateien

- `src/mesh.rs`: `ShapeMesh` (Flächen + Kanten), `TriMesh` (mit `face_ids`), `Polylines`,
  `Aabb`, `MeshReport` (dicht/manifold, Volumen; ADR-0013), `raycast` (ADR-0017).
- `src/printer/mod.rs`: `PrinterProfile`, `BedGeometry` (ADR-0021).
- `src/printer/data.rs`: **generiert** von `cargo xtask printers` aus OrcaSlicer-Profilen –
  nie von Hand ändern, sondern neu generieren.

## Relevante ADRs

ADR-0003, ADR-0006, ADR-0009, ADR-0010, ADR-0011, ADR-0013, ADR-0021, ADR-0017
