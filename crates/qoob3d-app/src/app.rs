use anyhow::Context as _;
use eframe::egui::{self, Color32, FontId, Sense, TextureId, pos2, vec2};
use eframe::egui_wgpu::{self, wgpu};
use qoob3d_core::TriMesh;
use qoob3d_kernel::{Kernel, Request, RequestId, Response};
use qoob3d_render::{Camera, ViewportRenderer};

use crate::navigation;
use crate::smoke::{Control, Observation, SmokeTest};

/// Mint accent colour of the QOOB3D theme (docs/design/theme.md).
pub const ACCENT: Color32 = Color32::from_rgb(0x3d, 0xe0, 0xb0);

pub struct QoobApp {
    render_state: egui_wgpu::RenderState,
    renderer: ViewportRenderer,
    viewport_texture: Option<TextureId>,
    camera: Camera,
    kernel: Kernel,
    pending: Option<RequestId>,
    mesh: Option<TriMesh>,
    status: String,
    smoke: Option<SmokeTest>,
}

/// The kernel request issued at startup (shared with `--render`).
pub(crate) fn initial_request() -> Request {
    Request::DemoCube { size_mm: 20.0 }
}

impl QoobApp {
    pub fn new(cc: &eframe::CreationContext<'_>, smoke: Option<SmokeTest>) -> anyhow::Result<Self> {
        let render_state =
            cc.wgpu_render_state.clone().context("QOOB3D requires the wgpu renderer")?;
        let renderer = ViewportRenderer::new(&render_state.device);

        let ctx = cc.egui_ctx.clone();
        let mut kernel = Kernel::spawn(move || ctx.request_repaint());
        let pending = Some(kernel.submit(initial_request())?);

        Ok(Self {
            render_state,
            renderer,
            viewport_texture: None,
            camera: Camera::default(),
            kernel,
            pending,
            mesh: None,
            status: "Berechne Würfel …".into(),
            smoke,
        })
    }

    fn poll_kernel(&mut self, aspect: f32) {
        while let Some(response) = self.kernel.try_recv() {
            match response {
                Response::Mesh { id, result } => {
                    if self.pending != Some(id) {
                        continue; // stale
                    }
                    self.pending = None;
                    match result {
                        Ok(mesh) => {
                            self.status = format!("{} Dreiecke", mesh.triangle_count());
                            self.renderer.set_mesh(&self.render_state.device, &mesh);
                            if let Some(bounds) = mesh.bounds() {
                                self.camera.fit(bounds, aspect);
                            }
                            self.mesh = Some(mesh);
                        }
                        Err(err) => {
                            tracing::error!(%err, "kernel request failed");
                            self.status = format!("Kernel-Fehler: {err}");
                        }
                    }
                }
            }
        }
    }

    /// Draws the viewport and returns its screen rectangle.
    fn viewport(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let aspect = rect.width() / rect.height().max(1.0);

        self.poll_kernel(aspect);

        let nav = navigation::handle_input(ui, &response, rect, &mut self.camera);
        if nav.fit_requested
            && let Some(bounds) = self.mesh.as_ref().and_then(TriMesh::bounds)
        {
            self.camera.fit(bounds, aspect);
        }

        // Render offscreen at physical resolution and show it as an image (ADR-0007).
        let ppp = ui.ctx().pixels_per_point();
        let size_px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        let rs = &self.render_state;
        let (view, recreated) = self.renderer.render(&rs.device, &rs.queue, size_px, &self.camera);
        if recreated || self.viewport_texture.is_none() {
            let mut egui_renderer = rs.renderer.write();
            match self.viewport_texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(
                    &rs.device,
                    view,
                    wgpu::FilterMode::Linear,
                    id,
                ),
                None => {
                    self.viewport_texture = Some(egui_renderer.register_native_texture(
                        &rs.device,
                        view,
                        wgpu::FilterMode::Linear,
                    ));
                }
            }
        }
        if let Some(id) = self.viewport_texture {
            ui.painter().image(
                id,
                rect,
                egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        // Minimal overlay: brand, status and navigation hint.
        let painter = ui.painter_at(rect);
        let margin = vec2(14.0, 12.0);
        painter.text(
            rect.left_top() + margin,
            egui::Align2::LEFT_TOP,
            "QOOB3D",
            FontId::proportional(18.0),
            ACCENT,
        );
        painter.text(
            rect.left_bottom() + vec2(margin.x, -margin.y),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "{}   ·   Mitte: Pan   Shift+Mitte: Orbit   Rad: Zoom   Doppelklick Mitte / F6: Einpassen",
                self.status
            ),
            FontId::proportional(12.0),
            Color32::from_gray(140),
        );
        rect
    }
}

impl eframe::App for QoobApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let rect = self.viewport(ui);

        if let Some(smoke) = &mut self.smoke {
            let obs = Observation {
                camera: &self.camera,
                mesh_bounds: self.mesh.as_ref().and_then(TriMesh::bounds),
                viewport_center: rect.center(),
                viewport_aspect: rect.width() / rect.height().max(1.0),
            };
            if smoke.after_frame(&obs) == Control::Close {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(smoke) = &mut self.smoke {
            smoke.inject(raw_input);
        }
    }
}
