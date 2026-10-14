# QOOB3D – Deutsch (Sprachpaket, ADR-0020). Schlüssel identisch zu en/main.ftl.

## Werkzeugleiste
workspace-design = KONSTRUKTION
group-file = DATEI
file-open = 📂 Öffnen
file-save = 💾 Speichern
group-sketch = SKIZZE
sketch-create = ✏ Skizze erstellen
group-create = ERSTELLEN
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
browser-origin = 📁 Ursprung
origin-x = X-Achse
origin-y = Y-Achse
origin-z = Z-Achse
browser-bodies = 📁 Körper
browser-sketches = 📁 Skizzen
browser-none = keine
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
status-not-watertight = NICHT dicht ✖
status-kernel-error = Kernel-Fehler: { $error }

## Dateien, Verlauf, Dialoge
untitled = Unbenannt
file-new = 📄 Neu
file-new-tip = Neues Dokument (Cmd/Strg+N)
file-open-tip = .qbd-Datei öffnen (Cmd/Strg+O)
file-save-tip = Speichern (Cmd/Strg+S, Speichern unter: Shift+Cmd/Strg+S)
undo-tip = Rückgängig: { $action } (Cmd/Strg+Z)
redo-tip = Wiederholen: { $action } (Shift+Cmd/Strg+Z)
history-create-sketch = Skizze anlegen
history-sketch = Skizze bearbeiten
history-delete-feature = Feature löschen
history-visibility = Ein-/Ausblenden
edit-draw = Zeichnen
edit-move = Verschieben
edit-delete = Löschen
edit-construction = Konstruktion
dialog-unsaved-title = Ungespeicherte Änderungen
dialog-unsaved-text = Sollen die Änderungen an „{ $name }“ gespeichert werden?
dialog-save = Speichern
dialog-discard = Nicht speichern
dialog-cancel = Abbrechen
dialog-error-title = Fehler
dialog-ok = OK
error-open = { $file } konnte nicht geöffnet werden: { $error }
error-save = { $file } konnte nicht gespeichert werden: { $error }

## Skizzieren
sketch-create-tip = Skizze auf einer Ebene anlegen (S)
sketch-finish = ✔ SKIZZE BEENDEN
sketch-finish-tip = Skizze beenden (Enter ohne aktives Werkzeug)
feature-sketch-name = Skizze{ $n }
feature-edit = Bearbeiten
feature-edit-tip = Doppelklick zum Bearbeiten
feature-delete = Löschen
tool-line = ∕ Linie
tool-line-tip = Linie / Polylinie (L). Punkte klicken; am Startpunkt schließen oder Esc zum Beenden.
tool-rectangle = ⬜ Rechteck
tool-rectangle-tip = Rechteck über zwei Punkte (R)
tool-center-rectangle = ⊞ Rechteck Mitte
tool-center-rectangle-tip = Rechteck aus Mittelpunkt und Ecke
tool-circle = ○ Kreis
tool-circle-tip = Kreis aus Mittelpunkt und Durchmesser (C)
tool-point = • Punkt
tool-point-tip = Skizzenpunkt
tool-construction = … Konstruktion
tool-construction-tip = Konstruktionsgeometrie (X): gestrichelt, kein Teil von Profilen
field-length = Länge
field-angle = Winkel
field-width = Breite
field-height = Höhe
field-diameter = Durchmesser
hint-tab-enter = Wert eintippen · Tab: weiter · Enter: übernehmen · Esc: abbrechen
hint-pick-plane = Ebene für die Skizze wählen (Esc bricht ab)
origin-plane = { $plane }-Ebene
origin-plane-tip = Klicken, um auf dieser Ebene zu skizzieren
browser-toggle-visibility = Ein-/Ausblenden
palette = SKIZZENPALETTE
palette-look-at = Draufsicht
palette-snap = Fangen
palette-snap-tip = Fangen an Punkten, Mittelpunkten, Schnittpunkten und Ausrichtungen (Cmd/Strg halten setzt aus)
palette-grid-snap = Am Raster fangen
palette-profiles = Profile zeigen
palette-points = Punkte zeigen
settings-sketch-look-at = Beim Skizzieren auf die Skizzenebene blicken
status-sketch = { $name }  ·  { $curves ->
        [one] { $curves } Kurve
       *[other] { $curves } Kurven
    }  ·  { $profiles ->
        [one] { $profiles } Profil
       *[other] { $profiles } Profile
    }
