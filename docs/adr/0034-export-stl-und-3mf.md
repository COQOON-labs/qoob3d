---
id: ADR-0034
title: Export als binäres STL und 3MF mit eigenen, abhängigkeitsfreien Writern
status: akzeptiert
date: 2026-10-14
crates: [qoob3d-core, qoob3d-kernel, qoob3d-app]
tags: [export, fdm]
supersedes: []
superseded_by: null
---

## Kontext

Ziel ist der Weg zum Drucker (ADR-0002). Bambu Studio und OrcaSlicer lesen STL und
bevorzugen 3MF (Einheiten, mehrere Objekte). Kernel-Netze haben doppelte Eckpunkte an
Flächengrenzen; Slicer prüfen Mannigfaltigkeit aber über Indizes.

## Entscheidung

- Der Kernel tesselliert für den Export feiner: `EXPORT_TESSELLATION_TOLERANCE_MM` = 0,01 mm
  (Viewport 0,05 mm), Anfrage `Request::Export`.
- `qoob3d_core::export`: `stl` (binär, alle Körper in einem Netz) und `three_mf`
  (3MF-Kernspezifikation, Einheit Millimeter, ein Objekt pro Körper, alle auf der Platte).
  Eckpunkte werden vorher verschweißt (`TriMesh::welded`), entartete Dreiecke entfernt.
- ZIP ohne Kompression mit eigenem Writer und CRC-32 (keine neue Abhängigkeit, Lizenz- und
  Lieferkettenaufwand null). Der Leser `unzip_stored` prüft lokale Header **und** das
  zentrale Verzeichnis.
- In der App: Werkzeugleiste EXPORT → STL / 3MF, Dateidialog, Hinweis in der Statuszeile.
  In der CLI: `--export`.

## Konsequenzen

- 3MF-Dateien sind größer als komprimierte, für typische FDM-Teile (wenige MB) egal.
- STEP folgt später über OCCT (Kernel), nicht über diese Writer.

## Verworfene Alternativen

- **`zip`-Crate** – zusätzliche Abhängigkeiten (Deflate, Zeit), für gespeicherte Einträge
  unnötig.
- **OCCT-STL-Writer** – kein 3MF, keine Kontrolle über Verschweißen und Objekte.

## Verweise

- Code: `crates/qoob3d-core/src/export.rs`, `crates/qoob3d-app/src/{app,batch}.rs`
- Verwandt: ADR-0002, ADR-0033
