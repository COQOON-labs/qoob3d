---
id: ADR-0015
title: ViewCube und Viewport-Overlays werden mit egui gezeichnet, nur die Szene mit wgpu
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-app]
tags: [ui, rendering, viewcube]
supersedes: []
superseded_by: null
---

## Kontext

Fusion-Bedienelemente im Viewport: ViewCube (klickbare Flächen, Kanten und Ecken mit
Beschriftung), Home-Button, Navigationsleiste, Orbit-Pivot-Markierung, Statuszeile. Text auf
3D-Flächen in wgpu bräuchte einen Font-Atlas, Hit-Tests und Hover-Logik doppelt.

## Entscheidung

- Die **Szene** (Körper, Kanten, Raster, Achsen) rendert wgpu offscreen (ADR-0019).
- **Overlays** zeichnet egui darüber:
  - ViewCube (`viewcube.rs`): Würfel orthografisch mit der Kamerarotation projiziert. Jede
    sichtbare Fläche ist in 3×3 Regionen geteilt (Mitte = Fläche, Ränder = Kante,
    Ecken = Ecke; Randbreite `BAND`). Ein Klick setzt die Blickrichtung auf die Summe der
    beteiligten Normalen. Beschriftungen auf Deutsch, an die sichtbare Fläche angepasst.
  - Home-Button, Navigationsleiste (`egui::Area`, unten mittig), Pivot-Punkt, Statuszeile
    (`viewport.rs`).
- Geometrie und Treffertest des ViewCubes sind **reine Funktionen** (`regions`, `hit`,
  `face_center`) mit Unit-Tests. Gezeichnet wird in `show`.
- Kamerawechsel (ViewCube, Home, Einpassen) werden in 0,35 s animiert
  (`Camera::interpolate`: Slerp, logarithmische Distanz). Jede Benutzereingabe bricht die
  Animation ab.

## Konsequenzen

- Overlays sind über egui_kittest testbar (Labels, Klicks) und erscheinen im Screenshot (ADR-0018).
- Beschriftungen auf dem ViewCube sind gedreht, aber nicht perspektivisch geschert (anders als
  in Fusion). Bei Bedarf später als egui-Mesh mit Textur.

## Verworfene Alternativen

- **ViewCube als 3D-Objekt in wgpu** – Text, Hover und Hit-Test wären aufwendig und schlechter testbar.

## Verweise

- `crates/qoob3d-app/src/{viewcube.rs,viewport.rs}`, `docs/design/bedienung.md`
- ADR-0008, ADR-0018
