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

- `src/worker.rs`: `Kernel`, `Request` (`DemoPart`, `Rebuild`, `Export`), `Response`,
  Worker-Schleife, `rebuild_blocking`/`export_blocking` (nur CLI/Tests). Von mehreren
  wartenden `Rebuild`s wird nur der neueste berechnet.
- `src/model.rs`: Rebuild der Features zu Körpern, Booleans (Join/Cut gegen berührte Körper),
  Hash-Ketten-Cache, Feature-Fehler (ADR-0031). Shapes als `UniquePtr<TopoDS_Shape>` nur hier.
- `tests/features.rs`: jede Form mit analytischem Volumen, Join/Cut, Fehlerfall, Cache,
  Export-Genauigkeit, v2-Fixture.
- `src/profiles.rs`: `sketch_profiles` → Regionen per General Fuse (`qoob3d_sketch_regions`
  in C++, ADR-0028). Konstruktions- und entartete Kurven zählen nicht.
- `src/tessellate.rs`: Aufruf des C++-Meshers → `ShapeMesh`, Strukturprüfung, Demo-Teil.
- `src/occt.rs` + `cpp/`: eigene cxx-Bridge. `build.rs` kompiliert sie gegen `target/OCCT`.

## Fallstricke

- Erster Build ≈ 4 min (OCCT). CMake ≥ 4 braucht `CMAKE_POLICY_VERSION_MINIMUM` (gesetzt in
  `.cargo/config.toml`).
- `Standard_Failure` leitet nicht von `std::exception` ab. cxx fängt es ohne `guarded` nicht,
  und der Prozess bricht ab.
- `BOPAlgo_Builder::Modified` ist nicht `const`; Hilfsfunktionen brauchen eine veränderliche
  Referenz.
- Die Hilfsfläche für General Fuse ist größer als alle Kurven. Regionen, die ihren Rand
  berühren (ID −1), sind Außenraum und werden verworfen.
- Neue OCCT-Operationen bekommen eine **analytische** Volumenprüfung in `tests/features.rs`.
- `BRepBuilderAPI_MakeEdge(gp_Circ, a0, a1)`: Winkel beziehen sich auf die X-Richtung der
  `gp_Ax2` – immer explizit setzen (`gp_Ax2(P, Z, X)`).
- Gedrehte Körper haben entartete Kanten an den Polen; der Mesher verwirft Dreiecke mit
  zusammenfallenden Ecken, sonst gelten sie nicht als dicht.
- Die Rotation zum Platzieren in eine Ebene: `gp_Trsf::SetDisplacement(gp_Ax3(), ziel)`.
- OCCT-Normalen folgen der Fläche, nicht der Face-Orientierung → bei `TopAbs_REVERSED`
  Wicklung und Normale drehen (macht `qoob3d_tessellate`).

## Relevante ADRs

ADR-0004, ADR-0005, ADR-0011, ADR-0012, ADR-0014, ADR-0028, ADR-0031, ADR-0032, ADR-0034
