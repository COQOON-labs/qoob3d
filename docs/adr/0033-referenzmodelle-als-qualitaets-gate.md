---
id: ADR-0033
title: Referenzmodelle mit analytischen Erwartungen sind ein Qualitäts-Gate; die CLI kann Dateien bauen, prüfen und exportieren
status: akzeptiert
date: 2026-10-14
crates: [qoob3d-core, qoob3d-app, xtask]
tags: [qualitaet, tests, ki]
supersedes: []
superseded_by: null
---

## Kontext

Geometrie-Fehler sieht man Unit-Tests oft nicht an: ein falsches Vorzeichen, eine nicht
geschlossene Fläche, ein Export, den der Slicer ablehnt. Die KI, die hier mitentwickelt,
braucht harte, maschinenprüfbare Wahrheiten, die nicht aus demselben Code stammen wie das
Ergebnis.

## Entscheidung

- `qoob3d_core::document::examples` baut Referenzmodelle so, wie ein Nutzer sie baut
  (Skizze → Feature). `cargo xtask models --write` schreibt sie nach `models/<name>.qbd`.
- Zu jedem Modell gehört `models/<name>.expect.json` mit **analytischen** Werten (Formel im
  Feld `why`): Körperzahl, Volumen mit Toleranz bzw. Bereich, Bounding Box.
- `cargo xtask models` (Teil von `cargo xtask check`) prüft über das gebaute Binary:
  Dateien aktuell, jede Feature- und Kurvenart in mindestens einem Modell,
  `qoob3d <datei> --report` ohne Fehler, gültige und dichte Körper, Volumen, Bounding Box,
  `--export` als 3MF (eigener Leser **und** `unzip -t`) und STL.
- Die CLI bekommt `--report` (JSON, Exit-Code 1 bei Fehlern oder nicht druckbaren Körpern)
  und `--export <stl|3mf>`; `--render` rendert Dateien statt des Demo-Körpers.
- `crates/qoob3d-app/tests/modeling.rs` baut jede Form Ende-zu-Ende durch die echte UI
  (Ebene, Werkzeuge mit getippten Werten, Dialog, OK) und vergleicht das Volumen.
- `cargo xtask screenshot` rendert jedes Modell nach `target/screenshots/model-<name>.png`.

## Konsequenzen

- **Neue Feature- oder Kurvenart = neues Referenzmodell + Erwartung**, sonst ist der Check rot.
- Erwartungen werden nie an ein Ergebnis angepasst, ohne die Formel zu prüfen (wie Golden
  Files, CLAUDE.md Regel 5).
- Der unabhängige Prüfer `unzip -t` hat beim ersten Lauf einen Fehler im eigenen ZIP-Writer
  gefunden, den der eigene Leser übersehen hatte. Unabhängige Prüfer sind deshalb Pflicht,
  wo es sie gibt.

## Verworfene Alternativen

- **Snapshot-Vergleich von Netzen** – bricht bei jeder Tessellierungsänderung, sagt nichts
  über Richtigkeit.
- **Nur Kernel-Tests** – prüfen weder Dateiformat noch CLI noch Export.

## Verweise

- Code: `xtask/src/models.rs`, `crates/qoob3d-app/src/batch.rs`, `models/`
- Verwandt: ADR-0013, ADR-0018, ADR-0031, ADR-0034, ADR-0036
