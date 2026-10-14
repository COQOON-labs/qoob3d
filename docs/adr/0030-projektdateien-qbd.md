---
id: ADR-0030
title: Projektdateien heißen .qbd, sind versioniertes JSON und enthalten nur die Feature-Liste
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-core, qoob3d-app]
tags: [dateiformat, dokumentmodell]
supersedes: [ADR-0009]
superseded_by: null
---

## Kontext

ADR-0009 legte das Projektformat mit der Endung `.qoob3d` fest. Die Endung ist lang und
umständlich zu tippen. Der Nutzer wünscht `.qbd` („QooB3D“). Es gibt noch keine Dateien bei
Nutzern, eine Umstellung kostet jetzt nichts.

Belegung der Endung: `.qbd` ist nur als alte Quicken-Tastaturlayout-Datei (Windows 3)
dokumentiert, praktisch tot. `.q3d` wurde verworfen, weil Quest3D (ein 3D-Werkzeug) und
Inspect3D sie benutzen.

## Entscheidung

- Endung: **`.qbd`** (`document::FILE_EXTENSION`). Dateidialoge filtern danach, „Speichern
  unter“ hängt sie an, wenn sie fehlt.
- Format: JSON via serde, Hülle `{ "format": "qoob3d", "format_version": N, "document": {…} }`.
  Die Kennung `"format": "qoob3d"` bleibt: Dateien sind am Inhalt erkennbar, unabhängig vom
  Namen.
- Gespeichert wird **nur** das Dokument (Feature-Liste mit Parametern), keine Geometrie.
- Beim Laden gilt: **erst validieren, dann verwenden.** Unbekannte `format_version`,
  NaN/∞, negative oder absurde Maße und Größenlimits führen zu einem klaren Fehler.
  Ungültige Daten erreichen nie den Kernel.
- Ältere Versionen werden über explizite Migrationsschritte `vN → vN+1` gelesen. Für jede
  Version liegt eine Fixture-Datei `crates/qoob3d-core/tests/fixtures/format_vN.qbd` im Repo,
  und ein Test stellt sicher, dass sie lädt. Der Inhalt einer Fixture wird nie geändert.
- `.qoob3d` wird nicht weiter unterstützt (es gibt keine solchen Dateien).

## Konsequenzen

- Dateien sind klein, diffbar und für Menschen und KI lesbar.
- Beim Öffnen muss neu berechnet werden. Bei großen Modellen kostet das Zeit, ein
  Geometrie-Cache wäre später eine reine Optimierung.
- Die v1-Fixture wurde nur umbenannt (`format_v1.qoob3d` → `format_v1.qbd`), ihr Inhalt ist
  unverändert.

## Verworfene Alternativen

- **`.qoob3d` behalten** – lang, Wunsch des Nutzers.
- **`.q3d`** – im 3D-Umfeld belegt (Quest3D, Inspect3D).
- **Binärformat / OCCT-BRep mitspeichern** – nicht diffbar, Redundanz, Angriffsfläche.
- **RON/TOML** – weniger verbreitet in Fremdwerkzeugen, JSON reicht.

## Verweise

- Code: `crates/qoob3d-core/src/document/mod.rs`, `crates/qoob3d-app/src/{app,project}.rs`
- Verwandt: ADR-0011, ADR-0029
