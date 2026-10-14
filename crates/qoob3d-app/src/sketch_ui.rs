//! Sketch mode in the UI (ADR-0027): maps the pointer onto the sketch plane,
//! feeds the pure tools/snapping of `qoob3d-sketch`, draws sketches as an egui
//! overlay and handles plane picking. Requirements: docs/requirements/skizzen.md.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Event, FontId, Key, Mesh, PointerButton, Pos2, Rect,
    Response, Shape, Stroke, vec2,
};
use glam::Vec3;
use qoob3d_core::document::{
    CurveKind, EntityId, FeatureId, OriginPlane, PlaneFrame, PlaneRef, Sketch,
};
use qoob3d_core::expr;
use qoob3d_core::profile::Profile;
use qoob3d_render::Camera;
use qoob3d_sketch::DVec2;
use qoob3d_sketch::edit::{self, Grab};
use qoob3d_sketch::pick::{self, Hit, HitKind};
use qoob3d_sketch::snap::{self, Snap, SnapKind, SnapSettings};
use qoob3d_sketch::tools::{FieldUnit, Outcome, Tool, ToolEvent, ToolKind};

use crate::i18n::{self, tr};
use crate::navigation::{from_ndc, to_ndc};
use crate::theme;

/// Snap tolerance and pick radius in screen pixels (SK-SN-03, SK-UI-09).
const SNAP_PX: f64 = 8.0;
const PICK_PX: f64 = 6.0;

/// Screen ↔ sketch mapping for one frame.
pub struct View<'a> {
    pub camera: &'a Camera,
    pub rect: Rect,
    pub frame: PlaneFrame,
}

impl View<'_> {
    fn aspect(&self) -> f32 {
        self.rect.width() / self.rect.height().max(1.0)
    }

    pub fn to_screen(&self, p: DVec2) -> Option<Pos2> {
        let w = self.frame.to_world(p.to_array());
        let ndc =
            self.camera.project(Vec3::new(w[0] as f32, w[1] as f32, w[2] as f32), self.aspect())?;
        Some(from_ndc(ndc.truncate(), self.rect))
    }

    /// Pointer → sketch coordinates by casting a ray onto the plane (SK-UI-01).
    pub fn to_local(&self, s: Pos2) -> Option<DVec2> {
        let ray = self.camera.ray(to_ndc(s, self.rect), self.aspect());
        let o = ray.origin.as_dvec3().to_array();
        let d = ray.dir.as_dvec3().to_array();
        // Nearly edge-on planes give no usable input (SK-UI-01: < 5°).
        let n = self.frame.n;
        let cos = (d[0] * n[0] + d[1] * n[1] + d[2] * n[2]).abs();
        if cos < 5f64.to_radians().sin() {
            return None;
        }
        self.frame.ray_hit(o, d).map(|p| DVec2::new(p[0], p[1]))
    }

    /// Sketch units per screen pixel at `s` (for pixel tolerances).
    pub fn units_per_px(&self, s: Pos2) -> f64 {
        match (self.to_local(s), self.to_local(s + vec2(1.0, 0.0))) {
            (Some(a), Some(b)) => a.distance(b).max(1e-9),
            _ => 0.1,
        }
    }
}

/// What the sketch session asks the app to do.
#[derive(Debug, Clone)]
pub enum Action {
    /// Replace the sketch as one undo step (label = translation key).
    Edit {
        sketch: Sketch,
        label: &'static str,
    },
    /// Live change without undo step (dragging).
    Preview(Sketch),
    CommitPreview(&'static str),
    Finish,
}

#[derive(Debug, Clone, Copy)]
pub struct SketchOptions {
    pub snap: bool,
    pub grid_snap: Option<f64>,
    pub show_profiles: bool,
    pub show_points: bool,
    /// Accept `,` as decimal separator in typed values (German UI).
    pub decimal_comma: bool,
}

struct Drag {
    grab: Grab,
    working: Sketch,
    /// Point under the cursor to merge with on release (dragging a point).
    merge_into: Option<EntityId>,
}

pub struct SketchSession {
    pub feature: FeatureId,
    pub frame: PlaneFrame,
    pub tool: Option<Tool>,
    pub selection: Vec<EntityId>,
    /// History position when the session started (one undo step, ADR-0029).
    pub history_marker: usize,
    hover: Option<Hit>,
    hover_profile: Option<usize>,
    drag: Option<Drag>,
    boxsel: Option<(Pos2, Pos2)>,
    hud_field: usize,
    hud_text: String,
    hud_error: bool,
    last_snap: Option<Snap>,
    cursor_screen: Option<Pos2>,
    /// Last click (screen position, time) for our own double-click test.
    last_click: Option<(Pos2, f64)>,
}

impl SketchSession {
    pub fn new(feature: FeatureId, frame: PlaneFrame, history_marker: usize) -> Self {
        Self {
            feature,
            frame,
            tool: None,
            selection: Vec::new(),
            history_marker,
            hover: None,
            hover_profile: None,
            drag: None,
            boxsel: None,
            hud_field: 0,
            hud_text: String::new(),
            hud_error: false,
            last_snap: None,
            cursor_screen: None,
            last_click: None,
        }
    }

