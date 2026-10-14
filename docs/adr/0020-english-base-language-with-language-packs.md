---
id: ADR-0020
title: English is the base language of the UI, further languages come as Fluent language packs
status: accepted
date: 2026-10-13
crates: [qoob3d-app, xtask]
tags: [i18n, ui, settings]
supersedes: []
superseded_by: null
---

## Context

Up to M2 all UI texts were German and hard-coded. The user's requirement: the first language
**must be English**; German and later further languages come as language packs and
are selectable in the settings.

## Decision

- **Project Fluent** (`fluent-bundle`): one file per language
  `crates/qoob3d-app/locales/<code>/main.ftl`. `en` is the reference, `de` the first
  language pack. Fluent supports variables and plural forms
  (`{ $edges -> [one] … *[other] … }`), so grammatical differences per language
  need no code logic.
- UI code contains **no visible literals** but calls `tr("key")` or
  `tr_args("key", &[…])` (`crates/qoob3d-app/src/i18n.rs`). Numbers are formatted by
  `i18n::decimal` with the decimal separator of the language. Brands (`QOOB3D`), format names
  (STL) and symbols are exempt.
- The **current language applies per thread** (`thread_local`). The UI runs on one thread,
  so UI tests running in parallel do not affect each other.
- The language is set in the **settings** (⚙ in the toolbar). `Settings` is
  stored via eframe persistence (language, printer, grid, build volume, left-drag orbit).
  The smoke test neither reads nor writes user settings.
- **`cargo xtask i18n`** (part of `check`) checks:
  - Every language has exactly the keys of `en`.
  - Every kebab-case literal in UI code is an existing key.
  - There are no unused keys.
  - A heuristic reports hard-coded texts in egui widgets.
- New language = new `.ftl` file plus a `Language` variant.

## Consequences

- Every new UI line needs keys in **all** language files, otherwise the check is red.
- Accessibility labels and thus UI tests are English. A test checks switching to German.
- Documentation (`docs/`, CLAUDE.md) stays German, code and comments English. (Replaced by
  ADR-0045: the whole repository is English.)

## Rejected alternatives

- **Own key tables as a Rust `match`** – the compiler would check completeness,
  but plural forms per language would need code, and translators do not work in Rust.
- **`rust-i18n` (YAML)** – no real plural rules. Fluent is the standard of Firefox.
- **Global language (`AtomicU8`)** – tests switched each other's language.

## References

- `crates/qoob3d-app/src/{i18n.rs,settings.rs}`, `crates/qoob3d-app/locales/`, `xtask/src/i18n.rs`
