---
id: ADR-0008
title: Kamera als Orbit-Kamera mit Quaternion, Navigation 1:1 wie Fusion 360
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-render, qoob3d-app]
tags: [ux, navigation, kamera]
supersedes: []
superseded_by: null
---

## Kontext

Der Nutzer arbeitet mit Fusion 360 und wünscht dessen Bedienung 1:1. Mac-Nutzer
arbeiten oft mit einem Trackpad ohne mittlere Maustaste.

## Entscheidung

- **Kamera** (`Camera`): `target`, `distance`, `rotation: Quat` (kamera-lokal: x rechts,
  y oben, z nach hinten) und `fov_y`. Der Orbit ist ein Turntable um Welt-Z plus eine
  Neigung um die Kamera-Rechtsachse, ohne Begrenzung (man kann über den Pol drehen).
- **Belegung** (`qoob3d-app/src/navigation.rs`):

  | Eingabe | Aktion |
| --- | --- |
  | Mittlere Maustaste ziehen | Pan |
  | Shift + mittlere Maustaste ziehen | Orbit |
  | Mausrad (vorwärts = hinein) | Zoom auf den Punkt unter dem Cursor |
  | Doppelklick mittlere Maustaste / F6 | Einpassen |
  | Trackpad: zwei Finger | Pan |
  | Trackpad: Shift + zwei Finger | Orbit |
  | Trackpad: Pinch | Zoom auf den Cursor |

- Mausrad und Trackpad werden über `MouseWheelUnit::Line`/`Page` bzw. `Point` unterschieden.
- Ab M2: Der Orbit dreht um den Punkt unter dem Cursor, wenn dort Geometrie ist
  (`Camera::orbit_around`), wie in Fusion.

## Konsequenzen

- Mäuse mit Smooth-Scrolling (Magic Mouse) senden `Point` und verhalten sich wie ein Trackpad.
- Die Linke Maustaste bleibt frei für Auswahl und Werkzeuge.

## Verworfene Alternativen

- **Linke Maustaste = Orbit (Blender/Onshape-Stil)** – widerspricht dem Fusion-Wunsch und
  kollidiert mit der Auswahl.
- **Yaw/Pitch-Winkel statt Quaternion** – ViewCube-Ansichten und Übergänge sind mit
  Quaternionen einfacher.

## Verweise

- `crates/qoob3d-render/src/camera.rs`, `crates/qoob3d-app/src/navigation.rs`
- `docs/design/bedienung.md`
