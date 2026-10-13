# qoob3d-kernel

OpenCascade-Anbindung. Läuft vollständig in einem eigenen Worker-Thread.

## Regeln

- **OCCT-Typen verlassen dieses Crate nie.** Die öffentliche API besteht nur aus Typen aus
  `qoob3d-core` und reinen Daten (ADR-0005). Das gilt auch für `occt_glam` (glam 0.24 von
  opencascade).
- Neue Kernel-Funktionalität = neue `Request`-/`Response`-Variante, verarbeitet in
  `worker.rs::handle`.
- **Eigene OCCT-Operationen** gehören in `cpp/qoob3d_occt.cpp` und die Bridge `src/occt.rs`
  (ADR-0014). Jede Funktion mit `guarded(...)` umschließen, sodass `Standard_Failure` →
  `std::runtime_error` → `Err` wird. Ergebnisse als flache Puffer (`rust::Vec`), Logik so weit
  wie möglich in Rust.
- `opencascade` (High-Level) ist ein Fork in `third_party/opencascade`. Jede Änderung dort
  gehört in dessen `README.md` (LGPL).
- Tests prüfen echte Geometrie: Volumen, `MeshReport::is_watertight()`, Normalenrichtung,
  Anzahl der Kanten (Nähte dürfen nicht als Kante erscheinen).

## Dateien

- `src/worker.rs`: `Kernel`, `Request`, `Response`, Worker-Schleife.
- `src/tessellate.rs`: Aufruf des C++-Meshers → `ShapeMesh`, Strukturprüfung, Demo-Teil.
- `src/occt.rs` + `cpp/`: eigene cxx-Bridge. `build.rs` kompiliert sie gegen `target/OCCT`.

## Fallstricke

- Erster Build ≈ 4 min (OCCT). CMake ≥ 4 braucht `CMAKE_POLICY_VERSION_MINIMUM` (gesetzt in
  `.cargo/config.toml`).
- `Standard_Failure` leitet nicht von `std::exception` ab. cxx fängt es ohne `guarded` nicht,
  und der Prozess bricht ab.
- OCCT-Normalen folgen der Fläche, nicht der Face-Orientierung → bei `TopAbs_REVERSED`
  Wicklung und Normale drehen (macht `qoob3d_tessellate`).

## Relevante ADRs

ADR-0004, ADR-0005, ADR-0011, ADR-0012, ADR-0014
