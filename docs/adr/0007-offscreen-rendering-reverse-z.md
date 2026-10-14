---
id: ADR-0007
title: The viewport renders offscreen with 4× MSAA and reverse Z and is shown as an egui texture
status: superseded
date: 2026-10-13
crates: [qoob3d-render, qoob3d-app]
tags: [rendering, wgpu]
supersedes: []
superseded_by: ADR-0019
---

## Context

egui offers two ways for custom 3D content: paint callbacks that draw in egui's render pass
(without an own depth buffer and without own MSAA), or an own texture.
CAD needs precise depth over large distances (0.01 mm up to meters), anti-aliasing and
later an ID buffer for picking.

## Decision

- `ViewportRenderer::render` draws into own targets: color `Rgba8UnormSrgb` with 4× MSAA,
  resolved into a single-sample texture, and depth `Depth32Float`.
- **Reverse Z** with infinite perspective: depth is cleared with 0, comparison `Greater`.
- The resolved texture is registered in egui via `register_native_texture` and, on
  resize, updated with `update_egui_texture_from_wgpu_texture`.
- Shaders compute linearly, the sRGB format takes care of gamma encoding.
- wgpu is used exclusively via `qoob3d_render::wgpu` (= `eframe::egui_wgpu::wgpu`).
- The texture has `COPY_SRC` so that headless tests and the CLI (`--render`) can read back.

## Consequences

- Own render passes can be combined freely (picking, edges, overlay).
- Headless rendering without a window is possible – the basis of the visual tests (ADR-0013).
- An additional copy and sample step per frame is negligible.

## Rejected alternatives

- **egui paint callback** – no own depth buffer and no own MSAA, hard to test headless.
- **Classic Z with near/far** – Z-fighting across large distance ranges.

## References

- `crates/qoob3d-render/src/renderer.rs`, `crates/qoob3d-render/src/shaders/scene.wgsl`
