//! The 3D viewport widget: offscreen scene (ADR-0019) plus egui overlays –
//! ViewCube, navigation bar, orbit pivot marker, status line (ADR-0015).

use eframe::egui::{
    self, Align2, Color32, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, TextureId, pos2, vec2,
};
use eframe::egui_wgpu::{self, wgpu};
use glam::Vec3;
use qoob3d_core::document::PlaneFrame;
use qoob3d_core::{Aabb, BedGeometry, PRINTERS, PrinterProfile, ShapeMesh};
use qoob3d_render::{Camera, GridPlane, Projection, SceneOptions, ViewPreset, ViewportRenderer};

use crate::i18n::{decimal, tr, tr_args};
use crate::navigation::{self, NavState};
use crate::settings::Settings;
use crate::theme;
use crate::viewcube::{self, CubeFace};

/// Duration of ViewCube / Home / fit transitions.
const ANIM_SECONDS: f64 = 0.35;
const VIEWCUBE_SIZE: f32 = 118.0;

struct CameraAnim {
    from: Camera,
    to: Camera,
    start: f64,
}

/// How the scene is shown this frame.
#[derive(Debug, Clone, Copy)]
pub struct SceneMode {
    /// Left drag may orbit (off in sketch mode, ADR-0027).
    pub left_drag_orbit: bool,
    /// Grid in this plane (active sketch), `None` = world XY with the plate.
    pub grid_plane: Option<PlaneFrame>,
    /// What "Fit" frames (bodies and sketches).
    pub fit_bounds: Option<Aabb>,
}

/// Result of [`Viewport::show_scene`].
pub struct SceneFrame {
    pub response: egui::Response,
    pub rect: Rect,
    pub now: f64,
    pub fit_bounds: Option<Aabb>,
}

/// Screen layout of the last frame (for tests and the smoke test).
#[derive(Debug, Clone, Copy)]
pub struct ViewportLayout {
    pub rect: Rect,
    pub viewcube: Rect,
}

pub struct Viewport {
    renderer: ViewportRenderer,
    texture: Option<TextureId>,
    pub camera: Camera,
    nav: NavState,
    anim: Option<CameraAnim>,
    layout: Option<ViewportLayout>,
    /// Pivot of the most recent orbit (smoke test checks it stays put).
    last_orbit_pivot: Option<Vec3>,
    /// Bed geometry of the selected printer, cached by printer id.
    bed: Option<(&'static str, BedGeometry)>,
    printer_search: String,
    /// Whether the printer list was open last frame (to reset the search
    /// and focus it when it opens).
    printer_picker_open: bool,
}

/// The selected printer, falling back to the default for unknown ids.
pub fn printer(settings: &Settings) -> &'static PrinterProfile {
    PrinterProfile::by_id(&settings.printer_id).unwrap_or_else(PrinterProfile::default_profile)
}

