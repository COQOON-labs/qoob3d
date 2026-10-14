---
id: ADR-0027
title: Skizziermodus – linke Maustaste zeichnet, Look-At und Fangen an, Skizzen als egui-Overlay
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-app, qoob3d-sketch]
tags: [skizze, ux, rendering]
supersedes: []
superseded_by: null
---

## Kontext

Aus den Skizzen-Anforderungen sind E-06, E-08 und E-09 zu entscheiden. Außerdem ist offen,
wie Skizzengeometrie gezeichnet wird: Linien, Punkte, gestrichelte Konstruktion, Glyphen,
Hover, Eingabefelder und Profile. Entschieden mit dem Nutzer am 2026-10-13.

## Entscheidung

- **Linke Maustaste im Skizziermodus** zeichnet, wählt und zieht. Der Linksziehen-Orbit
  (ADR-0022) ist im Skizziermodus aus. Orbit dort über Shift + zwei Finger bzw.
  Shift + mittlere Maustaste. Außerhalb des Modus gilt ADR-0022 unverändert.
- **Look-At** beim Betreten einer Skizze: an (animiert auf die Ebenennormale).
- **Fangen** ist standardmäßig an, Cmd/Ctrl unterdrückt es vorübergehend. Ein Fang auf einen
  vorhandenen Punkt verwendet dessen ID (ADR-0024).
- **Darstellung als egui-Overlay:** Skizzenkurven, Punkte, Vorschau, Fang-Glyphen,
  Eingabefelder und Profilfüllungen werden über die Kamera projiziert und mit dem
  egui-Painter gezeichnet. Profile kommen trianguliert aus dem Kernel (ADR-0028) und werden
  als egui-Mesh gefüllt.
  - Vorteile: gestrichelte Linien, Text und Glyphen ohne neue GPU-Pipelines, pixelgenaue
    Treffertests in denselben Koordinaten, sichtbar in UI-Tests und Screenshots.
  - Nachteil: keine Verdeckung durch Körper. Skizzen liegen immer obenauf, so wie Fusion
    aktive Skizzen hervorhebt. Mit Körpern (M4) wird das neu bewertet.
- Farben: unterbestimmt Mint, Konstruktion gestrichelt Bernstein-gedämpft, Vorschau
  gestrichelt Mint, Profile Mint mit 12 % (Hover 25 %), inaktive Skizzen grau
  (`docs/design/theme.md`).

## Konsequenzen

- Werkzeuge arbeiten in Skizzenkoordinaten. Pixel → Skizze per Raycast auf die Ebene
  (SK-UI-01), Skizze → Pixel über `Camera::project`.
- Fangtoleranz und Pick-Radius werden in Pixel in Skizzeneinheiten umgerechnet.

## Verworfene Alternativen

- **Skizzen über die wgpu-Linien-Pipeline** – Strichelung, Glyphen, Text und Hover wären
  zusätzlicher GPU-Code ohne Nutzen für M3a.

## Verweise

- `crates/qoob3d-app/src/sketch_ui.rs`, `crates/qoob3d-sketch/`
- ADR-0022, ADR-0024, ADR-0028, `docs/requirements/skizzen.md` (SK-UI-*, SK-VS-*, E-06/08/09)
