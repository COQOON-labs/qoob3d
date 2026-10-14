---
id: ADR-0036
title: Crate-Grenzen – wie ADR-0026, zusätzlich darf xtask den reinen qoob3d-core nutzen
status: akzeptiert
date: 2026-10-14
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-sketch, qoob3d-app, xtask]
tags: [architektur, qualitaet]
supersedes: [ADR-0026]
superseded_by: null
---

## Kontext

ADR-0026 erlaubte `xtask` keine Workspace-Abhängigkeiten. Das Referenzmodell-Gate
(ADR-0033) muss aber die Beispielmodelle aus `qoob3d_core::document::examples` schreiben,
ihre Abdeckung aller Feature-Arten prüfen und exportierte 3MF-Dateien lesen. Das geht nur mit
dem Dokumentmodell. `qoob3d-core` ist rein (kein OCCT, keine GPU), xtask bleibt also schnell
gebaut.

## Entscheidung

| Crate | Aufgabe | darf abhängen von |
| --- | --- | --- |
| `qoob3d-core` | Dokumentmodell (inkl. Skizzen-Daten, Features), Ausdrücke, Mesh-Typen, Generatoren (Zahnrad, Spirale), Export-Writer, Drucker | – (keine OCCT-, GPU- oder UI-Crates, auch nicht transitiv) |
| `qoob3d-sketch` | Skizzen-Logik: Werkzeuge (Zustandsautomaten), Fangen, Treffertest, Bearbeiten, später Solver | `core` |
| `qoob3d-kernel` | OCCT-Anbindung, Worker-Thread, Profile, Features, Export-Netze | `core` |
| `qoob3d-render` | wgpu-Viewport, Kamera | `core` |
| `qoob3d-app` | eframe-Binary, UI, Glue, CLI (`--report`, `--export`, `--render`) | `core`, `sketch`, `kernel`, `render` |
| `xtask` | Entwickler-Werkzeuge, Qualitäts-Gates | `core` |

- Kernel, Renderer und `sketch` kennen sich gegenseitig nicht. Daten fließen über `core`.
- `qoob3d-core` und `qoob3d-sketch` dürfen weder OCCT noch wgpu, egui oder eframe ziehen,
  auch nicht transitiv (geprüft von `cargo xtask arch`).
- Was Geometrie braucht, prüft `xtask` über das gebaute Binary (`qoob3d … --report`), nicht
  über eine Abhängigkeit auf den Kernel.
- Weitere Crates oder Kanten brauchen eine neue ADR, die diese ersetzt.

## Konsequenzen

- Skizzen-Werkzeuge sind mit reinen Unit- und Property-Tests prüfbar.
- Gates in xtask testen die echte Anwendung von außen, so wie ein Skript oder die KI sie
  benutzt.

## Verworfene Alternativen

- **xtask hängt vom Kernel ab** – OCCT würde ein zweites Mal gelinkt, und das Gate prüfte nicht
  das ausgelieferte Binary.
- **Beispielmodelle als handgeschriebene JSON-Dateien** – driften vom Code weg; so sind sie
  Code, getestet, und die Dateien werden daraus erzeugt.

## Verweise

- `Cargo.toml`, `xtask/src/arch.rs`, `xtask/src/models.rs`
- ADR-0026 (ersetzt), ADR-0033