    pub fn start_tool(&mut self, kind: ToolKind) {
        let construction = self.tool.as_ref().is_some_and(|t| t.construction);
        let mut tool = Tool::new(kind);
        tool.construction = construction;
        self.tool = Some(tool);
        self.selection.clear();
        self.reset_hud();
    }

    /// True while a tool places geometry: keys go to the value HUD.
    pub fn hud_has_keyboard(&self) -> bool {
        self.tool.as_ref().is_some_and(Tool::is_placing)
    }

    pub fn active_tool(&self) -> Option<ToolKind> {
        self.tool.as_ref().map(Tool::kind)
    }

    fn reset_hud(&mut self) {
        self.hud_field = 0;
        self.hud_text.clear();
        self.hud_error = false;
    }

    /// Pointer and keyboard handling for one frame.
    pub fn interact(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        view: &View<'_>,
        sketch: &Sketch,
        profiles: &[Profile],
        opts: &SketchOptions,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        // Query the response before ui.input (egui deadlock rule, CLAUDE.md).
        let clicked = response.clicked();
        let drag_started = response.drag_started_by(PointerButton::Primary);
        let dragging = response.dragged_by(PointerButton::Primary);
        let drag_stopped = response.drag_stopped_by(PointerButton::Primary);
        let hovered = response.hovered();
        let (pointer, press_origin, command, shift, now) = ui.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.press_origin(),
                i.modifiers.command,
                i.modifiers.shift,
                i.time,
            )
        });
        // egui's double click ignores the position; ending a polyline needs a
        // second click on the same spot, otherwise quick clicking breaks it.
        let double = clicked
            && pointer.is_some_and(|p| {
                self.last_click.is_some_and(|(q, t)| now - t < 0.35 && q.distance(p) < 4.0)
            });
        if clicked {
            self.last_click = pointer.map(|p| (p, now));
        }
        let in_view = pointer.filter(|p| view.rect.contains(*p));
        self.cursor_screen = if hovered || dragging { pointer } else { None };
        let cursor = self.cursor_screen.and_then(|p| view.to_local(p));
        let units_px = self.cursor_screen.map_or(0.1, |p| view.units_per_px(p));
        let settings = SnapSettings { enabled: opts.snap && !command, grid: opts.grid_snap };

        if let Some(tool) = &mut self.tool {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            if let Some(c) = cursor {
                let s = snap::snap(sketch, c, SNAP_PX * units_px, tool.anchor(), &settings, &[]);
                let mut scratch = sketch.clone();
                tool.handle(&mut scratch, ToolEvent::Move(&s));
                if clicked {
                    let mut edited = sketch.clone();
                    if tool.handle(&mut edited, ToolEvent::Click(&s)) == Outcome::Edited {
                        self.hud_field = 0;
                        self.hud_text.clear();
                        actions.push(Action::Edit { sketch: edited, label: "edit-draw" });
                    }
                }
                if double {
                    tool.handle(&mut scratch, ToolEvent::EndChain);
                }
                self.last_snap = Some(s);
            } else {
                self.last_snap = None;
            }
        } else {
            self.last_snap = None;
            self.select_mode(
                ui,
                view,
                sketch,
                profiles,
                opts,
                (cursor, units_px, press_origin, in_view),
                (clicked, drag_started, dragging, drag_stopped, shift),
                &settings,
                &mut actions,
            );
        }

        self.keys(ui, sketch, opts.decimal_comma, &mut actions);
        actions
    }

    #[allow(clippy::too_many_arguments)] // one call site; splitting adds noise
    fn select_mode(
        &mut self,
        ui: &egui::Ui,
        view: &View<'_>,
        sketch: &Sketch,
        profiles: &[Profile],
        opts: &SketchOptions,
        (cursor, units_px, press_origin, in_view): (Option<DVec2>, f64, Option<Pos2>, Option<Pos2>),
        (clicked, drag_started, dragging, drag_stopped, shift): (bool, bool, bool, bool, bool),
        settings: &SnapSettings,
        actions: &mut Vec<Action>,
    ) {
        if self.drag.is_none() {
            self.hover = cursor.and_then(|c| pick::pick(sketch, c, PICK_PX * units_px));
            self.hover_profile = match (self.hover, cursor) {
                (None, Some(c)) if opts.show_profiles => {
                    profiles.iter().position(|p| p.contains(c.to_array()))
                }
                _ => None,
            };
        }

        if drag_started && let Some(origin) = press_origin {
            let start = view.to_local(origin);
            let hit =
                start.and_then(|s| pick::pick(sketch, s, PICK_PX * view.units_per_px(origin)));
            match (hit, start) {
                (Some(hit), Some(s)) => {
                    if let Some(grab) = edit::grab(sketch, hit, s) {
                        self.drag = Some(Drag { grab, working: sketch.clone(), merge_into: None });
                    }
                }
                _ => self.boxsel = Some((origin, origin)),
            }
        }
        if dragging {
            if let (Some(drag), Some(c)) = (&mut self.drag, cursor) {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                let exclude: Vec<EntityId> = match &drag.grab {
                    Grab::Point(id) => vec![*id],
                    Grab::Line { points, .. } => points.to_vec(),
                    Grab::CircleRim(id) => vec![*id],
                };
                let s = snap::snap(&drag.working, c, SNAP_PX * units_px, None, settings, &exclude);
                drag.merge_into = match drag.grab {
                    Grab::Point(_) => s.entity,
                    _ => None,
                };
                edit::drag(&mut drag.working, &drag.grab, s.point);
                actions.push(Action::Preview(drag.working.clone()));
                self.last_snap = Some(s);
            }
            if let (Some(bs), Some(p)) = (&mut self.boxsel, in_view) {
                bs.1 = p;
            }
        }
        if drag_stopped {
            if let Some(mut drag) = self.drag.take() {
                if let (Grab::Point(id), Some(into)) = (&drag.grab, drag.merge_into) {
                    edit::merge_points(&mut drag.working, *id, into);
                    actions.push(Action::Preview(drag.working));
                }
                actions.push(Action::CommitPreview("edit-move"));
            }
            if let Some((a, b)) = self.boxsel.take() {
                let crossing = b.x < a.x; // right → left (Fusion)
                let found = box_select(view, sketch, a, b, crossing);
                if !shift {
                    self.selection.clear();
                }
                for id in found {
                    if !self.selection.contains(&id) {
                        self.selection.push(id);
                    }
                }
            }
        }
        if clicked {
            match self.hover {
                Some(hit) if shift => {
                    if let Some(i) = self.selection.iter().position(|id| *id == hit.id) {
                        self.selection.remove(i);
                    } else {
                        self.selection.push(hit.id);
                    }
                }
                Some(hit) => self.selection = vec![hit.id],
                None => self.selection.clear(),
            }
        }
    }

    fn keys(
        &mut self,
        ui: &egui::Ui,
        sketch: &Sketch,
        decimal_comma: bool,
        actions: &mut Vec<Action>,
    ) {
        if ui.ctx().egui_wants_keyboard_input() {
            return; // a text field elsewhere has focus
        }
        let events = ui.input(|i| i.events.clone());
        for event in events {
            let placing = self.tool.as_ref().is_some_and(Tool::is_placing);
            match event {
                Event::Text(text) if placing => {
                    let ok = |c: char| c.is_ascii_alphanumeric() || "+-*/()., ".contains(c);
                    self.hud_text.extend(text.chars().filter(|c| ok(*c)));
                    self.hud_error = false;
                }
                Event::Key { key, pressed: true, modifiers, .. } => {
                    if modifiers.command {
                        continue; // Cmd shortcuts are global (undo, save …)
                    }
                    self.key(key, placing, sketch, decimal_comma, actions);
                }
                _ => {}
            }
        }
    }

    fn key(
        &mut self,
        key: Key,
        placing: bool,
        sketch: &Sketch,
        decimal_comma: bool,
        actions: &mut Vec<Action>,
    ) {
        match key {
            Key::Escape => {
                if !self.hud_text.is_empty() {
                    self.hud_text.clear();
                    self.hud_error = false;
                } else if let Some(tool) = &mut self.tool {
                    let mut scratch = sketch.clone();
                    if tool.handle(&mut scratch, ToolEvent::Escape) == Outcome::Done {
                        self.tool = None;
                    }
                    self.reset_hud();
                } else {
                    self.selection.clear();
                }
            }
            Key::Enter => {
                if let Some(tool) = &mut self.tool {
                    let mut edited = sketch.clone();
                    if apply_hud(
                        tool,
                        &mut edited,
                        self.hud_field,
                        &mut self.hud_text,
                        decimal_comma,
                    ) {
                        if tool.handle(&mut edited, ToolEvent::Commit) == Outcome::Edited {
                            actions.push(Action::Edit { sketch: edited, label: "edit-draw" });
                        }
                        self.hud_field = 0;
                        self.hud_error = false;
                    } else {
                        self.hud_error = true;
                    }
                } else {
                    actions.push(Action::Finish);
                }
            }
            Key::Tab if placing => {
                if let Some(tool) = &mut self.tool {
                    let mut scratch = sketch.clone();
                    if apply_hud(
                        tool,
                        &mut scratch,
                        self.hud_field,
                        &mut self.hud_text,
                        decimal_comma,
                    ) {
                        let n = tool.fields().len().max(1);
                        self.hud_field = (self.hud_field + 1) % n;
                        self.hud_error = false;
                    } else {
                        self.hud_error = true;
                    }
                }
            }
            Key::Backspace if !self.hud_text.is_empty() => {
                self.hud_text.pop();
            }
            Key::Backspace | Key::Delete if self.tool.is_none() && !self.selection.is_empty() => {
                let mut edited = sketch.clone();
                edited.delete(&self.selection);
                self.selection.clear();
                actions.push(Action::Edit { sketch: edited, label: "edit-delete" });
            }
            // Tool shortcuts never fire while typing a value.
            Key::L if !placing => self.start_tool(ToolKind::Line),
            Key::R if !placing => self.start_tool(ToolKind::Rectangle),
            Key::C if !placing => self.start_tool(ToolKind::Circle),
            Key::A if !placing => self.start_tool(ToolKind::Arc),
            Key::X if !placing => self.toggle_construction(sketch, actions),
            _ => {}
        }
    }

    /// X: construction mode for the tool, or toggle on the selection.
    pub fn toggle_construction(&mut self, sketch: &Sketch, actions: &mut Vec<Action>) {
        if let Some(tool) = &mut self.tool {
            tool.construction = !tool.construction;
        } else if !self.selection.is_empty() {
            let mut edited = sketch.clone();
            edit::toggle_construction(&mut edited, &self.selection);
            actions.push(Action::Edit { sketch: edited, label: "edit-construction" });
        }
    }

    pub fn construction_mode(&self) -> bool {
        self.tool.as_ref().is_some_and(|t| t.construction)
    }

    /// Draws the active sketch with selection, preview, snap glyph and HUD.
    pub fn draw(
        &self,
        ui: &egui::Ui,
        view: &View<'_>,
        sketch: &Sketch,
        profiles: &[Profile],
        opts: &SketchOptions,
    ) {
        let painter = ui.painter_at(view.rect);
        let style = Style {
            active: true,
            selection: &self.selection,
            hover: self.hover,
            hover_profile: self.hover_profile,
            show_profiles: opts.show_profiles,
            show_points: opts.show_points,
        };
        draw_sketch(&painter, view, sketch, profiles, &style);

        if let Some(tool) = &self.tool {
            let preview = tool.preview();
            let colour = if tool.construction { theme::CONSTRUCTION } else { theme::ACCENT };
            let dash = |a: Pos2, b: Pos2, painter: &egui::Painter| {
                painter.extend(Shape::dashed_line(&[a, b], Stroke::new(1.4, colour), 6.0, 4.0));
            };
            for [a, b] in &preview.segments {
                if let (Some(a), Some(b)) = (view.to_screen(*a), view.to_screen(*b)) {
                    dash(a, b, &painter);
                }
            }
            for (c, r) in &preview.circles {
                let pts = circle_screen(view, *c, *r);
                for w in pts.windows(2) {
                    dash(w[0], w[1], &painter);
                }
            }
        }
        if let Some(s) = &self.last_snap {
            draw_snap(&painter, view, s);
        }
        if let Some((a, b)) = self.boxsel {
            let crossing = b.x < a.x;
            let color = if crossing { theme::ACCENT } else { Color32::from_rgb(0x6c, 0xa8, 0xff) };
            let r = Rect::from_two_pos(a, b);
            painter.rect_filled(r, 0.0, color.gamma_multiply(0.12));
            if crossing {
                painter.extend(Shape::dashed_line(
                    &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()],
                    Stroke::new(1.0, color),
                    5.0,
                    3.0,
                ));
            } else {
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
            }
        }
        if let (Some(tool), Some(cursor)) = (&self.tool, self.cursor_screen) {
            self.draw_hud(ui, tool, cursor);
        }
    }

    fn draw_hud(&self, ui: &egui::Ui, tool: &Tool, cursor: Pos2) {
        let fields = tool.fields();
        if fields.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("qoob3d-sketch-hud"))
            .fixed_pos(cursor + vec2(20.0, 18.0))
            .interactable(false)
            .order(egui::Order::Tooltip)
            .show(ui.ctx(), |ui| {
                // Opaque: the HUD may sit on top of the palette.
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
                    .stroke(Stroke::new(1.0, theme::STROKE))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        for (i, f) in fields.iter().enumerate() {
                            let active = i == self.hud_field;
                            let unit = match f.unit {
                                FieldUnit::Length => " mm",
                                FieldUnit::Angle => "°",
                            };
                            let value = if active && !self.hud_text.is_empty() {
                                format!("{}|", self.hud_text)
                            } else {
                                format!("{}{unit}", i18n::decimal(f.value, 2))
                            };
                            let lock = if f.locked { " 🔒" } else { "" };
                            let color = if active && self.hud_error {
                                theme::ERROR
                            } else if active {
                                theme::ACCENT
                            } else {
                                theme::TEXT
                            };
                            ui.label(
                                egui::RichText::new(format!("{}  {value}{lock}", tr(f.key)))
                                    .color(color)
                                    .monospace(),
                            );
                        }
                        ui.label(theme::caption(&tr("hint-tab-enter")));
                    });
            });
    }
}

