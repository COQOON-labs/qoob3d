# qoob3d-render

wgpu-Viewport: Kamera, Pipelines, Offscreen-Targets, Headless-Hilfen.

## Regeln

- wgpu **nur** über `crate::wgpu` (= `eframe::egui_wgpu::wgpu`) importieren, nie als eigene
  Abhängigkeit (ADR-0007).
- Keine Abhängigkeit auf `qoob3d-kernel` (ADR-0003). Geometrie kommt als `TriMesh`.
- Reverse-Z: Tiefe wird mit 0 gelöscht, Vergleich `Greater`. Neue Pipelines müssen das
  übernehmen.
- Shader rechnen linear. Farben stehen in `renderer.rs::palette` und in `docs/design/theme.md`.
- Visuelle Änderungen mit `qoob3d --render out.png` prüfen und das PNG ansehen.

## Dateien

- `src/camera.rs`: `Camera` (Orbit, Pan, Zoom auf den Cursor, Einpassen), mit Unit-Tests.
- `src/renderer.rs`: `ViewportRenderer`, Targets, Pipelines.
- `src/headless.rs`: Headless-Device und Texture-Readback (Tests, CLI).
- `src/shaders/scene.wgsl`: Hintergrund und Körper.
- `tests/offscreen.rs`: rendert einen Würfel headless und prüft Pixel.

## Fallstricke (wgpu 30 / glam 0.34)

- `depth_write_enabled`/`depth_compare` sind `Option`, `PipelineLayoutDescriptor::immediate_size`,
  Bind-Group-Layouts sind `&[Option<&_>]`, `multiview_mask`, `get_mapped_range()` liefert ein `Result`.
- Kamera-Matrizen: `glam::camera::rh::{view::look_at_mat4, proj::directx::perspective_infinite_reverse}`.

## Relevante ADRs

ADR-0003, ADR-0006, ADR-0007, ADR-0008
