# Bedienung

Beschreibende Spezifikation. Entscheidung und Begründung: ADR-0008.

## Viewport-Navigation (1:1 Fusion 360)

| Eingabe | Aktion | Status |
| --- | --- | --- |
| Mittlere Maustaste ziehen | Pan, der Inhalt folgt dem Cursor | ✅ M1 |
| Shift + mittlere Maustaste ziehen | Orbit (Turntable um Z) | ✅ M1 (um die Bildmitte); M2: um den Punkt unter dem Cursor |
| Mausrad | Zoom auf den Cursor, vorwärts = hinein | ✅ M1 |
| Doppelklick mittlere Maustaste / F6 | Alles einpassen | ✅ M1 |
| Trackpad: zwei Finger wischen | Pan | ✅ M1 |
| Trackpad: Shift + zwei Finger | Orbit | ✅ M1 |
| Trackpad: Pinch | Zoom auf den Cursor | ✅ M1 |
| ViewCube: Fläche, Kante oder Ecke klicken | Ansicht mit Übergang | M2 |

## Konstanten

`ORBIT_SPEED = 0.006 rad/pt`, `WHEEL_ZOOM_STEP = 0.88` pro Raststufe
(`crates/qoob3d-app/src/navigation.rs`).
