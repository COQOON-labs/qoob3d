# QOOB3D – Deutsch (Sprachpaket, ADR-0020). Schlüssel identisch zu en/main.ftl.

## Werkzeugleiste
workspace-design = KONSTRUKTION

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
browser-none = keine
browser-parts = 📁 Teile
part-state-current = aktuell
part-state-outdated = veraltet
part-state-failed = Fehler
part-state-never-built = nie gebaut
live-unreachable = ⚠ Dienst nicht erreichbar
live-unreachable-tip = Der Projektdienst antwortet nicht; die letzten Netze bleiben sichtbar.
body-demo = Demo-Teil
browser-toggle-visibility = Ein-/Ausblenden

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
status-model = { $name }  ·  { $triangles ->
        [one] { $triangles } Dreieck
       *[other] { $triangles } Dreiecke
    }  ·  { $edges ->
        [one] { $edges } Kante
       *[other] { $edges } Kanten
    }  ·  { $tight }  ·  { $volume } cm³
status-watertight = dicht ✔
status-not-watertight = NICHT dicht ✖

## Dateien und Dialoge
dialog-error-title = Fehler
dialog-ok = OK
error-open = { $file } konnte nicht geöffnet werden: { $error }

## Projekt
group-project = PROJEKT
project-open = 📂 Öffnen
project-open-tip = Projektordner öffnen (Cmd/Strg+O)
empty-title = Kein Projekt geöffnet
empty-text = Ein Projekt ist ein Ordner mit einer qoob3d.toml. Ein KI-Agent modelliert die Teile als Skripte, dieses Fenster zeigt sie live.
empty-open = 📂 Projektordner öffnen…
status-project = { $name }  ·  { $parts ->
        [one] 1 Teil
       *[other] { $parts } Teile
    }  ·  { $current } aktuell
status-project-failed = ⚠ { $failed } mit Fehlern
