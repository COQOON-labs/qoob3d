# Requirements: Bodies, Features and Export

As of: 2026-10-14 · Status: **implemented in M4** (exceptions in section 6) · Decisions:
ADR-0031 to ADR-0036

Goal: QOOB3D can build the basic shapes for FDM printing end-to-end and hand them to the
slicer as a file: **box, cylinder, sphere, pyramid, gear, coil**.
IDs `KO-<area>-<no>` are stable; code, tests and ADRs refer to them.

## 1. How each shape is made (reference flow)

| Shape | Flow in the UI | Reference model | E2E test (`tests/modeling.rs`) |
| --- | --- | --- | --- |
| Box | Sketch on XY → rectangle (R) with dimensions → Extrude (E) | `models/box.qbd` | `box_from_rectangle_extrude_and_3mf_export` |
| Cylinder | Sketch on XY → circle (C) → Extrude | `models/cylinder.qbd` | `cylinder_from_circle_and_extrude` |
| Sphere | Sketch on XZ → arc (A) 180° + line → Revolve around X or the line | `models/sphere.qbd` | `sphere_from_arc_and_revolve` |
| Pyramid | Square on XY → offset plane → point on it → Loft profile → point | `models/pyramid.qbd` | `pyramid_from_square_offset_plane_point_and_loft` |
| Gear | CREATE → Spur Gear (module, teeth, thickness, bore) | `models/gear.qbd` | `spur_gear_from_dialog` |
| Coil | CREATE → Coil (helix or flat spiral) | `models/coil.qbd`, `models/spiral.qbd` | `coil_then_edited_into_a_flat_spiral` |
| Join/Cut | Plate, peg (Join), hole (Cut) | `models/plate-with-hole.qbd` | kernel test `join_and_cut` |

## 2. Features

| ID | Requirement | Prio | Status |
| --- | --- | --- | --- |
| KO-FT-01 | Extrude: profiles (also several, also from several sketches) along the normal, negative distance = opposite direction, symmetric | MUST | ✅ |
| KO-FT-02 | Revolve: profiles around an origin axis or sketch line, angle (0, 360] | MUST | ✅ |
| KO-FT-03 | Loft: profiles in order, first/last section may be a point (tip) | MUST | ✅ |
| KO-FT-04 | Offset construction plane (also from another construction plane); sketches on it | MUST | ✅ |
| KO-FT-05 | Spur gear (involute, module, teeth 6–400, pressure angle 10–35°, thickness, bore) | MUST | ✅ |
| KO-FT-06 | Coil: helical spring (turns, pitch) and flat spiral, round wire | MUST | ✅ |
| KO-FT-07 | Operation per feature: new body, join, cut (against all touched bodies) | MUST | ✅ |
| KO-FT-08 | Missing reference (deleted profile) = feature error with reason, nothing guessed | MUST | ✅ |
| KO-FT-09 | Edit features afterwards (double-click in the timeline) | MUST | ✅ |
| KO-FT-10 | Taper when extruding, shell, fillet, chamfer | SHOULD | M6 |
| KO-FT-11 | Explicit participant selection for join/cut | SHOULD | open |

## 3. Operation

| ID | Requirement | Status |
| --- | --- | --- |
| KO-UI-01 | Dialog on the right in the viewport, fields with expressions, Enter = OK, Esc = cancel | ✅ |
| KO-UI-02 | Live preview = real result, without an undo step until OK | ✅ |
| KO-UI-03 | Selection in the viewport: profiles (hover light, selected strong), lines as axis, points | ✅ |
| KO-UI-04 | E opens Extrude; the only profile of the newest sketch is preselected | ✅ |
| KO-UI-05 | Consumed sketches are hidden | ✅ |
| KO-UI-06 | Browser: bodies with volume (invalid ones red), construction planes; timeline with icons, errors red with reason | ✅ |
| KO-UI-07 | Status bar: number of bodies, total volume, watertight ✔/✘ | ✅ |

## 4. Export and printability

| ID | Requirement | Status |
| --- | --- | --- |
| KO-EX-01 | STL (binary) and 3MF (one object per body, millimeters) from app and CLI | ✅ |
| KO-EX-02 | Export tessellation 0.01 mm, welded vertices, no degenerate triangles | ✅ |
| KO-EX-03 | Every body is checked: valid B-Rep, watertight mesh, exact volume | ✅ |
| KO-EX-04 | STEP export | M6 |

## 5. Quality (for humans and AI)

| ID | Requirement | Status |
| --- | --- | --- |
| KO-QA-01 | Every feature and curve type has a reference model with analytical expectations (`cargo xtask models`) | ✅ |
| KO-QA-02 | Every shape is built end-to-end through the UI and measured | ✅ |
| KO-QA-03 | CLI `--report` returns JSON and exit code 1 if something is not printable | ✅ |
| KO-QA-04 | File formats are additionally checked with third-party tools (`unzip -t`) | ✅ |
| KO-QA-05 | Format version 2 with a fixture that contains every type | ✅ |
| KO-QA-06 | Smoke test extrudes in the real app | ✅ |

## 6. Open

- Timeline rollback and suppression (M5), body selection in the viewport, taper/fillet (M6).
- Join/cut without explicit participants (KO-FT-11).
- Loft only "ruled" (straight connections); smooth lofts with guide curves later.

## References

- ADR-0031 (features, rebuild), ADR-0032 (gear, coil), ADR-0033 (reference models),
  ADR-0034 (export), ADR-0035 (dialogs), ADR-0036 (crate boundaries)
- `docs/requirements/sketches.md`, `docs/design/controls.md`