impl Viewport {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            renderer: ViewportRenderer::new(device),
            texture: None,
            camera: Camera::default(),
            nav: NavState::default(),
            anim: None,
            layout: None,
            last_orbit_pivot: None,
            bed: None,
            printer_search: String::new(),
            printer_picker_open: false,
        }
    }

    pub fn layout(&self) -> Option<ViewportLayout> {
        self.layout
    }

    pub fn last_orbit_pivot(&self) -> Option<Vec3> {
        self.last_orbit_pivot
    }

    pub fn is_animating(&self) -> bool {
        self.anim.is_some()
    }

    pub fn aspect(&self) -> f32 {
        self.layout.map_or(1.6, |l| l.rect.width() / l.rect.height().max(1.0))
    }

    pub fn set_scene(&mut self, device: &wgpu::Device, scene: &ShapeMesh) {
        self.renderer.set_scene(device, scene);
    }

    /// Frames `bounds` immediately (first model load).
    pub fn fit_now(&mut self, bounds: Aabb) {
        self.anim = None;
        self.camera.fit(bounds, self.aspect());
    }

    fn animate_to(&mut self, to: Camera, now: f64) {
        self.anim = Some(CameraAnim { from: self.camera, to, start: now });
    }

    fn fitted(&self, bounds: Option<Aabb>) -> Camera {
        let mut cam = self.camera;
        if let Some(b) = bounds {
            cam.fit(b, self.aspect());
        }
        cam
    }

    fn bed_for(&mut self, profile: &'static PrinterProfile) -> &BedGeometry {
        if self.bed.as_ref().is_none_or(|(id, _)| *id != profile.id) {
            self.bed = Some((profile.id, profile.bed()));
        }
        &self.bed.as_ref().expect("set above").1
    }

    /// Animates the camera to look straight at a sketch plane (SK-LC-03):
    /// the target moves onto the plane, the eye onto its normal side.
    pub fn look_at(&mut self, plane: &PlaneFrame, now: f64) {
        let n = Vec3::from(plane.n.map(|c| c as f32));
        let o = Vec3::from(plane.origin.map(|c| c as f32));
        let mut to = self.camera;
        to.target -= n * (to.target - o).dot(n);
        to.look_from(n);
        self.animate_to(to, now);
    }

    /// Navigation, camera animation and the 3D scene; returns what sketch
    /// interaction and overlays need. Call [`Viewport::show_overlays`] after
    /// drawing anything that belongs between scene and overlays.
    pub fn show_scene(
        &mut self,
        ui: &mut egui::Ui,
        rs: &egui_wgpu::RenderState,
        scene: Option<&ShapeMesh>,
        settings: &Settings,
        mode: SceneMode,
    ) -> SceneFrame {
        let rect = ui.available_rect_before_wrap();
        let response = ui.interact(rect, Id::new("qoob3d-viewport"), Sense::click_and_drag());
        let viewcube_rect = Rect::from_min_size(
            pos2(rect.right() - VIEWCUBE_SIZE - 24.0, rect.top() + 24.0),
            vec2(VIEWCUBE_SIZE, VIEWCUBE_SIZE),
        );
        self.layout = Some(ViewportLayout { rect, viewcube: viewcube_rect });
        let now = ui.input(|i| i.time);

        // --- navigation (ADR-0008, ADR-0022, ADR-0027) -------------------------
        let pick = |ray: qoob3d_render::Ray| {
            scene
                .and_then(|s| s.faces.raycast(ray.origin.to_array(), ray.dir.to_array()))
                .map(|hit| Vec3::from(hit.point))
        };
        let nav = navigation::handle_input(
            ui,
            &response,
            rect,
            &mut self.camera,
            &mut self.nav,
            &pick,
            settings.left_drag_orbit && mode.left_drag_orbit,
        );
        if nav.interacted {
            self.anim = None;
        }
        if let Some(pivot) = self.nav.orbit_pivot {
            self.last_orbit_pivot = Some(pivot);
        }
        if nav.fit_requested {
            self.animate_to(self.fitted(mode.fit_bounds), now);
        }

        // --- camera animation -------------------------------------------------
        if let Some(anim) = &self.anim {
            let t = ((now - anim.start) / ANIM_SECONDS).clamp(0.0, 1.0) as f32;
            let eased = t * t * (3.0 - 2.0 * t);
            self.camera = Camera::interpolate(&anim.from, &anim.to, eased);
            if t >= 1.0 {
                self.camera = anim.to;
                self.anim = None;
            } else {
                ui.ctx().request_repaint();
            }
        }

        // --- scene ------------------------------------------------------------
        let ppp = ui.ctx().pixels_per_point();
        let size_px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        let profile = printer(settings);
        let camera = self.camera;
        let bed = self.bed_for(profile).clone();
        let f = |a: [f64; 3]| Vec3::from(a.map(|c| c as f32));
        let options = SceneOptions {
            show_grid: settings.show_grid,
            bed: Some(&bed),
            show_build_volume: settings.show_build_volume,
            pixels_per_point: ppp,
            grid_plane: mode.grid_plane.map(|p| GridPlane {
                origin: f(p.origin),
                u: f(p.u),
                v: f(p.v),
                n: f(p.n),
            }),
        };
        let (view, recreated) =
            self.renderer.render(&rs.device, &rs.queue, size_px, &camera, &options);
        if recreated || self.texture.is_none() {
            let mut egui_renderer = rs.renderer.write();
            match self.texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(
                    &rs.device,
                    view,
                    wgpu::FilterMode::Linear,
                    id,
                ),
                None => {
                    self.texture = Some(egui_renderer.register_native_texture(
                        &rs.device,
                        view,
                        wgpu::FilterMode::Linear,
                    ));
                }
            }
        }
        if let Some(id) = self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            ui.painter().image(id, rect, uv, Color32::WHITE);
        }
        SceneFrame { response, rect, now, fit_bounds: mode.fit_bounds }
    }

    /// Orbit pivot, ViewCube, navigation bar and status line (ADR-0015).
    pub fn show_overlays(
        &mut self,
        ui: &mut egui::Ui,
        frame: &SceneFrame,
        status: &str,
        settings: &mut Settings,
    ) {
        let (rect, now, bounds) = (frame.rect, frame.now, frame.fit_bounds);
        let aspect = rect.width() / rect.height().max(1.0);
        let painter = ui.painter_at(rect);
        if let Some(pivot) = self.nav.orbit_pivot
            && let Some(ndc) = self.camera.project(pivot, aspect)
        {
            let p = navigation::from_ndc(ndc.truncate(), rect);
            painter.circle(p, 4.0, theme::ACCENT, Stroke::new(1.5, Color32::from_gray(20)));
        }

        let viewcube_rect = self.layout.map_or(rect, |l| l.viewcube);
        let cube = viewcube::show(ui, viewcube_rect, self.camera.rotation);
        if let Some(dir) = cube.clicked {
            let mut to = self.camera;
            to.look_from(dir);
            self.animate_to(to, now);
        }
        if cube.home_clicked {
            let mut to = self.camera;
            to.look_from(ViewPreset::Iso.eye_direction());
            if let Some(b) = bounds {
                to.fit(b, aspect);
            }
            self.animate_to(to, now);
        }

        if self.navigation_bar(ui.ctx(), rect, settings) {
            self.animate_to(self.fitted(bounds), now);
        }

        // Top left: the bottom centre belongs to the navigation bar.
        painter.text(
            rect.left_top() + vec2(12.0, 10.0),
            Align2::LEFT_TOP,
            status,
            FontId::proportional(12.0),
            theme::TEXT_MUTED,
        );
    }

    /// Fusion-like floating bar at the bottom centre. Returns true if
    /// "Fit" was clicked.
    fn navigation_bar(&mut self, ctx: &egui::Context, rect: Rect, settings: &mut Settings) -> bool {
        let mut fit = false;
        egui::Area::new(Id::new("qoob3d-navbar"))
            .fixed_pos(pos2(rect.center().x, rect.bottom() - 12.0))
            .pivot(Align2::CENTER_BOTTOM)
            .constrain_to(rect)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::OVERLAY_BG)
                    .stroke(Stroke::new(1.0, theme::STROKE))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            fit =
                                ui.button(tr("nav-fit")).on_hover_text(tr("nav-fit-tip")).clicked();
                            ui.separator();
                            ui.selectable_value(
                                &mut self.camera.projection,
                                Projection::Perspective,
                                tr("nav-perspective"),
                            );
                            ui.selectable_value(
                                &mut self.camera.projection,
                                Projection::Orthographic,
                                tr("nav-orthographic"),
                            );
                            ui.separator();
                            ui.checkbox(&mut settings.show_grid, tr("nav-grid"));
                            ui.checkbox(&mut settings.show_build_volume, tr("nav-volume"));
                            ui.separator();
                            self.printer_picker(ui, settings);
                        });
                    });
            });
        fit
    }

    /// Printer selection (ADR-0021, ADR-0023): search field, "my printers"
    /// on top, the rest grouped by vendor (hidden if the user only wants
    /// their own), markers for printers with restricted zones.
    fn printer_picker(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        let current = printer(settings);
        let mut open_now = false;
        egui::ComboBox::from_id_salt("qoob3d-printer")
            .selected_text(current.name)
            .width(210.0)
            .height(440.0)
            // Clicking into the search field must not close the list; a
            // selection closes it explicitly (`ui.close()`).
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                open_now = true;
                let just_opened = !self.printer_picker_open;
                if just_opened {
                    self.printer_search.clear();
                }
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.printer_search)
                        .hint_text(tr("printer-search"))
                        .desired_width(280.0),
                );
                if just_opened {
                    search.request_focus(); // type right away
                }
                ui.label(theme::caption(&tr("printer-legend")));
                let needle = self.printer_search.to_lowercase();
                let matches = |p: &PrinterProfile| {
                    needle.is_empty()
                        || p.name.to_lowercase().contains(&needle)
                        || p.vendor.to_lowercase().contains(&needle)
                };

                let favourites: Vec<&'static PrinterProfile> = settings
                    .favorite_printers
                    .iter()
                    .filter_map(|id| PrinterProfile::by_id(id))
                    .filter(|p| matches(p))
                    .collect();
                if !favourites.is_empty() {
                    ui.separator();
                    ui.label(theme::caption(&tr("printers-mine")));
                    for profile in favourites {
                        printer_row(ui, profile, settings);
                    }
                }

                let only_mine = settings.only_favorite_printers;
                if only_mine && !settings.favorite_printers.is_empty() {
                    return;
                }
                ui.separator();
                if only_mine {
                    ui.label(theme::caption(&tr("printers-only-mine-empty")));
                }
                ui.label(theme::caption(&tr("printers-all")));
                let mut last_vendor = "";
                let others: Vec<&'static PrinterProfile> = PRINTERS
                    .iter()
                    .filter(|p| {
                        matches(p) && !settings.favorite_printers.iter().any(|id| id == p.id)
                    })
                    .collect();
                for profile in others {
                    if profile.vendor != last_vendor {
                        ui.add_space(4.0);
                        ui.label(theme::caption(profile.vendor));
                        last_vendor = profile.vendor;
                    }
                    printer_row(ui, profile, settings);
                }
            })
            .response
            .on_hover_text(format!("{}\n{}", tr("nav-printer-tip"), printer_details(current)));
        self.printer_picker_open = open_now;
    }

    /// Screen position of a ViewCube face, if visible.
    pub fn viewcube_face(&self, face: CubeFace) -> Option<Pos2> {
        let layout = self.layout?;
        let half = 0.5 * layout.viewcube.width();
        viewcube::face_center(self.camera.rotation, layout.viewcube.center(), half, face)
    }

    pub fn printer_search(&self) -> &str {
        &self.printer_search
    }

    pub fn home_button(&self) -> Option<Pos2> {
        self.layout.map(|l| viewcube::home_button_center(l.viewcube))
    }
}

