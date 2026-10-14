---
id: ADR-0028
title: Profile entstehen im Kernel durch Teilen einer Hilfsfläche mit allen Skizzenkurven
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-kernel, qoob3d-core]
tags: [kernel, skizze, profile]
supersedes: []
superseded_by: null
---

## Kontext

ADR-0011 legt fest, dass der Kernel die Regionen berechnet und Profile über die Menge der
begrenzenden Kurven-IDs referenziert werden. Offen war das Verfahren. Es muss
Überschneidungen und Inseln beherrschen (SK-PR-01) und ohne Raten stabile Schlüssel liefern.

## Entscheidung

- Berechnet wird im **lokalen 2D-System** der Skizze (Ebene z = 0). Die Abbildung in den
  Raum übernimmt Rust bzw. später das Extrusions-Feature (`gp_Ax3`).
- Eingabe: alle Nicht-Konstruktions-Kurven (Linien, Kreise) mit ihren IDs. Entartete Kurven
  (Länge/Radius ≈ 0) filtert Rust vorher heraus.
- Verfahren (C++, `qoob3d_sketch_regions`, exception-sicher nach ADR-0014):
  1. Eine Hilfsfläche (Rechteck um alle Kurven mit Rand) erzeugen.
  2. `BOPAlgo_Builder` (General Fuse) mit Fläche und allen Kurven-Kanten → Fläche in Teile zerlegt.
  3. Über die Historie (`Modified`/`IsSame`) jeder Teilkante ihre Kurven-ID zuordnen.
     Kanten der Hilfsfläche markieren das Äußere.
  4. Teilflächen, die eine Kante der Hilfsfläche berühren, verwerfen. Jede übrige Teilfläche
     ist ein Profil.
  5. Pro Profil: Schlüssel = sortierte Menge der Kurven-IDs, Fläche (mm²) und eine
     Triangulierung (2D) für Darstellung und Treffertest.
- Profile werden mit jedem Rebuild neu berechnet (asynchron, ADR-0005).

## Konsequenzen

- Inseln entstehen automatisch: Ein Kreis im Rechteck ergibt einen Ring (Schlüssel
  {4 Linien, Kreis}) und eine Scheibe ({Kreis}).
- Offene Konturen teilen nichts und erzeugen kein Profil (SK-PR-04 zeigt die offenen Enden).

## Verworfene Alternativen

- **Eigene 2D-Arrangement-Berechnung in Rust** – Kreis-Schnitte, Toleranzen und
  Sonderfälle sind fehleranfällig. OCCT löst genau das robust.
- **Faces direkt aus geschlossenen Wires** – scheitert an sich überschneidenden Formen.

## Verweise

- `crates/qoob3d-kernel/cpp/qoob3d_occt.cpp` (`qoob3d_sketch_regions`), `crates/qoob3d-kernel/src/profiles.rs`
- ADR-0011, ADR-0014
