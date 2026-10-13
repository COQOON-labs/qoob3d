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

## Dateien

- `src/main.rs`: Einstieg, CLI-Weiche. `src/lib.rs`: Module für Tests.
- `src/cli.rs`: Argumente (`--render`, `--view`, `--size`, `--ortho`, `--zoom`, `--smoke-test`).
- `src/app.rs`: `QoobApp` (eframe::App), Panels, Kernel-Polling, Statuszeile.
- `src/panels.rs`: Werkzeugleiste, Browser, Timeline.
- `src/viewport.rs`: Viewport-Widget, Kamera-Animation, Overlays, Navigationsleiste.
- `src/viewcube.rs`: ViewCube (reine Geometrie + Zeichnen), mit Unit-Tests.
- `src/navigation.rs`: Maus/Trackpad → `Camera`, Orbit-Pivot per Raycast (ADR-0017).
- `src/theme.rs`: Farben und egui-Stil.
- `src/i18n.rs` + `locales/`: Übersetzungen (Fluent). `src/settings.rs`: gespeicherte Einstellungen.
- `src/smoke.rs`: Smoke-Test-Skript und Watchdog. Verwirft echte Eingaben (sonst flackert der
  Test, wenn jemand die Maus über das aufpoppende Fenster bewegt).
- `src/headless_render.rs`: `--render`, Kernel + Renderer ohne Fenster → PNG.
- `tests/ui.rs`: UI-Tests der echten App, Screenshot auf Anforderung.

## Relevante ADRs

ADR-0005, ADR-0008, ADR-0013, ADR-0015, ADR-0017, ADR-0018, ADR-0019, ADR-0020, ADR-0021, ADR-0022, ADR-0023
