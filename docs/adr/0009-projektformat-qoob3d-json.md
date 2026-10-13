---
id: ADR-0009
title: Projektdateien .qoob3d sind versioniertes JSON und enthalten nur die Feature-Liste
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-core]
tags: [dateiformat, dokumentmodell]
supersedes: []
superseded_by: null
---

## Kontext

Parametrisches CAD kann Geometrie jederzeit aus der Feature-Liste neu berechnen. Wird
Geometrie gespeichert, wird sie redundant und kann inkonsistent werden. Projektdateien
kommen auch aus fremden Quellen und sind damit nicht vertrauenswürdig.

## Entscheidung

- Format: JSON via serde, Hülle `{ "format": "qoob3d", "format_version": N, "document": {…} }`.
- Gespeichert wird **nur** das Dokument (Feature-Liste mit Parametern), keine Geometrie.
- Beim Laden gilt: **erst validieren, dann verwenden.** Unbekannte `format_version`,
  NaN/∞, negative oder absurde Maße und Größenlimits führen zu einem klaren Fehler.
  Ungültige Daten erreichen nie den Kernel.
- Ältere Versionen werden über explizite Migrationsschritte `vN → vN+1` gelesen. Für jede
  Version liegt eine Fixture-Datei im Repo, und ein Test stellt sicher, dass sie lädt.
- Umsetzung in M3.

## Konsequenzen

- Dateien sind klein, diffbar und für Menschen und KI lesbar.
- Beim Öffnen muss neu berechnet werden. Bei großen Modellen kostet das Zeit, ein
  Geometrie-Cache wäre später eine reine Optimierung.

## Verworfene Alternativen

- **Binärformat / OCCT-BRep mitspeichern** – nicht diffbar, Redundanz, Angriffsfläche.
- **RON/TOML** – weniger verbreitet in Fremdwerkzeugen, JSON reicht.

## Verweise

- ADR-0010, ADR-0011
