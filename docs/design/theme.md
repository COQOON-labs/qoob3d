# Theme und Layout

Beschreibende Spezifikation (keine Entscheidung). Begründungen stehen in den ADRs. Wird hier
etwas geändert, müssen die Konstanten im Code mitgezogen werden. Die Orte stehen jeweils dabei.
Aktuelle Optik prüfen: `cargo xtask screenshot` → `target/screenshots/ui.png`.

## Charakter

Dunkel, ruhig, wenig Kontrast im Chrome. Die Aufmerksamkeit gehört dem Modell.
**Eine** Akzentfarbe (Mint) für Auswahl, aktive Werkzeuge, Skizzenelemente und Fokus.
Kein zweiter Akzent. Fehler sind rot, Warnungen bernstein, beides sparsam.

## Farben – UI (sRGB, `crates/qoob3d-app/src/theme.rs`)

| Token | Wert | Verwendung |
| --- | --- | --- |
| `ACCENT` | `#3DE0B0` | Auswahl, aktives Werkzeug, Hover am ViewCube, Orbit-Pivot, Marke |
| `PANEL_BG` | `#1D1F23` | Werkzeugleiste, Browser |
| `PANEL_BG_DARK` | `#17191C` | Timeline, Eingabefelder |
| `STROKE` | `#2C3036` | Trennlinien, Rahmen |
| `TEXT` | `#D4D7DC` | Text |
| `TEXT_MUTED` | `#8A9099` | Beschriftungen (Captions), Statuszeile, Hinweise |
| `OVERLAY_BG` | `#1D1F23` mit 90 % Deckkraft | Navigationsleiste im Viewport |
| Warnung (Bernstein) | `#F5A623` | Sperrzonen (schraffiert), Einzeldüsen-Zonen (getönt) |

## Farben – Viewport (linear, `crates/qoob3d-render/src/renderer.rs::palette`)

| Token | Wert (linear) | Verwendung |
| --- | --- | --- |
| `BG_TOP` / `BG_BOTTOM` | `0.032, 0.036, 0.042` / `0.010, 0.011, 0.013` | Verlauf |
| `BODY` | `0.50, 0.53, 0.57` | Volumenkörper |
| `EDGE` | `0.006, 0.007, 0.008` | Kanten, 1,3 pt |
| `GRID_MINOR` / `GRID_MAJOR` | grau, α 0,35 / 0,6 | Raster (Zehnerstufen) |
| `PLATE_FILL` / `PLATE_BORDER` | etwas heller als Hintergrund / hellgrau | Druckbett |
| `AXIS_X` / `AXIS_Y` / `AXIS_Z` | rot / grün / blau | Achsen (Z als Linie bis ¼ Bettbreite) |

Licht: Hauptlicht von links oben aus Sicht des Betrachters, Aufhelllicht von rechts unten,
Himmel/Boden-Ambient. Dadurch sind Flächen unterschiedlicher Orientierung in jeder Ansicht
unterscheidbar.

## Layout

```text
┌──────────────────────────────────────────────────────────────────────┐
│ QOOB3D DESIGN │ FILE │ SKETCH │ CREATE │ EXPORT (Ribbon)   ⚙ Settings │
├────────────┬─────────────────────────────────────────────┬───────────┤
│ BROWSER    │ Status (triangles · edges · watertight · cm³)  🏠 ViewCube │
│ ▾ Untitled │                3D-Viewport                               │
│   ▸ Origin │                                                          │
│   ▾ Bodies │                                                          │
│   ▾ Sketches                                                         │
│            │ [⛶ Fit | Perspective Orthographic | Grid Build volume | Printer ▾] │
├────────────┴─────────────────────────────────────────────────────────┤
│ ⏮ ⏴ ⏵ ⏭ │ Timeline                                                  │
└──────────────────────────────────────────────────────────────────────┘
```

- Funktionen späterer Meilensteine sind sichtbar, aber deaktiviert, mit dem Hinweis
  „Kommt mit Mx“.
- ViewCube: 118 pt, oben rechts. Beschriftung FRONT/BACK/… (bzw. VORNE/HINTEN/…) liegt
  geschert in der Würfelfläche. Kanten und Ecken sind jeweils **eine** Auswahl über alle
  beteiligten Flächen und leuchten beim Überfahren gemeinsam Mint.

## Sprache

Basissprache ist **Englisch**, Deutsch ist ein Sprachpaket, wählbar unter ⚙ Settings
(ADR-0020). Texte sind kurz und ohne Ausrufezeichen, Workspace-Captions in Großbuchstaben
(DESIGN, SKETCH …). Einheiten werden immer angezeigt (`20 mm`). Das Dezimaltrennzeichen folgt
der Sprache (`15.70` / `15,70 cm³`).
