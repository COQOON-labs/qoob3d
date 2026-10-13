---
id: ADR-0018
title: UI-Tests und Screenshots der echten App headless mit egui_kittest und wgpu
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-app, xtask]
tags: [qualität, tests, ki, ui]
supersedes: []
superseded_by: null
---

## Kontext

ADR-0013 gibt dem Agenten mit `--render` Augen für die 3D-Szene, aber nicht für die UI
(Panels, ViewCube, Navigationsleiste). Ein Farbraumfehler bei der Darstellung der
Viewport-Textur in egui (ADR-0019) war nur in einem UI-Screenshot sichtbar.

## Entscheidung

- `qoob3d-app` ist **Bibliothek + dünnes Binary**, sodass Integrationstests `QoobApp`
  direkt erzeugen können.
- `crates/qoob3d-app/tests/ui.rs` startet die **echte App** mit
  `Harness::builder().wgpu().build_eframe(...)` (egui_kittest mit den Features
  `eframe`, `wgpu`, `snapshot`). Damit laufen Kernel-Thread, Renderer und native Textur
  wie im Programm.
- Getestet wird über Accessibility-Labels (`get_by_label(...).click()`) und
  App-Zustand (`state().viewport()`).
- **Pixeltests** prüfen Eigenschaften statt Golden Images (z. B. Helligkeit der Oberseite,
  Regression für ADR-0019). Golden Images kommen erst, wenn die Optik stabil ist (ADR-0013:
  nie ohne Rückfrage akzeptieren).
- `cargo xtask screenshot [verzeichnis]` schreibt `ui.png` (ganze App) und Szenen-Renders
  (`iso`, `front`, `top`) für die visuelle Kontrolle durch Mensch und Agent.

## Konsequenzen

- UI-Änderungen sind ohne Fenster und ohne Bildschirmaufnahme-Berechtigung überprüfbar.
- Die Tests brauchen eine GPU (lokal vorhanden). Für CI ist später ein Software-Rasterizer nötig.

## Verworfene Alternativen

- **Screenshots des echten Fensters** – Berechtigungen, nicht deterministisch, kein Zugriff auf den App-Zustand.

## Verweise

- `crates/qoob3d-app/tests/ui.rs`, `crates/qoob3d-app/src/lib.rs`, `xtask/src/main.rs`
- ADR-0013, ADR-0019
