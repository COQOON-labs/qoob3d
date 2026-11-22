# Konzept „Werft“ – 3D-Konstruktionsumgebung für den Mac, für KI-Agenten gebaut

Stand 22.11.2026. Entstanden aus 37 Bau-Etappen eines RC-Schiffsmodells
(Damen Stan Patrol 5009, 1:40) in Autodesk Fusion über den Fusion-MCP-Server.

## 1. Kurzfassung
- Kein Fusion-Ersatz, sondern eine Umgebung, in der das Modell Programmcode in
  einem Git-Ordner ist. Agent und Mensch arbeiten am selben, lesbaren Modell.
- Geometriekern nicht selbst schreiben: Open Cascade (OCCT) über die
  Python-Bibliothek build123d (Apache-2.0).
- Es gibt build123d-mcp (Apache-2.0), einen MCP-Server, der build123d-Code
  ausführt, rendert, misst und exportiert. Zuerst prüfen und darauf aufbauen.
- Neu zu bauen: Projektstruktur, Hintergrundaufträge, Zwischenspeicher,
  Prüfungen als Tests, Bildmessung aus Fotos, Druckanalyse, Mac-App.
- Vorrang haben: (a) Aufwand klein halten, eigenes Werkzeug, nichts Unnötiges;
  (b) der Mensch soll mit der Maus konstruieren können (7a); (c) Geometrie aus
  Fotos ableiten ist Kernmerkmal (6a), rechnerisch und mit möglichst wenig KI.
- Grenzen: OCCT ist bei schwierigen Verrundungen, Schalen und Freiformflächen
  empfindlicher als Fusion. Abhilfe: einfache, robuste Operationen; notfalls
  STEP-Import.

## 2. Erfahrungen mit Fusion und dem Fusion-MCP

### 2.1 Gut (übernehmen)
| Erfahrung | Folgerung |
|---|---|
| Freies Python-Skript mit voller API war das mächtigste Werkzeug | Kernwerkzeug „Code ausführen“; wenige, mächtige Werkzeuge |
| Exakte Körper, Schnitte, Abstand, Durchdringung, Strahlen | BREP-Kern; Messen und Kollision als fertige Werkzeuge |
| Form erst unverbindlich bauen, ansehen, dann übernehmen | Jeder Lauf ist Entwurf, bis er eingecheckt wird |
| Kamera aus dem Foto nachbilden, Umriss über das Foto legen | Foto-Abgleich als eingebautes Werkzeug |
| Prüfskript nach jeder Änderung | Prüfungen als pytest, laufen automatisch |
| Benannte Körper, Komponenten, Farben | Benannte Teile in Baugruppen, Farben als Daten |

### 2.2 Schlecht (lösen)
| Problem | Lösung |
|---|---|
| Jeder Aufruf musste unter ~80 s bleiben; lange Prüfungen nur über Tricks | Aufträge im Hintergrund mit Kennung, Fortschritt, Abbruch |
| Ein Analyseskript hat Fusion blockiert, MCP-Server fiel aus, ungespeicherte Arbeit gefährdet | Jeder Lauf im eigenen Prozess mit Zeit- und Speichergrenze; Dienst bleibt ansprechbar |
| Speichern nur als Ganzes, kein Vergleich, kein Zurück | Modell = Textdateien in Git; Absturz verliert nichts |
| Nur ein Agent darf schreiben | Kopflose Läufe; mehrere Agenten in Git-Zweigen/Worktrees |
| Zeitleiste mit 1.360 Einträgen, frühe Änderungen riskant | Abhängigkeitsgraph von Teilen; Zwischenspeicher (Hash aus Code + Parametern + Eingängen) |
| Sollwerte und Prüfskript als Attribute im Dokument | Sollwerte als JSON/TOML, Prüfungen als Python-Dateien |
| Bilder nur über Umstellen der einen Ansicht | Rendern zustandslos und kopflos; Kamera, Sichtbarkeit, Schnitt als Parameter |
| Druckanalyse von Hand über Netze, Auslöser des Hängers | Eingebaute Druckanalyse als Hintergrundauftrag |
| Fehldeutungen von Fotos fielen erst dem Nutzer auf | Foto-Abgleich ist Teil der Abnahme jedes Teils |
| API-Fallen, Einheit cm | Einheit durchgehend mm; kleine getestete Hilfsbibliothek |

