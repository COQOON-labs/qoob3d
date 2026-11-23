# Controls

Descriptive specification. Decisions and rationale: ADR-0008 (navigation, pivot via raycast,
left-drag orbit), ADR-0019 (ViewCube, overlays).
Project behaviour: `docs/requirements/project-app.md`.

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

## Project

| Input | Action | Status |
| --- | --- | --- |
| Cmd/Ctrl+O, "📂 Open" in the toolbar, "Open project folder…" in the empty state | Pick a project folder (with `qoob3d.toml`) in the system dialog and show it live | ✅ W1 |
| 👁 in the browser | Show/hide a part | ✅ W1 |

A double click counts even right after a click elsewhere, where egui reports it as a triple
click: code uses `navigation::double_clicked_by`, never `Response::double_clicked`.

## Cursor

| Place / state | Cursor |
| --- | --- |
| Everywhere (viewport, panels, buttons, labels) | Arrow (macOS and CAD convention, precise) |
| Over ViewCube regions and the home button | Pointing hand (clickable navigation targets) |
| During orbit or pan | Grabbing hand |
| Text fields (e.g. printer search) | Text cursor |

Labels are not selectable as text, which is why no text cursor appears there
(test: `cursors_follow_the_policy`).

## Constants

`ORBIT_SPEED = 0.006 rad/pt`, `WHEEL_ZOOM_STEP = 0.88` per detent, the trackpad orbit gesture ends
after a 0.25 s pause (`crates/qoob3d-app/src/navigation.rs`). Animation duration `ANIM_SECONDS = 0.35`
(`viewport.rs`).
