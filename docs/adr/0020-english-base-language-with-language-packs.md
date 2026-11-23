---
id: ADR-0020
title: English is the base language of the UI, further languages come as Fluent language packs selected in the settings
status: accepted
date: 2026-11-23
crates: [qoob3d-app, xtask]
tags: [i18n, ui, settings]
supersedes: []
superseded_by: null
---

## Context

Up to M2 all UI texts were German and hard-coded. The user's requirement: the first language
of the UI **must be English**; German and later further languages come as language packs that
can be chosen in the settings. Grammatical differences such as plural forms must not need code
per language.

## Decision

- **Project Fluent** (`fluent-bundle`): one file per language,
  `crates/qoob3d-app/locales/<code>/main.ftl`, embedded with `include_str!`. `en` is the
  reference, `de` the first language pack. Plurals and variables are written in Fluent
  (`{ $parts -> [one] 1 part *[other] … }`), not in Rust.
- UI code contains **no visible literals**: it calls `tr("key")` or `tr_args("key", &[…])`
  (`crates/qoob3d-app/src/i18n.rs`); keys are always literals, never assembled with
  `format!`. Numbers go through `i18n::decimal` with the decimal separator of the language.
  Brand (`QOOB3D`), format names and symbols are exempt.
- The **current language is per thread** (`thread_local`). The UI runs on one thread, so UI
  tests running in parallel do not switch each other's language. A new thread that translates
  must call `set_language`.
- The language is chosen in **⚙ Settings** in the toolbar. `Settings` (language, printer and
  favourites, grid, build volume, left-drag orbit) is stored through eframe persistence; the
  smoke test neither reads nor writes it.
- **`cargo xtask i18n`** (part of `check`, ADR-0013) requires: every language has exactly the
  keys of `en`; every kebab-case literal in UI code is an existing key; no key is unused; a
  heuristic finds literals passed to egui widgets (`label`, `button`, `on_hover_text`, …).
  Developer-facing files (CLI, smoke test, headless render) are not UI.
- Output for agents and scripts (service and CLI JSON envelopes, MCP, `render-project`) is
  English and not translated.
- A new language is a new `.ftl` file plus a `Language` variant.

## Consequences

- Every new UI text needs keys in **all** language files, otherwise the check is red.
- Accessibility labels, and therefore UI tests, are English; one test switches to German
  (`language_can_be_switched_to_german`). Only `locales/de/` and tests asserting German texts
  contain German (ADR-0001).
- Glyphs outside the egui fonts appear as "□"; `every_ui_glyph_exists_in_the_fonts` checks
  the language files and the UI sources.

## Rejected alternatives

- **Own key tables as a Rust `match`** – the compiler would check completeness, but plural
  forms per language would need code, and translators do not work in Rust.
- **`rust-i18n` (YAML)** – no real plural rules. Fluent is the standard of Firefox.
- **One global language (`AtomicU8`)** – parallel tests switched each other's language.

## References

- `crates/qoob3d-app/src/i18n.rs`, `crates/qoob3d-app/src/settings.rs`,
  `crates/qoob3d-app/src/panels.rs` (`settings_menu`), `crates/qoob3d-app/locales/`
- `xtask/src/i18n.rs`, `crates/qoob3d-app/tests/ui.rs`
- ADR-0001, ADR-0013
