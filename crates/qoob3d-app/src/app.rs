//! The desktop app (`eframe::App`): opens a script project (`--project`, the folder dialog,
//! Cmd/Ctrl+O; WA-OP-01..04) and shows its parts live in the viewport (ADR-0053,
//! docs/requirements/project-app.md). Without a project it shows the empty state; the demo
//! body serves the UI tests and the smoke test. Rendering per ADR-0019.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use eframe::egui::{self, Key, Modifiers};
use eframe::egui_wgpu;
use qoob3d_core::{Aabb, ShapeMesh};
use qoob3d_kernel::VIEW_TESSELLATION_TOLERANCE_MM;
use qoob3d_kernel::script::{self, RunOptions};

use crate::i18n::{self, decimal, tr, tr_args};
use crate::live::{Live, Poller};
use crate::panels::{self, UiAction};
use crate::settings::Settings;
use crate::smoke::{Control, Observation, SmokeTest};
use crate::theme;
use crate::viewcube::CubeFace;
use crate::viewport::Viewport;
use qoob3d_project::client::{PartState, ProjectDir};

/// The demo body: a block with a hole, something to navigate around.
const DEMO_SCRIPT: &str = r#"show(box(40, 30, 15) - cylinder(14, 15), "demo");"#;

/// How the app is started.
#[derive(Default)]
pub struct StartOptions {
    /// Show the demo body instead of a project (UI tests, smoke test).
    pub demo: bool,
    pub smoke: Option<SmokeTest>,
    /// Show the script project in this folder live (ADR-0053).
    pub project_dir: Option<PathBuf>,
}

/// An open script project: what is shown, the poller feeding it and the
/// [`Live::version`] last uploaded to the renderer.
struct Open {
    name: String,
    live: Live,
    poller: Poller,
    shown: u64,
}

pub struct QoobApp {
    render_state: egui_wgpu::RenderState,
    viewport: Viewport,
    project: Option<Open>,
    /// What the viewport shows: the visible parts merged, or the demo body.
    scene: Option<ShapeMesh>,
    demo: bool,
    needs_fit: bool,
    settings: Settings,
    smoke: Option<SmokeTest>,
    /// Message of the error dialog, while it is open.
    error: Option<String>,
    title: String,
}

