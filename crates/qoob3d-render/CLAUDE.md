# qoob3d-render

wgpu viewport: camera, pipelines, offscreen targets, headless helpers.

## Rules

- Import wgpu **only** via `crate::wgpu` (= `eframe::egui_wgpu::wgpu`), never as a separate
  dependency (ADR-0019).
- No dependency on `qoob3d-kernel` (ADR-0036). Geometry arrives as `ShapeMesh`
  (faces + edges).
- **Color space (ADR-0019):** render in `COLOR_FORMAT` (sRGB), egui gets the
  `DISPLAY_FORMAT` view (Unorm) of the same texture. Shaders compute linearly. Colors are in
  `renderer.rs::palette` and `docs/design/theme.md`.
- **Reverse-Z:** depth is cleared to 0, comparison `Greater`/`GreaterEqual`. For ortho
  there is a separate linear reverse-Z matrix (`Camera::projection`).
- Lines are instances that are expanded into pixel-wide quads in the vertex shader
  (`vs_line`). Edges are offset towards the camera (fraction of the camera distance) so that
  they win the depth test against their face.
- Grid, print bed (polygon), exclusion zones, single-nozzle zones and X/Y axes are drawn by the
  analytic full-screen shader `fs_grid` (ADR-0021). The polygons are in the uniform `bed`
  (`MAX_BED_POINTS` = 192, same value in `xtask printers`). The grid lies in an
  arbitrary plane (`SceneOptions::grid_plane`, `GridPlane`); the print bed appears only in
  the world XY plane. Sketches use this for their grid.
- Check visual changes: `cargo xtask screenshot`, then look at the PNGs.

## Files

- `src/camera.rs`: `Camera` (orbit/orbit around pivot, pan, zoom to the cursor, fit,
  perspective/ortho, `ray`, `project`, `interpolate`), `ViewPreset`, `grid_snap_step`
  (grid spacing for snapping, matching the visible grid). With unit tests.
- `src/renderer.rs`: `ViewportRenderer` (`set_scene`, `render`), `SceneOptions`, pipelines
  (background, bodies, lines, grid), targets.
- `src/headless.rs`: headless device and texture readback (tests, CLI).
- `src/shaders/scene.wgsl`: all shaders. **`target` is reserved in WGSL.**
- `tests/offscreen.rs`: renders a cube headless and checks pixels (fails with
  `QOOB3D_REQUIRE_GPU` instead of skipping).

## Pitfalls (wgpu 30 / glam 0.34 / WGSL)

- `depth_write_enabled`/`depth_compare` are `Option`, `PipelineLayoutDescriptor::immediate_size`,
  bind group layouts are `&[Option<&_>]`, `multiview_mask`, `get_mapped_range()` returns a `Result`.
- Camera matrices: `glam::camera::rh::{view::look_at_mat4, proj::directx::perspective_infinite_reverse}`.
- Declare dynamically indexed arrays in WGSL as `var`.
- Shader errors only show up at runtime. `tests/offscreen.rs` creates all pipelines
  and thereby catches them.

## Relevant ADRs

ADR-0036, ADR-0006, ADR-0008, ADR-0021, ADR-0017, ADR-0019