### 2.3 Was Fusion besser kann (nicht nachbauen)
Sehr robuste Verrundungen/Schalen/Lofts, Freiformflächen, CAM,
Zeichnungsableitung, Fotorendering. Dafür bleibt STEP hinein/hinaus.

## 3. Ziele und Nicht-Ziele
Ziele:
1. Ein Agent baut ein Modell vollständig als Code, prüft, bebildert und
   exportiert es für den FDM-Druck, ohne Bedienoberfläche.
2. Kein Lauf kann die Umgebung blockieren; nichts geht durch Absturz verloren.
3. Der Mensch sieht jederzeit den Stand: drehen, schneiden, messen, Teile
   ausblenden, Stände vergleichen, Anmerkungen an Stellen im Modell heften.
4. Jede Zahl hat eine Quelle oder ist als Annahme markiert; die Datenstruktur
   erzwingt das.
5. Der Mensch kann die üblichen Handgriffe mit der Maus ausführen (7a).
6. Aus Fotos echter Gegenstände lassen sich Masse und Geometrie nachvollziehbar
   ableiten (6a).

Nicht-Ziele: CAM, Simulation, technische Zeichnungen, Fotorendering, Cloud,
Mehrbenutzer, Verkauf, eigener Geometriekern, Windows/Linux-App.

## 4. Architektur
```
Mac-App „Werft“ (SwiftUI): 3D-Ansicht, Teilebaum, Schnitt, Messen, Vergleich,
                           Foto-Abgleich, Anmerkungen, Prüfampel, Maus-Bedienung
        │ lokale Schnittstelle (HTTP/WebSocket)
werftd (Python-Dienst):    Auftragsverwaltung (Warteschlange, Arbeitsprozesse,
                           Grenzen, Abbruch); Baugraph + Zwischenspeicher;
                           Werkzeuge bauen, prüfen, messen, rendern, foto,
                           druck, export
        │ MCP (stdio)                         │ CLI `werft …`
KI-Agent                                 Mensch/Skripte
        │
Projektordner (Git): parts/*.py, soll/*.json, checks/*.py, fotos/, out/
```
- Kern: Python 3.12, build123d. Jeder Bau- oder Analyseauftrag läuft in einem
  eigenen Arbeitsprozess. Der Dienst selbst rechnet nie Geometrie.
- Projekt = Ordner in Git, keine Binär-Projektdatei. Zwischenspeicher
  (.werft/cache) nicht in Git, jederzeit neu erzeugbar.
- MCP-Server und CLI sind dünne Hüllen um dieselben Funktionen.
- Rendern für den Agenten kopflos im Kern (VTK oder pyrender, offscreen), damit
  Bilder auch bei gesperrtem Mac oder geschlossener App entstehen.

## 5. Projektstruktur und Datenmodell
```
projekt/
  werft.toml         # Einheit mm, Massstab, Drucker (Bett, Düse), Material
  soll/*.json        # Sollwerte: {"id","wert","quelle","status"}
  fotos/             # Fotos + Kameramodelle (cam_*.json)
  lib/               # eigene Hilfsfunktionen (Draht, Zapfen, Relingfeld …)
  parts/*.py         # je Teil eine Funktion
  assembly.py        # Baugruppe: Lage, fest/abnehmbar, Verbindungen
  checks/*.py        # pytest
  views.toml         # benannte Ansichten
  print/plan.toml    # Drucklage und Einstellungen je Teil
  out/               # Exporte, Bilder, Berichte
```
```python
from werft import teil, soll, zapfen
from build123d import *

@teil("Kran_Saeule", gruppe="Ausruestung", material="PLA", farbe="weiss",
      druck=dict(lage="stehend"))
def bauen(dep):
    s = soll("kran.K1")          # Zugriff protokolliert die Quelle
    saeule = Cylinder(s.d/2, s.h, align=(Align.CENTER, Align.CENTER, Align.MIN))
    return saeule + zapfen(d=4, l=2.45, richtung=-Z)
```
- Jedes Teil ist eine reine Funktion: gleiche Eingaben, gleicher Körper.
- Stabile Bezüge: Flächen/Kanten über Auswahl nach Lage/Art oder über beim Bau
  vergebene Namen, nie über Indizes.
