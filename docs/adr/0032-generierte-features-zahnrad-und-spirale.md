---
id: ADR-0032
title: Zahnrad und Spirale sind parametrische Features; ihre Geometrie entsteht in reinem Rust (core::generate)
status: akzeptiert
date: 2026-10-14
crates: [qoob3d-core, qoob3d-kernel]
tags: [features, geometrie]
supersedes: []
superseded_by: null
---

## Kontext

Zahnräder und Federn/Spiralen sind typische FDM-Teile. In Fusion entstehen sie über ein
Skript (Spur Gear) bzw. das Coil-Primitiv. Von Hand gezeichnet (Evolventen ohne Splines,
Helix ohne 3D-Skizzen) wären sie mit unseren Skizzen nicht sauber machbar, und sie sollen
später änderbar bleiben (Zähnezahl, Windungen).

## Entscheidung

- `SpurGear` (Ebene, Modul, Zähnezahl, Eingriffswinkel, Dicke, Bohrung) und `Coil` (Ebene,
  Art Schraube/flache Spirale, Durchmesser, Windungen, Steigung, Drahtdurchmesser) sind
  eigene Features mit Operation (ADR-0031).
- Die Geometrie rechnet `qoob3d_core::generate` ohne OCCT:
  - **Evolventen-Stirnrad** mit Normmaßen (Kopfhöhe m, Fußhöhe 1,25 m). Die Kontur besteht aus
    Stücken (Flanken und Kopf-/Fußbögen als Splines, radiale Linien unterhalb des
    Grundkreises), exakt verkettet.
  - **Spiralbahn**: Helix (Steigung pro Windung nach oben) oder archimedische Spirale in der
    Ebene, 36 Punkte pro Windung; `coil_length` liefert die exakte Länge für Prüfungen.
- Der Kernel baut daraus generisch Körper: `qoob3d_outline_prism` (Kontur aus Linien- und
  Spline-Stücken, Löcher, Extrusion) und `qoob3d_sweep_circle` (Spline durch die Bahn,
  runder Querschnitt, Frenet bei der Helix, feste Binormale bei der flachen Spirale).

## Konsequenzen

- Die Mathematik ist mit Unit- und Property-Tests prüfbar (geschlossene Kontur für alle
  gültigen Zahnräder, Zahndicke am Teilkreis, Bahnlänge).
- Weitere generierte Teile (Gewinde, Zahnstangen) folgen demselben Muster.

## Verworfene Alternativen

- **Zahnrad als Skizzen-Generator** (Kurven in eine Skizze schreiben) – die Parameter wären
  danach verloren; ohne Splines in Skizzen auch nur als Polygon möglich.
- **Polygonale Flanken** – jede Kante wäre im Viewport sichtbar, die Flächen facettiert.

## Verweise

- Code: `crates/qoob3d-core/src/generate.rs`, `crates/qoob3d-kernel/src/model.rs`
- Verwandt: ADR-0031
