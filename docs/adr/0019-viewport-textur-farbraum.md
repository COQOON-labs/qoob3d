---
id: ADR-0019
title: Viewport offscreen in sRGB rendern, egui eine Unorm-View derselben Textur geben
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-render, qoob3d-app]
tags: [rendering, wgpu, farbraum]
supersedes: [ADR-0007]
superseded_by: null
---

## Kontext

ADR-0007 begründete das Format `Rgba8UnormSrgb` der an egui übergebenen Textur damit, dass
egui sie „wie eigene sRGB-Texturen“ sampelt. Das ist falsch: egui speichert seine Texturen als
`Rgba8Unorm` mit gamma-kodierten Werten und behandelt Samples als gamma
(`tex_gamma = sample_texture(...)`). Eine sRGB-View liefert beim Sampeln aber *lineare*
Werte. Folge: Der Viewport erschien deutlich zu dunkel (Oberseite Luma ≈ 117 statt ≈ 175).
Aufgefallen ist das erst im UI-Screenshot (ADR-0018).

## Entscheidung

Alles aus ADR-0007 gilt weiter: offscreen, 4× MSAA, Depth32Float, Reverse-Z (Clear 0,
`Greater`), Anzeige als native egui-Textur, `COPY_SRC` für Readback, wgpu nur über
`qoob3d_render::wgpu`. Geändert wird:

- Gerendert und MSAA-aufgelöst wird weiterhin in **`Rgba8UnormSrgb`** (`COLOR_FORMAT`),
  damit Shading, Blending und Resolve linear rechnen.
- Die aufgelöste Textur erlaubt zusätzlich die View `Rgba8Unorm` (`view_formats`).
  **egui bekommt diese Unorm-View** (`DISPLAY_FORMAT`). Sie liest die gespeicherten
  sRGB-Bytes unverändert, also gamma, genau wie egui es erwartet.
- Readback (`--render`, Tests) liest die Bytes der Textur. Die sind sRGB-kodiert und damit
  direkt PNG-tauglich.
- Regressionstest: `viewport_is_not_gamma_darkened` in `crates/qoob3d-app/tests/ui.rs`.

## Konsequenzen

- UI und `--render` zeigen identische Farben.
- Neue Texturen, die egui anzeigt, müssen dieselbe Regel befolgen (gamma-Werte liefern).

## Verworfene Alternativen

- **In `Rgba8Unorm` rendern und im Shader selbst gamma-kodieren** – dann würden Blending und
  MSAA-Resolve im Gamma-Raum rechnen.
- **Die Farbe im egui-Callback korrigieren** – egui bietet dafür keinen Hook.

## Verweise

- `crates/qoob3d-render/src/renderer.rs` (`COLOR_FORMAT`, `DISPLAY_FORMAT`, `create_targets`)
- ADR-0007 (ersetzt), ADR-0018
