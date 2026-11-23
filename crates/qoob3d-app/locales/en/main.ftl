# QOOB3D – English (base language, ADR-0020).
# Every key here must exist in every other language (checked by `cargo xtask check`).

## Toolbar
workspace-design = DESIGN

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
browser-none = none
browser-parts = 📁 Parts
part-state-current = up to date
part-state-outdated = outdated
part-state-failed = error
part-state-never-built = never built
live-unreachable = ⚠ service not reachable
live-unreachable-tip = The project service does not answer; the last meshes stay visible.
body-demo = Demo part
browser-toggle-visibility = Show/hide

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
status-model = { $name }  ·  { $triangles ->
        [one] { $triangles } triangle
       *[other] { $triangles } triangles
    }  ·  { $edges ->
        [one] { $edges } edge
       *[other] { $edges } edges
    }  ·  { $tight }  ·  { $volume } cm³
status-watertight = watertight ✔
status-not-watertight = NOT watertight ✖

## Files and dialogs
dialog-error-title = Error
dialog-ok = OK
error-open = Could not open { $file }: { $error }

## Project
group-project = PROJECT
project-open = 📂 Open
project-open-tip = Open a project folder (Cmd/Ctrl+O)
empty-title = No project open
empty-text = A project is a folder with a qoob3d.toml. An AI agent models its parts as scripts, this window shows them live.
empty-open = 📂 Open project folder…
status-project = { $name }  ·  { $parts ->
        [one] 1 part
       *[other] { $parts } parts
    }  ·  { $current } up to date
status-project-failed = ⚠ { $failed } with errors