- Verbindungen (Zapfen/Loch, Spiel, Magnet) einmal in assembly.py erklären; die
  Umgebung erzeugt Zapfen, passendes Loch und die zugehörige Prüfung.
- Quellenpflicht: soll() verlangt quelle und status (gemessen / eine Quelle /
  Annahme). Ein Bericht listet alle Annahmen eines Teils.

## 6. Werkzeuge für den Agenten (MCP)
| Werkzeug | Zweck |
|---|---|
| projekt_status | Teile, Zwischenspeicher, letzte Prüfung, Aufträge, Git-Stand |
| bauen(teile, warten_s) | baut Geändertes; Ergebnis oder Auftragskennung |
| auftrag(id, status/ergebnis/abbrechen) | Fortschritt, Ergebnis, Abbruch |
| ausfuehren(code, teile) | freier Code in Wegwerf-Sitzung; ändert das Projekt nicht |
| messen(abstand/durchdringung/strahl/masse/huelle) | fertige Messungen |
| rendern(ansicht/kamera, teile, schnitt, stil, groesse) | PNG kopflos |
| foto_abgleich(foto, teile, modus) | Foto entzerrt, Umriss darüber, Abweichung in px/mm |
| pruefen(auswahl) | pytest; Liste OK/HINWEIS/FEHLER mit Messwert |
| druck_analyse(teil, lage) | Überhänge, Brücken, Wandstärke, Querschnitte, Auflage, Bett |
| vergleich(stand_a, stand_b, teile) | Volumen, Hülle, grösste Abweichung, Differenzbild |
| export(teile, step/stl/3mf, drucklage) | Dateien nach out/ |
| anmerkungen(lesen/erledigen) | Anmerkungen des Menschen aus der App |
| stand_sichern(nachricht) | Git-Commit nach bestandener Prüfung |

Dateien bearbeitet der Agent mit seinen normalen Dateiwerkzeugen. Antworten
immer als JSON mit ok, dauer_s, warnungen, Ergebnis; Zahlen in mm; Fehler mit
Datei/Zeile; Listen gekürzt mit Zähler.

## 6a. Geometrie aus Fotos („Bildmessung“) – Kernmerkmal
Grundsatz: klassische Photogrammetrie, jede Zahl mit Fehlerangabe; KI nur als
Vorschlag, nie als Messwert. Umsetzung mit OpenCV/NumPy/SciPy.

| Baustein | Was er tut | Ergebnis |
|---|---|---|
| Kamera einmessen | aus ≥ 6 Passpunkten Lage, Brennweite, Verzeichnung (PnP + Ausgleich) | Kameramodell mit Restfehler in px |
| Kamera ohne Passpunkte | Fluchtpunkte aus parallelen Kanten; bekannte Strecke als Massstab; oder Fotos gegenseitig orientieren (optional COLMAP) | Orientierung |
| Punkt triangulieren | derselbe Punkt in ≥ 2 Fotos; Epipolarlinie im zweiten Foto als Hilfe | 3D-Punkt mit Unsicherheit in mm, Schnittwinkel-Warnung |
| Punkt auf bekannter Fläche | Sehstrahl mit Ebene/Modellfläche schneiden | 3D-Punkt aus einem Foto |
| Ebene entzerren | Homographie → massstäbliches Orthobild | Skizzenunterlage mit mm-Massstab |
| Umriss und Kanten | Silhouette im Foto gegen Modellsilhouette aus derselben Kamera | Abweichung in px und mm |
| Profilkörper | Umriss eines Drehteils → Profilkurve; Ellipse → Durchmesser, Achse | Profil als Skizze |
| Dichte Rekonstruktion (optional) | viele Fotos → Punktwolke/Netz (COLMAP/OpenMVS) als Referenz | Netz zum Vergleichen |

