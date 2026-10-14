---
id: ADR-0023
title: "My printers" as favorites at the top of the list, others can optionally be hidden
status: accepted
date: 2026-10-13
crates: [qoob3d-app]
tags: [fdm, printers, ux, settings]
supersedes: []
superseded_by: null
---

## Context

With 441 printer profiles (ADR-0021) the list is long. Anyone who regularly works with only two
or three models wants to see them at the top at all times and possibly hide the rest entirely.
In addition, the list closed when clicking into the search field (egui ComboBoxes close on every
click by default).

## Decision

- `Settings::favorite_printers` (in the order they were added) and
  `Settings::only_favorite_printers` (persisted, ADR-0020).
- In the printer list every entry has a **star** (☆ / ★ in mint) for marking it.
  Marked printers appear under **MY PRINTERS** at the top, all others under **ALL PRINTERS**,
  grouped by manufacturer.
- **⚙ Settings → Printers:** "Show only my printers in the list" hides the others.
  The marked printers can be removed there with ✕. If nothing is marked, all are shown despite
  the option, with a hint.
- The list only closes on a click **outside** (`CloseOnClickOutside`) or after a
  selection (`ui.close()`). On opening, the search is cleared and focused so you can
  start typing immediately.

## Consequences

- UI tests cover clicking into the search field, favorites and hiding
  (`printer_can_be_selected_via_search`, `favourite_printers_are_listed_first_and_others_can_be_hidden`).
- Possible later: custom printer profiles (user-defined dimensions) in the same list.

## Rejected alternatives

- **Recently used printers automatically at the top** – unpredictable, the user wants a fixed
  selection.
- **A separate dialog just for managing printers** – too heavy for a list with stars.

## References

- `crates/qoob3d-app/src/{viewport.rs,panels.rs,settings.rs}`
- ADR-0020, ADR-0021
