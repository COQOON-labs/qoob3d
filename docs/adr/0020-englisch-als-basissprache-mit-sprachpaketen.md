---
id: ADR-0020
title: Englisch ist die Basissprache der UI, weitere Sprachen kommen als Fluent-Sprachpakete
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-app, xtask]
tags: [i18n, ui, einstellungen]
supersedes: []
superseded_by: null
---

## Kontext

Bis M2 waren alle UI-Texte deutsch und fest im Code. Vorgabe des Nutzers: Die erste Sprache
**muss Englisch** sein, Deutsch und später weitere Sprachen kommen als Sprachpakete und
sind in den Einstellungen wählbar.

## Entscheidung

- **Project Fluent** (`fluent-bundle`): pro Sprache eine Datei
  `crates/qoob3d-app/locales/<code>/main.ftl`. `en` ist die Referenz, `de` das erste
  Sprachpaket. Fluent kann Variablen und Pluralformen
  (`{ $edges -> [one] … *[other] … }`), damit kommen grammatische Unterschiede pro Sprache
  ohne Code-Logik aus.
- UI-Code enthält **keine sichtbaren Literale**, sondern ruft `tr("key")` bzw.
  `tr_args("key", &[…])` auf (`crates/qoob3d-app/src/i18n.rs`). Zahlen formatiert
  `i18n::decimal` mit dem Dezimaltrennzeichen der Sprache. Marken (`QOOB3D`), Formatnamen
  (STL) und Symbole sind ausgenommen.
- Die **aktuelle Sprache gilt pro Thread** (`thread_local`). Die UI läuft auf einem Thread,
  parallel laufende UI-Tests beeinflussen sich dadurch nicht.
- Die Sprache steht in den **Einstellungen** (⚙ in der Werkzeugleiste). `Settings` wird
  über eframe-Persistenz gespeichert (Sprache, Drucker, Raster, Bauraum, Linksziehen-Orbit).
  Der Smoke-Test liest und schreibt keine Benutzereinstellungen.
- **`cargo xtask i18n`** (Teil von `check`) prüft:
  - Jede Sprache hat genau die Schlüssel von `en`.
  - Jedes kebab-case-Literal im UI-Code ist ein existierender Schlüssel.
  - Es gibt keine unbenutzten Schlüssel.
  - Eine Heuristik meldet feste Texte in egui-Widgets.
- Neue Sprache = neue `.ftl`-Datei plus eine `Language`-Variante.

## Konsequenzen

- Jede neue UI-Zeile braucht Schlüssel in **allen** Sprachdateien, sonst ist der Check rot.
- Accessibility-Labels und damit UI-Tests sind englisch. Ein Test prüft den Wechsel auf Deutsch.
- Dokumentation (`docs/`, CLAUDE.md) bleibt deutsch, Code und Kommentare englisch.

## Verworfene Alternativen

- **Eigene Schlüssel-Tabellen als Rust-`match`** – Vollständigkeit prüft zwar der Compiler,
  aber Pluralformen pro Sprache bräuchten Code, und Übersetzer arbeiten nicht in Rust.
- **`rust-i18n` (YAML)** – keine echten Pluralregeln. Fluent ist der Standard von Firefox.
- **Globale Sprache (`AtomicU8`)** – Tests haben sich gegenseitig die Sprache umgestellt.

## Verweise

- `crates/qoob3d-app/src/{i18n.rs,settings.rs}`, `crates/qoob3d-app/locales/`, `xtask/src/i18n.rs`
