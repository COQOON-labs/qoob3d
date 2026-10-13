---
id: ADR-0021
title: Druckerdatenbank aus OrcaSlicer-Profilen generiert, mit Sperrzonen und Düsenbereichen
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-core, qoob3d-render, qoob3d-app, xtask]
tags: [fdm, drucker, viewport, daten]
supersedes: [ADR-0016]
superseded_by: null
---

## Kontext

ADR-0016 führte fünf von Hand gepflegte Drucker mit rechteckigem Bett ein. Der Nutzer wünscht
deutlich mehr Modelle und deren Besonderheiten. Manche Drucker haben Bereiche, die nicht
bedruckt werden dürfen oder können, etwa Bambu X1/P1 (Ecke vorne links 18 × 28 mm),
Qidi Q1 Pro (Aussparungen hinten) oder Dual-Düsen-Drucker wie Bambu H2D, bei denen Streifen nur
von einer Düse erreicht werden. Delta-Drucker haben runde Betten. Die Slicer-Profile von
OrcaSlicer (Fork von Bambu Studio/PrusaSlicer) enthalten genau diese Geometrie für über
400 Modelle.

## Entscheidung

- **Generierte Datenbank** `crates/qoob3d-core/src/printer/data.rs`, erzeugt von
  `cargo xtask printers <OrcaSlicer resources/profiles>`. Herkunft (Commit) steht im
  Dateikopf. **Die Datei nie von Hand bearbeiten.**
- Der Generator verarbeitet pro Hersteller `machine_model_list`, wählt die 0,4-mm-Variante
  und löst `inherits` auf (inklusive Unterordner). Übernommen werden nur Fakten:
  `printable_area` (Polygon), `bed_exclude_area` (Even-Odd-Polygon),
  `extruder_printable_area` (pro Düse) und `printable_height`. Pseudo-Hersteller
  (`Custom`, `OrcaArena`) bleiben draußen. Polygone mit zusammen mehr als 192 Punkten
  werden übersprungen und gemeldet.
- `PrinterProfile` (core) ist rein statisch. `bed()` liefert die um den Ursprung zentrierte
  `BedGeometry` (wie bisher: Slicer ordnen selbst an).
- **Darstellung im Raster-Shader** (`fs_grid`): Polygone liegen in einem Uniform-Array.
  Pro Pixel gibt es einen Even-Odd-Test (funktioniert auch für Löcher und unregelmäßige
  Formen) sowie den Abstand zur Kante für pixelbreite Ränder.
  - Druckfläche: hell mit grauem Rand.
  - **Sperrzone: gelb schraffiert** mit gelbem Rand.
  - **Einzeldüsen-Zone: gelb getönt.**
  - Optional der **Bauraum** als Drahtgitter bis zur maximalen Höhe.
- Auswahl in der Navigationsleiste mit Suchfeld, gruppiert nach Hersteller. Markierungen:
  ▨ Sperrzone, ◐ Einzeldüsen-Zonen. Der Tooltip zeigt die Maße.

## Konsequenzen

- Neue Druckermodelle kommen durch erneutes Generieren aus einer neueren OrcaSlicer-Version.
- OrcaSlicer steht unter AGPL-3.0. Übernommen werden nur Maße und Koordinaten (Fakten),
  keine Texte oder Bilder. Das kommt zur Lizenzprüfung vor der Veröffentlichung dazu
  (CLAUDE.md, offene Punkte).
- Eine Warnung „Teil liegt in Sperrzone / überragt Bauraum“ ist mit denselben Polygonen
  später eine reine Geometrieprüfung.

## Verworfene Alternativen

- **Von Hand gepflegte Liste** – veraltet schnell, Sonderzonen werden vergessen.
- **Polygone triangulieren** – scheitert an den selbstüberschneidenden Sperrzonen einiger
  Profile (z. B. Qidi). Der Even-Odd-Test im Shader ist robust und exakt.

## Verweise

- `xtask/src/printers.rs`, `crates/qoob3d-core/src/printer/`,
  `crates/qoob3d-render/src/shaders/scene.wgsl` (`fs_grid`, `poly_inside`),
  `crates/qoob3d-render/src/renderer.rs` (`BedUniform`, `build_volume_segments`)
- ADR-0002, ADR-0016 (ersetzt)