impl QoobApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: StartOptions) -> anyhow::Result<Self> {
        theme::apply(&cc.egui_ctx);
        // The smoke test must not depend on (or change) the user's settings.
        let settings = match (&options.smoke, cc.storage) {
            (None, Some(storage)) => {
                eframe::get_value::<Settings>(storage, Settings::STORAGE_KEY).unwrap_or_default()
            }
            _ => Settings::default(),
        };
        i18n::set_language(settings.language);
        let render_state =
            cc.wgpu_render_state.clone().context("QOOB3D requires the wgpu renderer")?;
        let mut app = Self {
            viewport: Viewport::new(&render_state.device),
            render_state,
            project: None,
            scene: None,
            demo: options.demo,
            needs_fit: false,
            settings,
            smoke: options.smoke,
            error: None,
            title: String::new(),
        };
        if options.demo {
            app.set_scene(Some(demo_mesh()?));
            app.needs_fit = true;
        }
        if let Some(dir) = &options.project_dir {
            app.open_project(dir, &cc.egui_ctx);
        }
        Ok(app)
    }

    /// Shows the project in `dir` live (WA-OP-01..03); a folder that is no
    /// project gives the error dialog and keeps what was open. Tests call
    /// this instead of the folder dialog.
    pub fn open_project(&mut self, dir: &Path, ctx: &egui::Context) {
        match ProjectDir::open(dir) {
            Ok(project) => {
                let name = project.name();
                let ctx = ctx.clone();
                let cmd = qoob3d_project::client::service_command_from_env();
                let poller = Poller::open(project, cmd, move || ctx.request_repaint());
                self.project = Some(Open { name, live: Live::default(), poller, shown: 0 });
                self.demo = false;
                self.set_scene(None);
            }
            Err(err) => {
                self.error = Some(tr_args(
                    "error-open",
                    &[
                        ("file", dir.display().to_string().into()),
                        ("error", err.to_string().into()),
                    ],
                ));
            }
        }
    }

    fn set_scene(&mut self, scene: Option<ShapeMesh>) {
        let empty = ShapeMesh::default();
        self.viewport.set_scene(&self.render_state.device, scene.as_ref().unwrap_or(&empty));
        self.scene = scene;
    }

    /// True once geometry is shown (tests wait for this).
    pub fn has_model(&self) -> bool {
        self.scene.is_some()
    }

    /// The open script project, if any.
    pub fn live(&self) -> Option<&Live> {
        self.project.as_ref().map(|p| &p.live)
    }

    /// Takes the poller's updates and uploads changed geometry; the first
    /// meshes fit the camera (WA-LV-02, WA-LV-06).
    fn poll_live(&mut self) {
        let Some(project) = &mut self.project else { return };
        let had_meshes = project.live.has_meshes();
        while let Ok(update) = project.poller.updates().try_recv() {
            project.live.apply(update);
        }
        if project.live.version() == project.shown {
            return;
        }
        project.shown = project.live.version();
        let scene = project.live.scene();
        let first_meshes = !had_meshes && project.live.has_meshes();
        self.set_scene((!scene.faces.is_empty()).then_some(scene));
        self.needs_fit |= first_meshes;
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn model_bounds(&self) -> Option<Aabb> {
        self.scene.as_ref().and_then(|m| m.faces.bounds())
    }

    fn status(&self) -> String {
        let Some(project) = &self.project else {
            return match &self.scene {
                Some(mesh) => model_status(mesh),
                None => tr("empty-title"),
            };
        };
        if !project.live.reachable() {
            return tr("live-unreachable");
        }
        let parts = project.live.parts();
        let count = |state: PartState| parts.iter().filter(|p| p.state == state).count();
        let mut text = tr_args(
            "status-project",
            &[
                ("name", project.name.clone().into()),
                ("parts", parts.len().into()),
                ("current", count(PartState::Current).into()),
            ],
        );
        let failed = count(PartState::Failed);
        if failed > 0 {
            text.push_str("  ·  ");
            text.push_str(&tr_args("status-project-failed", &[("failed", failed.into())]));
        }
        text
    }

    // --- actions ---------------------------------------------------------------

    fn apply_ui(&mut self, action: UiAction, ctx: &egui::Context) {
        match action {
            UiAction::OpenProject => {
                let picked = rfd::FileDialog::new().set_title(tr("project-open-tip")).pick_folder();
                if let Some(dir) = picked {
                    self.open_project(&dir, ctx);
                }
            }
            UiAction::TogglePart(index) => {
                if let Some(project) = &mut self.project
                    && let Some(part) = project.live.parts().get(index)
                {
                    let (name, visible) = (part.name.clone(), part.visible);
                    project.live.set_visible(&name, !visible);
                }
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::O)) {
            self.apply_ui(UiAction::OpenProject, ctx);
        }
    }

    fn error_dialog(&mut self, ctx: &egui::Context) {
        let Some(message) = self.error.clone() else { return };
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("qoob3d-error")).show(ctx, |ui| {
            ui.heading(tr("dialog-error-title"));
            ui.label(message);
            close = ui.button(tr("dialog-ok")).clicked();
        });
        // OK, Escape or a click next to the dialog close it.
        if close || modal.should_close() {
            self.error = None;
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match &self.project {
            Some(project) => format!("{} – QOOB3D", project.name),
            None => "QOOB3D".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn observe(&self) -> Option<Observation<'_>> {
        let layout = self.viewport.layout()?;
        Some(Observation {
            camera: &self.viewport.camera,
            mesh_bounds: self.model_bounds(),
            viewport_center: layout.rect.center(),
            viewport_aspect: layout.rect.width() / layout.rect.height().max(1.0),
            viewcube_top: self.viewport.viewcube_face(CubeFace::Top),
            home_button: self.viewport.home_button(),
            last_orbit_pivot: self.viewport.last_orbit_pivot(),
            animating: self.viewport.is_animating(),
        })
    }
}

/// The demo body, built by the script kernel like every part.
fn demo_mesh() -> anyhow::Result<ShapeMesh> {
    let output = script::run(DEMO_SCRIPT, &RunOptions::default())
        .map_err(|err| anyhow::anyhow!("demo body: {err}"))?;
    let (_, shape) = output.shapes.first().context("the demo script shows no shape")?;
    Ok(shape.mesh(VIEW_TESSELLATION_TOLERANCE_MM)?)
}

/// "Demo part · 12 triangles · 18 edges · watertight ✔ · 6.00 cm³".
fn model_status(mesh: &ShapeMesh) -> String {
    let report = mesh.faces.analyze();
    let tight = if report.is_watertight() { "status-watertight" } else { "status-not-watertight" };
    tr_args(
        "status-model",
        &[
            ("name", tr("body-demo").into()),
            ("triangles", report.triangles.into()),
            ("edges", mesh.edges.len().into()),
            ("tight", tr(tight).into()),
            ("volume", decimal(report.volume_mm3 / 1000.0, 2).into()),
        ],
    )
}

impl eframe::App for QoobApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_live();
        if self.error.is_none() {
            self.shortcuts(&ctx);
        }

        let live = self.project.as_ref().map(|p| &p.live);
        let mut actions = Vec::new();
        egui::Panel::top("qoob3d-toolbar")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| actions.extend(panels::toolbar(ui, &mut self.settings)));
        egui::Panel::left("qoob3d-browser")
            .default_size(230.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::PANEL_BG).inner_margin(egui::Margin::same(8)))
            .show(ui, |ui| actions.extend(panels::browser(ui, live)));

        let status = self.status();
        let fit_bounds = self.model_bounds();
        let empty = self.project.is_none() && !self.demo;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            if self.needs_fit
                && self.viewport.layout().is_some()
                && let Some(bounds) = fit_bounds
            {
                self.viewport.fit_now(bounds);
                self.needs_fit = false;
            }
            let frame = self.viewport.show_scene(
                ui,
                &self.render_state,
                self.scene.as_ref(),
                &self.settings,
                fit_bounds,
            );
            if empty {
                actions.extend(panels::empty_state(&ctx, frame.rect));
            }
            self.viewport.show_overlays(ui, &frame, &status, &mut self.settings);
        });

        for action in actions {
            self.apply_ui(action, &ctx);
        }
        self.error_dialog(&ctx);
        self.update_title(&ctx);

        if let Some(mut smoke) = self.smoke.take() {
            let control = self.observe().map(|obs| smoke.after_frame(&obs));
            self.smoke = Some(smoke);
            if control == Some(Control::Close) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
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
