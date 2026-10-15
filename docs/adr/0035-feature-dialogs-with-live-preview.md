---
id: ADR-0035
title: Feature commands are dialogs with selection in the viewport and a live preview via the normal rebuild
status: accepted
date: 2026-10-14
crates: [qoob3d-app]
tags: [ui, interaction]
supersedes: []
superseded_by: null
---

## Context

Extrude, revolve, loft, plane, gear and coil need parameters and selections
(profiles, axis, points). For this, Fusion shows a dialog on the right and a preview that
updates with every input.

## Decision

- `commands.rs`: a `Command` holds the parameters of **one** feature (`FeatureKind`). The
  mode `Mode::Command` shows the dialog at the right edge of the viewport (fields with
  expressions as in the HUD, plane as a drop-down list, operation, OK/Cancel).
- Selection in the viewport via an active **selector**: profiles (extrude, revolve),
  axis (sketch line; origin axes as buttons), sections (loft: profiles and
  points in click order). Hits in space via `View::to_local` (ray onto the
  sketch plane) or screen distance for points and lines.
- **Preview without a special path:** every frame, the document with the feature is set via
  `Project::preview_doc` (without an undo step); the kernel computes as for any
  change, the cache (ADR-0031) makes that cheap. OK = one undo step, Cancel =
  `cancel_preview`.
- Convenience like Fusion: E opens extrude; if the newest visible sketch has exactly one
  profile, it is preselected (Enter is enough). Consumed sketches are hidden on OK.
  Double-clicking a feature in the timeline opens the same dialog for editing.
- Enter = OK, Esc = Cancel, as long as no text field has focus.

## Consequences

- Every dialog input triggers a rebuild; with expensive models the preview may
  lag behind (the old geometry stays visible, ADR-0005).
- UI tests must wait a few frames after opening a dialog: egui measures a new
  area in the first frame and places it in the second.

## Rejected alternatives

- **Separate preview geometry in the dialog** – duplicate code, deviations between preview
  and result.
- **Modal windows** – cover the model, selection in the viewport would be blocked.

## References

- Code: `crates/qoob3d-app/src/{commands,app,project}.rs`
- Tests: `crates/qoob3d-app/tests/modeling.rs`
- Related: ADR-0027, ADR-0029, ADR-0031