/// Applies typed text to the active field; false if it does not parse.
fn apply_hud(
    tool: &mut Tool,
    sketch: &mut Sketch,
    field: usize,
    text: &mut String,
    decimal_comma: bool,
) -> bool {
    if text.trim().is_empty() {
        text.clear();
        return true;
    }
    let unit = tool.fields().get(field).map(|f| f.unit);
    let value = match unit {
        Some(FieldUnit::Length) => expr::eval_length_mm(text, decimal_comma),
        Some(FieldUnit::Angle) => expr::eval_number(text, decimal_comma),
        None => return false,
    };
    match value {
        Ok(v) => {
            tool.handle(sketch, ToolEvent::SetField { index: field, value: v });
            text.clear();
            true
        }
        Err(_) => false,
    }
}

/// Screen-space box selection (works for any view direction).
fn box_select(view: &View<'_>, sketch: &Sketch, a: Pos2, b: Pos2, crossing: bool) -> Vec<EntityId> {
    let r = Rect::from_two_pos(a, b);
    let mut out = Vec::new();
    for p in &sketch.points {
        if view.to_screen(DVec2::new(p.x, p.y)).is_some_and(|s| r.contains(s)) {
            out.push(p.id);
        }
    }
    for c in &sketch.curves {
        let pts = curve_screen(view, sketch, &c.kind);
        let inside = pts.iter().filter(|p| r.contains(**p)).count();
        let hit = if crossing {
            inside > 0 || pts.windows(2).any(|w| segment_crosses_rect(w[0], w[1], r))
        } else {
            !pts.is_empty() && inside == pts.len()
        };
        if hit {
            out.push(c.id);
        }
    }
    out
}

