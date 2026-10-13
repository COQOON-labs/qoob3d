# QOOB3D – English (base language, ADR-0020).
# Every key here must exist in every other language (checked by `cargo xtask check`).

## Toolbar
workspace-design = DESIGN
group-file = FILE
file-open = 📂 Open
file-save = 💾 Save
group-sketch = SKETCH
sketch-create = ✏ Create Sketch
group-create = CREATE
create-extrude = ⬆ Extrude
group-export = EXPORT
export-stl = STL
export-3mf = 3MF
export-step = STEP
coming-in = Coming in { $milestone }

## Settings
settings = ⚙ Settings
settings-language = Language
settings-left-drag-orbit = Left drag orbits (incl. three-finger drag)
settings-left-drag-orbit-tip = macOS turns three-finger drag into a left-button drag. When enabled, it orbits like Shift + two fingers.
settings-printers = Printers
settings-only-favorites = Show only my printers in the list
settings-printers-hint = Mark printers with ☆ in the printer list (navigation bar).
settings-printers-none = No printers marked yet.

## Browser
browser = BROWSER
browser-untitled = 📄 Untitled
browser-origin = 📁 Origin
origin-xy = XY Plane
origin-xz = XZ Plane
origin-yz = YZ Plane
origin-x = X Axis
origin-y = Y Axis
origin-z = Z Axis
browser-bodies = 📁 Bodies
browser-sketches = 📁 Sketches
browser-none = none
browser-sketches-later = none – coming in M3
body-demo = Demo part
body-demo-tip = Placeholder geometry until the document model exists (M3)

## Timeline
timeline-start = Go to start
timeline-back = Step back
timeline-forward = Step forward
timeline-end = Go to end
timeline-placeholder = Timeline – sketches and extrusions appear here (from M3)

## Viewport navigation bar
nav-fit = ⛶ Fit
nav-fit-tip = Fit all (F6, double-click middle mouse button)
nav-perspective = Perspective
nav-orthographic = Orthographic
nav-grid = Grid
nav-volume = Build volume
nav-printer-tip = Build plate shown in the viewport
printer-search = Search printer…
printers-mine = MY PRINTERS
printers-all = ALL PRINTERS
printer-favorite-add = Add to my printers
printer-favorite-remove = Remove from my printers
printers-only-mine-empty = No printers marked yet – showing all. Mark printers with ☆.
printer-legend = Hatched amber: not printable. Tinted amber: reachable by one nozzle only.
printer-note-exclude = Contains a non-printable zone
printer-note-nozzles = Some areas are reachable by one nozzle only
printer-size = { $width } × { $depth } × { $height } mm

## ViewCube
viewcube-front = FRONT
viewcube-back = BACK
viewcube-left = LEFT
viewcube-right = RIGHT
viewcube-top = TOP
viewcube-bottom = BOTTOM
viewcube-home = Home view
viewcube-face-tip = { $face } view
viewcube-edge-tip = Edge view
viewcube-corner-tip = Corner view

## Status line
status-computing = Computing model…
status-model = { $name }  ·  { $triangles ->
        [one] { $triangles } triangle
       *[other] { $triangles } triangles
    }  ·  { $edges ->
        [one] { $edges } edge
       *[other] { $edges } edges
    }  ·  { $tight }  ·  { $volume } cm³
status-watertight = watertight ✔
status-not-watertight = NOT watertight ✘
status-kernel-error = Kernel error: { $error }