Regeln:
- Jeder Messwert trägt Herkunft (Foto, Pixel, Verfahren), Restfehler und Status
  und landet automatisch in den Sollwert-Dateien.
- Schlechte Geometrie melden statt verschweigen: flacher Schnittwinkel,
  Passpunkte fast in einer Ebene, grosse Reste.
- KI darf helfen, Stellen zu finden; der Wert kommt immer aus der Rechnung, und
  Mensch oder Agent bestätigt den Punkt im Bild.
- In der App: Fotos nebeneinander, Punkt im ersten anklicken, Epipolarlinie im
  zweiten, zweiter Klick, 3D-Punkt erscheint im Modell.
Werkzeuge: kamera_einmessen, kamera_aus_fluchtpunkten, triangulieren,
strahl_auf_flaeche, entzerren, umriss_vergleich, profil_aus_foto,
rekonstruktion (Hintergrundauftrag).
Grenzen: Es entstehen Punkte, Kanten und Profile, keine fertigen Körper.
Verdeckte Seiten bleiben Annahme. Erfahrungswert: ±1–2 mm bei 1:40 aus
Drohnenfotos.

## 7. Die Mac-App (ansehen und eingreifen)
- 3D-Ansicht mit Teilebaum (ein/aus, durchsichtig), Schnittebene,
  Explosionsansicht, Massband.
- Aktualisiert sich selbst, sobald der Agent baut; zeigt neue Teile.
- Vergleich zweier Stände; Foto-Abgleich mit Überblendregler.
- Anmerkungen: Klick auf eine Stelle + Text → landet beim Agenten mit Teil,
  Punkt und Bildausschnitt.
- Prüfampel, Auftragsliste mit Abbrechen, Druckansicht mit eingefärbten
  Überhängen.
- Technik: SwiftUI + SceneKit/RealityKit, Netze als glTF/USDZ vom Dienst. Als
  schnelle erste Stufe genügt eine lokale Web-Ansicht (three.js) im WKWebView.

## 7a. Konstruieren mit der Maus
Leitidee: Jeder Handgriff mit der Maus schreibt eine Zeile Code ins Teil. Es
gibt keinen zweiten Datenbestand.
Erste Stufe:
- Grundkörper setzen (Quader, Zylinder, Kugel, Rohr), mit Griffen ziehen, Masse
  eintippen.
- Skizze auf Ebene oder Fläche: Linie, Rechteck, Kreis, Bogen, Masse eintippen,
  Fang auf Punkte/Mitten/Raster. Zunächst ohne Zwangslöser.
- Extrudieren, Drehen, Ausschneiden; Vereinigen/Abziehen/Schneiden;
  Verschieben/Drehen/Spiegeln/Muster.
- Runden, Fasen, Wandstärke, Bohrung, Zapfen-Loch-Verbindung mit Spiel.
- Messen, Schnitt, Rückgängig/Wiederholen über die Code-Historie.
- Fotounterlage auf der Skizzenebene (aus 6a).
Später, nur bei Bedarf: Zwangslöser, Loft/Sweep, Flächen ziehen, Gewinde, Text.

## 8. Meilensteine und Abnahme
M0 – Machbarkeit:
- build123d und build123d-mcp installieren, ausprobieren, bewerten.
- Drei Probeteile: Kransäule (Drehkörper mit Fasen und Zapfen), Treppe (zwei
  Wangen, 11 Stufen mit Schlitzen), Relingabschnitt aus Draht (Rohre mit
  Biegungen). Dazu ein grosser STEP-Körper: importieren, Abstand,
  Durchdringung und Strahl auf eine Fläche messen.
