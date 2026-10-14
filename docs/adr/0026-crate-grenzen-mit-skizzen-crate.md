---
id: ADR-0026
title: Cargo-Workspace mit fünf Crates – Skizzen-Logik in eigenem, reinem Crate qoob3d-sketch
status: ersetzt
date: 2026-10-13
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-sketch, qoob3d-app, xtask]
tags: [architektur]
supersedes: [ADR-0003]
superseded_by: ADR-0036
---

## Kontext

ADR-0003 legte vier Crates fest. Für Skizzen braucht es Werkzeug-Automaten, Fangen,
Treffertests und später den Solver. Diese Logik soll ohne egui, GPU und OCCT testbar sein
(SK-QA-01, E-07). Im App-Crate würde sie mit der UI verwachsen.

## Entscheidung

| Crate | Aufgabe | darf abhängen von |
| --- | --- | --- |
| `qoob3d-core` | Dokumentmodell (inkl. Skizzen-Daten), Ausdrücke, Mesh-Typen, Drucker | – (keine OCCT-, GPU- oder UI-Crates, auch nicht transitiv) |
| `qoob3d-sketch` | Skizzen-Logik: Werkzeuge (Zustandsautomaten), Fangen, Treffertest, Bearbeiten, später Solver | `core` |
| `qoob3d-kernel` | OCCT-Anbindung, Worker-Thread, Profile, Features, Export | `core` |
| `qoob3d-render` | wgpu-Viewport, Kamera | `core` |
| `qoob3d-app` | eframe-Binary, UI, Glue | `core`, `sketch`, `kernel`, `render` |
| `xtask` | Entwickler-Werkzeuge | – |

- Kernel, Renderer und `sketch` kennen sich gegenseitig nicht. Daten fließen über `core`.
- `qoob3d-sketch` darf weder OCCT noch wgpu, egui oder eframe ziehen, auch nicht transitiv
  (geprüft von `cargo xtask arch`).
- Weitere Crates oder Kanten brauchen eine neue ADR, die diese ersetzt.

## Konsequenzen

- Skizzen-Werkzeuge sind mit reinen Unit- und Property-Tests prüfbar.
- Die App übersetzt nur Eingaben (Pixel → Skizzenkoordinaten) und zeichnet.

## Verworfene Alternativen

- **Skizzen-Logik im App-Crate** – nur über UI-Tests prüfbar, vermischt sich mit egui.
- **Skizzen-Logik in core** – core soll Datenmodell bleiben. Der Solver bringt schwere Mathematik mit.

## Verweise

- `Cargo.toml`, `xtask/src/arch.rs`
- ADR-0003 (ersetzt)