fn segment_crosses_rect(a: Pos2, b: Pos2, r: Rect) -> bool {
    use qoob3d_sketch::geom::{Shape as G, intersections};
    let d = |p: Pos2| DVec2::new(f64::from(p.x), f64::from(p.y));
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()];
    (0..4).any(|i| {
        !intersections(G::Segment(d(a), d(b)), G::Segment(d(corners[i]), d(corners[(i + 1) % 4])))
            .is_empty()
    })
}

/// A curve as a screen polyline.
pub fn curve_screen(view: &View<'_>, sketch: &Sketch, kind: &CurveKind) -> Vec<Pos2> {
    use qoob3d_sketch::geom::v;
    match *kind {
        CurveKind::Line { start, end } => {
            [start, end].iter().filter_map(|id| view.to_screen(v(sketch.pos(*id)))).collect()
        }
        CurveKind::Circle { center, radius } => circle_screen(view, v(sketch.pos(center)), radius),
        CurveKind::Arc { center, start, end } => {
            let g = sketch.arc(center, start, end);
            let n = (g.sweep / std::f64::consts::TAU * 128.0).ceil().max(8.0) as usize;
            qoob3d_sketch::geom::arc_polyline(v(g.center), g.radius, g.start_angle, g.sweep, n)
                .into_iter()
                .filter_map(|p| view.to_screen(p))
                .collect()
        }
    }
}

