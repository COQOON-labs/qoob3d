# qoob3d-app

Bibliothek + Binary `qoob3d` (ADR-0018). Hier kommen UI, Werkzeuge, Navigation und
Kernel-Anbindung zusammen.

## Regeln

- **egui-Deadlock vermeiden:** innerhalb von `ui.input(|i| …)`/`ctx.input(…)` **keine**
  `Response`-Methoden und keine anderen `ctx`-Zugriffe. Den Zustand vorher in lokale
  Variablen holen.
- Kernel-Antworten immer gegen die erwartete `RequestId` prüfen und veraltete verwerfen (ADR-0005).
- Navigation genau nach `docs/design/bedienung.md` (ADR-0008). Bei Änderungen beides anpassen.
- Overlays (ViewCube, Navigationsleiste, Pivot) zeichnet egui, die Szene wgpu (ADR-0015).
- **Keine sichtbaren Texte im Code** (ADR-0020): immer `tr("key")` / `tr_args(…)`.
  Neue Schlüssel in **allen** `locales/*/main.ftl` anlegen (Englisch = Basis). `cargo xtask i18n`
  prüft das. Zahlen über `i18n::decimal`.
- Farben aus `theme.rs` / `docs/design/theme.md`. Funktionen späterer Meilensteine erscheinen
  deaktiviert mit Hinweis (`panels::planned`).
- Cursor-Regel (docs/design/bedienung.md): Pfeil überall, Zeigerhand nur auf dem ViewCube,
  greifende Hand beim Ziehen. Labels sind nicht auswählbar.
- Persistente Einstellungen gehören in `Settings` (serde, `#[serde(default)]`).
- Jede neue Interaktion braucht: einen UI-Test in `tests/ui.rs` (egui_kittest) und, wenn sie
  die Kamera betrifft, einen Schritt in `--smoke-test`.
- Texturen, die egui anzeigt, müssen gamma-kodierte Werte liefern (ADR-0019).
- **Dokument nur über `Project` ändern** (`edit`, `replace_sketch`, `preview_sketch` +
  `commit_preview`). Jede Änderung ist ein Undo-Schritt mit übersetztem Label; eine
  Skizzensitzung wird beim Beenden zu einem Schritt zusammengefasst (ADR-0029).
- Skizzenlogik gehört nach `qoob3d-sketch`. `sketch_ui.rs` übersetzt nur Pixel ↔ Ebene, Tasten
  und Zeichnen (ADR-0027). Toleranzen in Pixeln, umgerechnet über `View::units_per_px`.
- Werkzeugkürzel (L/R/C/A/X) feuern nie, während das HUD einen Wert annimmt.
- Feature-Dialoge (`commands.rs`, ADR-0035) ändern das Dokument nur über
  `Project::preview_doc` / `commit_preview` / `cancel_preview`. Eine eigene Vorschau-Geometrie
  gibt es nicht: Die Vorschau ist der normale Rebuild.
- Übersetzungsschlüssel immer als Literal im Code (z. B. `CommandKind::keys`), nie per
  `format!` zusammensetzen – sonst findet `cargo xtask i18n` sie nicht.

## Dateien

- `src/main.rs`: Einstieg, CLI-Weiche. `src/lib.rs`: Module für Tests.
- `src/cli.rs`: Argumente (Datei, `--demo`, `--render`, `--view`, `--size`, `--ortho`, `--zoom`,
  `--printer`, `--volume`, `--smoke-test`).
- `src/app.rs`: `QoobApp` (eframe::App), `Mode` (Modell/Ebenenwahl/Skizze), Dialoge
  (ungespeichert, Fehler), Tastenkürzel, Kernel-Polling, Statuszeile.
- `src/panels.rs`: Werkzeugleiste (Datei, Undo/Redo, Skizzenwerkzeuge, CREATE, CONSTRUCT,
  EXPORT), Browser (Ebenen, Körper, Skizzen), Timeline, Skizzenpalette. Liefert `UiAction`s,
  `app.rs` führt sie aus.
- `src/commands.rs`: Feature-Dialoge (Extrude, Revolve, Loft, Plane, Gear, Coil), Auswahl
  im Viewport, Icons, Testhilfen für Bildschirmpositionen.
- `src/batch.rs`: `--report` (JSON), `--export`, Szene für `--render` (ADR-0033).
- `src/sketch_ui.rs`: Skizziermodus (`SketchSession`: Werkzeuge, Auswahl, Ziehen,
  Rahmenauswahl, HUD), Ebenenwahl (`plane_pick`), Zeichnen inaktiver Skizzen.
- `src/project.rs`: offenes Dokument, Undo/Redo, Rebuild-Anbindung, Laden/Speichern (atomar).
  `src/history.rs`: Schnappschuss-Verlauf.
- `src/viewport.rs`: Viewport-Widget, Kamera-Animation, Overlays, Navigationsleiste.
- `src/viewcube.rs`: ViewCube (reine Geometrie + Zeichnen), mit Unit-Tests.
- `src/navigation.rs`: Maus/Trackpad → `Camera`, Orbit-Pivot per Raycast (ADR-0017).
- `src/theme.rs`: Farben und egui-Stil.
- `src/i18n.rs` + `locales/`: Übersetzungen (Fluent). `src/settings.rs`: gespeicherte Einstellungen.
- `src/smoke.rs`: Smoke-Test-Skript und Watchdog. Verwirft echte Eingaben (sonst flackert der
  Test, wenn jemand die Maus über das aufpoppende Fenster bewegt).
- `src/headless_render.rs`: `--render`, Kernel + Renderer ohne Fenster → PNG.
- `tests/ui.rs`: UI-Tests der echten App, Screenshot auf Anforderung.
- `tests/modeling.rs`: jede Form Ende-zu-Ende durch die UI, Export, Bearbeiten; Screenshots
  von Dialog und Modell (60-Hz-Schritte für Doppelklicks).

## Relevante ADRs

ADR-0005, ADR-0008, ADR-0013, ADR-0015, ADR-0017, ADR-0018, ADR-0019, ADR-0020, ADR-0021, ADR-0022, ADR-0023, ADR-0027, ADR-0029, ADR-0033, ADR-0034, ADR-0035
