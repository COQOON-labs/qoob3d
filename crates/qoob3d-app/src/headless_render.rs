//! `qoob3d --render out.png`: kernel + renderer without a window (ADR-0013).
//! Lets humans and the coding agent inspect results as images.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, bail};
use qoob3d_kernel::{Kernel, Response};
use qoob3d_render::headless::{HeadlessGpu, read_rgba8};
use qoob3d_render::{Camera, Projection, SceneOptions, ViewPreset, ViewportRenderer};

/// Camera tweaks for `--render`.
pub struct RenderView {
    pub preset: ViewPreset,
    pub orthographic: bool,
    pub zoom: f32,
    /// Printer id; `None` = default printer.
    pub printer: Option<String>,
    pub build_volume: bool,
}

pub fn run(out: &Path, view: RenderView, size: [u32; 2]) -> anyhow::Result<()> {
    let mut kernel = Kernel::spawn(|| {});
    let id = kernel.submit(crate::app::initial_request())?;
    let mesh = match kernel.recv_timeout(Duration::from_secs(120)) {
        Some(Response::Mesh { id: got, result }) if got == id => result?,
        Some(other) => bail!("unexpected kernel response: {other:?}"),
        None => bail!("kernel did not answer within 120 s"),
    };
    let bounds = mesh.faces.bounds().context("kernel returned an empty mesh")?;
    let report = mesh.faces.analyze();

    let gpu = HeadlessGpu::new()?;
    let mut renderer = ViewportRenderer::new(&gpu.device);
    renderer.set_scene(&gpu.device, &mesh);
    let mut camera = Camera::default();
    camera.look_from(view.preset.eye_direction());
    camera.fit(bounds, size[0] as f32 / size[1] as f32);
    camera.distance *= view.zoom.max(0.01);
    if view.orthographic {
        camera.projection = Projection::Orthographic;
    }
    let profile = match &view.printer {
        Some(id) => qoob3d_core::PrinterProfile::by_id(id)
            .with_context(|| format!("unknown printer id {id:?}"))?,
        None => qoob3d_core::PrinterProfile::default_profile(),
    };
    let bed = profile.bed();
    let options =
        SceneOptions { show_build_volume: view.build_volume, ..SceneOptions::new(Some(&bed)) };
    let (texture_view, _) = renderer.render(&gpu.device, &gpu.queue, size, &camera, &options);
    let rgba = read_rgba8(&gpu.device, &gpu.queue, texture_view.texture());

    write_png(out, size, &rgba).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "wrote {} ({}x{}, view {}) | triangles {} | edges {} | watertight {} | volume {:.3} mm³",
        out.display(),
        size[0],
        size[1],
        view.preset.name(),
        report.triangles,
        mesh.edges.len(),
        report.is_watertight(),
        report.volume_mm3
    );
    Ok(())
}

fn write_png(path: &Path, [w, h]: [u32; 2], rgba: &[u8]) -> anyhow::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // The texture is sRGB-encoded already.
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}
