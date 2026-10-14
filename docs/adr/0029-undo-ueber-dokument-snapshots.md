---
id: ADR-0029
title: Undo/Redo über Snapshots des ganzen Dokuments, eine Skizzensitzung als eine Timeline-Einheit
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-app, qoob3d-core]
tags: [undo, dokumentmodell]
supersedes: []
superseded_by: null
---

## Kontext

SK-LC-08 verlangt feingranulares Undo im Skizziermodus und eine Einheit pro Skizzensitzung in
der Timeline. Das Dokument ist klein (nur Parameter, keine Geometrie, ADR-0009).

## Entscheidung

- Der Undo-Stapel speichert **unveränderliche Snapshots** des Dokuments
  (`Arc<Document>`) mit einer übersetzbaren Beschriftung. Jede Änderung legt einen neuen
  Snapshot an, Redo verwirft sich bei neuer Änderung.
- Beim Beenden einer Skizze werden alle Snapshots der Sitzung zu **einem** Eintrag
  zusammengefasst („Edit Sketch1“).
- Begrenzung: 200 Einträge.
- Speicherzustand (dirty) wird über den Snapshot zum Speicherzeitpunkt bestimmt
  (`Arc::ptr_eq`), nicht über einen Zähler.

## Konsequenzen

- Undo ist trivial korrekt und unabhängig von den einzelnen Werkzeugen.
- Der Rebuild-Cache (ADR-0012) profitiert, da alte Dokumentstände identische Schlüssel ergeben.

## Verworfene Alternativen

- **Command-Pattern mit inversen Operationen** – jede Operation bräuchte eine fehlerfreie
  Umkehrung. Bei der Dokumentgröße ohne Nutzen.

## Verweise

- `crates/qoob3d-app/src/history.rs`
- ADR-0009, ADR-0012
