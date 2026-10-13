# Bedienung

Beschreibende Spezifikation. Entscheidungen und Begründungen: ADR-0008 (Navigation),
ADR-0015 (ViewCube, Overlays), ADR-0017 (Pivot per Raycast), ADR-0022 (Linksziehen-Orbit).

## Viewport-Navigation (1:1 Fusion 360)

| Eingabe | Aktion | Status |
| --- | --- | --- |
| Mittlere Maustaste ziehen | Pan, der Inhalt folgt dem Cursor | ✅ M1 |
| Shift + mittlere Maustaste ziehen | Orbit um den **Modellpunkt unter dem Cursor** (Mint-Punkt); neben dem Modell um das Kameraziel | ✅ M2 |
| Mausrad | Zoom auf den Cursor, vorwärts = hinein | ✅ M1 |
| Doppelklick mittlere Maustaste / F6 / „⛶ Einpassen“ | Alles einpassen (animiert) | ✅ M2 |
| Trackpad: zwei Finger wischen | Pan | ✅ M1 |
| Trackpad: Shift + zwei Finger | Orbit um den Punkt unter dem Cursor | ✅ M2 |
| Trackpad: Pinch | Zoom auf den Cursor | ✅ M1 |
| Linke Maustaste ziehen / Trackpad: drei Finger ziehen (macOS-Bedienungshilfe) | Orbit um den Punkt unter dem Cursor; abschaltbar unter ⚙ Settings | ✅ M2 |
| ViewCube: Fläche, Kante oder Ecke klicken | Ansicht aus dieser Richtung (animiert, 0,35 s). Kanten und Ecken sind je **ein** Ziel über alle Flächen | ✅ M2 |
| ViewCube: 🏠 | Startansicht (vorne-rechts-oben) + Einpassen | ✅ M2 |
| Navigationsleiste: Perspektive / Orthogonal | Projektion umschalten | ✅ M2 |
| Navigationsleiste: Raster | Raster und Achsen ein/aus (Druckbett bleibt sichtbar) | ✅ M2 |
| Navigationsleiste: Build volume | Bauraum als Drahtgitter bis zur max. Höhe | ✅ M2 |
| Navigationsleiste: Drucker | 441 Profile; Suchfeld ist beim Öffnen fokussiert; MY PRINTERS (☆ markiert) oben, ALL PRINTERS nach Hersteller; Sperrzonen ▨, Einzeldüsen-Zonen ◐ | ✅ M2 |
| ⚙ Settings → Printers | „Nur meine Drucker zeigen“, markierte Drucker entfernen (✕) | ✅ M2 |

Jede Eingabe bricht eine laufende Kamera-Animation ab.

## Cursor

| Ort / Zustand | Cursor |
| --- | --- |
| Überall (Viewport, Panels, Buttons, Labels) | Pfeil (macOS- und CAD-Konvention, präzise) |
| Über ViewCube-Regionen und dem Home-Button | Zeigende Hand (klickbare Navigationsziele) |
| Während Orbit oder Pan | Greifende Hand |
| Textfelder (z. B. Druckersuche) | Textcursor |

Labels sind nicht als Text auswählbar, deshalb erscheint dort kein Textcursor
(Test: `cursors_follow_the_policy`).

## Konstanten

`ORBIT_SPEED = 0.006 rad/pt`, `WHEEL_ZOOM_STEP = 0.88` pro Raststufe, Trackpad-Orbit-Geste endet
nach 0,25 s Pause (`crates/qoob3d-app/src/navigation.rs`). Animationsdauer `ANIM_SECONDS = 0.35`
(`viewport.rs`).
