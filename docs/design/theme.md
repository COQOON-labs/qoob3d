# Theme and layout

Descriptive specification (not a decision). The rationale is in the ADRs. If something is
changed here, the constants in the code must be updated too. The locations are given alongside.
Check the current look: `cargo xtask screenshot` → `target/screenshots/ui.png`.

## Character

Dark, calm, low contrast in the chrome. The attention belongs to the model.
**One** accent color (mint) for selection, active tools, sketch elements and focus.
No second accent. Errors are red, warnings amber, both used sparingly.

## Colors – UI (sRGB, `crates/qoob3d-app/src/theme.rs`)

| Token | Value | Usage |
| --- | --- | --- |
| `ACCENT` | `#3DE0B0` | Selection, active tool, hover on the ViewCube, orbit pivot, brand |
| `PANEL_BG` | `#1D1F23` | Toolbar, browser |
| `PANEL_BG_DARK` | `#17191C` | Timeline, input fields |
| `STROKE` | `#2C3036` | Separators, frames |
| `TEXT` | `#D4D7DC` | Text |
| `TEXT_MUTED` | `#8A9099` | Captions, status line, hints |
| `OVERLAY_BG` | `#1D1F23` at 90 % opacity | Navigation bar in the viewport |
| Warning (amber) | `#F5A623` | Exclusion zones (hatched), single-nozzle zones (tinted) |
| `CONSTRUCTION` | `#C08A3A` | Construction geometry (dashed), also in the preview |
| `ERROR` | `#E5534B` | Open curve ends, invalid HUD input, kernel errors |

## Sketches (`crates/qoob3d-app/src/sketch_ui.rs`, SK-VS-*)

| Element | Appearance |
| --- | --- |
| Curves of the active sketch | `ACCENT`, 1.6 pt; selection/hover brighter and wider |
| Construction | `CONSTRUCTION`, dashed |
| Profiles | `ACCENT` at ~12 % opacity, hover ~25 % |
| Open ends | `ERROR` dot |
| Preview while drawing | dashed, `ACCENT` (in construction mode `CONSTRUCTION`) |
| Snapping | Symbol at the cursor, alignment lines dashed |
| Value HUD | opaque `PANEL_BG`, monospace, active field `ACCENT`, error `ERROR` |
| Inactive sketches | thin, gray, without profiles |

Check the appearance: `cargo xtask screenshot` → `sketch-plane-pick.png`, `sketch.png`.

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
│ QOOB3D DESIGN │ FILE │ SKETCH │ CREATE │ EXPORT (Ribbon)   ⚙ Settings │
├────────────┬─────────────────────────────────────────────┬───────────┤
│ BROWSER    │ Status (triangles · edges · watertight · cm³)  🏠 ViewCube │
│ ▾ Untitled │                3D-Viewport                               │
│   ▸ Origin │                                                          │
│   ▾ Bodies │                                                          │
│   ▾ Sketches                                                         │
│            │ [⛶ Fit | Perspective Orthographic | Grid Build volume | Printer ▾] │
├────────────┴─────────────────────────────────────────────────────────┤
│ ⏮ ⏴ ⏵ ⏭ │ Timeline                                                  │
└──────────────────────────────────────────────────────────────────────┘
```

- Functions of later milestones are visible but disabled, with the hint
  "Coming in Mx".
- ViewCube: 118 pt, top right. The labels FRONT/BACK/… (or VORNE/HINTEN/… in German) lie
  sheared in the cube face. Edges and corners are each **one** selection across all
  involved faces and light up mint together on hover.

## Language

The base language is **English**, German is a language pack, selectable under ⚙ Settings
(ADR-0020). Texts are short and without exclamation marks, workspace captions in capitals
(DESIGN, SKETCH …). Units are always shown (`20 mm`). The decimal separator follows
the language (`15.70` / `15,70 cm³`).
