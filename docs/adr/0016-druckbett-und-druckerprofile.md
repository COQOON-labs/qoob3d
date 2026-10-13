---
id: ADR-0016
title: Druckbett aus Druckerprofilen, zentriert um den Ursprung, im Raster-Shader gezeichnet
status: ersetzt
date: 2026-10-13
crates: [qoob3d-core, qoob3d-render, qoob3d-app]
tags: [fdm, viewport, raster]
supersedes: []
superseded_by: ADR-0021
---

## Kontext

Für FDM-Druck (ADR-0002) muss beim Konstruieren sichtbar sein, ob ein Teil aufs Druckbett
passt. Die Druckbetten unterscheiden sich je Drucker (A1 mini 180×180, X1/P1/A1 256×256 …).

## Entscheidung

- `qoob3d_core::printer::{PrinterProfile, PRINTERS}`: fest eingebaute Profile mit
  Herstellerangaben (Bett B×T, max. Höhe). Standard: Bambu Lab P1P/P1S.
- Das Bett wird **um den Ursprung zentriert** dargestellt. Slicer ordnen Teile selbst an,
  deshalb zählt nur die Größe.
- Raster, Druckbett (Fläche und Rand) und X/Y-Achsen zeichnet **ein analytischer
  Vollbild-Shader** (`fs_grid`): Schnitt mit der Ebene z = 0, Linien per `fwidth`
  geglättet, Rasterweite in Zehnerpotenzen an den Zoom angepasst, Ausblenden zum Horizont.
  Er schreibt `frag_depth` und wird gegen die Körper getestet.
- Die Auswahl erfolgt in der Navigationsleiste. Sie wird noch nicht gespeichert (kommt mit den
  Einstellungen).

## Konsequenzen

- Das Raster ist unendlich und ohne Geometrie-Buffer. Moiré bei flachem Blick dämpft das Ausblenden.
- Eine Warnung „Teil größer als Druckbett/-höhe“ ist später eine reine Bounding-Box-Prüfung.

## Verworfene Alternativen

- **Raster als Liniengeometrie** – endlich, Aliasing, Neuaufbau beim Zoomen.
- **Bettursprung in der Ecke wie im Drucker-Koordinatensystem** – für die Konstruktion irrelevant und unübersichtlich.

## Verweise

- `crates/qoob3d-core/src/printer.rs`, `crates/qoob3d-render/src/shaders/scene.wgsl` (`fs_grid`)
- ADR-0002, ADR-0006
