---
id: ADR-0035
title: Feature-Befehle sind Dialoge mit Auswahl im Viewport und Live-Vorschau über den normalen Rebuild
status: akzeptiert
date: 2026-10-14
crates: [qoob3d-app]
tags: [ui, interaktion]
supersedes: []
superseded_by: null
---

## Kontext

Extrudieren, Drehen, Erhebung, Ebene, Zahnrad und Spirale brauchen Parameter und Auswahlen
(Profile, Achse, Punkte). Fusion zeigt dafür einen Dialog rechts und eine Vorschau, die sich
bei jeder Eingabe aktualisiert.

## Entscheidung

- `commands.rs`: ein `Command` hält die Parameter **eines** Features (`FeatureKind`). Der
  Modus `Mode::Command` zeigt den Dialog am rechten Rand des Viewports (Felder mit
  Ausdrücken wie im HUD, Ebene als Auswahlliste, Operation, OK/Abbrechen).
- Auswahl im Viewport über einen aktiven **Selektor**: Profile (Extrudieren, Drehen),
  Achse (Skizzenlinie; Ursprungsachsen als Knöpfe), Abschnitte (Erhebung: Profile und
  Punkte in Klickreihenfolge). Treffer im Raum über `View::to_local` (Strahl auf die
  Skizzenebene) bzw. Bildschirmabstand für Punkte und Linien.
- **Vorschau ohne Sonderweg:** Jedes Bild wird das Dokument mit dem Feature über
  `Project::preview_doc` gesetzt (ohne Undo-Schritt); der Kernel rechnet wie bei jeder
  Änderung, der Cache (ADR-0031) macht das billig. OK = ein Undo-Schritt, Abbrechen =
  `cancel_preview`.
- Komfort wie Fusion: E öffnet Extrudieren; hat die neueste sichtbare Skizze genau ein
  Profil, ist es vorgewählt (Enter genügt). Verbrauchte Skizzen werden beim OK ausgeblendet.
  Doppelklick auf ein Feature in der Timeline öffnet denselben Dialog zum Bearbeiten.
- Enter = OK, Esc = Abbrechen, solange kein Textfeld den Fokus hat.

## Konsequenzen

- Jede Dialogeingabe löst einen Rebuild aus; bei teuren Modellen kann die Vorschau
  nachlaufen (die alte Geometrie bleibt sichtbar, ADR-0005).
- UI-Tests müssen nach dem Öffnen eines Dialogs einige Frames warten: egui misst eine neue
  Area im ersten Frame und platziert sie im zweiten.

## Verworfene Alternativen

- **Eigene Vorschau-Geometrie im Dialog** – doppelter Code, Abweichungen zwischen Vorschau
  und Ergebnis.
- **Modale Fenster** – verdecken das Modell, Auswahl im Viewport wäre blockiert.

## Verweise

- Code: `crates/qoob3d-app/src/{commands,app,project}.rs`
- Tests: `crates/qoob3d-app/tests/modeling.rs`
- Verwandt: ADR-0027, ADR-0029, ADR-0031
