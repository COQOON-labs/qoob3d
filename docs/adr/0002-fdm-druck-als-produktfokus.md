---
id: ADR-0002
title: QOOB3D ist primär ein Konstruktionswerkzeug für den FDM-Druck
status: akzeptiert
date: 2026-10-13
crates: []
tags: [produkt, fdm]
supersedes: []
superseded_by: null
---

## Kontext

Parametrisches CAD kann vieles bedienen (Maschinenbau, Baugruppen, Zeichnungsableitung).
Der Nutzer konstruiert vor allem Teile für FDM-Drucker wie Bambu Lab. Ohne klaren Fokus
konkurrieren Features um Priorität.

## Entscheidung

Bei Abwägungen gewinnt, was den Weg **Konstruieren → Slicer → Drucken** verbessert.
Konkret gilt:

- XY-Ebene = Druckbett, +Z = Aufbaurichtung (siehe ADR-0006).
- Exportierte Meshes müssen **dicht und manifold** sein. Das wird in Tests geprüft.
- **3MF** ist neben STL und STEP Export-Ziel erster Klasse (Bambu Studio arbeitet nativ damit).
- Die Export-Tessellierung ist getrennt von der Viewport-Tessellierung und feiner
  (Standard ~0,01 mm Sehnenabweichung).
- Der Viewport zeigt optional das Druckbett eines Druckerprofils (z. B. A1 mini 180×180,
  X1/P1/A1 256×256 mm).
- Nach dem MVP haben Vorrang: Fasen und Rundungen (Elefantenfuß), Toleranz- und
  Passungsparameter, Überhang- und Wandstärkenanalyse, Gewinde, Text prägen.

## Konsequenzen

- Zeichnungsableitung, Baugruppen und CAM haben niedrige Priorität.
- Die Qualität der Meshes ist ein Testkriterium, nicht nur die Optik.

## Verworfene Alternativen

- **Allgemeines Fusion-Klon-Ziel** – zu breit für ein MVP, kein Kriterium für Prioritäten.

## Verweise

- ADR-0006, ADR-0013
