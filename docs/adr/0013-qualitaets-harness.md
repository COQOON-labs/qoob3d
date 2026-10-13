---
id: ADR-0013
title: Ein Befehl definiert „fertig“ – cargo xtask check mit Smoke-Test, Render-CLI und Geometrie-Prüfungen
status: akzeptiert
date: 2026-10-13
crates: [xtask, qoob3d-app, qoob3d-core, qoob3d-render]
tags: [qualität, tests, ki, sicherheit]
supersedes: []
superseded_by: null
---

## Kontext

Der KI-Agent kann die laufende App nicht sehen. In M1 hat er die App 9 s gestartet,
„läuft“ gemeldet und dabei einen Deadlock übersehen, den egui erst nach 10 s meldet.
Qualität darf nicht von Sorgfalt abhängen, sondern muss mechanisch geprüft werden.

## Entscheidung

1. **`cargo xtask check` ist die Definition von „fertig“.** Er führt der Reihe nach aus:
   `fmt --check`, `clippy -D warnings`, `test`, Architekturregeln (ADR-0003),
   Doku-Lint (ADR-0001), `cargo deny` und den Smoke-Test. `cargo xtask check --fast`
   lässt Smoke-Test und deny weg (für Hooks).
2. **Der Agent bekommt Augen:**
   - `qoob3d --render out.png [--view iso|front|top|right] [--size WxH]` rendert headless
     ohne Fenster.
   - `qoob3d --smoke-test` startet die echte App, speist synthetische Eingaben (Orbit, Pan,
     Zoom) über den regulären Eingabepfad ein, prüft die Wirkung und beendet sich mit
     Exit-Code 0 oder 1. Ein Watchdog bricht nach 15 s ohne Frame-Fortschritt mit Fehler ab.
3. **Geometrie wird geprüft, nicht nur gezählt:** `qoob3d_core::mesh::MeshReport` liefert
   dicht/manifold, offene und nicht-manifolde Kanten sowie das vorzeichenbehaftete Volumen.
   Kernel-Tests prüfen ein exaktes Volumen und dass das Mesh dicht ist.
4. **Lieferkette:** `cargo deny` prüft Advisories, Lizenzen (LGPL nur explizit für die
   OCCT-Crates erlaubt) und Quellen (nur crates.io).
5. **Regeln für den Agenten** (stehen in der CLAUDE.md):
   - Bugfixes beginnen mit einem fehlschlagenden Test.
   - Kein `#[ignore]` und kein `#[allow(…)]` ohne Begründungskommentar.
   - Golden Files und Snapshots niemals ohne Rückfrage beim Nutzer akzeptieren.
6. **Claude-Code-Stop-Hook** führt `cargo xtask check --fast` aus, wenn sich Rust-Code geändert hat.

## Konsequenzen

- „Fertig“ ist überprüfbar. Der Agent kann Ergebnisse visuell kontrollieren
  (`--render` → PNG lesen).
- Der Smoke-Test braucht eine grafische Sitzung (lokal ok, auf Linux-CI später `xvfb`).
- Später folgen: `egui_kittest` für UI-Abläufe (M2/M3), `cargo-fuzz` für das Laden von
  Dateien (M3), `cargo-mutants` gelegentlich für die Testqualität, CI für macOS und Linux.

## Verworfene Alternativen

- **Nur manuelle Tests durch den Nutzer** – zu langsam, und Fehler fallen beim Nutzer auf.
- **Screenshots des echten Fensters** – braucht Berechtigung zur Bildschirmaufnahme und ist
  nicht deterministisch.

## Verweise

- `xtask/`, `crates/qoob3d-app/src/cli.rs`, `crates/qoob3d-app/src/smoke.rs`,
  `crates/qoob3d-core/src/mesh.rs`, `deny.toml`, `.claude/settings.json`
- ADR-0001, ADR-0003
