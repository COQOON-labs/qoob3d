---
id: ADR-0023
title: „Meine Drucker“ als Favoriten oben in der Liste, andere optional ausblendbar
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-app]
tags: [fdm, drucker, ux, einstellungen]
supersedes: []
superseded_by: null
---

## Kontext

Mit 441 Druckerprofilen (ADR-0021) ist die Liste lang. Wer regelmäßig nur mit zwei oder drei
Modellen arbeitet, will diese immer oben sehen und die übrigen eventuell ganz ausblenden.
Außerdem schloss sich die Liste beim Klick ins Suchfeld (egui-ComboBoxen schließen
standardmäßig bei jedem Klick).

## Entscheidung

- `Settings::favorite_printers` (Reihenfolge des Hinzufügens) und
  `Settings::only_favorite_printers` (gespeichert, ADR-0020).
- In der Druckerliste hat jeder Eintrag einen **Stern** (☆ / ★ in Mint) zum Markieren.
  Markierte Drucker stehen unter **MY PRINTERS** oben, alle übrigen unter **ALL PRINTERS**,
  gruppiert nach Hersteller.
- **⚙ Settings → Printers:** „Show only my printers in the list“ blendet die übrigen aus.
  Die markierten Drucker sind dort mit ✕ entfernbar. Ist nichts markiert, werden trotz der
  Option alle gezeigt, mit Hinweis.
- Die Liste schließt nur bei Klick **außerhalb** (`CloseOnClickOutside`) oder nach einer
  Auswahl (`ui.close()`). Beim Öffnen wird die Suche geleert und fokussiert, sodass man
  sofort tippen kann.

## Konsequenzen

- UI-Tests decken Klick ins Suchfeld, Favoriten und Ausblenden ab
  (`printer_can_be_selected_via_search`, `favourite_printers_are_listed_first_and_others_can_be_hidden`).
- Später möglich: eigene Druckerprofile (benutzerdefinierte Maße) in derselben Liste.

## Verworfene Alternativen

- **Zuletzt benutzte Drucker automatisch oben** – unvorhersehbar, der Nutzer will eine feste Auswahl.
- **Eigener Dialog nur für die Druckerverwaltung** – zu schwer für eine Liste mit Sternen.

## Verweise

- `crates/qoob3d-app/src/{viewport.rs,panels.rs,settings.rs}`
- ADR-0020, ADR-0021
