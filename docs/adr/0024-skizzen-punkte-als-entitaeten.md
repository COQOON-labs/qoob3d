---
id: ADR-0024
title: Skizzen bestehen aus Punkten als eigenen Entitäten und Kurven, die Punkte referenzieren
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-core, qoob3d-sketch]
tags: [skizze, dokumentmodell]
supersedes: [ADR-0010]
superseded_by: null
---

## Kontext

ADR-0010 sah Grundformen (Linie mit eigenen Koordinaten, Rechteck, Kreis) ohne Solver vor.
Die Skizzen-Anforderungen (`docs/requirements/skizzen.md`, SK-DT-01/02, E-02) zeigen: Eine
Koinzidenz als bloße Koordinatengleichheit trägt nicht. Ein Constraint-Solver kommt in M3b
(ADR-0025), eine spätere Migration wäre teurer als ein sauberes Modell jetzt. Entschieden mit
dem Nutzer am 2026-10-13.

## Entscheidung

- Eine `Sketch` enthält **Punkte** (`SketchPoint { id, x, y, construction }`, lokale
  Koordinaten in mm, `f64`) und **Kurven**, die Punkt-IDs referenzieren:
  `Line { start, end }`, `Circle { center, radius }`. Bögen, Splines usw. folgen nach
  demselben Muster.
- Zwei Linien, die sich einen Endpunkt teilen, referenzieren **dieselbe Punkt-ID**. Fangen
  auf einen vorhandenen Punkt verwendet ihn wieder, statt einen neuen anzulegen. Damit ist
  Verbundenheit eine Beziehung und keine Toleranzfrage.
- Ein Rechteck ist kein eigener Typ, sondern 4 Punkte + 4 Linien.
- IDs (`EntityId`) zählen pro Skizze hoch und werden nie wiederverwendet.
- Bis zum Solver (M3a) sind die Koordinaten die Wahrheit. Danach sind sie der Startwert des
  Solvers. Bedingungen und Bemaßungen kommen als zusätzliche Felder mit `#[serde(default)]`
  hinzu, alte Dateien laden also ohne Migration.

## Konsequenzen

- Löschen eines Punkts löscht alle Kurven, die ihn benutzen. Löschen einer Kurve lässt
  Punkte stehen, die noch benutzt werden, und entfernt verwaiste Punkte.
- Profilschlüssel (ADR-0011) bestehen aus Kurven-IDs.

## Verworfene Alternativen

- **Grundformen weiterführen (ADR-0010)** – Verbindungen wären Toleranzvergleiche,
  Bedingungen bräuchten später eine Migration aller Dateien.

## Verweise

- `crates/qoob3d-core/src/document/sketch.rs`, `docs/requirements/skizzen.md` (SK-DT-*)
- ADR-0010 (ersetzt), ADR-0011, ADR-0025
