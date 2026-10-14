---
id: ADR-0025
title: Constraint-Solver kommt in den MVP, Reihenfolge M3a → M4 → M3b → M3c
status: akzeptiert
date: 2026-10-13
crates: []
tags: [planung, skizze, solver]
supersedes: []
superseded_by: null
---

## Kontext

Der ursprüngliche MVP schloss einen Constraint-Solver aus. Die Skizzen-Anforderungen
(Abschnitt 16/17, E-03, E-10) zeigen, dass parametrisches Skizzieren ohne Bedingungen
provisorisch bleibt. Gleichzeitig soll früh ein druckbares Teil entstehen. Entschieden mit
dem Nutzer am 2026-10-13.

## Entscheidung

- **M3a:** Dokumentmodell, Speichern/Laden, Skizzenmodus auf XY/XZ/YZ, Linie/Polylinie,
  Rechteck, Kreis, Punkt, Konstruktion, Werteingabe am Cursor, Fangen, Auswahl/Ziehen/Löschen,
  Profile mit Inseln, Undo/Redo, Timeline/Browser.
- **M4:** Extrusion (Neuer Körper/Vereinigen/Ausschneiden).
- **M3b:** Constraint-Solver, Bedingungen, Bemaßungen, Benutzerparameter, Bogen, Langloch,
  Polygon, Trimmen/Verlängern, Abrunden, Versatz, Spiegeln.
- **M3c:** Konstruktionsebenen (versetzt, unter Winkel, 3 Punkte, Mittelebene), Skizzen auf
  Flächen, Projizieren, Slice.
- Die Wahl des Solvers (E-01) fällt in einem kurzen Spike vor M3b: eigener Löser in Rust
  gegen planegcs (LGPL). Das Ergebnis wird eine eigene ADR.

## Konsequenzen

- Nach M3a + M4 lassen sich einfache Teile drucken.
- M5/M6 (Timeline-Bearbeitung, Export) bleiben danach. Die Meilenstein-Tabelle in der
  CLAUDE.md ist entsprechend angepasst.

## Verworfene Alternativen

- **Solver nach dem MVP** – Skizzen wären bis dahin nur eingeschränkt änderbar.
- **M3 komplett vor M4** – das erste druckbare Teil käme deutlich später.

## Verweise

- `docs/requirements/skizzen.md` (Abschnitt 16, 17)
- ADR-0024
