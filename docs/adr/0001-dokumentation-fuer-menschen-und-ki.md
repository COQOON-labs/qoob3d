---
id: ADR-0001
title: Entscheidungen als ADRs, gestufte CLAUDE.md-Dateien und maschinell geprüfte Doku
status: akzeptiert
date: 2026-10-13
crates: [xtask]
tags: [dokumentation, prozess, ki]
supersedes: []
superseded_by: null
---

## Kontext

QOOB3D wird überwiegend mit einem KI-Agenten (Claude Code) entwickelt, über viele Sessions
hinweg. Der Agent hat kein Gedächtnis zwischen Sessions, außer dem, was im Repo steht.
Entscheidungen, die nur im Chat getroffen werden, gehen verloren oder werden später
unbemerkt rückgängig gemacht. Eine einzige lange Doku-Datei wird unübersichtlich und
veraltet still.

## Entscheidung

1. **Jede Architektur- oder Designentscheidung ist eine eigene Datei** `docs/adr/NNNN-titel.md`
   mit YAML-Kopfblock (`id, title, status, date, crates, tags, supersedes, superseded_by`)
   und den Abschnitten Kontext / Entscheidung / Konsequenzen / Verworfene Alternativen /
   Verweise. Vorlage: `docs/adr/0000-vorlage.md`. Index: `docs/adr/README.md`.
2. **Akzeptierte ADRs sind unveränderlich.** Eine Änderung der Entscheidung ist eine neue ADR,
   die die alte ersetzt. So bleibt das *Warum* nachvollziehbar.
3. **Gestufte Einstiegspunkte für den Agenten:**
   - `CLAUDE.md` (Wurzel): kurz – Zweck, Befehle, Arbeitsregeln, Fallstricke, Links.
   - `crates/<crate>/CLAUDE.md`: Zweck, Regeln und relevante ADRs dieses Crates. Claude Code
     lädt sie automatisch, sobald in dem Verzeichnis gearbeitet wird.
   - `docs/architecture.md`: Gesamtbild (Datenfluss, Threads, Crate-Grenzen).
   - `docs/design/`: Gestaltungs-Spezifikationen (Theme, Layout, Bedienung) – beschreibend,
     keine Entscheidungen.
4. **Code verweist auf Entscheidungen** mit `ADR-NNNN` in Kommentaren an den Stellen, die eine
   Entscheidung umsetzen.
5. **`cargo xtask docs` prüft maschinell** (und ist Teil von `cargo xtask check`):
   Kopfblock vollständig und gültig, ID passt zum Dateinamen, Index listet jede ADR mit
   aktuellem Titel und Status, jeder `ADR-NNNN`-Verweis in Code und Doku existiert,
   `supersedes`/`superseded_by` sind wechselseitig konsistent.

## Konsequenzen

- Der Agent findet Entscheidungen über den Index oder per Suche nach `ADR-` und kann
  ihren Status maschinell lesen.
- Eine Entscheidung zu ändern kostet eine neue Datei – gewollt, weil es zum Nachdenken zwingt.
- Wird beim Arbeiten eine neue Entscheidung getroffen, gehört die ADR in denselben Commit.

## Verworfene Alternativen

- **Alles in einer CLAUDE.md** – wächst unkontrolliert, Kontextfenster wird mit Irrelevantem
  gefüllt, Historie geht verloren.
- **Wiki / externes Tool** – für den Agenten nicht erreichbar, nicht versioniert mit dem Code.

## Verweise

- `CLAUDE.md`, `docs/adr/README.md`, `xtask/src/docs.rs`
