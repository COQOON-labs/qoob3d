# qoob3d-core

Dokumentmodell und gemeinsame reine Datentypen. Wird von allen anderen Crates benutzt.

## Regeln

- **Keine Abhängigkeit auf OCCT, wgpu, egui oder eframe, auch nicht transitiv** (ADR-0036,
  geprüft von `cargo xtask check`). Neue Abhängigkeiten nur, wenn sie klein und rein sind
  (z. B. `serde`).
- Einheiten: mm, Z oben (ADR-0006). `TriMesh` nutzt `f32`, das Dokumentmodell `f64`.
- **Dateiformat (ADR-0030):** Jede Änderung an serialisierten Typen braucht eine neue
  `FORMAT_VERSION`, eine Migration in `document::migrate` und eine neue Fixture. Bestehende
  Fixtures in `tests/fixtures/` nie ändern, sie beweisen, dass alte Dateien laden.
- IDs (`FeatureId`, `EntityId`) sind stabil und werden nie wiederverwendet (Zähler im Dokument
  bzw. in der Skizze, SK-DT-03). `validate` prüft das beim Laden.
- Ebenen-Konvention: `OriginPlane::frame()` (XY: u=X, v=Y; XZ: u=X, v=Z, n=−Y; YZ: u=Y, v=Z)
  muss zu den ViewCube-Flächen passen (Oben/Vorne/Rechts).
- Alles hier ist ohne GPU und Kernel testbar, also gehören hierher gründliche Unit- und
  Property-Tests.

## Dateien

- `src/mesh.rs`: `ShapeMesh` (Flächen + Kanten), `TriMesh` (mit `face_ids`), `Polylines`,
  `Aabb`, `MeshReport` (dicht/manifold, Volumen; ADR-0013), `raycast` (ADR-0017).
- `src/document/mod.rs`: `Document`, `Feature`, `FeatureKind`, Hülle, `to_json`/`from_json`,
  `validate`, `migrate`. `src/document/sketch.rs`: `Sketch`, Punkte, Kurven, `OriginPlane`,
  `PlaneFrame` (lokal ↔ Welt, Strahl-Schnitt). ADR-0024.
- `src/expr.rs`: Ausdrücke in Eingabefeldern (`+ − * /`, Klammern, Einheiten mm/cm/m/in/um,
  Komma je nach Sprache).
- `src/profile.rs`: `Profile` (Schlüssel = sortierte Kurven-IDs, Fläche, 2D-Dreiecke; ADR-0011,
  ADR-0028).
- `src/document/feature.rs`: Körper-Features (Extrude, Revolve, Loft, Plane, SpurGear, Coil),
  `ProfileRef`, `AxisRef`, `BodyOp`, Grenzwerte (ADR-0031). Neue Felder brauchen
  `#[serde(default)]` oder eine neue Formatversion.
- `src/document/examples.rs`: Referenzmodelle wie in der UI gebaut; Quelle für `models/`
  (`cargo xtask models --write`) und `all_kinds()` (Quelle der v2-Fixture, ADR-0033).
- `src/generate.rs`: Evolventen-Stirnrad (Kontur aus Stücken), Spiralbahn, Bahnlänge (ADR-0032).
- `src/export.rs`: STL, 3MF, ZIP-Writer/-Leser mit CRC-32 (ADR-0034).
- `tests/fixtures/`: `format_v1.qbd`, `format_v2.qbd` (alle Feature- und Kurvenarten).
- `src/printer/mod.rs`: `PrinterProfile`, `BedGeometry` (ADR-0021).
- `src/printer/data.rs`: **generiert** von `cargo xtask printers` aus OrcaSlicer-Profilen –
  nie von Hand ändern, sondern neu generieren.

## Relevante ADRs

ADR-0036, ADR-0006, ADR-0030, ADR-0011, ADR-0013, ADR-0021, ADR-0017, ADR-0024, ADR-0028
