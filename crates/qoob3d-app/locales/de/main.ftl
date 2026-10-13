# QOOB3D – Deutsch (Sprachpaket, ADR-0020). Schlüssel identisch zu en/main.ftl.

## Werkzeugleiste
workspace-design = KONSTRUKTION
group-file = DATEI
file-open = 📂 Öffnen
file-save = 💾 Speichern
group-sketch = SKIZZE
sketch-create = ✏ Skizze erstellen
group-create = ERSTELLEN
create-extrude = ⬆ Extrusion
group-export = EXPORT
export-stl = STL
export-3mf = 3MF
export-step = STEP
coming-in = Kommt mit { $milestone }

## Einstellungen
settings = ⚙ Einstellungen
settings-language = Sprache
settings-left-drag-orbit = Linke Maustaste ziehen dreht (inkl. Drei-Finger-Ziehen)
settings-left-drag-orbit-tip = macOS macht aus Drei-Finger-Ziehen ein Ziehen mit der linken Maustaste. Ist das aktiv, dreht es die Ansicht wie Shift + zwei Finger.
settings-printers = Drucker
settings-only-favorites = Nur meine Drucker in der Liste zeigen
settings-printers-hint = Drucker in der Druckerliste (Navigationsleiste) mit ☆ markieren.
settings-printers-none = Noch keine Drucker markiert.

## Browser
browser = BROWSER
browser-untitled = 📄 Unbenannt
browser-origin = 📁 Ursprung
origin-xy = XY-Ebene
origin-xz = XZ-Ebene
origin-yz = YZ-Ebene
origin-x = X-Achse
origin-y = Y-Achse
origin-z = Z-Achse
browser-bodies = 📁 Körper
browser-sketches = 📁 Skizzen
browser-none = keine
browser-sketches-later = keine – kommt mit M3
body-demo = Demo-Teil
body-demo-tip = Platzhalter-Geometrie bis zum Dokumentmodell (M3)

## Zeitachse
timeline-start = Zum Anfang
timeline-back = Schritt zurück
timeline-forward = Schritt vor
timeline-end = Zum Ende
timeline-placeholder = Zeitachse – Skizzen und Extrusionen erscheinen hier (ab M3)

## Navigationsleiste
nav-fit = ⛶ Einpassen
nav-fit-tip = Alles einpassen (F6, Doppelklick mittlere Maustaste)
nav-perspective = Perspektive
nav-orthographic = Orthogonal
nav-grid = Raster
nav-volume = Bauraum
nav-printer-tip = Druckbett im Viewport
printer-search = Drucker suchen …
printers-mine = MEINE DRUCKER
printers-all = ALLE DRUCKER
printer-favorite-add = Zu meinen Druckern hinzufügen
printer-favorite-remove = Aus meinen Druckern entfernen
printers-only-mine-empty = Noch keine Drucker markiert – alle werden gezeigt. Drucker mit ☆ markieren.
printer-legend = Gelb schraffiert: nicht bedruckbar. Gelb getönt: nur mit einer Düse erreichbar.
printer-note-exclude = Enthält einen nicht bedruckbaren Bereich
printer-note-nozzles = Manche Bereiche erreicht nur eine Düse
printer-size = { $width } × { $depth } × { $height } mm

## ViewCube
viewcube-front = VORNE
viewcube-back = HINTEN
viewcube-left = LINKS
viewcube-right = RECHTS
viewcube-top = OBEN
viewcube-bottom = UNTEN
viewcube-home = Startansicht
viewcube-face-tip = Ansicht { $face }
viewcube-edge-tip = Kantenansicht
viewcube-corner-tip = Eckansicht

## Statuszeile
status-computing = Berechne Modell …
status-model = { $name }  ·  { $triangles ->
        [one] { $triangles } Dreieck
       *[other] { $triangles } Dreiecke
    }  ·  { $edges ->
        [one] { $edges } Kante
       *[other] { $edges } Kanten
    }  ·  { $tight }  ·  { $volume } cm³
status-watertight = dicht ✔
status-not-watertight = NICHT dicht ✘
status-kernel-error = Kernel-Fehler: { $error }
