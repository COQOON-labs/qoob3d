//! Renderer without a window (ADR-0013): the built parts of a project as a
//! PNG, so humans and the coding agent can inspect results as images.

use std::path::Path;

use anyhow::Context as _;
use qoob3d_core::ShapeMesh;
use qoob3d_render::headless::{HeadlessGpu, read_rgba8};
use qoob3d_render::{Camera, Projection, SceneOptions, ViewPreset, ViewportRenderer};

/// Renders `mesh` from `view` into the PNG `out` and prints what it shows.
pub fn run(out: &Path, view: ViewPreset, size: [u32; 2], mesh: &ShapeMesh) -> anyhow::Result<()> {
    let report = mesh.faces.analyze();
    let rgba = render_views(mesh, &[view], size)?.pop().context("no view rendered")?;
    write_png(out, size, &rgba).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "wrote {} ({}x{}, view {}) | triangles {} | edges {} | watertight {} | volume {:.3} mm³",
        out.display(),
        size[0],
        size[1],
        view.name(),
        report.triangles,
        mesh.edges.len(),
        report.is_watertight(),
        report.volume_mm3
    );
    Ok(())
}

/// `mesh` once per view, orthographic and fitted, each `size` pixels of
/// sRGB-encoded RGBA, with one GPU setup for all of them.
pub(crate) fn render_views(
    mesh: &ShapeMesh,
    views: &[ViewPreset],
    size: [u32; 2],
) -> anyhow::Result<Vec<Vec<u8>>> {
    let bounds = mesh.faces.bounds().context("nothing to render: the mesh is empty")?;
    let gpu = HeadlessGpu::new()?;
    let mut renderer = ViewportRenderer::new(&gpu.device);
    renderer.set_scene(&gpu.device, mesh);
    let bed = qoob3d_core::PrinterProfile::default_profile().bed();
    let options = SceneOptions::new(Some(&bed));
    let mut images = Vec::with_capacity(views.len());
    for view in views {
        let mut camera = Camera::default();
        camera.look_from(view.eye_direction());
        camera.fit(bounds, size[0] as f32 / size[1] as f32);
        camera.projection = Projection::Orthographic;
        let (texture_view, _) = renderer.render(&gpu.device, &gpu.queue, size, &camera, &options);
        images.push(read_rgba8(&gpu.device, &gpu.queue, texture_view.texture()));
    }
    Ok(images)
}

pub(crate) fn write_png(path: &Path, [w, h]: [u32; 2], rgba: &[u8]) -> anyhow::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // The texture is sRGB-encoded already.
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}
