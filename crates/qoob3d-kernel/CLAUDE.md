# qoob3d-kernel

OpenCascade-Anbindung. Läuft vollständig in einem eigenen Worker-Thread.

## Regeln

- **OCCT-Typen verlassen dieses Crate nie.** Die öffentliche API besteht nur aus Typen aus
  `qoob3d-core` und reinen Daten (ADR-0005). Das gilt auch für `occt_glam` (glam 0.24 von
  opencascade).
- Neue Kernel-Funktionalität = neue `Request`-/`Response`-Variante, verarbeitet in
  `worker.rs::handle`.
- Eigene cxx-Wrapper: **C++-Exceptions immer fangen** und als `Result` zurückgeben (ADR-0004).
- Tests prüfen echte Geometrie: Volumen, Bounding-Box, `MeshReport::is_watertight()` und
  die Richtung der Normalen.

## Dateien

- `src/worker.rs`: `Kernel`, `Request`, `Response`, Worker-Schleife.
- `src/tessellate.rs`: OCCT-Shape → `TriMesh`, inklusive Korrektur der Normalen
  (opencascade-rs-Bug).

## Fallstricke

- Erster Build ≈ 4 min (OCCT). CMake ≥ 4 braucht `CMAKE_POLICY_VERSION_MINIMUM` (gesetzt in
  `.cargo/config.toml`).
- Der `Mesher` von opencascade-rs ignoriert die Face-Orientierung bei den Normalen und die
  Location-Transformation.

## Relevante ADRs

ADR-0004, ADR-0005, ADR-0011, ADR-0012
