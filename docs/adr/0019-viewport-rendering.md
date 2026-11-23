---
id: ADR-0019
title: The viewport renders offscreen in sRGB with 4× MSAA and reverse Z, egui shows a Unorm view of the texture and draws the overlays itself
status: accepted
date: 2026-11-23
crates: [qoob3d-render, qoob3d-app]
tags: [rendering, wgpu, color-space, ui, viewcube]
supersedes: []
superseded_by: null
---

## Context

egui offers two ways for custom 3D content: paint callbacks inside egui's render pass (no own
depth buffer, no own MSAA, hard to test headless) or an own texture. CAD needs precise depth
from 0.01 mm to meters and anti-aliased edges. The same renderer must work without a window for
`render-project` and the tests (ADR-0013).

The first version gave egui an sRGB view of the texture, assuming egui samples it like its own
textures. It does not: egui stores gamma-encoded `Rgba8Unorm` textures and treats samples as
gamma, while an sRGB view returns linear values. The viewport appeared far too dark (top face
luma ≈ 117 instead of ≈ 175), which only a UI screenshot showed.

Fusion-style controls in the viewport (ViewCube with clickable faces, edges and corners and
their labels, home button, navigation bar, pivot marker, status line) would need a font atlas,
hit tests and hover logic twice if drawn in wgpu.

This topic absorbs the former decisions 0007 and 0015.

## Decision

### Offscreen with MSAA and reverse Z

- `ViewportRenderer::render` draws into own targets: color `COLOR_FORMAT` = `Rgba8UnormSrgb`
  with 4× MSAA, resolved into a single-sample texture; depth `Depth32Float`.
- **Reverse Z:** depth is cleared with 0 and compared with `Greater`. Perspective uses an
  infinite reverse projection (`perspective_infinite_reverse`), orthographic a linear
  reverse-Z mapping over a generous range around the camera distance.
- The renderer draws background, bodies, edges as pixel-wide quads, and grid, print bed and
  axes in one analytic full-screen pass (ADR-0021).
- wgpu comes only from eframe's re-export (`qoob3d_render::wgpu` = `egui_wgpu::wgpu`), so its
  version always matches egui's.

### Color space: sRGB render target, Unorm view for egui

- Shading, blending and MSAA resolve compute linearly in `Rgba8UnormSrgb`.
- The resolved texture also allows the view format `DISPLAY_FORMAT` = `Rgba8Unorm`. **egui gets
  this Unorm view** (`register_native_texture`, on resize
  `update_egui_texture_from_wgpu_texture`) and reads the stored sRGB bytes unchanged as gamma,
  exactly as it expects.
- The texture has `COPY_SRC`. Readback (`qoob3d_render::headless`, `render-project`, tests)
  reads the sRGB-encoded bytes, which go into the PNG as they are.

### Overlays are drawn with egui

- wgpu renders only the scene. egui draws on top: ViewCube (`viewcube.rs`), home button,
  navigation bar (`egui::Area` at the bottom centre, with fit, projection, grid, build volume
  and printer picker), orbit pivot marker and status line (`viewport.rs`).
- The ViewCube is projected orthographically with the camera rotation. Each visible face is
  split into 3×3 regions (middle = face, borders = edge, corners = corner, border width
  `BAND`); a click looks along the sum of the involved normals. Labels are translated
  (ADR-0020) and rotated, not perspectively sheared.
- ViewCube geometry and hit test are pure functions (`regions`, `hit`, `face_center`) with unit
  tests; `show` only draws.
- Camera changes from ViewCube, home and fit are animated over 0.35 s (`Camera::interpolate`);
  navigation itself is ADR-0008.

## Consequences

- UI and `render-project` show identical colors. Regression test:
  `viewport_is_not_gamma_darkened` in `crates/qoob3d-app/tests/ui.rs`.
- Every further texture egui displays must deliver gamma-encoded values.
- Overlays are testable with egui_kittest (labels, clicks) and appear in the screenshots.
- One extra copy and sample step per frame; negligible.

## Rejected alternatives

- **egui paint callback** – no own depth buffer or MSAA, hard to test headless.
- **Classic Z with near/far** – Z-fighting across the large distance range.
- **Render in `Rgba8Unorm` and gamma-encode in the shader** – blending and MSAA resolve would
  compute in gamma space.
- **Correct the color in the egui callback** – egui offers no hook for that.
- **ViewCube as a 3D object in wgpu** – text, hover and hit test would be laborious and less
  testable.

## References

- `crates/qoob3d-render/src/renderer.rs` (`COLOR_FORMAT`, `DISPLAY_FORMAT`, `create_targets`),
  `crates/qoob3d-render/src/camera.rs` (`projection`, `interpolate`),
  `crates/qoob3d-render/src/shaders/scene.wgsl`, `crates/qoob3d-render/src/headless.rs`
- `crates/qoob3d-app/src/viewport.rs`, `crates/qoob3d-app/src/viewcube.rs`,
  `docs/design/controls.md`
- ADR-0008, ADR-0013, ADR-0020, ADR-0021
