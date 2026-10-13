# Theme und Layout

Beschreibende Spezifikation (keine Entscheidung). Die Begründungen stehen in den ADRs.
Wenn hier etwas geändert wird, müssen die Konstanten im Code mitgezogen werden. Die Orte
stehen jeweils dabei.

## Charakter

Dunkel, ruhig, wenig Kontrast im Chrome. Die Aufmerksamkeit gehört dem Modell.
**Eine** Akzentfarbe (Mint) für Auswahl, aktive Werkzeuge, Skizzenelemente und Fokus.
Kein zweiter Akzent. Fehler sind rot, Warnungen bernstein, beides sparsam.

## Farben

| Token | Wert | Verwendung | Code |
| --- | --- | --- | --- |
| `accent` | `#3DE0B0` (sRGB) | Auswahl, aktives Werkzeug, Skizze, Marke | `qoob3d-app/src/app.rs::ACCENT` |
| `viewport.bg_top` | linear `0.032, 0.036, 0.042` | Verlauf oben | `qoob3d-render/src/renderer.rs::palette` |
| `viewport.bg_bottom` | linear `0.010, 0.011, 0.013` | Verlauf unten | dito |
| `viewport.body` | linear `0.50, 0.53, 0.57` | Volumenkörper | dito |
| `text.muted` | `gray(140)` | Statuszeile, Hinweise | `app.rs` |

Viewport-Farben sind **linear** (Shader), UI-Farben sind **sRGB** (egui).

## Layout (Ziel ab M2)

```text
┌──────────────────────────────────────────────────────────────┐
│ Werkzeugleiste (Skizze · Extrusion · …)            [Datei]   │
├────────────┬─────────────────────────────────────┬───────────┤
│ Bauteil-   │                                     │  ViewCube │
│ baum       │            3D-Viewport              │           │
│ (Ursprung, │                                     │           │
│  Skizzen,  │                                     │           │
│  Körper)   │                                     │           │
├────────────┴─────────────────────────────────────┴───────────┤
│ Timeline: [Skizze1] [Extrusion1] [Skizze2] … ▮ Marker        │
└──────────────────────────────────────────────────────────────┘
```

## Sprache

UI-Texte sind deutsch, kurz und ohne Ausrufezeichen. Einheiten werden immer angezeigt (`20 mm`).
