# qoob3d-app

eframe-Binary `qoob3d`. Hier kommen UI, Werkzeuge, Navigation und Kernel-Anbindung zusammen.

## Regeln

- **egui-Deadlock vermeiden:** innerhalb von `ui.input(|i| …)`/`ctx.input(…)` **keine**
  `Response`-Methoden und keine anderen `ctx`-Zugriffe. Den Zustand vorher in lokale
  Variablen holen.
- Kernel-Antworten immer gegen die erwartete `RequestId` prüfen und veraltete verwerfen (ADR-0005).
- Navigation genau nach `docs/design/bedienung.md` (ADR-0008). Bei Änderungen beides anpassen.
- UI-Texte deutsch. Farben aus `docs/design/theme.md`.
- Neue Interaktionen müssen über `--smoke-test` (synthetische Eingaben) mitgeprüft werden.

## Dateien

- `src/main.rs`: Einstieg, CLI-Weiche.
- `src/cli.rs`: Argumente (`--render`, `--smoke-test`, `--view`, `--size`).
- `src/app.rs`: `QoobApp` (eframe::App), Viewport-Widget, Kernel-Polling.
- `src/navigation.rs`: Maus- und Trackpad-Eingaben → `Camera`.
- `src/smoke.rs`: Smoke-Test-Skript und Watchdog.
- `src/headless_render.rs`: `--render`, Kernel + Renderer ohne Fenster → PNG.

## Relevante ADRs

ADR-0005, ADR-0007, ADR-0008, ADR-0013
