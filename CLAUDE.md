# QOOB3D (gesprochen „cubed“)

Parametrisches 3D-CAD im Stil von Fusion 360, native Desktop-App für macOS und Linux.
**Hauptzweck: Modelle für FDM-Drucker (z. B. Bambu Lab) konstruieren** (ADR-0002).
Im Zweifel das wählen, was den Weg „Konstruieren → Slicer → Drucken“ verbessert.

## Orientierung – zuerst lesen

| Was | Wo |
| --- | --- |
| Gesamtbild: Schichten, Threads, Datenfluss, „wo finde ich was“ | `docs/architecture.md` |
| **Alle Entscheidungen mit Begründung** (Index mit Status) | `docs/adr/README.md` |
| Theme, Farben, Layout | `docs/design/theme.md` |
| Maus-, Trackpad- und Tastenbelegung | `docs/design/bedienung.md` |
| Regeln eines Crates | `crates/<crate>/CLAUDE.md` (wird automatisch geladen) |

Stack: Rust 1.99 (Edition 2024, `rust-toolchain.toml`) · OpenCascade 7.8.1 über
`opencascade` 0.3.0 · eframe/egui 0.36.2 · wgpu 30 (nur über `qoob3d_render::wgpu`) · glam 0.34.

## Befehle

```sh
cargo run                                   # App starten
cargo xtask check                           # DEFINITION VON „FERTIG“ (fmt, clippy, test, arch, docs, deny, smoke)
cargo xtask check --fast                    # ohne smoke + deny (läuft automatisch im Stop-Hook,
                                            #   .claude/hooks/check-on-stop.sh)
cargo xtask docs [--fix]                    # Doku-Lint; --fix erzeugt den ADR-Index neu
cargo run -- --render /tmp/x.png --view iso # headless Bild rendern → mit Read ansehen
cargo run -- --smoke-test                   # echte App mit synthetischer Eingabe, Exit-Code 0/1
RUST_LOG=debug cargo run                    # mit Logging
```

In Tool-Shells ggf. vorher `export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"`
(rustup ist keg-only über Homebrew installiert).

## Arbeitsregeln (verbindlich)

1. **Fertig heißt: `cargo xtask check` ist grün.** Vorher nichts als erledigt melden.
   Visuelle Änderungen zusätzlich mit `--render` prüfen und das PNG ansehen.
2. **Entscheidungen dokumentieren:** Jede neue Architektur- oder Designentscheidung bekommt
   eine ADR (`docs/adr/0000-vorlage.md` kopieren, dann `cargo xtask docs --fix`), im selben
   Commit wie der Code. Akzeptierte ADRs nicht umschreiben, sondern durch eine neue ersetzen.
   Im Code mit `ADR-NNNN` auf die Entscheidung verweisen.
3. **Bugfix = erst ein fehlschlagender Test**, dann der Fix.
4. **Kein `#[ignore]`, kein `#[allow(…)]` ohne Begründungskommentar.** Keine Assertions
   abschwächen, um Tests grün zu bekommen.
5. **Golden Files und Snapshots nie ohne Rückfrage beim Nutzer akzeptieren.**
6. **Crate-Grenzen** (ADR-0003): `core` ohne OCCT/GPU/UI; Kernel und Renderer kennen sich nicht.
7. Nach jedem Meilenstein: Testliste für den Nutzer, Status unten aktualisieren,
   `docs/architecture.md` anpassen, wenn sich das Gesamtbild geändert hat.
8. Commits nur auf Anweisung des Nutzers.

## Konventionen

- Code, Kommentare, Commit-Messages: **Englisch**. UI-Texte und Doku (`docs/`, CLAUDE.md): **Deutsch**.
- Fehler: `thiserror` in Bibliotheken, `anyhow` nur in `qoob3d-app`. Logging mit `tracing`.
- `unsafe_code = "deny"` im ganzen Workspace. FFI ausschließlich über cxx.
- Einheiten: mm, +Z oben (ADR-0006).

## Fallstricke (workspace-weit)

Crate-spezifische Fallstricke stehen in der jeweiligen `crates/<crate>/CLAUDE.md`.

- **CMake ≥ 4** braucht `CMAKE_POLICY_VERSION_MINIMUM=3.5` (`.cargo/config.toml`). Nicht entfernen.
- **`[profile.dev.package."*"]` ändern** löst einen kompletten OCCT-Neubau aus (~4 min).
- **eframe 0.36:** Einstiegspunkt ist `App::ui(&mut self, ui, frame)`. Für synthetische
  Eingaben gibt es `App::raw_input_hook`.
- **egui-Deadlock:** in `ui.input(|i| …)` keine `Response`-/`ctx`-Aufrufe. Egui meldet den
  Deadlock in Debug-Builds erst nach 10 s, deshalb fängt `--smoke-test` (Watchdog) ihn ab.
- **C++-Exceptions aus OCCT** brechen den Prozess ab, wenn sie nicht gefangen werden (ADR-0004).

## Meilensteine

| #  | Inhalt | Status |
| --- | --- | --- |
| M1 | OCCT bauen, Würfel → Dreiecke → wgpu-Fenster, mit der Maus drehbar | ✅ fertig |
| M1.5 | Qualitäts-Harness: `xtask check`, `--render`, `--smoke-test`, `MeshReport`, cargo-deny, Stop-Hook, ADR-Struktur | ✅ fertig |
| M2 | Viewport komplett: Orbit um Cursor-Pivot, Raster mit Druckbett, Achsen, ViewCube, Kanten, Perspektive/Ortho, eigener Mesher. App-Rahmen (Toolbar, Baum, Timeline) im Theme. `egui_kittest` | offen |
| M3 | Dokumentmodell, Skizzen auf XY/XZ/YZ (Linie/Rechteck/Kreis, Maßeingabe, Fangen), Profilerkennung, Timeline/Baum, Speichern/Laden `.qoob3d` mit Validierung, Fuzzing | offen |
| M4 | Extrusion (Neuer Körper/Vereinigen/Ausschneiden), Vorschau, Rebuild-Cache | offen |
| M5 | Editierbare Timeline (Parameter, Neuberechnung ab Feature, Unterdrücken, Löschen, Rollback, Fehlerzustände), Undo/Redo | offen |
| M6 | Export **STL, 3MF**, STEP mit eigener Export-Tessellierung, Menü, zuletzt geöffnete Dateien | offen |

## Offene Punkte

- **Lizenz:** OCCT ist LGPL-2.1 und wird statisch gelinkt. Muss vor der Weitergabe von
  Binaries geklärt werden.
- **Linux-Build** ist noch nicht getestet. Benötigt CMake, C++-Compiler, Vulkan und die
  X11/Wayland/xkbcommon-Dev-Pakete.
- **Prozess-Isolation des Kernels** (Absturzschutz, untrusted STEP-Import), spätestens beim Import.
