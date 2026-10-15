use std::path::PathBuf;

use anyhow::Context as _;
use eframe::egui::{self, Key, Modifiers};
use eframe::egui_wgpu;
use qoob3d_core::document::{
    CurveKind, EntityId, FILE_EXTENSION, FeatureId, FeatureKind, OriginPlane, PlaneRef, Sketch,
};
use qoob3d_core::export::{ExportObject, stl, three_mf};
use qoob3d_core::{Aabb, ShapeMesh};
use qoob3d_kernel::{
    BodyResult, EXPORT_TESSELLATION_TOLERANCE_MM, ExportBody, Kernel, Request, RequestId, Response,
};

use crate::commands::{self, Command, CommandKind, CommandOutcome};
use crate::panels::ExportFormat;

use crate::i18n::{self, decimal, tr, tr_args};
use crate::panels::{self, PanelState, UiAction};
use crate::project::Project;
use crate::settings::Settings;
use crate::sketch_ui::{self, Action, PlanePick, SketchOptions, SketchSession, Style, View};
use crate::smoke::{Control, Observation, SmokeTest};
use crate::theme;
use crate::viewcube::CubeFace;
use crate::viewport::{SceneMode, Viewport};

/// How the app is started.
#[derive(Default)]
pub struct StartOptions {
    /// Show the demo body (a block with a hole) – tests and `--demo`.
    pub demo: bool,
    pub smoke: Option<SmokeTest>,
    /// Open this `.qbd` file at start.
    pub open: Option<PathBuf>,
}

/// The kernel request for the demo body (shared with `--render`).
pub fn initial_request() -> Request {
    Request::DemoPart
}

pub enum Mode {
    Model,
    /// Waiting for the user to pick a sketch plane (SK-LC-01).
    PickPlane,
    Sketch(Box<SketchSession>),
    /// A feature dialog (Extrude, Revolve …) is open.
    Command(Box<Command>),
}

/// Something that must wait for "save changes?" to be answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    New,
    Open,
    Close,
}

enum Dialog {
    Unsaved(Pending),
    Error(String),
}

pub struct QoobApp {
    render_state: egui_wgpu::RenderState,
    viewport: Viewport,
    kernel: Kernel,
    project: Project,
    mode: Mode,
    demo: Option<ShapeMesh>,
    demo_pending: Option<RequestId>,
    /// All bodies merged, as uploaded to the renderer.
    scene: Option<ShapeMesh>,
    scene_version: u64,
    /// Running export: request and target file.
    export: Option<(RequestId, PathBuf)>,
    /// Short message in the status line (text, until time).
    notice: Option<(String, f64)>,
    needs_fit: bool,
    settings: Settings,
    smoke: Option<SmokeTest>,
    dialog: Option<Dialog>,
    allow_close: bool,
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
        let viewport = Viewport::new(&render_state.device);

        let ctx = cc.egui_ctx.clone();
        let mut kernel = Kernel::spawn(move || ctx.request_repaint());
        let demo_pending =
            if options.demo { Some(kernel.submit(initial_request())?) } else { None };

        let mut dialog = None;
        let project = match &options.open {
            Some(path) => Project::open(path).unwrap_or_else(|err| {
                dialog = Some(Dialog::Error(tr_args(
                    "error-open",
                    &[
                        ("file", path.display().to_string().into()),
                        ("error", err.to_string().into()),
                    ],
                )));
                Project::default()
            }),
            None => Project::default(),
        };