status-document = { $features ->
        [0] Leeres Dokument – Skizze mit S anlegen
        [one] { $features } Feature
       *[other] { $features } Features
    }

# Körper-Features (ADR-0031)
group-construct = KONSTRUIEREN
command-extrude = Extrudieren
command-extrude-tip = Profile entlang der Skizzennormalen extrudieren (E)
command-revolve = Drehen
command-revolve-tip = Profile um eine Achse drehen (z. B. wird ein Halbkreis zur Kugel)
command-loft = Erhebung
command-loft-tip = Profile verbinden, oder ein Profil mit einem Punkt (Pyramide, Kegel)
command-plane-offset = Versetzte Ebene
command-plane-offset-tip = Konstruktionsebene parallel zu einer anderen Ebene
command-gear = Stirnrad
command-gear-tip = Evolventen-Stirnrad aus Modul und Zähnezahl
command-coil = Spirale
command-coil-tip = Schraubenfeder oder flache Spirale aus einem runden Draht
feature-extrude-name = Extrusion{ $n }
feature-revolve-name = Drehung{ $n }
feature-loft-name = Erhebung{ $n }
feature-plane-name = Ebene{ $n }
feature-gear-name = Zahnrad{ $n }
feature-coil-name = Spirale{ $n }
command-profiles = { $n ->
    [0] Profile wählen
    [one] 1 Profil
   *[other] { $n } Profile
}
command-sections = { $n ->
    [0] Profile / Punkte wählen
    [one] 1 Abschnitt
   *[other] { $n } Abschnitte
}
command-axis = Achse: { $axis }
command-axis-none = wählen
command-axis-line = Linie in { $sketch }
command-plane = Ebene
command-symmetric = Symmetrisch (mittig zur Skizze)
command-coil-type = Art
coil-helix = Schraube
coil-spiral = Flache Spirale
command-operation = Operation
op-new-body = Neuer Körper
op-join = Verbinden
op-cut = Ausschneiden
field-distance = Abstand
field-offset = Versatz
field-module = Modul
field-teeth = Zähne
field-pressure-angle = Eingriffswinkel °
field-thickness = Dicke
field-bore = Bohrung Ø
field-revolutions = Umdrehungen
field-pitch = Steigung
field-section = Draht Ø
field-radius = Radius
field-sweep = Winkel °
hint-select-profile = Ein Profil im Ansichtsfenster anklicken.
hint-select-axis = Eine Skizzenlinie anklicken oder X / Y / Z als Achse wählen.
hint-select-sections = Mindestens zwei Abschnitte anklicken (Profile oder einen Spitzenpunkt).
hint-check-values = Rot markierte Werte prüfen.
history-create-feature = Feature erstellen
history-edit-feature = Feature bearbeiten
body-name = Körper{ $n }
status-bodies = { $bodies ->
    [one] 1 Körper
   *[other] { $bodies } Körper
}  ·  { $volume } cm³  ·  { $tight }
export-stl-tip = Alle Körper als STL exportieren (ein Netz)
export-3mf-tip = Alle Körper als 3MF exportieren (ein Objekt pro Körper, für Bambu Studio / OrcaSlicer)
export-nothing = Es gibt noch keine Körper zum Exportieren.
export-done = { $n ->
    [one] 1 Körper
   *[other] { $n } Körper
} nach { $file } exportiert
tool-arc = ⤴ Bogen
tool-arc-tip = Bogen um Mittelpunkt (A): Mitte, Start, Ende (gegen den Uhrzeigersinn)
