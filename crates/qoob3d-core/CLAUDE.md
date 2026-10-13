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

- `src/mesh.rs`: `TriMesh`, `Aabb`, `MeshReport` (dicht/manifold, Volumen; ADR-0013).

## Relevante ADRs

ADR-0003, ADR-0006, ADR-0009, ADR-0010, ADR-0011, ADR-0013
