# Controls

Descriptive specification. Decisions and rationale: ADR-0008 (navigation),
ADR-0015 (ViewCube, overlays), ADR-0017 (pivot via raycast), ADR-0022 (left-drag orbit),
ADR-0027 (sketching), ADR-0029 (undo), ADR-0035 (feature dialogs). Complete sketch requirements:
`docs/requirements/sketches.md`.

## Viewport navigation (1:1 Fusion 360)

| Input | Action | Status |
| --- | --- | --- |
| Middle mouse button drag | Pan, the content follows the cursor | ✅ M1 |
| Shift + middle mouse button drag | Orbit around the **model point under the cursor** (mint dot); next to the model around the camera target | ✅ M2 |
| Mouse wheel | Zoom to the cursor, forward = in | ✅ M1 |
| Double-click middle mouse button / F6 / "⛶ Fit" | Fit all (animated) | ✅ M2 |
| Trackpad: two-finger swipe | Pan | ✅ M1 |
| Trackpad: Shift + two fingers | Orbit around the point under the cursor | ✅ M2 |
| Trackpad: pinch | Zoom to the cursor | ✅ M1 |
| Left mouse button drag / trackpad: three-finger drag (macOS accessibility) | Orbit around the point under the cursor; can be switched off under ⚙ Settings | ✅ M2 |
| ViewCube: click face, edge or corner | View from this direction (animated, 0.35 s). Edges and corners are **one** target each across all faces | ✅ M2 |
| ViewCube: 🏠 | Home view (front-right-top) + fit | ✅ M2 |
| Navigation bar: Perspective / Orthographic | Toggle projection | ✅ M2 |
| Navigation bar: Grid | Grid and axes on/off (print bed stays visible) | ✅ M2 |
| Navigation bar: Build volume | Build volume as a wireframe up to the max. height | ✅ M2 |
| Navigation bar: printer | 441 profiles; search field is focused on opening; MY PRINTERS (marked ☆) at the top, ALL PRINTERS by manufacturer; exclusion zones ▣, single-nozzle zones ◑ | ✅ M2 |
| ⚙ Settings → Printers | "Show only my printers in the list", remove marked printers (×) | ✅ M2 |

Every input cancels a running camera animation.

## Document

| Input | Action | Status |
| --- | --- | --- |
| Cmd+N / Cmd+O / Cmd+S / Shift+Cmd+S | New / Open / Save / Save As (`.qbd`); with unsaved changes a dialog asks | ✅ M3a |
| Cmd+Z / Shift+Cmd+Z, ⟲ ⟳ in the toolbar | Undo / Redo. A sketch session is **one** step after it ends | ✅ M3a |
| Double-click on a sketch (browser, timeline) | Edit sketch | ✅ M3a |
| Right-click on a timeline entry | Edit / Delete | ✅ M3a |
| 👁 in the browser | Show/hide sketch | ✅ M3a |

A double click counts even right after a click elsewhere, where egui reports it as a triple
click: code uses `navigation::double_clicked_by`, never `Response::double_clicked`; an entry
that is already being edited ignores it.

## Sketching

| Input | Action | Status |
| --- | --- | --- |
| S / "Create Sketch" | Plane pick: click XY, XZ or YZ (hover highlights), Esc cancels. Clicking an origin plane in the browser starts directly | ✅ M3a |
| Entry | Camera turns to the plane, animated (Look At, can be switched off in ⚙ Settings), grid lies in the sketch plane | ✅ M3a |
| L / R / C / A | Line (polyline) / rectangle (2 points) / circle (center–diameter) / arc (center → start = radius → end = angle, counter-clockwise). Rectangle from center and point only via the toolbar | ✅ M3a/M4 |
| Click | Set point. Line: the chain continues, clicking the start point closes it, double-click **on the same point** ends it | ✅ M3a |
| Typing while drawing | Value into the active HUD field (length/angle, width/height, diameter). Expressions like `10+5`, `2*12.5`, `1in`; comma in German | ✅ M3a |
| Tab | Accept and lock the value (🔒), next field | ✅ M3a |
| Enter | Accept values and create the element; without a tool: finish the sketch | ✅ M3a |
| Esc | Clear input → cancel step → end tool → clear selection | ✅ M3a |
| X | Toggle the tool's construction mode or the construction state of the selection | ✅ M3a |
| Cmd/Ctrl held | Snapping temporarily off | ✅ M3a |
| Without a tool: click / Shift+click | Select / extend selection | ✅ M3a |
| Without a tool: drag on point, line, circle edge | Move; dropping a point onto a point joins both | ✅ M3a |
| Without a tool: drag a box | left→right: only fully contained, right→left: also intersected (dashed) | ✅ M3a |
| Del / Backspace | Delete selection (points take their curves with them) | ✅ M3a |
| "FINISH SKETCH" | End sketch mode | ✅ M3a |

## Bodies (model mode)

| Input | Action | Status |
| --- | --- | --- |
| E / CREATE → Extrude | Extrude. If the newest visible sketch has exactly one profile, it is preselected (E, Enter is enough) | ✅ M4 |
| CREATE → Revolve | Revolve: click a profile, then the axis (click a sketch line or X / Y / Z) | ✅ M4 |
| CREATE → Loft | Loft: click profiles and/or a sketch point (tip) in order | ✅ M4 |
| CREATE → Spur Gear / Coil | Spur gear or spring/flat spiral, parameters in the dialog, plane as a selection | ✅ M4 |
| CONSTRUCT → Offset Plane | Offset plane; appears in the browser under Origin, a click there starts a sketch | ✅ M4 |
| In the dialog | Click selects/deselects in the viewport, fields accept expressions (`10+5`, `1in`), operation New body/Join/Cut. **Enter** = OK, **Esc** = Cancel (as long as no field has focus) | ✅ M4 |
| Double-click on a feature (timeline) | Dialog for editing | ✅ M4 |
| EXPORT → STL / 3MF | Export all bodies (3MF: one object per body) | ✅ M4 |

The preview in the dialog is the real result (the kernel computes on every change).
Consumed sketches are hidden on OK (eye in the browser).

In sketch mode, left drag does not orbit (ADR-0027); orbit/pan/zoom as above via the middle
mouse button, trackpad and ViewCube. Snap tolerance 8 px, pick tolerance 6 px.

## Cursor

| Place / state | Cursor |
| --- | --- |
| Everywhere (viewport, panels, buttons, labels) | Arrow (macOS and CAD convention, precise) |
| Over ViewCube regions and the home button | Pointing hand (clickable navigation targets) |
| During orbit or pan | Grabbing hand |
| Sketch mode with an active tool | Crosshair |
| Dragging sketch geometry | Grabbing hand |
| Text fields (e.g. printer search) | Text cursor |

Labels are not selectable as text, which is why no text cursor appears there
(test: `cursors_follow_the_policy`).

## Constants

`ORBIT_SPEED = 0.006 rad/pt`, `WHEEL_ZOOM_STEP = 0.88` per detent, the trackpad orbit gesture ends
after a 0.25 s pause (`crates/qoob3d-app/src/navigation.rs`). Animation duration `ANIM_SECONDS = 0.35`
(`viewport.rs`).
