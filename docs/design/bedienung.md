# Bedienung

Beschreibende Spezifikation. Entscheidungen und Begründungen: ADR-0008 (Navigation),
ADR-0015 (ViewCube, Overlays), ADR-0017 (Pivot per Raycast), ADR-0022 (Linksziehen-Orbit),
ADR-0027 (Skizzieren), ADR-0029 (Undo), ADR-0035 (Feature-Dialoge). Vollständige Skizzen-Anforderungen:
`docs/requirements/skizzen.md`.

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
| Navigationsleiste: Drucker | 441 Profile; Suchfeld ist beim Öffnen fokussiert; MY PRINTERS (☆ markiert) oben, ALL PRINTERS nach Hersteller; Sperrzonen ▣, Einzeldüsen-Zonen ◑ | ✅ M2 |
| ⚙ Settings → Printers | „Nur meine Drucker zeigen“, markierte Drucker entfernen (×) | ✅ M2 |

Jede Eingabe bricht eine laufende Kamera-Animation ab.

## Dokument

| Eingabe | Aktion | Status |
| --- | --- | --- |
| Cmd+N / Cmd+O / Cmd+S / Shift+Cmd+S | Neu / Öffnen / Speichern / Speichern unter (`.qbd`); bei ungespeicherten Änderungen fragt ein Dialog | ✅ M3a |
| Cmd+Z / Shift+Cmd+Z, ⟲ ⟳ in der Werkzeugleiste | Rückgängig / Wiederholen. Eine Skizzensitzung ist nach dem Beenden **ein** Schritt | ✅ M3a |
| Doppelklick auf Skizze (Browser, Timeline) | Skizze bearbeiten | ✅ M3a |
| Rechtsklick auf Timeline-Eintrag | Bearbeiten / Löschen | ✅ M3a |
| 👁 im Browser | Skizze ein-/ausblenden | ✅ M3a |

## Skizzieren

| Eingabe | Aktion | Status |
| --- | --- | --- |
| S / „Create Sketch“ | Ebenenwahl: XY, XZ oder YZ anklicken (Hover hebt hervor), Esc bricht ab. Klick auf eine Ursprungsebene im Browser startet direkt | ✅ M3a |
| Eintritt | Kamera dreht animiert auf die Ebene (Look At, abschaltbar in ⚙ Settings), Raster liegt in der Skizzenebene | ✅ M3a |
| L / R / C / A | Linie (Polylinie) / Rechteck (2 Punkte) / Kreis (Mitte–Durchmesser) / Bogen (Mitte → Start = Radius → Ende = Winkel, gegen den Uhrzeigersinn). Rechteck aus Mitte und Punkt nur über die Werkzeugleiste | ✅ M3a/M4 |
| Klick | Punkt setzen. Linie: Kette läuft weiter, Klick auf den Startpunkt schließt sie, Doppelklick **auf denselben Punkt** beendet sie | ✅ M3a |
| Tippen während des Zeichnens | Wert ins aktive HUD-Feld (Länge/Winkel, Breite/Höhe, Durchmesser). Ausdrücke wie `10+5`, `2*12.5`, `1in`; Komma bei Deutsch | ✅ M3a |
| Tab | Wert übernehmen und sperren (🔒), nächstes Feld | ✅ M3a |
| Enter | Werte übernehmen und Element erzeugen; ohne Werkzeug: Skizze beenden | ✅ M3a |
| Esc | Eingabe löschen → Schritt abbrechen → Werkzeug beenden → Auswahl aufheben | ✅ M3a |
| X | Konstruktionsmodus des Werkzeugs bzw. Konstruktion der Auswahl umschalten | ✅ M3a |
| Cmd/Ctrl gedrückt | Fangen vorübergehend aus | ✅ M3a |
| Ohne Werkzeug: Klick / Shift+Klick | Auswählen / Auswahl erweitern | ✅ M3a |
| Ohne Werkzeug: Ziehen auf Punkt, Linie, Kreisrand | Verschieben; Punkt auf Punkt fallen lassen verbindet beide | ✅ M3a |
| Ohne Werkzeug: Rahmen ziehen | links→rechts: nur vollständig enthaltene, rechts→links: auch angeschnittene (gestrichelt) | ✅ M3a |
| Entf / Backspace | Auswahl löschen (Punkte nehmen ihre Kurven mit) | ✅ M3a |
| „FINISH SKETCH“ | Skizziermodus beenden | ✅ M3a |

## Körper (Modellmodus)

| Eingabe | Aktion | Status |
| --- | --- | --- |
| E / CREATE → Extrude | Extrudieren. Hat die neueste sichtbare Skizze genau ein Profil, ist es vorgewählt (E, Enter genügt) | ✅ M4 |
| CREATE → Revolve | Drehen: Profil anklicken, dann Achse (Skizzenlinie anklicken oder X / Y / Z) | ✅ M4 |
| CREATE → Loft | Erhebung: Profile und/oder einen Skizzenpunkt (Spitze) in Reihenfolge anklicken | ✅ M4 |
| CREATE → Spur Gear / Coil | Stirnrad bzw. Feder/flache Spirale, Parameter im Dialog, Ebene als Auswahl | ✅ M4 |
| CONSTRUCT → Offset Plane | Versetzte Ebene; erscheint im Browser unter Origin, Klick startet dort eine Skizze | ✅ M4 |
| Im Dialog | Klick wählt/abwählt im Viewport, Felder nehmen Ausdrücke (`10+5`, `1in`), Operation Neu/Verbinden/Ausschneiden. **Enter** = OK, **Esc** = Abbrechen (solange kein Feld den Fokus hat) | ✅ M4 |
| Doppelklick auf Feature (Timeline) | Dialog zum Bearbeiten | ✅ M4 |
| EXPORT → STL / 3MF | Alle Körper exportieren (3MF: ein Objekt pro Körper) | ✅ M4 |

Die Vorschau im Dialog ist das echte Ergebnis (der Kernel rechnet bei jeder Änderung).
Verbrauchte Skizzen werden beim OK ausgeblendet (Auge im Browser).

Im Skizziermodus orbitet Linksziehen nicht (ADR-0027); Orbit/Pan/Zoom wie oben über mittlere
Maustaste, Trackpad und ViewCube. Fangtoleranz 8 px, Pick-Toleranz 6 px.

## Cursor

| Ort / Zustand | Cursor |
| --- | --- |
| Überall (Viewport, Panels, Buttons, Labels) | Pfeil (macOS- und CAD-Konvention, präzise) |
| Über ViewCube-Regionen und dem Home-Button | Zeigende Hand (klickbare Navigationsziele) |
| Während Orbit oder Pan | Greifende Hand |
| Skizziermodus mit aktivem Werkzeug | Fadenkreuz |
| Ziehen von Skizzengeometrie | Greifende Hand |
| Textfelder (z. B. Druckersuche) | Textcursor |

Labels sind nicht als Text auswählbar, deshalb erscheint dort kein Textcursor
(Test: `cursors_follow_the_policy`).

## Konstanten

`ORBIT_SPEED = 0.006 rad/pt`, `WHEEL_ZOOM_STEP = 0.88` pro Raststufe, Trackpad-Orbit-Geste endet
nach 0,25 s Pause (`crates/qoob3d-app/src/navigation.rs`). Animationsdauer `ANIM_SECONDS = 0.35`
(`viewport.rs`).