/// One entry of the printer list: favourite star + selectable name.
fn printer_row(ui: &mut egui::Ui, profile: &'static PrinterProfile, settings: &mut Settings) {
    ui.horizontal(|ui| {
        let favourite = settings.favorite_printers.iter().any(|id| id == profile.id);
        let (star, tip, color) = if favourite {
            ("★", tr("printer-favorite-remove"), theme::ACCENT)
        } else {
            ("☆", tr("printer-favorite-add"), theme::TEXT_MUTED)
        };
        let star = ui
            .add(egui::Button::new(RichText::new(star).color(color)).frame(false))
            .on_hover_text(tip);
        if star.clicked() {
            if favourite {
                settings.favorite_printers.retain(|id| id != profile.id);
            } else {
                settings.favorite_printers.push(profile.id.to_string());
            }
        }

        let mut text = profile.name.to_string();
        if profile.has_exclude_area() {
            text.push_str("  ▣");
        }
        if profile.has_single_nozzle_zones() {
            text.push_str("  ◑");
        }
        let selected = profile.id == settings.printer_id;
        let row = ui.selectable_label(selected, text).on_hover_text(printer_details(profile));
        if row.clicked() {
            settings.printer_id = profile.id.to_string();
            ui.close();
        }
    });
}

/// Size and special zones of a printer, for tooltips.
fn printer_details(p: &PrinterProfile) -> String {
    let [w, d] = p.size();
    let mut lines = vec![tr_args(
        "printer-size",
        &[
            ("width", decimal(f64::from(w), 0).into()),
            ("depth", decimal(f64::from(d), 0).into()),
            ("height", decimal(f64::from(p.max_height_mm), 0).into()),
        ],
    )];
    if p.has_exclude_area() {
        lines.push(format!("▣ {}", tr("printer-note-exclude")));
    }
    if p.has_single_nozzle_zones() {
        lines.push(format!("◑ {}", tr("printer-note-nozzles")));
    }
    lines.join("\n")
}
