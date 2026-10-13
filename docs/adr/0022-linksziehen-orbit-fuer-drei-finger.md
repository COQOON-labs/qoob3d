---
id: ADR-0022
title: Ziehen mit linker Maustaste (macOS Drei-Finger-Ziehen) dreht die Ansicht, abschaltbar
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-app]
tags: [ux, navigation, macos]
supersedes: []
superseded_by: null
---

## Kontext

Der Nutzer möchte, dass Drei-Finger-Ziehen auf dem Trackpad wie Shift + zwei Finger dreht.
macOS reicht Drei-Finger-Gesten nicht an Apps weiter. Ist in den Bedienungshilfen
„Mit drei Fingern ziehen“ aktiv (beim Nutzer: ja), erzeugt das System daraus ein normales
**Ziehen mit der linken Maustaste**. Beides ist für die App nicht unterscheidbar.

## Entscheidung

- Einstellung `left_drag_orbit` (⚙ → „Left drag orbits (incl. three-finger drag)“),
  **standardmäßig an**: Ziehen mit der linken Maustaste dreht um den Punkt unter dem Cursor
  (wie Shift + mittlere Maustaste, ADR-0017).
- Während des Ziehens zeigt der Cursor eine greifende Hand.
- ADR-0008 gilt unverändert weiter. Das ist eine zusätzliche Belegung.

## Konsequenzen

- **Offener Punkt für M3:** In Fusion zieht die linke Maustaste einen Auswahlrahmen. Kommt die
  Auswahl, muss geklärt werden, ob der Rahmen eine Taste bekommt (z. B. Alt) oder die
  Einstellung dann standardmäßig aus ist. Das wird eine neue ADR.

## Verworfene Alternativen

- **Echte Drei-Finger-Erkennung** – nicht möglich, winit/eframe erhalten keine
  Mehrfinger-Trackpad-Gesten auf macOS.

## Verweise

- `crates/qoob3d-app/src/navigation.rs`, `crates/qoob3d-app/src/settings.rs`
- ADR-0008, ADR-0017