- Abnahme: Alle drei Teile entstehen; Volumen weicht < 1 % von den Werten in
  Abschnitt 10 ab, soweit die Masse dort vorgegeben sind; Bauzeit je Teil < 5 s;
  STEP-Import < 60 s und aus dem Zwischenspeicher < 2 s.
M1 – Kern und CLI: Projektstruktur, @teil, Baugraph, Zwischenspeicher,
  Arbeitsprozesse mit Grenzen. Abnahme: Ein endloses Teil wird nach Zeitgrenze
  beendet, der Dienst antwortet währenddessen; ein geändertes Teil baut nur
  sich und seine Abhängigen neu.
M2 – MCP-Server: Werkzeuge aus Abschnitt 6 ohne Foto/Druck. Abnahme: Ein
  frischer Agent baut nur über MCP eine Kiste mit Deckel und Zapfenverbindung
  samt Prüfung.
M3 – Rendern und Foto-Abgleich; parallel beginnt die Bildmessung (6a).
M4 – Druckanalyse und Export (3MF mit Drucklage).
M5 – Mac-App zum Ansehen; Beginn Maus-Bedienung (7a). Abnahme 7a: Ich baue
  ohne Hilfe mit der Maus eine Kiste mit Deckel, Zapfen und runden Kanten.
M6 – Anmerkungen und Vergleich.
M7 – Praxisprobe mit einem neuen kleinen Vorhaben.
Die Meilensteine sind Reihenfolge, keine Monatspläne. M0–M2 sind Stunden bis
wenige Tage Agentenarbeit.

## 9. Risiken
| Risiko | Gegenmassnahme |
|---|---|
| OCCT scheitert an Verrundungen/Schalen | M0-Probe; robuste Bauweise in lib/; STEP-Import; Fehler sauber melden |
| Boolesche Operationen an grossen Körpern langsam | Zwischenspeicher; mit Hüllkörpern vorfiltern; Hintergrundaufträge |
| Doppelte Arbeit zu build123d-mcp | M0-Entscheidung; lieber erweitern |
| Mac-App wird Zeitfresser | erst Web-Ansicht im Fenster |
| Umfang wächst | Nicht-Ziele gelten; Erweiterung nur mit Anlass aus der Praxis |
| Agentencode läuft auf dem Mac | Arbeitsprozess nur im Projektordner, ohne Netz |

## 10. Vergleichswerte aus dem Schiffsprojekt (Fusion)
- Treppe: Druckteil 2,4285 cm³; 11 Steigungen zu 5,91 mm, Neigung 52,4°, zwei
  Wangen 1,6 × 5 mm, aussen 20 mm breit, Höhe 65 mm, Stufen mit je 2 Schlitzen
  0,6 mm, 6 Pfostenaugen mit Bohrung Ø 0,9.
- Kransäule: Säule Ø 13 mm, Zapfen Ø 4 mm unten, Tasche 4,4 × 5,4 × 4,2 mm oben.
  Volumen nicht einzeln protokolliert; hier nur Bauzeit und Gültigkeit prüfen.
- Reling: Pfosten und Handlauf Draht Ø 0,8, zwei Durchzüge Ø 0,5 auf +8 und +15,
  Handlauf auf +22 mm, Pfostenteilung ~24 mm, Biegeradius ≥ 1,5 mm. Acht
  Drahtkörper zusammen 1,841 cm³ bei rund 2,8 m Ø 0,8 und 2,4 m Ø 0,5.
- Prüfskript in Fusion: 47 Prüfungen, Laufzeit 103–185 s.

## 11. Quellen
- build123d: https://github.com/gumyr/build123d , https://build123d.readthedocs.io/
- build123d-mcp: https://github.com/pzfreo/build123d-mcp , https://pypi.org/project/build123d-mcp/
