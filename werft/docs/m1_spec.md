# M1-Spezifikation: Kern und CLI

Grundlage: `docs/konzept.md` (Abschnitte 4, 5, 8) und `docs/m0_bericht.md`
(Abschnitte 5–7). Diese Datei ist verbindlich für Tests und Implementierung.
Bezeichner sind deutsch wie im Konzept, ohne Umlaute im Code (ae/oe/ue/ss).

## Paket

- Python-Paket `werft` unter `werft/src/werft/`, installiert über
  `pyproject.toml` (src-Layout, Build-Backend hatchling), Einstiegspunkt
  `werft = "werft.cli:main"`.
- Tests unter `werft/tests/` (pytest). Laufzeit aller M1-Tests < 120 s.
- Neue Abhängigkeiten nur: `psutil`, `pytest` (dev).

## Projektordner

```
projekt/
  werft.toml
  soll/*.json
  lib/*.py          # Hilfsfunktionen des Projekts, importiert als `lib.<name>`
  parts/*.py        # Teile
  .werft/           # Zwischenspeicher und Dienstdaten, nicht in Git
```

`werft.toml` (alle Schlüssel optional, das sind die Vorgaben):
```toml
[projekt]
name = "<Ordnername>"
einheit = "mm"        # nur "mm" erlaubt

[grenzen]
zeit_s = 60           # Zeitgrenze je Bauauftrag (je Teil)
speicher_mb = 4096    # RSS-Grenze je Arbeitsprozess

[arbeiter]
anzahl = 2            # Anzahl warmer Arbeitsprozesse
```

## Sollwerte (`soll/*.json`)

Jede Datei ist eine JSON-Liste von Einträgen:
```json
[{"id": "kran.K1", "wert": {"d": 13, "h": 20}, "quelle": "Foto 12, Seitenansicht", "status": "gemessen"}]
```
- Pflichtfelder: `id` (eindeutig über alle Dateien), `wert` (Zahl oder Objekt
  aus Zahlen), `quelle` (nicht leer), `status` aus `gemessen`, `eine_quelle`,
  `annahme`.
- Fehler beim Laden: `werft.SollFehler`, Meldung enthält Dateiname und `id`
  (bzw. Index des Eintrags, wenn `id` fehlt).
- `werft.soll(id)` liefert ein Objekt: bei Zahlenwert `.wert`, bei Objekt
  zusätzlich Attributzugriff (`s.d`, `s.h`) und `.wert` (das dict); dazu
  `.quelle`, `.status`. Jeder Aufruf während eines Baus wird protokolliert
  (id und Wert).
- `werft.annahmen(projekt)` → Liste aller Einträge mit Status `annahme`.

## Teile (`parts/*.py`)

```python
from build123d import Box
from werft import teil, soll

@teil("Deckel", haengt_von=["Kiste"], material="PLA", farbe="weiss")
def bauen(dep):
    kiste = dep["Kiste"]          # fertig gebauter Körper der Abhängigkeit
    ...
    return koerper                # build123d Shape (Part/Solid/Compound)
```
- `@teil(name, *, haengt_von=(), gruppe=None, material=None, farbe=None, druck=None)`.
- Eine Datei darf mehrere Teile enthalten. Teilenamen sind projektweit eindeutig
  (sonst Fehler beim Laden mit beiden Dateinamen).
- Rückgabe muss ein build123d-Shape mit Volumen > 0 sein, sonst Baufehler.
- Unbekannter Name in `haengt_von` oder ein Zyklus: Fehler beim Laden, Meldung
  nennt die Teile.

## Baugraph und Zwischenspeicher

Ein Teil wird neu gebaut genau dann, wenn sich eines davon geändert hat:
1. Inhalt seiner Datei in `parts/`,
2. Inhalt jeder Datei in `lib/`, die die Teildatei direkt oder transitiv
   importiert (`import lib.x`, `from lib.x import y`, `from lib import x`;
   ermittelt per AST, nicht per Ausführen),
3. die Werte der Sollwerte, die beim letzten Bau gelesen wurden,
4. der Cache-Schlüssel einer Abhängigkeit aus `haengt_von`,
5. die Version von build123d oder werft.

Ändert sich Datei A in `parts/`, werden alle Teile dieser Datei neu gebaut.
Ändert sich etwas anderes in `soll/` als die gelesenen Werte, wird nichts neu
gebaut.

Ablage: `.werft/cache/<teil>/<schluessel>.brep` und `<schluessel>.json`
(Metadaten: `volumen`, `bbox` [dx, dy, dz], `gueltig`, `flaechen`,
`soll_zugriffe` {id: wert}, `dauer_s`, `warnungen`). Ein Bau aus dem
Zwischenspeicher liest nur das JSON (und das BREP nur, wenn ein abhängiges
Teil es braucht).

## Arbeitsprozesse

- Pool aus `arbeiter.anzahl` Prozessen (multiprocessing, `spawn`), die beim
  Start `build123d` importieren (warm).
- Jeder Bauauftrag läuft in einem Pool-Prozess. Überschreitet er `zeit_s` oder
  `speicher_mb` (RSS, per psutil alle 0,2 s geprüft), wird der Prozess getötet
  (SIGKILL) und durch einen neuen ersetzt. Das Ergebnis des Auftrags ist dann
  ein Fehler mit `art = "zeitgrenze"` bzw. `"speichergrenze"`.
