---
id: ADR-0003
title: Cargo-Workspace mit vier Crates und festen Abhängigkeitsrichtungen
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-core, qoob3d-kernel, qoob3d-render, qoob3d-app]
tags: [architektur]
supersedes: []
superseded_by: null
---

## Kontext

Dokumentmodell, Geometrie-Kernel und Rendering haben sehr unterschiedliche Abhängigkeiten
(OCCT mit mehreren Minuten Build-Zeit, wgpu) und Testbarkeit. Ohne harte Grenzen sickern
Kernel- oder GPU-Typen ins Modell, und alles hängt an allem.

## Entscheidung

| Crate | Aufgabe | darf abhängen von |
| --- | --- | --- |
| `qoob3d-core` | Dokumentmodell, gemeinsame reine Datentypen (`TriMesh`, `Aabb`) | – (keine OCCT-, GPU- oder UI-Crates, auch nicht transitiv) |
| `qoob3d-kernel` | OCCT-Anbindung, Worker-Thread, Export | `core` |
| `qoob3d-render` | wgpu-Viewport, Kamera | `core` |
| `qoob3d-app` | eframe-Binary, UI, Werkzeuge, Glue | `core`, `kernel`, `render` |
| `xtask` | Entwickler-Werkzeuge (Checks, Doku-Lint) | – (kein Produkt-Crate) |

**Kernel und Renderer kennen sich nicht.** Daten zwischen ihnen fließen nur über Typen aus
`core`, vermittelt von der App. `cargo xtask check` prüft diese Regeln anhand von
`cargo metadata` – auch transitiv (z. B. darf `core` niemals `wgpu` oder `opencascade-sys`
ziehen).

## Konsequenzen

- `core` kompiliert in Sekunden und ist ohne GPU und OCCT testbar.
- Der Renderer ist ohne OCCT-Build testbar (Headless-Render-Test).
- Neue Crates oder Kanten im Abhängigkeitsgraphen brauchen eine neue ADR.

## Verworfene Alternativen

- **Ein einzelnes Crate** – Build-Zeiten, keine erzwungenen Grenzen.
- **Renderer liest Kernel-Typen direkt** – koppelt GPU-Code an OCCT, nicht Send-fähige Typen.

## Verweise

- `Cargo.toml`, `xtask/src/arch.rs`
