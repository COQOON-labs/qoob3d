# qoob3d-render

wgpu-Viewport: Kamera, Pipelines, Offscreen-Targets, Headless-Hilfen.

## Regeln

- wgpu **nur** über `crate::wgpu` (= `eframe::egui_wgpu::wgpu`) importieren, nie als eigene
  Abhängigkeit (ADR-0019).
- Keine Abhängigkeit auf `qoob3d-kernel` (ADR-0036). Geometrie kommt als `ShapeMesh`
  (Flächen + Kanten).
- **Farbraum (ADR-0019):** Rendern in `COLOR_FORMAT` (sRGB), egui bekommt die
  `DISPLAY_FORMAT`-View (Unorm) derselben Textur. Shader rechnen linear. Farben stehen in
  `renderer.rs::palette` und `docs/design/theme.md`.
- **Reverse-Z:** Tiefe wird mit 0 gelöscht, Vergleich `Greater`/`GreaterEqual`. Für Ortho
  gibt es eine eigene lineare Reverse-Z-Matrix (`Camera::projection`).
- Linien sind Instanzen, die im Vertex-Shader zu Quads in Pixelbreite expandiert werden
  (`vs_line`). Kanten werden in Richtung Kamera versetzt (Anteil der Kameradistanz), damit sie
  den Tiefentest gegen ihre Fläche gewinnen.
- Raster, Druckbett (Polygon), Sperrzonen, Einzeldüsen-Zonen und X/Y-Achsen zeichnet der
  analytische Vollbild-Shader `fs_grid` (ADR-0021). Die Polygone liegen im Uniform `bed`
  (`MAX_BED_POINTS` = 192, gleicher Wert in `xtask printers`). Das Raster liegt in einer
  beliebigen Ebene (`SceneOptions::grid_plane`, `GridPlane`); das Druckbett erscheint nur in
  der Welt-XY-Ebene. Skizzen nutzen das für ihr Raster.
- Visuelle Änderungen prüfen: `cargo xtask screenshot`, dann die PNGs ansehen.

## Dateien

- `src/camera.rs`: `Camera` (Orbit/Orbit um Pivot, Pan, Zoom auf den Cursor, Einpassen,
  Perspektive/Ortho, `ray`, `project`, `interpolate`), `ViewPreset`, `grid_snap_step`
  (Rasterweite fürs Fangen, passend zum sichtbaren Raster). Mit Unit-Tests.
- `src/renderer.rs`: `ViewportRenderer` (`set_scene`, `render`), `SceneOptions`, Pipelines
  (Hintergrund, Körper, Linien, Raster), Targets.
- `src/headless.rs`: Headless-Device und Texture-Readback (Tests, CLI).
- `src/shaders/scene.wgsl`: alle Shader. **`target` ist in WGSL reserviert.**
- `tests/offscreen.rs`: rendert einen Würfel headless und prüft Pixel (schlägt mit
  `QOOB3D_REQUIRE_GPU` fehl statt zu überspringen).

## Fallstricke (wgpu 30 / glam 0.34 / WGSL)

- `depth_write_enabled`/`depth_compare` sind `Option`, `PipelineLayoutDescriptor::immediate_size`,
  Bind-Group-Layouts sind `&[Option<&_>]`, `multiview_mask`, `get_mapped_range()` liefert ein `Result`.
- Kamera-Matrizen: `glam::camera::rh::{view::look_at_mat4, proj::directx::perspective_infinite_reverse}`.
- Dynamisch indizierte Arrays in WGSL als `var` deklarieren.
- Shader-Fehler zeigen sich erst zur Laufzeit. `tests/offscreen.rs` erzeugt alle Pipelines
  und fängt sie dadurch ab.

## Relevante ADRs

ADR-0036, ADR-0006, ADR-0008, ADR-0021, ADR-0017, ADR-0019
