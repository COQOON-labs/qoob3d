# qoob3d-sketch

Skizzen-Logik ohne UI, GPU und Kernel (ADR-0036): Werkzeuge, Fangen, Treffertest,
Bearbeiten, ab M3b der Solver. Anforderungen: `docs/requirements/skizzen.md` (IDs `SK-…`).

## Regeln

- **Rein:** nur `qoob3d-core` und Mathe-Crates. Kein egui, wgpu, eframe, OCCT, auch nicht
  transitiv (`cargo xtask arch`).
- Alles in **Skizzenkoordinaten** (mm, `f64`, lokales System der Ebene). Pixel kennt dieses
  Crate nicht: Toleranzen kommen in Skizzeneinheiten herein.
- Punkte sind Entitäten (ADR-0024). Fangen auf einen Punkt liefert dessen ID, Werkzeuge
  verwenden sie wieder statt neue Punkte anzulegen.
- Werkzeuge sind Zustandsautomaten (`tools::Tool`). Neue Werkzeuge bekommen Tests für Ablauf,
  Escape, Werteingabe und entartete Eingaben.
- Texte (Feldnamen) sind nur Schlüssel (`field-length` …). Übersetzt wird in der App (ADR-0020).

## Dateien

- `src/tools.rs`: Linie/Polylinie, Rechteck (2 Punkte, Mitte), Kreis, Bogen (Mitte → Start →
  Ende, Felder nacheinander), Punkt.
- `src/snap.rs`: Fangen und Ausrichtung mit Priorität.
- `src/pick.rs`: Treffertest, Fenster- und Kreuzungsauswahl.
- `src/edit.rs`: Ziehen, Punkte zusammenführen, Konstruktion umschalten.
- `src/geom.rs`: Schnittpunkte (auch Bögen), nächste Punkte, Kreis-/Bogen-Polylinie.
- Bögen: Das Endpunkt-Entity muss auf dem Kreis liegen (`Sketch::repair_arcs` nach dem
  Ziehen), sonst lehnt `validate` die Datei ab.

## Relevante ADRs

ADR-0024, ADR-0025, ADR-0036, ADR-0027