        Ok(Self {
            render_state,
            viewport,
            kernel,
            project,
            mode: Mode::Model,
            demo: None,
            demo_pending,
            scene: None,
            scene_version: 0,
            export: None,
            notice: None,
            needs_fit: options.open.is_some(),
            settings,
            smoke: options.smoke,
            dialog,
            allow_close: false,
            title: String::new(),
        })
    }

    /// True once a body is shown (tests wait for this).
    pub fn has_model(&self) -> bool {
        self.demo.is_some() || self.scene.is_some()
    }

    /// Bodies of the last rebuild.
    pub fn bodies(&self) -> &[BodyResult] {
        self.project.bodies()
    }

    /// The open feature dialog, if any.
    pub fn command(&self) -> Option<&Command> {
        match &self.mode {
            Mode::Command(c) => Some(c),
            _ => None,
        }
    }

    /// Screen position inside profile `index` of a sketch (tests).
    pub fn profile_screen_pos(&self, sketch: FeatureId, index: usize) -> Option<egui::Pos2> {
        let layout = self.viewport.layout()?;
        commands::profile_screen_pos(
            self.project.doc(),
            &self.project,
            &self.viewport.camera,
            layout.rect,
            sketch,
            index,
        )
    }

    /// Screen position of a sketch point / line midpoint (tests).
    pub fn entity_screen_pos(&self, sketch: FeatureId, entity: EntityId) -> Option<egui::Pos2> {
        let layout = self.viewport.layout()?;
        commands::entity_screen_pos(
            self.project.doc(),
            &self.viewport.camera,
            layout.rect,
            sketch,
            entity,
        )
    }

    /// Starts an STL/3MF export into `path` (the format follows the
    /// extension). Tests call this directly instead of the file dialog.
    pub fn export_to(&mut self, path: PathBuf) {
        match self.kernel.submit(Request::Export {
            document: self.project.doc().clone(),
            tolerance: EXPORT_TESSELLATION_TOLERANCE_MM,
        }) {
            Ok(id) => self.export = Some((id, path)),
            Err(err) => self.dialog = Some(Dialog::Error(err.to_string())),
        }
    }

    /// True while an export is running.
    pub fn is_exporting(&self) -> bool {
        self.export.is_some()
    }

    fn finish_export(&mut self, bodies: Vec<ExportBody>, path: &std::path::Path, now: f64) {
        let objects: Vec<ExportObject<'_>> = bodies
            .iter()
            .map(|b| ExportObject {
                name: tr_args("body-name", &[("n", b.number.into())]),
                mesh: &b.mesh,
            })
            .collect();
        if objects.is_empty() {
            self.dialog = Some(Dialog::Error(tr("export-nothing")));
            return;
        }
        let three = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("3mf"));
        let bytes = if three { three_mf(&objects) } else { stl(&objects) };
        match std::fs::write(path, bytes) {
            Ok(()) => {
                let file =
                    path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into());
                let text =
                    tr_args("export-done", &[("n", objects.len().into()), ("file", file.into())]);
                self.notice = Some((text, now + 6.0));
            }
            Err(err) => {
                self.dialog = Some(Dialog::Error(tr_args(
                    "error-save",
                    &[
                        ("file", path.display().to_string().into()),
                        ("error", err.to_string().into()),
                    ],
                )));
            }
        }
    }

    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn model_bounds(&self) -> Option<Aabb> {
        self.scene.as_ref().or(self.demo.as_ref()).and_then(|m| m.faces.bounds())
    }

    /// Screen position of an origin plane while picking (tests, smoke test).
    pub fn plane_screen_pos(&self, plane: OriginPlane) -> Option<egui::Pos2> {
        let layout = self.viewport.layout()?;
        sketch_ui::plane_center(&self.viewport.camera, layout.rect, plane)
    }

    /// Screen position of a point in the active sketch (tests, smoke test).
    pub fn sketch_screen_pos(&self, local: [f64; 2]) -> Option<egui::Pos2> {
        let Mode::Sketch(session) = &self.mode else { return None };
        let layout = self.viewport.layout()?;
        let view = View { camera: &self.viewport.camera, rect: layout.rect, frame: session.frame };
        view.to_screen(qoob3d_sketch::DVec2::new(local[0], local[1]))
    }

    fn poll_kernel(&mut self, now: f64) {
        while let Some(response) = self.kernel.try_recv() {
            match response {
                Response::Mesh { id, result } => {
                    if self.demo_pending != Some(id) {
                        continue; // stale (ADR-0005)
                    }
                    self.demo_pending = None;
                    match result {
                        Ok(mesh) => {
                            self.viewport.set_scene(&self.render_state.device, &mesh);
                            self.demo = Some(mesh);
                            self.needs_fit = true;
                        }
                        Err(err) => {
                            tracing::error!(%err, "demo body failed");
                            self.dialog = Some(Dialog::Error(err.to_string()));
                        }
                    }
                }
                Response::Rebuilt { id, result } => self.project.accept_rebuild(id, result),
                Response::Exported { id, result } => {
                    let Some((want, path)) = self.export.take() else { continue };
                    if want != id {
                        self.export = Some((want, path));
                        continue;
                    }
                    match result {
                        Ok(bodies) => self.finish_export(bodies, &path, now),
                        Err(err) => self.dialog = Some(Dialog::Error(err.to_string())),
                    }
                }
            }
        }
    }

    /// Everything "Fit" should frame: bodies and visible sketches.
    fn fit_bounds(&self) -> Option<Aabb> {
        let mut pts: Vec<[f32; 3]> = Vec::new();
        if let Some(b) = self.model_bounds() {
            pts.extend([b.min, b.max]);
        }
        let doc = self.project.doc();
        for (_, s) in doc.sketches().filter(|(_, s)| s.visible) {
            let Some(frame) = doc.plane_frame(s.plane) else { continue };
            for c in &s.curves {
                let r = match c.kind {
                    CurveKind::Circle { radius, .. } => radius,
                    CurveKind::Arc { center, start, end } => s.arc(center, start, end).radius,
                    CurveKind::Line { .. } => 0.0,
                };
                for p in c.points() {
                    let q = s.pos(p);
                    for (dx, dy) in [(-r, -r), (r, r)] {
                        pts.push(frame.to_world([q[0] + dx, q[1] + dy]).map(|v| v as f32));
                    }
                }
            }
        }
        let mut it = pts.into_iter();
        let first = it.next()?;
        Some(it.fold(Aabb { min: first, max: first }, |b, p| Aabb {
            min: std::array::from_fn(|k| b.min[k].min(p[k])),
            max: std::array::from_fn(|k| b.max[k].max(p[k])),
        }))
    }

    fn status(&self) -> String {
        match &self.mode {
            Mode::PickPlane => tr("hint-pick-plane"),
            Mode::Sketch(session) => {
                let doc = self.project.doc();
                let name = doc.feature(session.feature).map_or("", |f| f.name.as_str());
                let curves = doc.sketch(session.feature).map_or(0, |s| s.curves.len());
                let profiles = self.project.profiles(session.feature).len();
                let mut text = tr_args(
                    "status-sketch",
                    &[
                        ("name", name.to_string().into()),
                        ("curves", curves.into()),
                        ("profiles", profiles.into()),
                    ],
                );
                if let Some(err) = self.project.error(session.feature) {
                    text.push_str("  ·  ");
                    text.push_str(&tr_args("status-kernel-error", &[("error", err.into())]));
                } else if self.project.is_rebuilding() {
                    text.push_str("  ·  ");
                    text.push_str(&tr("status-computing"));
                }
                text
            }
            Mode::Command(c) => {
                let name = c.kind.title();
                if self.project.is_rebuilding() {
                    format!("{name}  ·  {}", tr("status-computing"))
                } else {
                    format!("{name}  ·  {}", self.bodies_status())
                }
            }
            Mode::Model if self.demo_pending.is_some() || self.project.is_rebuilding() => {
                tr("status-computing")
            }
            Mode::Model if !self.project.bodies().is_empty() => self.bodies_status(),
            Mode::Model => match &self.demo {
                Some(mesh) => {
                    let report = mesh.faces.analyze();
                    let tight = if report.is_watertight() {
                        "status-watertight"
                    } else {
                        "status-not-watertight"
                    };
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
                None => tr_args(
                    "status-document",
                    &[("features", self.project.doc().features.len().into())],
                ),
            },
        }
    }

    /// "2 bodies · 12.34 cm³ · watertight ✔" (FDM check at a glance).
    fn bodies_status(&self) -> String {
        let bodies = self.project.bodies();
        let volume: f64 = bodies.iter().map(|b| b.volume_mm3).sum();
        let tight = bodies.iter().all(|b| b.valid && b.mesh.faces.analyze().is_watertight());
        let tight = if tight { "status-watertight" } else { "status-not-watertight" };
        let mut text = tr_args(
            "status-bodies",
            &[
                ("bodies", bodies.len().into()),
                ("volume", decimal(volume / 1000.0, 2).into()),
                ("tight", tr(tight).into()),
            ],
        );
        let mut errors =
            self.project.doc().features.iter().filter(|f| self.project.error(f.id).is_some());
        if let Some(f) = errors.next_back() {
            text.push_str("  ·  ⚠ ");
            text.push_str(&f.name);
        }
        text
    }

    // --- actions ---------------------------------------------------------------

    fn create_sketch(&mut self, plane: PlaneRef, now: f64) {
        self.finish_sketch();
        let marker = self.project.history_marker();
        let mut feature = FeatureId(0);
        let n = self.project.doc().sketch_count() + 1;
        let name = tr_args("feature-sketch-name", &[("n", n.into())]);
        self.project.edit("history-create-sketch", |d| {
            feature = d.add_feature(name, FeatureKind::Sketch(Sketch::new(plane)));
        });
        self.enter_sketch(feature, marker, now);
    }

    fn enter_sketch(&mut self, feature: FeatureId, marker: usize, now: f64) {
        let doc = self.project.doc();
        let Some(frame) = doc.sketch(feature).and_then(|s| doc.plane_frame(s.plane)) else {
            return;
        };
        if self.settings.sketch_look_at {
            self.viewport.look_at(&frame, now);
        }
        self.mode = Mode::Sketch(Box::new(SketchSession::new(feature, frame, marker)));
    }

    /// Leaves sketch mode; the whole session becomes one undo step. An open
    /// feature dialog is cancelled.
    fn finish_sketch(&mut self) {
        match std::mem::replace(&mut self.mode, Mode::Model) {
            Mode::Sketch(session) => {
                self.project.commit_preview("edit-move");
                self.project.collapse_history(session.history_marker, "history-sketch");
            }
            Mode::Command(_) => self.project.cancel_preview(),
            other => self.mode = other,
        }
    }

    fn start_command(&mut self, command: Command) {
        self.finish_sketch();
        self.mode = Mode::Command(Box::new(command));
    }

    fn apply_ui(&mut self, action: UiAction, ctx: &egui::Context, now: f64) {
        match action {
            UiAction::NewDocument => self.guard(Pending::New, ctx),
            UiAction::Open => self.guard(Pending::Open, ctx),
            UiAction::Save => {
                self.save(false);
            }
            UiAction::Undo => self.undo_redo(true),
            UiAction::Redo => self.undo_redo(false),
            UiAction::CreateSketch => {
                self.finish_sketch();
                self.mode = Mode::PickPlane;
            }
            UiAction::SketchOnPlane(plane) => self.create_sketch(plane, now),
            UiAction::Tool(kind) => {
                if let Mode::Sketch(s) = &mut self.mode {
                    s.start_tool(kind);
                }
            }
            UiAction::ToggleConstruction => {
                if let Mode::Sketch(s) = &mut self.mode
                    && let Some(sketch) = self.project.doc().sketch(s.feature).cloned()
                {
                    let mut actions = Vec::new();
                    s.toggle_construction(&sketch, &mut actions);
                    let feature = s.feature;
                    self.apply_sketch_actions(feature, actions);
                }
            }
            UiAction::FinishSketch => self.finish_sketch(),
            UiAction::LookAt => {
                if let Mode::Sketch(s) = &self.mode {
                    let frame = s.frame;
                    self.viewport.look_at(&frame, now);
                }
            }
            UiAction::EditFeature(id) => {
                self.finish_sketch();
                if self.project.doc().sketch(id).is_some() {
                    let marker = self.project.history_marker();
                    self.enter_sketch(id, marker, now);
                } else if let Some(cmd) = Command::edit(id, &self.project) {
                    self.start_command(cmd);
                }
            }
            UiAction::DeleteFeature(id) => {
                self.finish_sketch();
                self.project.edit("history-delete-feature", |d| d.remove_feature(id));
            }
            UiAction::Command(kind) => {
                self.finish_sketch();
                let cmd = Command::new(kind, &self.project);
                self.start_command(cmd);
            }
            UiAction::Export(format) => {
                let ext = match format {
                    ExportFormat::Stl => "stl",
                    ExportFormat::ThreeMf => "3mf",
                };
                let stem = self
                    .project
                    .path
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .map_or_else(|| tr("untitled"), |s| s.to_string_lossy().into_owned());
                if let Some(mut path) = rfd::FileDialog::new()
                    .add_filter(ext.to_uppercase(), &[ext])
                    .set_file_name(format!("{stem}.{ext}"))
                    .save_file()
                {
                    if path.extension().is_none() {
                        path.set_extension(ext);
                    }
                    self.export_to(path);
                }
            }
            UiAction::ToggleVisible(id) => {
                self.project.edit("history-visibility", |d| {
                    if let Some(s) = d.sketch_mut(id) {
                        s.visible = !s.visible;
                    }
                });
            }
        }
    }

    fn apply_sketch_actions(&mut self, feature: FeatureId, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Edit { sketch, label } => {
                    self.project.replace_sketch(feature, sketch, label);
                }
                Action::Preview(sketch) => self.project.preview_sketch(feature, sketch),
                Action::CommitPreview(label) => self.project.commit_preview(label),
                Action::Finish => self.finish_sketch(),
            }
        }
    }

    fn undo_redo(&mut self, undo: bool) {
        if matches!(self.mode, Mode::Command(_)) {
            self.finish_sketch(); // cancels the dialog first
        }
        let changed = if undo { self.project.undo() } else { self.project.redo() };
        // Leave sketch mode if its sketch no longer exists.
        if changed
            && let Mode::Sketch(s) = &self.mode
            && self.project.doc().sketch(s.feature).is_none()
        {
            self.mode = Mode::Model;
        }
    }

    /// Asks to save unsaved changes before `pending` (new, open, close).
    fn guard(&mut self, pending: Pending, ctx: &egui::Context) {
        if self.project.is_dirty() {
            self.dialog = Some(Dialog::Unsaved(pending));
        } else {
            self.run_pending(pending, ctx);
        }
    }

    fn run_pending(&mut self, pending: Pending, ctx: &egui::Context) {
        match pending {
            Pending::New => {
                self.mode = Mode::Model;
                self.project = Project::default();
            }
            Pending::Open => {
                let file = rfd::FileDialog::new()
                    .add_filter("QOOB3D", &[FILE_EXTENSION])
                    .set_title(tr("file-open-tip"))
                    .pick_file();
                if let Some(path) = file {
                    match Project::open(&path) {
                        Ok(project) => {
                            self.mode = Mode::Model;
                            self.project = project;
                            self.needs_fit = true;
                        }
                        Err(err) => {
                            self.dialog = Some(Dialog::Error(tr_args(
                                "error-open",
                                &[
                                    ("file", path.display().to_string().into()),
                                    ("error", err.to_string().into()),
                                ],
                            )));
                        }
                    }
                }
            }
            Pending::Close => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Saves (asking for a path if needed); true on success.
    fn save(&mut self, ask: bool) -> bool {
        self.project.commit_preview("edit-move");
        let path = match (&self.project.path, ask) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new()
                .add_filter("QOOB3D", &[FILE_EXTENSION])
                .set_file_name(format!("{}.{FILE_EXTENSION}", tr("untitled")))
                .save_file(),
        };
        let Some(mut path) = path else { return false };
        if path.extension().is_none() {
            path.set_extension(FILE_EXTENSION);
        }
        match self.project.save_to(&path) {
            Ok(()) => true,
            Err(err) => {
                self.dialog = Some(Dialog::Error(tr_args(
                    "error-save",
                    &[
                        ("file", path.display().to_string().into()),
                        ("error", err.to_string().into()),
                    ],
                )));
                false
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, now: f64) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let cmd = Modifiers::COMMAND;
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        let (redo, undo, save_as, save, open, new) = ctx.input_mut(|i| {
            (
                i.consume_key(shift_cmd, Key::Z),
                i.consume_key(cmd, Key::Z),
                i.consume_key(shift_cmd, Key::S),
                i.consume_key(cmd, Key::S),
                i.consume_key(cmd, Key::O),
                i.consume_key(cmd, Key::N),
            )
        });
        if redo {
            self.undo_redo(false);
        } else if undo {
            self.undo_redo(true);
        }
        if save_as {
            self.save(true);
        } else if save {
            self.save(false);
        }
        if open {
            self.guard(Pending::Open, ctx);
        }
        if new {
            self.guard(Pending::New, ctx);
        }
        // S = Create Sketch, E = Extrude (Fusion); only in model mode.
        if matches!(self.mode, Mode::Model) {
            let (s, e) = ctx.input_mut(|i| {
                (i.consume_key(Modifiers::NONE, Key::S), i.consume_key(Modifiers::NONE, Key::E))
            });
            if s {
                self.apply_ui(UiAction::CreateSketch, ctx, now);
            } else if e {
                self.apply_ui(UiAction::Command(CommandKind::Extrude), ctx, now);
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.dialog else { return };
        let mut close = false;
        let mut then: Option<(bool, Pending)> = None; // (save first, action)
        match dialog {
            Dialog::Unsaved(pending) => {
                let pending = *pending;
                let name = self.project.file_name().unwrap_or_else(|| tr("untitled"));
                egui::Modal::new(egui::Id::new("qoob3d-unsaved")).show(ctx, |ui| {
                    ui.heading(tr("dialog-unsaved-title"));
                    ui.label(tr_args("dialog-unsaved-text", &[("name", name.into())]));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr("dialog-save")).clicked() {
                            then = Some((true, pending));
                        }
                        if ui.button(tr("dialog-discard")).clicked() {
                            then = Some((false, pending));
                        }
                        if ui.button(tr("dialog-cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::Error(message) => {
                let message = message.clone();
                egui::Modal::new(egui::Id::new("qoob3d-error")).show(ctx, |ui| {
                    ui.heading(tr("dialog-error-title"));
                    ui.label(message);
                    if ui.button(tr("dialog-ok")).clicked() {
                        close = true;
                    }
                });
            }
        }
        if close {
            self.dialog = None;
        }
        if let Some((save_first, pending)) = then {
            self.dialog = None;
            if !save_first || self.save(false) {
                self.run_pending(pending, ctx);
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.project.file_name().unwrap_or_else(|| tr("untitled"));
        let dirty = if self.project.is_dirty() { " •" } else { "" };
        let title = format!("{name}{dirty} – QOOB3D");
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// Keeps the command's preview in the project; commits or cancels.
    fn run_command(&mut self, outcome: CommandOutcome) {
        let Mode::Command(cmd) = &self.mode else { return };
        match outcome {
            CommandOutcome::Running => {
                if let Some(doc) = cmd.preview_document() {
                    self.project.preview_doc(doc);
                } else {
                    self.project.cancel_preview();
                }
            }
            CommandOutcome::Commit => {
                if let Some(doc) = cmd.final_document() {
                    let label = cmd.history_label();
                    self.project.preview_doc(doc);
                    self.project.commit_preview(label);
                }
                self.mode = Mode::Model;
            }
            CommandOutcome::Cancel => {
                self.project.cancel_preview();
                self.mode = Mode::Model;
            }
        }
    }

    fn observe(&self) -> Option<Observation<'_>> {
        let layout = self.viewport.layout()?;
        let (sketching, curves, profiles) = match &self.mode {
            Mode::Sketch(s) => (
                true,
                self.project.doc().sketch(s.feature).map_or(0, |k| k.curves.len()),
                self.project.profiles(s.feature).len(),
            ),
            _ => (false, 0, 0),
        };
        Some(Observation {
            camera: &self.viewport.camera,
            mesh_bounds: self.model_bounds(),
            viewport_center: layout.rect.center(),
            viewport_aspect: layout.rect.width() / layout.rect.height().max(1.0),
            viewcube_top: self.viewport.viewcube_face(CubeFace::Top),
            home_button: self.viewport.home_button(),
            last_orbit_pivot: self.viewport.last_orbit_pivot(),
            animating: self.viewport.is_animating(),
            xy_plane: self.plane_screen_pos(OriginPlane::XY),
            sketch_corners: [[0.0, 0.0], [12.0, 8.0]].map(|p| self.sketch_screen_pos(p)),
            sketching,
            picking_plane: matches!(self.mode, Mode::PickPlane),
            sketch_curves: curves,
            sketch_profiles: profiles,
            rebuilding: self.project.is_rebuilding(),
            features: self.project.doc().features.len(),
            commanding: matches!(self.mode, Mode::Command(_)),
            bodies: self.project.bodies().len(),
        })
    }
}

impl eframe::App for QoobApp {
    #[expect(clippy::too_many_lines, reason = "predates ADR-0040; split when next changed")]
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        self.poll_kernel(now);
        self.project.sync(&mut self.kernel);
        if self.project.bodies_version() != self.scene_version {
            self.scene_version = self.project.bodies_version();
            let bodies = self.project.bodies();
            self.scene = (!bodies.is_empty())
                .then(|| ShapeMesh::merge(bodies.iter().map(|b| b.mesh.as_ref())));
            let empty = ShapeMesh::default();
            let mesh = self.scene.as_ref().or(self.demo.as_ref()).unwrap_or(&empty);
            self.viewport.set_scene(&self.render_state.device, mesh);
        }

        // Unsaved changes on quit.
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.project.is_dirty()
            && self.smoke.is_none()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::Unsaved(Pending::Close));
        }
        if self.dialog.is_none() {
            self.shortcuts(&ctx, now);
        }
        // Tab/arrows belong to the value HUD while a tool places geometry;
        // without this egui moves keyboard focus to a toolbar widget first.
        if let Mode::Sketch(s) = &self.mode
            && s.hud_has_keyboard()
        {
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }

        let doc = self.project.doc().clone();
        let project = &self.project;
        let errors = |id: FeatureId| project.error(id).map(str::to_string);
        let (sketching, active_tool, construction) = match &self.mode {
            Mode::Sketch(s) => (Some(s.feature), s.active_tool(), s.construction_mode()),
            _ => (None, None, false),
        };
        let state = PanelState {
            doc: &doc,
            sketching,
            picking_plane: matches!(self.mode, Mode::PickPlane),
            active_tool,
            construction,
            undo: self.project.history().undo_label(),
            redo: self.project.history().redo_label(),
            has_demo_body: self.demo.is_some() && self.scene.is_none(),
            bodies: self.project.bodies(),
            command: self.command().map(|c| (c.kind, c.editing)),
            feature_error: &errors,
        };
        let title = self.project.file_name().unwrap_or_else(|| tr("untitled"));

        let mut actions = Vec::new();
        egui::Panel::top("qoob3d-toolbar")
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| actions.extend(panels::toolbar(ui, &mut self.settings, &state)));
        egui::Panel::bottom("qoob3d-timeline")
            .exact_size(40.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG_DARK)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| actions.extend(panels::timeline(ui, &state)));
        egui::Panel::left("qoob3d-browser")
            .default_size(230.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(theme::PANEL_BG).inner_margin(egui::Margin::same(8)))
            .show(ui, |ui| actions.extend(panels::browser(ui, &state, &title)));

        let status = match &self.notice {
            Some((text, until)) if now < *until => text.clone(),
            _ => self.status(),
        };
        let fit_bounds = self.fit_bounds();
        let mut sketch_actions = Vec::new();
        let mut command_outcome = CommandOutcome::Running;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            if self.needs_fit
                && self.viewport.layout().is_some()
                && let Some(bounds) = fit_bounds
            {
                self.viewport.fit_now(bounds);
                self.needs_fit = false;
            }
            let grid_plane = match &self.mode {
                Mode::Sketch(s) => Some(s.frame),
                _ => None,
            };
            let scene_mode = SceneMode {
                left_drag_orbit: matches!(self.mode, Mode::Model),
                grid_plane,
                fit_bounds,
            };
            let model = self.scene.as_ref().or(self.demo.as_ref());
            let frame =
                self.viewport.show_scene(ui, &self.render_state, model, &self.settings, scene_mode);
            let camera = self.viewport.camera;

            // Inactive sketches (SK-VS-10).
            for (id, s) in doc.sketches() {
                if !s.visible || sketching == Some(id) {
                    continue;
                }
                let Some(plane) = doc.plane_frame(s.plane) else { continue };
                let view = View { camera: &camera, rect: frame.rect, frame: plane };
                sketch_ui::draw_sketch(
                    &ui.painter_at(frame.rect),
                    &view,
                    s,
                    &[],
                    &Style::inactive(),
                );
            }

            match &mut self.mode {
                Mode::Model => {}
                Mode::Command(cmd) => {
                    cmd.viewport(ui, &frame.response, &camera, frame.rect, &self.project);
                    command_outcome = cmd.dialog(&ctx, frame.rect, &doc);
                }
                Mode::PickPlane => {
                    let planes = sketch_ui::pickable_planes(&doc);
                    match sketch_ui::plane_pick(ui, &frame.response, &camera, frame.rect, &planes) {
                        PlanePick::Picked(plane) => actions.push(UiAction::SketchOnPlane(plane)),
                        PlanePick::Cancel => self.mode = Mode::Model,
                        PlanePick::Nothing => {}
                    }
                }
                Mode::Sketch(session) => {
                    if let Some(sketch) = doc.sketch(session.feature) {
                        let view = View { camera: &camera, rect: frame.rect, frame: session.frame };
                        let profiles = self.project.profiles(session.feature);
                        let opts = SketchOptions {
                            snap: self.settings.sketch_snap,
                            grid_snap: self
                                .settings
                                .sketch_grid_snap
                                .then(|| f64::from(qoob3d_render::grid_snap_step(camera.distance))),
                            show_profiles: self.settings.show_profiles,
                            show_points: self.settings.show_points,
                            decimal_comma: i18n::language().decimal_separator() == ',',
                        };
                        sketch_actions =
                            session.interact(ui, &frame.response, &view, sketch, profiles, &opts);
                        session.draw(ui, &view, sketch, profiles, &opts);
                    }
                    let construction = session.construction_mode();
                    actions.extend(panels::sketch_palette(
                        &ctx,
                        frame.rect,
                        &mut self.settings,
                        construction,
                    ));
                }
            }
            self.viewport.show_overlays(ui, &frame, &status, &mut self.settings);
        });

        if let Mode::Sketch(s) = &self.mode {
            let feature = s.feature;
            self.apply_sketch_actions(feature, sketch_actions);
        }
        self.run_command(command_outcome);
        for action in actions {
            self.apply_ui(action, &ctx, now);
        }
        self.dialogs(&ctx);
        self.update_title(&ctx);
        // Keep polling while the kernel works on a rebuild or an export.
        if self.project.is_rebuilding() || self.export.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }

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
