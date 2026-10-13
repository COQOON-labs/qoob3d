use anyhow::Context as _;
use eframe::egui;
use eframe::egui_wgpu;
use qoob3d_core::ShapeMesh;
use qoob3d_kernel::{Kernel, Request, RequestId, Response};

use crate::i18n::{self, decimal, tr, tr_args};
use crate::panels::{self, BrowserInfo};
use crate::settings::Settings;
use crate::smoke::{Control, Observation, SmokeTest};
use crate::theme;
use crate::viewcube::CubeFace;
use crate::viewport::Viewport;

/// The kernel request issued at startup (shared with `--render`).
pub fn initial_request() -> Request {
    Request::DemoPart
}

pub struct QoobApp {
    render_state: egui_wgpu::RenderState,
    viewport: Viewport,
    kernel: Kernel,
    pending: Option<RequestId>,
    mesh: Option<ShapeMesh>,
    needs_fit: bool,
    status: Status,
    settings: Settings,
    smoke: Option<SmokeTest>,
}

/// What the status line shows; translated every frame so a language
/// switch takes effect immediately.
enum Status {
    Computing,
    Model { triangles: usize, edges: usize, watertight: bool, volume_mm3: f64 },
    KernelError(String),
}

impl Status {
    fn text(&self) -> String {
        match self {
            Status::Computing => tr("status-computing"),
            Status::Model { triangles, edges, watertight, volume_mm3 } => tr_args(
                "status-model",
                &[
                    ("name", tr("body-demo").into()),
                    ("triangles", (*triangles).into()),
                    ("edges", (*edges).into()),
                    (
                        "tight",
                        tr(if *watertight { "status-watertight" } else { "status-not-watertight" })
                            .into(),
                    ),
                    ("volume", decimal(volume_mm3 / 1000.0, 2).into()),
                ],
            ),
            Status::KernelError(err) => {
                tr_args("status-kernel-error", &[("error", err.clone().into())])
            }
        }
    }
}

impl QoobApp {
    pub fn new(cc: &eframe::CreationContext<'_>, smoke: Option<SmokeTest>) -> anyhow::Result<Self> {
        theme::apply(&cc.egui_ctx);
        // The smoke test must not depend on (or change) the user's settings.
        let settings = match (&smoke, cc.storage) {
            (None, Some(storage)) => {
                eframe::get_value::<Settings>(storage, Settings::STORAGE_KEY).unwrap_or_default()
            }
            _ => Settings::default(),
        };
        i18n::set_language(settings.language);
        let render_state =
            cc.wgpu_render_state.clone().context("QOOB3D requires the wgpu renderer")?;
        let viewport = Viewport::new(&render_state.device);

        let ctx = cc.egui_ctx.clone();
        let mut kernel = Kernel::spawn(move || ctx.request_repaint());
        let pending = Some(kernel.submit(initial_request())?);

        Ok(Self {
            render_state,
            viewport,
            kernel,
            pending,
            mesh: None,
            needs_fit: false,
            status: Status::Computing,
            settings,
            smoke,
        })
    }

    /// True once the kernel has delivered the model (tests wait for this).
    pub fn has_model(&self) -> bool {
        self.mesh.is_some()
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn model_bounds(&self) -> Option<qoob3d_core::Aabb> {
        self.mesh.as_ref().and_then(|m| m.faces.bounds())
    }

    fn poll_kernel(&mut self) {
        while let Some(response) = self.kernel.try_recv() {
            match response {
                Response::Mesh { id, result } => {
                    if self.pending != Some(id) {
                        continue; // stale (ADR-0005)
                    }
                    self.pending = None;
                    match result {
                        Ok(mesh) => {
                            let report = mesh.faces.analyze();
                            self.status = Status::Model {
                                triangles: report.triangles,
                                edges: mesh.edges.len(),
                                watertight: report.is_watertight(),
                                volume_mm3: report.volume_mm3,
                            };
                            self.viewport.set_scene(&self.render_state.device, &mesh);
                            self.mesh = Some(mesh);
                            self.needs_fit = true;
                        }
                        Err(err) => {
                            tracing::error!(%err, "kernel request failed");
                            self.status = Status::KernelError(err.to_string());
                        }
                    }
                }
            }
        }
    }
}

impl eframe::App for QoobApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_kernel();

        egui::Panel::top("qoob3d-toolbar")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| panels::toolbar(ui, &mut self.settings));
        egui::Panel::bottom("qoob3d-timeline")
            .exact_size(40.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG_DARK)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, panels::timeline);
        let has_body = self.mesh.is_some();
        egui::Panel::left("qoob3d-browser")
            .default_size(230.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::PANEL_BG).inner_margin(egui::Margin::same(8)))
            .show(ui, |ui| panels::browser(ui, &BrowserInfo { has_body }));

        egui::CentralPanel::no_frame().show(ui, |ui| {
            if self.needs_fit
                && self.viewport.layout().is_some()
                && let Some(bounds) = self.mesh.as_ref().and_then(|m| m.faces.bounds())
            {
                self.viewport.fit_now(bounds);
                self.needs_fit = false;
            }
            let status = self.status.text();
            self.viewport.show(
                ui,
                &self.render_state,
                self.mesh.as_ref(),
                &status,
                &mut self.settings,
            );
        });

        if let Some(smoke) = &mut self.smoke
            && let Some(layout) = self.viewport.layout()
        {
            let obs = Observation {
                camera: &self.viewport.camera,
                mesh_bounds: self.mesh.as_ref().and_then(|m| m.faces.bounds()),
                viewport_center: layout.rect.center(),
                viewport_aspect: layout.rect.width() / layout.rect.height().max(1.0),
                viewcube_top: self.viewport.viewcube_face(CubeFace::Top),
                home_button: self.viewport.home_button(),
                last_orbit_pivot: self.viewport.last_orbit_pivot(),
                animating: self.viewport.is_animating(),
            };
            if smoke.after_frame(&obs) == Control::Close {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.smoke.is_none() {
            eframe::set_value(storage, Settings::STORAGE_KEY, &self.settings);
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(smoke) = &mut self.smoke {
            smoke.inject(raw_input);
        }
    }
}
