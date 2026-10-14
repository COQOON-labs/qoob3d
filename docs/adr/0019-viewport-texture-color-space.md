---
id: ADR-0019
title: Render the viewport offscreen in sRGB, give egui a Unorm view of the same texture
status: accepted
date: 2026-10-13
crates: [qoob3d-render, qoob3d-app]
tags: [rendering, wgpu, color-space]
supersedes: [ADR-0007]
superseded_by: null
---

## Context

ADR-0007 justified the format `Rgba8UnormSrgb` of the texture passed to egui by saying that
egui samples it "like its own sRGB textures". That is wrong: egui stores its textures as
`Rgba8Unorm` with gamma-encoded values and treats samples as gamma
(`tex_gamma = sample_texture(...)`). An sRGB view, however, returns *linear* values when
sampled. Result: the viewport appeared considerably too dark (top face luma ≈ 117 instead of
≈ 175). This was only noticed in the UI screenshot (ADR-0018).

## Decision

Everything from ADR-0007 still applies: offscreen, 4× MSAA, Depth32Float, reverse Z (clear 0,
`Greater`), display as a native egui texture, `COPY_SRC` for readback, wgpu only via
`qoob3d_render::wgpu`. What changes:

- Rendering and MSAA resolve still happen in **`Rgba8UnormSrgb`** (`COLOR_FORMAT`),
  so that shading, blending and resolve compute linearly.
- The resolved texture additionally allows the view `Rgba8Unorm` (`view_formats`).
  **egui gets this Unorm view** (`DISPLAY_FORMAT`). It reads the stored
  sRGB bytes unchanged, i.e. gamma, exactly as egui expects.
- Readback (`--render`, tests) reads the bytes of the texture. They are sRGB-encoded and thus
  directly suitable for PNG.
- Regression test: `viewport_is_not_gamma_darkened` in `crates/qoob3d-app/tests/ui.rs`.

## Consequences

- UI and `--render` show identical colors.
- New textures that egui displays must follow the same rule (deliver gamma values).

## Rejected alternatives

- **Render in `Rgba8Unorm` and gamma-encode in the shader ourselves** – then blending and
  MSAA resolve would compute in gamma space.
- **Correct the color in the egui callback** – egui offers no hook for that.

## References

- `crates/qoob3d-render/src/renderer.rs` (`COLOR_FORMAT`, `DISPLAY_FORMAT`, `create_targets`)
- ADR-0007 (superseded), ADR-0018
