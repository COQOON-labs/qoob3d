---
id: ADR-0007
title: Der Viewport rendert offscreen mit 4× MSAA und Reverse-Z und wird als egui-Textur angezeigt
status: akzeptiert
date: 2026-10-13
crates: [qoob3d-render, qoob3d-app]
tags: [rendering, wgpu]
supersedes: []
superseded_by: null
---

## Kontext

egui bietet zwei Wege für eigene 3D-Inhalte: Paint-Callbacks, die im Render-Pass von egui
zeichnen (ohne eigenen Tiefenpuffer und ohne eigenes MSAA), oder eine eigene Textur.
CAD braucht präzise Tiefe über große Distanzen (0,01 mm bis Meter), Kantenglättung und
später einen ID-Puffer fürs Picking.

## Entscheidung

- `ViewportRenderer::render` zeichnet in eigene Targets: Farbe `Rgba8UnormSrgb` mit 4× MSAA,
  aufgelöst in eine einzelne Sample-Textur, und Tiefe `Depth32Float`.
- **Reverse-Z** mit unendlicher Perspektive: Tiefe wird mit 0 gelöscht, Vergleich `Greater`.
- Die aufgelöste Textur wird über `register_native_texture` in egui registriert und bei
  Größenänderung mit `update_egui_texture_from_wgpu_texture` aktualisiert.
- Shader rechnen linear, das sRGB-Format übernimmt die Gamma-Kodierung.
- wgpu wird ausschließlich über `qoob3d_render::wgpu` (= `eframe::egui_wgpu::wgpu`) benutzt.
- Die Textur hat `COPY_SRC`, damit Headless-Tests und die CLI (`--render`) zurücklesen können.

## Konsequenzen

- Eigene Render-Passes sind frei kombinierbar (Picking, Kanten, Overlay).
- Headless-Rendering ohne Fenster ist möglich – Grundlage der visuellen Tests (ADR-0013).
- Ein zusätzlicher Kopier- und Sample-Schritt pro Frame ist vernachlässigbar.

## Verworfene Alternativen

- **egui-Paint-Callback** – kein eigener Tiefenpuffer und kein eigenes MSAA, schwer headless zu testen.
- **Klassisches Z mit near/far** – Z-Fighting bei großen Distanzbereichen.

## Verweise

- `crates/qoob3d-render/src/renderer.rs`, `crates/qoob3d-render/src/shaders/scene.wgsl`