fn circle_screen(view: &View<'_>, c: DVec2, r: f64) -> Vec<Pos2> {
    let px = view
        .to_screen(c)
        .zip(view.to_screen(c + DVec2::new(r, 0.0)))
        .map_or(32.0, |(a, b)| a.distance(b));
    let n = (px as usize / 2).clamp(32, 256);
    qoob3d_sketch::geom::circle_polyline(c, r, n)
        .into_iter()
        .filter_map(|p| view.to_screen(p))
        .collect()
}

/// Display style of a sketch.
pub struct Style<'a> {
    pub active: bool,
    pub selection: &'a [EntityId],
    pub hover: Option<Hit>,
    pub hover_profile: Option<usize>,
    pub show_profiles: bool,
    pub show_points: bool,
}

impl Style<'_> {
    pub fn inactive() -> Style<'static> {
        Style {
            active: false,
            selection: &[],
            hover: None,
            hover_profile: None,
            show_profiles: false,
            show_points: false,
        }
    }
}

/// Draws a sketch (SK-VS-*): profiles, curves, points, open ends.
pub fn draw_sketch(
    painter: &egui::Painter,
    view: &View<'_>,
    sketch: &Sketch,
    profiles: &[Profile],
    style: &Style<'_>,
) {
    if style.active && style.show_profiles {
        for (i, profile) in profiles.iter().enumerate() {
            let alpha = if style.hover_profile == Some(i) { 0.25 } else { 0.12 };
            let color = theme::ACCENT.gamma_multiply(alpha);
            let mut mesh = Mesh::default();
            let screen: Vec<Option<Pos2>> =
                profile.positions.iter().map(|p| view.to_screen(DVec2::new(p[0], p[1]))).collect();
            if screen.iter().any(Option::is_none) {
                continue;
            }
            for p in screen.into_iter().flatten() {
                mesh.colored_vertex(p, color);
            }
            for t in &profile.triangles {
                mesh.add_triangle(t[0], t[1], t[2]);
            }
            painter.add(Shape::mesh(mesh));
        }
    }

    for c in &sketch.curves {
        let selected = style.selection.contains(&c.id);
        let hovered = style.hover.is_some_and(|h| h.id == c.id);
        let (color, width) = if !style.active {
            (Color32::from_gray(120), 1.0)
        } else if selected {
            (Color32::WHITE, 2.4)
        } else if hovered {
            (theme::ACCENT.gamma_multiply(1.0), 2.6)
        } else if c.construction {
            (theme::CONSTRUCTION, 1.4)
        } else {
            (theme::ACCENT, 1.6)
        };
        let pts = curve_screen(view, sketch, &c.kind);
        if pts.len() < 2 {
            continue;
        }
        let stroke = Stroke::new(width, color);
        if c.construction && !selected {
            painter.extend(Shape::dashed_line(&pts, stroke, 6.0, 4.0));
        } else {
            painter.add(Shape::line(pts, stroke));
        }
    }

    if !style.active {
        return;
    }
    // Open ends of non-construction lines (SK-PR-04).
    let mut uses = std::collections::HashMap::<EntityId, usize>::new();
    for c in sketch.curves.iter().filter(|c| !c.construction) {
        if let CurveKind::Line { start, end } | CurveKind::Arc { start, end, .. } = c.kind {
            *uses.entry(start).or_default() += 1;
            *uses.entry(end).or_default() += 1;
        }
    }
    for p in &sketch.points {
        let Some(s) = view.to_screen(DVec2::new(p.x, p.y)) else { continue };
        let open = uses.get(&p.id) == Some(&1);
        let selected = style.selection.contains(&p.id);
        let hovered = style.hover.is_some_and(|h| h.kind == HitKind::Point && h.id == p.id);
        if open {
            painter.circle_filled(s, 3.5, theme::ERROR);
        } else if style.show_points || selected || hovered {
            let size = if hovered || selected { 3.5 } else { 2.5 };
            let fill = if selected { Color32::WHITE } else { Color32::from_gray(220) };
            painter.rect_filled(Rect::from_center_size(s, vec2(size, size) * 2.0), 0.0, fill);
        }
    }
    // Sketch origin.
    if let Some(o) = view.to_screen(DVec2::ZERO) {
        painter.circle_stroke(o, 3.0, Stroke::new(1.2, theme::TEXT_MUTED));
    }
}

