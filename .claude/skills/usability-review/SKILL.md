---
name: usability-review
description: Review usability and accessibility of a UI workflow in QOOB3D – cognitive walkthrough, Nielsen heuristics, interaction count, AccessKit names, keyboard, contrast, target sizes. Use for every UI change and when the user asks about ease of use.
---

# Usability and accessibility review (K-14, K-15)

The goal is the shortest, error-free path "design → slicer → print" (ADR-0002) for
a maker with an FDM printer who roughly knows Fusion 360.

1. **Look:** run `cargo xtask screenshot` and look at all PNGs in `target/screenshots/`. For
   the concrete workflow, use the screenshot steps of the E2E tests in
   `crates/qoob3d-app/tests/modeling.rs` and `tests/ui.rs`.
2. **Cognitive walkthrough** for the task of the workflow, step by step:
   Does the user know what to do now? Do they see the control? Do they connect it with
   their goal? Do they notice that it worked? Every "no" is a finding.
3. **Nielsen heuristics:** system status visible, the user's language, control (Escape,
   undo), consistency with `docs/design/controls.md` and Fusion 360, error prevention,
   recognition rather than recall, efficiency (shortcuts), minimalist design, understandable
   error messages, help.
4. **Count interactions:** clicks, keys and dialog switches for the workflow (readable from the
   E2E test). If the number rises compared to before, that is a finding, unless there is
   a reason.
5. **Accessibility:**
   - Every interactive element has an accessible name. `egui_kittest` finds widgets
     via AccessKit: what a test cannot find with `get_by_label`, a screen reader cannot find
     either.
   - Everything is reachable by keyboard (viewport actions need shortcut alternatives).
   - Contrast per WCAG 2.2 AA from the colors in `crates/qoob3d-app/src/theme.rs`: text
     ≥ 4.5:1, large text and UI elements ≥ 3:1. Calculate the ratio.
   - Target sizes ≥ 24 × 24 px (WCAG 2.5.8), measured on the screenshot.
   - Color is never the only carrier of information (e.g. axes red/green: additionally a
     label or shape?).
   - At 150 % and 200 % scaling (`pixels_per_point`) nothing overlaps.
6. **Result:** findings in the JSON format from `harness/critic-checklist.md` with rule `K-14`
   or `K-15`, plus `"evidence"`: screenshot path and step. Report anything that could be
   checked deterministically as a proposal for a test (to the `test-author`).
