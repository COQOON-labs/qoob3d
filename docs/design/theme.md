# Theme and layout

Descriptive specification (not a decision). The rationale is in the ADRs. If something is
changed here, the constants in the code must be updated too. The locations are given alongside.
Check the current look: `cargo xtask screenshot` → `target/screenshots/ui.png`.

## Character

Dark, calm, low contrast in the chrome. The attention belongs to the model.
**One** accent color (mint) for selection, hover, the primary button and focus.
No second accent. Errors are red, warnings amber, both used sparingly.

## Colors – UI (sRGB, `crates/qoob3d-app/src/theme.rs`)

| Token | Value | Usage |
| --- | --- | --- |
| `ACCENT` | `#3DE0B0` | Hover on the ViewCube, orbit pivot, primary button, brand |
| `PANEL_BG` | `#1D1F23` | Toolbar, browser |
| `PANEL_BG_DARK` | `#17191C` | Input fields, text on the primary button |
| `STROKE` | `#2C3036` | Separators, frames |
| `TEXT` | `#D4D7DC` | Text |
| `TEXT_MUTED` | `#8A9099` | Captions, status line, hints |
| `OVERLAY_BG` | `#1D1F23` at 90 % opacity | Navigation bar and empty state in the viewport |
| Warning (amber) | `#F5A623` | Exclusion zones (hatched), single-nozzle zones (tinted) |
| `ERROR` | `#E5534B` | Failed parts, service not reachable |

## Colors – viewport (linear, `crates/qoob3d-render/src/renderer.rs::palette`)

| Token | Value (linear) | Usage |
| --- | --- | --- |
| `BG_TOP` / `BG_BOTTOM` | `0.032, 0.036, 0.042` / `0.010, 0.011, 0.013` | Gradient |
| `BODY` | `0.50, 0.53, 0.57` | Solid bodies |
| `EDGE` | `0.006, 0.007, 0.008` | Edges, 1.3 pt |
| `GRID_MINOR` / `GRID_MAJOR` | gray, α 0.35 / 0.6 | Grid (powers of ten) |
| `PLATE_FILL` / `PLATE_BORDER` | slightly brighter than the background / light gray | Print bed |
| `AXIS_X` / `AXIS_Y` / `AXIS_Z` | red / green / blue | Axes (Z as a line up to ¼ of the bed width) |

Lighting: key light from the top left as seen by the viewer, fill light from the bottom right,
sky/ground ambient. This makes faces with different orientations distinguishable in every
view.

## Layout

```text
┌──────────────────────────────────────────────────────────────────────┐
│ QOOB3D DESIGN │ 📂 Open (PROJECT)                         ⚙ Settings │
├────────────┬─────────────────────────────────────────────────────────┤
│ BROWSER    │ Status (parts · up to date · errors)         🏠 ViewCube │
│ ▾ Parts    │                3D-Viewport                               │
│   👁 ◼ A · up to date   (empty state without a project:             │
│   👁 ◼ B · error         "No project open" + "Open project folder…") │
│            │ [⛶ Fit | Perspective Orthographic | Grid Build volume | Printer ▾] │
└────────────┴─────────────────────────────────────────────────────────┘
```

- ViewCube: 118 pt, top right. The labels FRONT/BACK/… (or VORNE/HINTEN/… in German) lie
  sheared in the cube face. Edges and corners are each **one** selection across all
  involved faces and light up mint together on hover.

## Language

The base language is **English**, German is a language pack, selectable under ⚙ Settings
(ADR-0020). Texts are short and without exclamation marks, workspace captions in capitals
(DESIGN, PROJECT …). Units are always shown (`20 mm`). The decimal separator follows
the language (`15.70` / `15,70 cm³`).