fn draw_snap(painter: &egui::Painter, view: &View<'_>, s: &Snap) {
    for g in &s.guides {
        if let (Some(a), Some(b)) = (view.to_screen(g.from), view.to_screen(g.to)) {
            painter.extend(Shape::dashed_line(
                &[a, b],
                Stroke::new(1.0, theme::TEXT_MUTED),
                4.0,
                4.0,
            ));
        }
    }
    let Some(p) = view.to_screen(s.point) else { return };
    let stroke = Stroke::new(1.5, theme::ACCENT);
    let r = 5.0;
    match s.kind {
        SnapKind::Endpoint | SnapKind::Origin => {
            painter.rect_stroke(
                Rect::from_center_size(p, vec2(2.0 * r, 2.0 * r)),
                0.0,
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        SnapKind::Center => {
            painter.circle_stroke(p, r, stroke);
            painter.circle_filled(p, 1.5, theme::ACCENT);
        }
        SnapKind::Midpoint => {
            painter.add(Shape::closed_line(
                vec![p + vec2(0.0, -r), p + vec2(r, r * 0.8), p + vec2(-r, r * 0.8)],
                stroke,
            ));
        }
        SnapKind::Quadrant => {
            painter.add(Shape::closed_line(
                vec![p + vec2(0.0, -r), p + vec2(r, 0.0), p + vec2(0.0, r), p + vec2(-r, 0.0)],
                stroke,
            ));
        }
        SnapKind::Intersection => {
            painter.line_segment([p + vec2(-r, -r), p + vec2(r, r)], stroke);
            painter.line_segment([p + vec2(-r, r), p + vec2(r, -r)], stroke);
        }
        SnapKind::Grid => {
            painter.line_segment([p + vec2(-r, 0.0), p + vec2(r, 0.0)], stroke);
            painter.line_segment([p + vec2(0.0, -r), p + vec2(0.0, r)], stroke);
        }
        SnapKind::OnCurve => {
            painter.circle_stroke(p, 3.0, stroke);
        }
        SnapKind::Aligned | SnapKind::Free => {}
    }
}

// ---------------------------------------------------------------- plane pick

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanePick {
    Nothing,
    Picked(PlaneRef),
    Cancel,
}

/// A plane the user can pick: reference, frame and label.
#[derive(Debug, Clone)]
pub struct PickablePlane {
    pub plane: PlaneRef,
    pub frame: PlaneFrame,
    pub label: String,
}

/// The origin planes plus all construction planes of `doc`.
pub fn pickable_planes(doc: &qoob3d_core::document::Document) -> Vec<PickablePlane> {
    let mut out: Vec<PickablePlane> = OriginPlane::ALL
        .iter()
        .map(|&p| PickablePlane {
            plane: PlaneRef::Origin(p),
            frame: p.frame(),
            label: plane_label(p).to_string(),
        })
        .collect();
    for (id, _) in doc.planes() {
        let plane = PlaneRef::Construction(id);
        if let (Some(frame), Some(f)) = (doc.plane_frame(plane), doc.feature(id)) {
            out.push(PickablePlane { plane, frame, label: f.name.clone() });
        }
    }
    out
}

/// Plane selection when creating a sketch (SK-LC-01, SK-PL-04): planes are
/// shown as squares; hover highlights, click picks.
pub fn plane_pick(
    ui: &egui::Ui,
    response: &Response,
    camera: &Camera,
    rect: Rect,
    planes: &[PickablePlane],
) -> PlanePick {
    let clicked = response.clicked();
    let pointer = response.hover_pos();
    let escape = ui.input(|i| i.key_pressed(Key::Escape));
    let a = f64::from(camera.half_height_at_target()) * 0.45;
    let hovered = pointer.and_then(|p| plane_under(camera, rect, p, a, planes));
    let painter = ui.painter_at(rect);
    for pick in planes {
        let plane = pick.plane;
        let view = View { camera, rect, frame: pick.frame };
        let corners: Vec<Pos2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .iter()
            .filter_map(|&(x, y)| view.to_screen(DVec2::new(x * a, y * a)))
            .collect();
        if corners.len() < 4 {
            continue;
        }
        let hot = hovered == Some(plane);
        let fill =
            if hot { theme::ACCENT.gamma_multiply(0.28) } else { Color32::from_white_alpha(14) };
        let stroke = Stroke::new(
            if hot { 1.6 } else { 1.0 },
            if hot { theme::ACCENT } else { Color32::from_gray(150) },
        );
        painter.add(Shape::convex_polygon(corners.clone(), fill, stroke));
        painter.text(
            corners[2],
            Align2::LEFT_BOTTOM,
            &pick.label,
            FontId::proportional(12.0),
            if hot { theme::ACCENT } else { theme::TEXT_MUTED },
        );
    }
    painter.text(
        rect.center_top() + vec2(0.0, 40.0),
        Align2::CENTER_TOP,
        tr("hint-pick-plane"),
        FontId::proportional(14.0),
        theme::TEXT,
    );
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if escape {
        PlanePick::Cancel
    } else if clicked {
        hovered.map_or(PlanePick::Nothing, PlanePick::Picked)
    } else {
        PlanePick::Nothing
    }
}

pub fn plane_label(plane: OriginPlane) -> &'static str {
    match plane {
        OriginPlane::XY => "XY",
        OriginPlane::XZ => "XZ",
        OriginPlane::YZ => "YZ",
    }
}

/// The nearest plane square (half size `a`) under `p`.
pub fn plane_under(
    camera: &Camera,
    rect: Rect,
    p: Pos2,
    a: f64,
    planes: &[PickablePlane],
) -> Option<PlaneRef> {
    let aspect = rect.width() / rect.height().max(1.0);
    let ray = camera.ray(to_ndc(p, rect), aspect);
    let o = ray.origin.as_dvec3();
    let d = ray.dir.as_dvec3();
    planes
        .iter()
        .filter_map(|pick| {
            let f = pick.frame;
            let hit = f.ray_hit(o.to_array(), d.to_array())?;
            (hit[0].abs() <= a && hit[1].abs() <= a).then(|| {
                let w = glam::DVec3::from(f.to_world(hit));
                (pick.plane, w.distance(o))
            })
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(plane, _)| plane)
}

/// Screen position of a plane's centre (tests, smoke test).
pub fn plane_center(camera: &Camera, rect: Rect, plane: OriginPlane) -> Option<Pos2> {
    let a = f64::from(camera.half_height_at_target()) * 0.45;
    let view = View { camera, rect, frame: plane.frame() };
    // A spot in the part of each square that no other square covers in the
    // default (iso, front-right-top) view: in front of XZ, right of YZ, above XY.
    let offset = match plane {
        OriginPlane::XY => DVec2::new(0.5, -0.5),
        OriginPlane::XZ => DVec2::new(0.5, 0.5),
        OriginPlane::YZ => DVec2::new(-0.5, 0.5),
    };
    view.to_screen(offset * a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;
    use qoob3d_render::ViewPreset;

    fn view_for(camera: &Camera, plane: OriginPlane) -> View<'_> {
        View {
            camera,
            rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)),
            frame: plane.frame(),
        }
    }

    #[test]
    fn screen_to_sketch_round_trip_on_every_plane() {
        for (plane, preset) in [
            (OriginPlane::XY, ViewPreset::Iso),
            (OriginPlane::XZ, ViewPreset::Front),
            (OriginPlane::YZ, ViewPreset::Iso),
        ] {
            let mut camera = Camera::default();
            camera.look_from(preset.eye_direction());
            let view = view_for(&camera, plane);
            let p = DVec2::new(12.5, -7.25);
            let s = view.to_screen(p).unwrap();
            let back = view.to_local(s).unwrap();
            assert!(back.distance(p) < 1e-3, "{plane:?}: {back:?}");
            assert!(view.units_per_px(s) > 0.0);
        }
    }

    #[test]
    fn edge_on_plane_gives_no_input() {
        let mut camera = Camera::default();
        camera.look_from(ViewPreset::Front.eye_direction());
        // Looking along −Y at the XY plane: edge-on.
        let view = view_for(&camera, OriginPlane::XY);
        assert!(view.to_local(pos2(400.0, 300.0)).is_none());
    }

    #[test]
    fn plane_centres_pick_their_plane() {
        let camera = Camera::default(); // iso
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        let a = f64::from(camera.half_height_at_target()) * 0.45;
        let planes = pickable_planes(&qoob3d_core::document::Document::default());
        for plane in OriginPlane::ALL {
            let p = plane_center(&camera, rect, plane).unwrap();
            assert_eq!(
                plane_under(&camera, rect, p, a, &planes),
                Some(PlaneRef::Origin(plane)),
                "{plane:?}"
            );
        }
    }
}
