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
browser-origin = 📁 Origin
origin-x = X Axis
origin-y = Y Axis
origin-z = Z Axis
browser-bodies = 📁 Bodies
browser-sketches = 📁 Sketches
browser-none = none
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
status-not-watertight = NOT watertight ✖
status-kernel-error = Kernel error: { $error }

## Files, history, dialogs
untitled = Untitled
file-new = 📄 New
file-new-tip = New document (Cmd/Ctrl+N)
file-open-tip = Open a .qbd file (Cmd/Ctrl+O)
file-save-tip = Save (Cmd/Ctrl+S, Save As: Shift+Cmd/Ctrl+S)
undo-tip = Undo: { $action } (Cmd/Ctrl+Z)
redo-tip = Redo: { $action } (Shift+Cmd/Ctrl+Z)
history-create-sketch = Create sketch
history-sketch = Edit sketch
history-delete-feature = Delete feature
history-visibility = Show/hide
edit-draw = Draw
edit-move = Move
edit-delete = Delete
edit-construction = Construction
dialog-unsaved-title = Unsaved changes
dialog-unsaved-text = Do you want to save the changes to “{ $name }”?
dialog-save = Save
dialog-discard = Don't Save
dialog-cancel = Cancel
dialog-error-title = Error
dialog-ok = OK
error-open = Could not open { $file }: { $error }
error-save = Could not save { $file }: { $error }

## Sketching
sketch-create-tip = Create a sketch on a plane (S)
sketch-finish = ✔ FINISH SKETCH
sketch-finish-tip = Finish the sketch (Enter without an active tool)
feature-sketch-name = Sketch{ $n }
feature-edit = Edit
feature-edit-tip = Double-click to edit
feature-delete = Delete
tool-line = ∕ Line
tool-line-tip = Line / polyline (L). Click points; close at the start point or press Esc to end.
tool-rectangle = ⬜ Rectangle
tool-rectangle-tip = Two-point rectangle (R)
tool-center-rectangle = ⊞ Center Rectangle
tool-center-rectangle-tip = Rectangle from centre and corner
tool-circle = ○ Circle
tool-circle-tip = Centre–diameter circle (C)
tool-point = • Point
tool-point-tip = Sketch point
tool-construction = … Construction
tool-construction-tip = Construction geometry (X): dashed, not part of profiles
field-length = Length
field-angle = Angle
field-width = Width
field-height = Height
field-diameter = Diameter
hint-tab-enter = Type a value · Tab: next · Enter: apply · Esc: cancel
hint-pick-plane = Select a plane for the sketch (Esc to cancel)
origin-plane = { $plane } Plane
origin-plane-tip = Click to create a sketch on this plane
browser-toggle-visibility = Show/hide
palette = SKETCH PALETTE
palette-look-at = Look At
palette-snap = Snap
palette-snap-tip = Snap to points, midpoints, intersections and alignments (hold Cmd/Ctrl to suspend)
palette-grid-snap = Snap to grid
palette-profiles = Show profiles
palette-points = Show points
settings-sketch-look-at = Look at the sketch plane when sketching
status-sketch = { $name }  ·  { $curves ->
        [one] { $curves } curve
       *[other] { $curves } curves
    }  ·  { $profiles ->
        [one] { $profiles } profile
       *[other] { $profiles } profiles
    }
status-document = { $features ->
        [0] Empty document – create a sketch with S
        [one] { $features } feature
       *[other] { $features } features
    }

# Solid features (ADR-0031)
group-construct = CONSTRUCT
command-extrude = Extrude
command-extrude-tip = Extrude profiles along the sketch normal (E)
command-revolve = Revolve
command-revolve-tip = Rotate profiles around an axis (e.g. a half circle makes a sphere)
command-loft = Loft
command-loft-tip = Connect profiles, or a profile and a point (pyramid, cone)
command-plane-offset = Offset Plane
command-plane-offset-tip = Construction plane parallel to another plane
command-gear = Spur Gear
command-gear-tip = Involute spur gear from module and number of teeth
command-coil = Coil
command-coil-tip = Helical spring or flat spiral made of a round wire
feature-extrude-name = Extrude{ $n }
feature-revolve-name = Revolve{ $n }
feature-loft-name = Loft{ $n }
feature-plane-name = Plane{ $n }
feature-gear-name = Gear{ $n }
feature-coil-name = Coil{ $n }
command-profiles = { $n ->
    [0] Select profiles
    [one] 1 profile
   *[other] { $n } profiles
}
command-sections = { $n ->
    [0] Select profiles / points
    [one] 1 section
   *[other] { $n } sections
}
command-axis = Axis: { $axis }
command-axis-none = select
command-axis-line = line in { $sketch }
command-plane = Plane
command-symmetric = Symmetric (centred on the sketch)
command-coil-type = Type
coil-helix = Helix
coil-spiral = Flat spiral
command-operation = Operation
op-new-body = New body
op-join = Join
op-cut = Cut
field-distance = Distance
field-offset = Offset
field-module = Module
field-teeth = Teeth
field-pressure-angle = Pressure angle °
field-thickness = Thickness
field-bore = Bore Ø
field-revolutions = Revolutions
field-pitch = Pitch
field-section = Wire Ø
field-radius = Radius
field-sweep = Sweep °
hint-select-profile = Click a profile in the viewport.
hint-select-axis = Click a sketch line or choose X / Y / Z as axis.
hint-select-sections = Click at least two sections (profiles or a tip point).
hint-check-values = Check the values marked red.
history-create-feature = Create feature
history-edit-feature = Edit feature
body-name = Body{ $n }
status-bodies = { $bodies ->
    [one] 1 body
   *[other] { $bodies } bodies
}  ·  { $volume } cm³  ·  { $tight }
export-stl-tip = Export all bodies as STL (one mesh)
export-3mf-tip = Export all bodies as 3MF (one object per body, for Bambu Studio / OrcaSlicer)
export-nothing = There are no bodies to export yet.
export-done = Exported { $n ->
    [one] 1 body
   *[other] { $n } bodies
} to { $file }
tool-arc = ⤴ Arc
tool-arc-tip = Centre-point arc (A): centre, start, end (counter-clockwise)
