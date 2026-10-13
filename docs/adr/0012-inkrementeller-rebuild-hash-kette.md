---
id: ADR-0012
title: Der Rebuild ist inkrementell über eine Hash-Kette pro Feature
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-kernel]
tags: [kernel, performance, timeline]
supersedes: []
superseded_by: null
---

## Kontext

Ändert sich ein Parameter, muss die Timeline ab diesem Feature neu berechnet werden.
Dirty-Flags von Hand zu pflegen ist fehleranfällig.

## Entscheidung

- Cache-Schlüssel von Feature *i*: `key_i = hash(key_{i-1}, Parameter_i)`.
- Der Kernel-Worker hält `key → OCCT-Ergebnis`. Ein Rebuild läuft die Liste ab und rechnet
  ab dem ersten Schlüssel, der nicht im Cache ist.
- Unterdrückte Features und der Rollback-Marker fließen in den Schlüssel ein.
- Umsetzung in M4.

## Konsequenzen

- Korrekt per Konstruktion: Jede Änderung invalidiert automatisch alles danach.
- Undo/Redo profitiert ohne Zusatzaufwand, weil alte Schlüssel noch im Cache sind.
  Der Speicher muss begrenzt werden (LRU).

## Verworfene Alternativen

- **Dirty-Flags** – fehleranfällig, besonders bei Undo und Umsortieren.
- **Immer alles neu berechnen** – zu langsam bei großen Modellen.

## Verweise

- ADR-0005
