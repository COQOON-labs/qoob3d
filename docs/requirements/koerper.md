# Anforderungen: Körper, Features und Export

Stand: 2026-10-14 · Status: **umgesetzt in M4** (Ausnahmen in Abschnitt 6) · Entscheidungen:
ADR-0031 bis ADR-0036

Ziel: Mit QOOB3D lassen sich die Grundformen für den FDM-Druck Ende-zu-Ende bauen und als
Datei an den Slicer geben: **Quader, Zylinder, Kugel, Pyramide, Zahnrad, Spirale/Feder**.
IDs `KO-<Bereich>-<Nr>` sind stabil; Code, Tests und ADRs verweisen darauf.

## 1. Wie jede Form entsteht (Referenzablauf)

| Form | Ablauf in der UI | Referenzmodell | E2E-Test (`tests/modeling.rs`) |
| --- | --- | --- | --- |
| Quader | Skizze auf XY → Rechteck (R) mit Maßen → Extrudieren (E) | `models/box.qbd` | `box_from_rectangle_extrude_and_3mf_export` |
| Zylinder | Skizze auf XY → Kreis (C) → Extrudieren | `models/cylinder.qbd` | `cylinder_from_circle_and_extrude` |
| Kugel | Skizze auf XZ → Bogen (A) 180° + Linie → Drehen um X oder die Linie | `models/sphere.qbd` | `sphere_from_arc_and_revolve` |
| Pyramide | Quadrat auf XY → versetzte Ebene → Punkt darauf → Erhebung Profil → Punkt | `models/pyramid.qbd` | `pyramid_from_square_offset_plane_point_and_loft` |
| Zahnrad | CREATE → Spur Gear (Modul, Zähne, Dicke, Bohrung) | `models/gear.qbd` | `spur_gear_from_dialog` |
| Spirale/Feder | CREATE → Coil (Schraube oder flache Spirale) | `models/coil.qbd`, `models/spiral.qbd` | `coil_then_edited_into_a_flat_spiral` |
| Verbinden/Ausschneiden | Platte, Zapfen (Join), Bohrung (Cut) | `models/plate-with-hole.qbd` | Kernel-Test `join_and_cut` |

## 2. Features

| ID | Anforderung | Prio | Stand |
| --- | --- | --- | --- |
| KO-FT-01 | Extrudieren: Profile (auch mehrere, auch aus mehreren Skizzen) entlang der Normalen, Abstand negativ = Gegenrichtung, symmetrisch | MUSS | ✅ |
| KO-FT-02 | Drehen: Profile um Ursprungsachse oder Skizzenlinie, Winkel (0, 360] | MUSS | ✅ |
| KO-FT-03 | Erhebung: Profile in Reihenfolge, erster/letzter Abschnitt darf ein Punkt sein (Spitze) | MUSS | ✅ |
| KO-FT-04 | Versetzte Konstruktionsebene (auch von einer anderen Konstruktionsebene); Skizzen darauf | MUSS | ✅ |
| KO-FT-05 | Stirnrad (Evolvente, Modul, Zähne 6–400, Eingriffswinkel 10–35°, Dicke, Bohrung) | MUSS | ✅ |
| KO-FT-06 | Spirale: Schraubenfeder (Windungen, Steigung) und flache Spirale, runder Draht | MUSS | ✅ |
| KO-FT-07 | Operation je Feature: Neuer Körper, Verbinden, Ausschneiden (gegen alle berührten Körper) | MUSS | ✅ |
| KO-FT-08 | Fehlende Referenz (gelöschtes Profil) = Feature-Fehler mit Grund, nichts raten | MUSS | ✅ |
| KO-FT-09 | Features nachträglich bearbeiten (Doppelklick in der Timeline) | MUSS | ✅ |
| KO-FT-10 | Verjüngung (Taper) beim Extrudieren, Schale, Abrunden, Fasen | SOLL | M6 |
| KO-FT-11 | Explizite Teilnehmer-Auswahl für Verbinden/Ausschneiden | SOLL | offen |

## 3. Bedienung

| ID | Anforderung | Stand |
| --- | --- | --- |
| KO-UI-01 | Dialog rechts im Viewport, Felder mit Ausdrücken, Enter = OK, Esc = Abbrechen | ✅ |
| KO-UI-02 | Live-Vorschau = echtes Ergebnis, ohne Undo-Schritt bis OK | ✅ |
| KO-UI-03 | Auswahl im Viewport: Profile (Hover hell, gewählt kräftig), Linien als Achse, Punkte | ✅ |
| KO-UI-04 | E öffnet Extrudieren; einziges Profil der neuesten Skizze ist vorgewählt | ✅ |
| KO-UI-05 | Verbrauchte Skizzen werden ausgeblendet | ✅ |
| KO-UI-06 | Browser: Körper mit Volumen (ungültige rot), Konstruktionsebenen; Timeline mit Icons, Fehler rot mit Grund | ✅ |
| KO-UI-07 | Statuszeile: Körperzahl, Gesamtvolumen, dicht ✔/✘ | ✅ |

## 4. Export und Druckbarkeit

| ID | Anforderung | Stand |
| --- | --- | --- |
| KO-EX-01 | STL (binär) und 3MF (ein Objekt pro Körper, Millimeter) aus App und CLI | ✅ |
| KO-EX-02 | Export-Tessellierung 0,01 mm, verschweißte Ecken, keine entarteten Dreiecke | ✅ |
| KO-EX-03 | Jeder Körper wird geprüft: gültiges B-Rep, dichtes Netz, exaktes Volumen | ✅ |
| KO-EX-04 | STEP-Export | M6 |

## 5. Qualität (für Menschen und KI)

| ID | Anforderung | Stand |
| --- | --- | --- |
| KO-QA-01 | Jede Feature- und Kurvenart hat ein Referenzmodell mit analytischen Erwartungen (`cargo xtask models`) | ✅ |
| KO-QA-02 | Jede Form wird Ende-zu-Ende durch die UI gebaut und gemessen | ✅ |
| KO-QA-03 | CLI `--report` liefert JSON und Exit-Code 1, wenn etwas nicht druckbar ist | ✅ |
| KO-QA-04 | Dateiformate werden zusätzlich mit Fremdwerkzeugen geprüft (`unzip -t`) | ✅ |
| KO-QA-05 | Formatversion 2 mit Fixture, die jede Art enthält | ✅ |
| KO-QA-06 | Smoke-Test extrudiert in der echten App | ✅ |

## 6. Offen

- Timeline-Rollback und Unterdrücken (M5), Körperauswahl im Viewport, Taper/Abrunden (M6).
- Join/Cut ohne explizite Teilnehmer (KO-FT-11).
- Loft nur „ruled“ (gerade Verbindungen); glatte Lofts mit Führungskurven später.

## Verweise

- ADR-0031 (Features, Rebuild), ADR-0032 (Zahnrad, Spirale), ADR-0033 (Referenzmodelle),
  ADR-0034 (Export), ADR-0035 (Dialoge), ADR-0036 (Crate-Grenzen)
- `docs/requirements/skizzen.md`, `docs/design/bedienung.md`
