---
id: ADR-0011
title: Profile werden über die Menge ihrer begrenzenden Entity-IDs referenziert
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-core, qoob3d-kernel]
tags: [topological-naming, skizze, extrusion]
supersedes: []
superseded_by: null
---

## Kontext

Eine Extrusion muss sich merken, welche Region einer Skizze sie extrudiert, auch nachdem
die Skizze bearbeitet wurde. Das ist eine kleine Form des Topological-Naming-Problems.

## Entscheidung

- Regionen berechnet **der Kernel** (OCCT) aus allen Kurven einer Skizze. Das schließt
  Überschneidungen und Verschachtelung ein, z. B. ergibt ein Kreis im Rechteck einen Ring
  und eine Scheibe.
- `ProfileRef = { sketch: FeatureId, region: RegionKey }`, wobei `RegionKey` die **sortierte
  Menge der Entity-IDs** ist, die die Region begrenzen.
- Existiert eine referenzierte Region nach einer Änderung nicht mehr, wird das Feature als
  Fehler markiert. **Es wird nicht geraten.**
- Körper werden über die ID des Features referenziert, das sie erzeugt hat (`BodyId`).
- „Skizze auf Körperfläche“ bleibt bis nach dem MVP draußen.

## Konsequenzen

- Robust bei Maßänderungen, weil die IDs dieselben bleiben.
- Wenn zwei Regionen dieselbe Randmenge haben, ist das mehrdeutig (selten). Dann gibt es
  einen Fehler statt einer stillen Falschwahl.

## Verworfene Alternativen

- **Region über Index** – ändert sich bei jeder Skizzenänderung.
- **Region über einen Punkt im Inneren** – fragil bei Größenänderungen.

## Verweise

- ADR-0010, ADR-0012
