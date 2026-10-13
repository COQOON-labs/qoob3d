---
id: ADR-0010
title: Skizzen bestehen im MVP aus Grundformen mit eigenen Maßen, ohne Constraint-Solver
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-core]
tags: [skizze, dokumentmodell]
supersedes: []
superseded_by: null
---

## Kontext

Ein Constraint-Solver ist ausdrücklich nicht im MVP. Trotzdem sollen Maße nachträglich
editierbar sein, z. B. „Rechteckbreite = 40“.

## Entscheidung

- `SketchEntity` ∈ { `Line {a, b}`, `Rect {corner, width, height}`, `Circle {center, radius}` },
  jeweils mit stabiler `EntityId`. Maße sind vom Typ `Length` (Newtype, mm).
- Linien verbinden sich über Fangen an gleichen Koordinaten (Toleranz), nicht über geteilte
  Punkt-IDs.
- Die Skizzenebene ist im MVP eine von XY, XZ und YZ.

## Konsequenzen

- Rechteck und Kreis bleiben als Einheit editierbar, einfach und ohne Solver.
- Mit dem Solver folgt `format_version` 2: aus einem Rechteck werden 4 Linien, 4 Punkte und
  Constraints (horizontal, vertikal, Maß). Die Migration ist eindeutig.
- `Length` als Newtype erlaubt später Ausdrücke und Benutzerparameter ohne Umbau.

## Verworfene Alternativen

- **Schon jetzt Punkte + Constraints ohne Solver** – nachträgliches Editieren wäre ohne
  Solver umständlich (Punkte einzeln verschieben).

## Verweise

- ADR-0009, ADR-0011