- Ein abgestürzter Prozess (z. B. Segfault) wird ersetzt; Fehler `art = "absturz"`.
- Ein Fehler im Teilcode liefert `art = "teilfehler"` mit Datei, Zeile und
  Meldung; der Prozess bleibt im Pool.
- Der Dienstprozess importiert build123d nie.

## Dienst `werftd`

- Start: `werft dienst start` (im Hintergrund) bzw. automatisch durch jeden
  CLI-Befehl, wenn kein Dienst für das Projekt läuft. `werft dienst stop` beendet
  ihn samt Arbeitsprozessen.
- Lauscht auf 127.0.0.1, freier Port; schreibt `.werft/dienst.json`
  (`{"port": …, "pid": …}`). Ein Dienst je Projektordner.
- HTTP mit JSON. Jede Antwort hat die Hülle
  `{"ok": bool, "dauer_s": float, "warnungen": [str], "ergebnis": …}`;
  bei `ok = false` zusätzlich `"fehler": {"art": str, "meldung": str, …}`.
- Endpunkte:
  - `GET /status` → `ergebnis`: `{"projekt", "teile": [{"name", "datei",
    "zustand": "aktuell"|"veraltet"|"fehler"|"nie_gebaut"}], "auftraege":
    [{"id", "zustand"}], "arbeiter": {"anzahl", "frei"}}`. Rechnet keine
    Geometrie; muss auch während laufender Bauaufträge < 100 ms antworten.
  - `POST /bauen` Body `{"teile": [..] | null, "warten_s": float}` → baut die
    genannten Teile (null = alle) samt nötiger Abhängigkeiten. Ist der Auftrag
    innerhalb `warten_s` fertig, enthält `ergebnis` direkt das Ergebnis, sonst
    `{"auftrag": id, "zustand": "laeuft"}`.
  - `GET /auftrag/<id>` → `{"id", "zustand": "wartet"|"laeuft"|"fertig"|"fehler"|"abgebrochen", "fortschritt": {"fertig": n, "gesamt": m}, "ergebnis": …}`.
  - `POST /auftrag/<id>/abbrechen` → bricht ab (laufende Teile: Prozess töten
    und ersetzen).
  - `POST /export` Body `{"teile": [..], "format": "step"|"stl"|"3mf"}` →
    schreibt nach `out/<teil>.<format>`, baut vorher wenn nötig.
- Ergebnis eines Bauauftrags: `{"teile": {name: {"zustand":
  "gebaut"|"aus_cache"|"fehler", "volumen", "bbox", "gueltig", "dauer_s",
  "fehler"?}}, "gebaut": [namen], "aus_cache": [namen]}`.

## CLI `werft`

Alle Befehle arbeiten im Projektordner (aktuelles Verzeichnis oder `-C pfad`),
geben die JSON-Hülle des Dienstes auf stdout aus und enden mit Exit-Code 0 bei
`ok = true`, sonst 1.

- `werft status`
- `werft bauen [TEIL ...] [--warten S]` (Vorgabe 60)
- `werft auftrag ID [--abbrechen]`
- `werft export TEIL ... --format step|stl|3mf`
- `werft dienst start|stop`
- `werft annahmen` (lokal, ohne Dienst)

## Hilfsbibliothek `werft.hilfen` (Regeln aus M0)

- `draht(punkte, d, biegeradius)`: Rohr entlang einer Polylinie mit Bögen
  (`FilletPolyline` + `sweep`). `ValueError`, wenn `biegeradius <= d/2`.
- `zapfen(d, l)` / `loch(d, l, spiel=0.2)`: Zylinder ab z=0 nach −Z; das Loch
  hat Durchmesser `d + 2*spiel` und Tiefe `l + spiel`.
- `strahl(form, start, richtung)` → nach Abstand sortierte Liste von
  Trefferpunkten `(x, y, z)` auf Flächen (nicht `Solid & Axis`).
- `pruefe_volumen(vorher, nachher, art)`: `art` in `"abziehen"`, `"vereinigen"`,
  `"schale"`. Wirft `werft.PlausibilitaetsFehler`, wenn das Ergebnis Volumen
  ≤ 1e-6 mm³ hat, bei `abziehen`/`schale` nicht kleiner wird oder bei
  `vereinigen` nicht wächst bzw. gleich bleibt.

## Abnahme M1 (als pytest)

1. Ein Teil mit Endlosschleife wird bei `zeit_s = 3` nach 3–5 s beendet;
   `GET /status` antwortet währenddessen bei jeder von 20 Abfragen in < 100 ms;
   danach baut ein normales Teil wieder erfolgreich.
2. Projekt mit Teilen A, B (haengt_von A), C (nutzt `lib/form.py`), D
   (unabhängig). Erster Bau: alle vier gebaut. Zweiter Bau ohne Änderung: alle
   aus dem Cache. Änderung an A → genau A und B gebaut. Änderung an
   `lib/form.py` → genau C. Änderung eines von A gelesenen Sollwerts → A und B.
   Änderung eines nicht gelesenen Sollwerts → nichts.
3. Bau komplett aus dem Cache in < 0,5 s (Dienst läuft, warm).
4. Sollwert ohne `quelle` → `SollFehler` mit Dateiname.
5. Teil, das schrittweise 2 GB belegt (z. B. Liste aus `b"x" * 10_000_000`),
   bei `speicher_mb = 500` → `speichergrenze`, innerhalb von 10 s.
6. Teilfehler (`1/0`) → `teilfehler` mit Zeilennummer; Pool bleibt nutzbar.
7. CLI: `werft bauen` im Beispielprojekt liefert Exit 0 und gültiges JSON.
