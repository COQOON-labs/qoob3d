//! Feature commands (ADR-0031, ADR-0035): Extrude, Revolve, Loft,
//! Construction Plane, Spur Gear, Coil. A command owns the parameters of
//! one feature, shows them in a dialog at the right edge of the viewport,
//! takes selections from the viewport (profiles, axes, points) and keeps a
//! live preview in the project (the kernel rebuilds it like any change).
//! OK commits one undo step, Cancel restores the document.

use std::collections::{BTreeSet, HashMap};

use eframe::egui::{self, Color32, CursorIcon, Key, Mesh, Pos2, Rect, Response, Shape, Stroke};
use qoob3d_core::document::{
    AxisRef, BodyOp, Coil, CoilKind, ConstructionPlane, CurveKind, Document, EntityId, Extrude,
    FeatureId, FeatureKind, Loft, LoftSection, OriginAxis, OriginPlane, PlaneRef, ProfileRef,
    Revolve, SpurGear,
};
use qoob3d_core::expr;
use qoob3d_core::profile::Profile;
use qoob3d_render::Camera;
use qoob3d_sketch::DVec2;

use crate::i18n::{self, tr, tr_args};
use crate::project::Project;
use crate::sketch_ui::{View, curve_screen, pickable_planes};
use crate::theme;

/// Pick tolerance in pixels for points and lines.
const PICK_PX: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Extrude,
    Revolve,
    Loft,
    Plane,
    Gear,
    Coil,
}

impl CommandKind {
    pub const ALL: [CommandKind; 6] = [
        CommandKind::Extrude,
        CommandKind::Revolve,
        CommandKind::Loft,
        CommandKind::Plane,
        CommandKind::Gear,
        CommandKind::Coil,
    ];

    /// `FeatureKind::kind_id` of the feature this command makes.
    pub fn kind_id(self) -> &'static str {
        match self {
            CommandKind::Extrude => "extrude",
            CommandKind::Revolve => "revolve",
            CommandKind::Loft => "loft",
            CommandKind::Plane => "plane",
            CommandKind::Gear => "gear",
            CommandKind::Coil => "coil",
        }
    }

    /// Translation keys: (title, tooltip, default feature name).
    pub fn keys(self) -> (&'static str, &'static str, &'static str) {
        match self {
            CommandKind::Extrude => {
                ("command-extrude", "command-extrude-tip", "feature-extrude-name")
            }
            CommandKind::Revolve => {
                ("command-revolve", "command-revolve-tip", "feature-revolve-name")
            }
            CommandKind::Loft => ("command-loft", "command-loft-tip", "feature-loft-name"),
            CommandKind::Plane => {
                ("command-plane-offset", "command-plane-offset-tip", "feature-plane-name")
            }
            CommandKind::Gear => ("command-gear", "command-gear-tip", "feature-gear-name"),
            CommandKind::Coil => ("command-coil", "command-coil-tip", "feature-coil-name"),
        }
    }

    pub fn title(self) -> String {
        tr(self.keys().0)
    }

    pub fn of(kind: &FeatureKind) -> Option<CommandKind> {
        Some(match kind {
            FeatureKind::Extrude(_) => CommandKind::Extrude,
            FeatureKind::Revolve(_) => CommandKind::Revolve,
            FeatureKind::Loft(_) => CommandKind::Loft,
            FeatureKind::Plane(_) => CommandKind::Plane,
            FeatureKind::SpurGear(_) => CommandKind::Gear,
            FeatureKind::Coil(_) => CommandKind::Coil,
            FeatureKind::Sketch(_) => return None,
        })
    }
}

/// Icon of a feature kind (timeline, browser, toolbar).
pub fn icon(kind_id: &str) -> &'static str {
    match kind_id {
        "sketch" => "✏",
        "plane" => "▫",
        "extrude" => "⬆",
        "revolve" => "🔄",
        "loft" => "⛶",
        "gear" => "⚙",
        "coil" => "🌀",
        _ => "•",
    }
}

/// What a viewport click selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selector {
    Profiles,
    Axis,
    /// Loft sections: profiles and sketch points, in click order.
    Sections,
    None,
}

/// Something under the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pickable {
    Profile(ProfileRef),
    Line { sketch: FeatureId, line: EntityId },
    Point { sketch: FeatureId, point: EntityId },
}

/// What the app should do after a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Running,
    /// OK: keep the feature (already in the preview), one undo step.
    Commit,
    Cancel,
}

pub struct Command {
    pub kind: CommandKind,
    /// Feature being edited; `None` for a new one.
    pub editing: Option<FeatureId>,
    /// The document when the command started.
    base: Document,
    name: String,
    pub feature: FeatureKind,
    /// Revolve axis until one is chosen.
    axis: Option<AxisRef>,
    pub selector: Selector,
    texts: HashMap<&'static str, String>,
    bad: BTreeSet<&'static str>,
    hover: Option<Pickable>,
}

impl Command {
    /// Starts a command. For a new feature with exactly one candidate
    /// profile (the newest visible sketch has one), that profile is
    /// preselected (Fusion: Extrude right after finishing a sketch).
    pub fn new(kind: CommandKind, project: &Project) -> Self {
        let doc = (**project.doc()).clone();
        let n = doc.count_of(kind.kind_id()) + 1;
        let name = tr_args(kind.keys().2, &[("n", n.into())]);
        let xy = PlaneRef::Origin(OriginPlane::XY);
        let preselect: Vec<ProfileRef> = doc
            .sketches()
            .filter(|(_, s)| s.visible)
            .last()
            .and_then(|(id, _)| match project.profiles(id) {
                [only] => Some(vec![ProfileRef { sketch: id, key: only.key.clone() }]),
                _ => None,
            })
            .unwrap_or_default();
        let feature = match kind {
            CommandKind::Extrude => FeatureKind::Extrude(Extrude {
                profiles: preselect,
                distance: 10.0,
                symmetric: false,
                operation: BodyOp::NewBody,
            }),
            CommandKind::Revolve => FeatureKind::Revolve(Revolve {
                profiles: preselect,
                axis: AxisRef::Origin(OriginAxis::X),
                angle_deg: 360.0,
                operation: BodyOp::NewBody,
            }),
            CommandKind::Loft => FeatureKind::Loft(Loft {
                sections: preselect.into_iter().map(LoftSection::Profile).collect(),
                operation: BodyOp::NewBody,
            }),
            CommandKind::Plane => FeatureKind::Plane(ConstructionPlane { base: xy, offset: 10.0 }),
            CommandKind::Gear => FeatureKind::SpurGear(SpurGear {
                plane: xy,
                module: 2.0,
                teeth: 20,
                pressure_angle_deg: 20.0,
                thickness: 8.0,
                bore: 5.0,
                operation: BodyOp::NewBody,
            }),
            CommandKind::Coil => FeatureKind::Coil(Coil {
                plane: xy,
                kind: CoilKind::Helix,
                diameter: 20.0,
                revolutions: 5.0,
                pitch: 4.0,
                section: 2.0,
                operation: BodyOp::NewBody,
            }),
        };
        let selector = match kind {
            CommandKind::Extrude | CommandKind::Revolve => Selector::Profiles,
            CommandKind::Loft => Selector::Sections,
            _ => Selector::None,
        };
        let mut cmd = Self {
            kind,
            editing: None,
            base: doc,
            name,
            feature,
            axis: None,
            selector,
            texts: HashMap::new(),
            bad: BTreeSet::new(),
            hover: None,
        };
        cmd.reset_texts();
        cmd
    }

    /// Edits an existing feature (timeline double-click).
    pub fn edit(id: FeatureId, project: &Project) -> Option<Self> {
        let doc = (**project.doc()).clone();
        let f = doc.feature(id)?;
        let kind = CommandKind::of(&f.kind)?;
        let mut cmd = Self::new(kind, project);
        cmd.editing = Some(id);
        cmd.name = f.name.clone();
        cmd.feature = f.kind.clone();
        if let FeatureKind::Revolve(r) = &f.kind {
            cmd.axis = Some(r.axis);
        }
        cmd.reset_texts();
        Some(cmd)
    }

    fn reset_texts(&mut self) {
        let dec = |v: f64| {
            i18n::decimal(v, 3).trim_end_matches('0').trim_end_matches(['.', ',']).to_string()
        };
        self.texts.clear();
        for (key, value) in self.numbers() {
            self.texts.insert(key, dec(value));
        }
    }

    /// The numeric parameters: (field key, value).
    fn numbers(&self) -> Vec<(&'static str, f64)> {
        match &self.feature {
            FeatureKind::Extrude(e) => vec![("field-distance", e.distance)],
            FeatureKind::Revolve(r) => vec![("field-angle", r.angle_deg)],
            FeatureKind::Plane(p) => vec![("field-offset", p.offset)],
            FeatureKind::SpurGear(g) => vec![
                ("field-module", g.module),
                ("field-teeth", f64::from(g.teeth)),
                ("field-pressure-angle", g.pressure_angle_deg),
                ("field-thickness", g.thickness),
                ("field-bore", g.bore),
            ],
            FeatureKind::Coil(c) => vec![
                ("field-diameter", c.diameter),
                ("field-revolutions", c.revolutions),
                ("field-pitch", c.pitch),
                ("field-section", c.section),
            ],
            FeatureKind::Loft(_) | FeatureKind::Sketch(_) => Vec::new(),
        }
    }

    fn set_number(&mut self, key: &str, v: f64) {
        match &mut self.feature {
            FeatureKind::Extrude(e) => e.distance = v,
            FeatureKind::Revolve(r) => r.angle_deg = v,
            FeatureKind::Plane(p) => p.offset = v,
            FeatureKind::SpurGear(g) => match key {
                "field-module" => g.module = v,
                "field-teeth" => g.teeth = v.round().clamp(0.0, 10_000.0) as u32,
                "field-pressure-angle" => g.pressure_angle_deg = v,
                "field-thickness" => g.thickness = v,
                _ => g.bore = v,
            },
            FeatureKind::Coil(c) => match key {
                "field-diameter" => c.diameter = v,
                "field-revolutions" => c.revolutions = v,
                "field-pitch" => c.pitch = v,
                _ => c.section = v,
            },
            FeatureKind::Loft(_) | FeatureKind::Sketch(_) => {}
        }
    }

    /// Plain numbers (no unit) for counts and angles.
    fn is_unitless(key: &str) -> bool {
        matches!(key, "field-teeth" | "field-revolutions" | "field-angle" | "field-pressure-angle")
    }

    /// The document with this feature applied, if the parameters are complete.
    pub fn preview_document(&self) -> Option<Document> {
        let mut feature = self.feature.clone();
        if let FeatureKind::Revolve(r) = &mut feature {
            r.axis = self.axis?;
        }
        let mut doc = self.base.clone();
        match self.editing {
            Some(id) => doc.feature_mut(id)?.kind = feature,
            None => {
                doc.add_feature(self.name.clone(), feature);
            }
        }
        doc.validate().ok().map(|()| doc)
    }

    /// The document to commit: like the preview, plus used sketches hidden
    /// (Fusion hides a sketch once a feature consumes it).
    pub fn final_document(&self) -> Option<Document> {
        let mut doc = self.preview_document()?;
        if self.editing.is_none() {
            let id = doc.features.last()?.id;
            let used: Vec<FeatureId> = doc.feature(id)?.kind.dependencies().into_iter().collect();
            for sketch in used {
                if let Some(s) = doc.sketch_mut(sketch) {
                    s.visible = false;
                }
            }
        }
        Some(doc)
    }

    pub fn is_complete(&self) -> bool {
        self.bad.is_empty() && self.preview_document().is_some()
    }

    pub fn history_label(&self) -> &'static str {
        if self.editing.is_some() { "history-edit-feature" } else { "history-create-feature" }
    }

    // ------------------------------------------------------------- selection

    fn profiles_mut(&mut self) -> Option<&mut Vec<ProfileRef>> {
        match &mut self.feature {
            FeatureKind::Extrude(e) => Some(&mut e.profiles),
            FeatureKind::Revolve(r) => Some(&mut r.profiles),
            _ => None,
        }
    }

    fn toggle(&mut self, pick: Pickable) {
        match (self.selector, pick) {
            (Selector::Profiles, Pickable::Profile(p)) => {
                if let Some(list) = self.profiles_mut() {
                    if let Some(i) = list.iter().position(|q| *q == p) {
                        list.remove(i);
                    } else {
                        list.push(p);
                    }
                }
                // Revolve: after the profile comes the axis (Fusion order).
                if self.kind == CommandKind::Revolve && self.axis.is_none() {
                    self.selector = Selector::Axis;
                }
            }
            (Selector::Axis, Pickable::Line { sketch, line }) => {
                self.axis = Some(AxisRef::SketchLine { sketch, line });
            }
            (Selector::Sections, pick) => {
                let section = match pick {
                    Pickable::Profile(p) => LoftSection::Profile(p),
                    Pickable::Point { sketch, point } => LoftSection::Point { sketch, point },
                    Pickable::Line { .. } => return,
                };
                if let FeatureKind::Loft(l) = &mut self.feature {
                    if let Some(i) = l.sections.iter().position(|s| *s == section) {
                        l.sections.remove(i);
                    } else {
                        l.sections.push(section);
                    }
                }
            }
            _ => {}
        }
    }

    fn selected_profiles(&self) -> Vec<&ProfileRef> {
        match &self.feature {
            FeatureKind::Extrude(e) => e.profiles.iter().collect(),
            FeatureKind::Revolve(r) => r.profiles.iter().collect(),
            FeatureKind::Loft(l) => l
                .sections
                .iter()
                .filter_map(|s| match s {
                    LoftSection::Profile(p) => Some(p),
                    LoftSection::Point { .. } => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Sketches whose geometry can be picked: visible ones and those the
    /// feature already uses.
    fn candidate_sketches(&self, doc: &Document) -> Vec<FeatureId> {
        let used = self.feature.dependencies();
        let limit = self.editing.and_then(|id| doc.position(id)).unwrap_or(doc.features.len());
        doc.features[..limit]
            .iter()
            .filter_map(|f| match &f.kind {
                FeatureKind::Sketch(s) if s.visible || used.contains(&f.id) => Some(f.id),
                _ => None,
            })
            .collect()
    }

    // ------------------------------------------------------------- viewport

    /// Hover, click selection and drawing in the viewport.
    #[expect(clippy::cognitive_complexity, reason = "predates ADR-0040; split when next changed")]
    pub fn viewport(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        camera: &Camera,
        rect: Rect,
        project: &Project,
    ) {
        let doc = &self.base;
        let clicked = response.clicked();
        let pointer = response.hover_pos();
        let painter = ui.painter_at(rect);
        let selected: Vec<ProfileRef> = self.selected_profiles().into_iter().cloned().collect();
        let mut best: Option<(f32, Pickable)> = None;

        for id in self.candidate_sketches(doc) {
            let Some(sketch) = doc.sketch(id) else { continue };
            let Some(frame) = doc.plane_frame(sketch.plane) else { continue };
            let view = View { camera, rect, frame };
            let profiles: &[Profile] = project.profiles(id);
            let local = pointer.and_then(|p| view.to_local(p));

            // Profiles (Extrude, Revolve, Loft).
            if matches!(self.selector, Selector::Profiles | Selector::Sections) {
                for profile in profiles {
                    let r = ProfileRef { sketch: id, key: profile.key.clone() };
                    let inside = local.is_some_and(|l| profile.contains(l.to_array()));
                    if inside {
                        // Prefer the smallest region (an island over its ring).
                        let score = 1000.0 + profile.area_mm2 as f32 * 1e-6;
                        if best.as_ref().is_none_or(|b| score < b.0) {
                            best = Some((score, Pickable::Profile(r.clone())));
                        }
                    }
                    let is_selected = selected.contains(&r);
                    let is_hover = self.hover == Some(Pickable::Profile(r));
                    let alpha = match (is_selected, is_hover) {
                        (true, _) => 0.45,
                        (false, true) => 0.3,
                        _ => 0.1,
                    };
                    fill_profile(&painter, &view, profile, theme::ACCENT.gamma_multiply(alpha));
                }
            }
            // Curves for orientation; lines are axis candidates.
            for c in &sketch.curves {
                let pts = curve_screen(&view, sketch, &c.kind);
                let is_axis = matches!(self.axis, Some(AxisRef::SketchLine { sketch: s, line })
                    if s == id && line == c.id);
                let hover_line = self.hover == Some(Pickable::Line { sketch: id, line: c.id });
                let color = if is_axis || hover_line {
                    Color32::WHITE
                } else if c.construction {
                    theme::CONSTRUCTION
                } else {
                    theme::ACCENT.gamma_multiply(0.8)
                };
                let width = if is_axis || hover_line { 2.4 } else { 1.2 };
                if pts.len() >= 2 {
                    painter.add(Shape::line(pts.clone(), Stroke::new(width, color)));
                }
                if self.selector == Selector::Axis
                    && matches!(c.kind, CurveKind::Line { .. })
                    && let Some(p) = pointer
                    && let [a, b] = pts[..]
                {
                    let d = distance_to_segment(p, a, b);
                    if d <= PICK_PX && best.as_ref().is_none_or(|x| d < x.0) {
                        best = Some((d, Pickable::Line { sketch: id, line: c.id }));
                    }
                }
            }
            // Points (loft tips).
            if self.selector == Selector::Sections {
                for p in &sketch.points {
                    let Some(s) = view.to_screen(DVec2::new(p.x, p.y)) else { continue };
                    let pick = Pickable::Point { sketch: id, point: p.id };
                    let chosen = matches!(&self.feature, FeatureKind::Loft(l)
                        if l.sections.contains(&LoftSection::Point { sketch: id, point: p.id }));
                    let hot = self.hover.as_ref() == Some(&pick);
                    let size = if chosen || hot { 5.0 } else { 3.0 };
                    painter.circle_filled(
                        s,
                        size,
                        if chosen { Color32::WHITE } else { theme::ACCENT },
                    );
                    if let Some(c) = pointer {
                        let d = c.distance(s);
                        if d <= PICK_PX && best.as_ref().is_none_or(|x| d < x.0) {
                            best = Some((d, pick));
                        }
                    }
                }
            }
        }
        self.hover = best.map(|(_, p)| p);
        if self.hover.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if clicked && let Some(pick) = self.hover.clone() {
            self.toggle(pick);
        }
    }

    // ------------------------------------------------------------- dialog

    /// The parameter dialog; returns what the app should do.
    pub fn dialog(
        &mut self,
        ctx: &egui::Context,
        viewport: Rect,
        doc: &Document,
    ) -> CommandOutcome {
        let mut outcome = CommandOutcome::Running;
        let decimal_comma = i18n::language().decimal_separator() == ',';
        egui::Area::new(egui::Id::new("qoob3d-command"))
            .fixed_pos(viewport.right_top() + egui::vec2(-16.0, 170.0))
            .pivot(egui::Align2::RIGHT_TOP)
            .constrain_to(viewport)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
                    .stroke(Stroke::new(1.0, theme::STROKE))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(250.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "{} {}",
                                icon(self.kind.kind_id()),
                                self.kind.title()
                            ))
                            .strong()
                            .color(theme::ACCENT),
                        );
                        ui.add_space(4.0);
                        self.selection_rows(ui, doc);
                        self.plane_row(ui, doc);
                        let numbers = self.numbers();
                        egui::Grid::new("qoob3d-command-fields").num_columns(2).show(ui, |ui| {
                            for (key, _) in numbers {
                                self.number_row(ui, key, decimal_comma);
                                ui.end_row();
                            }
                        });
                        self.option_rows(ui);
                        ui.separator();
                        ui.horizontal(|ui| {
                            let ok = ui.add_enabled(
                                self.is_complete(),
                                egui::Button::new(
                                    egui::RichText::new(tr("dialog-ok"))
                                        .strong()
                                        .color(theme::PANEL_BG_DARK),
                                )
                                .fill(theme::ACCENT),
                            );
                            if ok.clicked() {
                                outcome = CommandOutcome::Commit;
                            }
                            if ui.button(tr("dialog-cancel")).clicked() {
                                outcome = CommandOutcome::Cancel;
                            }
                        });
                        if !self.is_complete() {
                            ui.label(theme::caption(&tr(self.missing_hint())));
                        }
                    });
            });
        if !ctx.egui_wants_keyboard_input() {
            let (enter, escape) =
                ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
            if escape {
                outcome = CommandOutcome::Cancel;
            } else if enter && self.is_complete() {
                outcome = CommandOutcome::Commit;
            }
        }
        outcome
    }

    fn missing_hint(&self) -> &'static str {
        match &self.feature {
            FeatureKind::Extrude(e) if e.profiles.is_empty() => "hint-select-profile",
            FeatureKind::Revolve(r) if r.profiles.is_empty() => "hint-select-profile",
            FeatureKind::Revolve(_) if self.axis.is_none() => "hint-select-axis",
            FeatureKind::Loft(l) if l.sections.len() < 2 => "hint-select-sections",
            _ => "hint-check-values",
        }
    }

    fn selector_button(&mut self, ui: &mut egui::Ui, selector: Selector, label: String) {
        if ui.selectable_label(self.selector == selector, label).clicked() {
            self.selector = selector;
        }
    }

    fn selection_rows(&mut self, ui: &mut egui::Ui, doc: &Document) {
        match self.kind {
            CommandKind::Extrude | CommandKind::Revolve => {
                let n = self.selected_profiles().len();
                let label = tr_args("command-profiles", &[("n", n.into())]);
                self.selector_button(ui, Selector::Profiles, label);
            }
            CommandKind::Loft => {
                let n = match &self.feature {
                    FeatureKind::Loft(l) => l.sections.len(),
                    _ => 0,
                };
                let label = tr_args("command-sections", &[("n", n.into())]);
                self.selector_button(ui, Selector::Sections, label);
            }
            _ => {}
        }
        if self.kind == CommandKind::Revolve {
            let current = match self.axis {
                Some(AxisRef::Origin(a)) => axis_label(a),
                Some(AxisRef::SketchLine { sketch, .. }) => tr_args(
                    "command-axis-line",
                    &[(
                        "sketch",
                        doc.feature(sketch).map_or_else(String::new, |f| f.name.clone()).into(),
                    )],
                ),
                None => tr("command-axis-none"),
            };
            ui.horizontal(|ui| {
                let label = tr_args("command-axis", &[("axis", current.into())]);
                self.selector_button(ui, Selector::Axis, label);
                for a in OriginAxis::ALL {
                    if ui
                        .selectable_label(self.axis == Some(AxisRef::Origin(a)), axis_label(a))
                        .clicked()
                    {
                        self.axis = Some(AxisRef::Origin(a));
                    }
                }
            });
        }
    }

    fn plane_row(&mut self, ui: &mut egui::Ui, doc: &Document) {
        let own = self.editing;
        let plane = match &mut self.feature {
            FeatureKind::Plane(p) => &mut p.base,
            FeatureKind::SpurGear(g) => &mut g.plane,
            FeatureKind::Coil(c) => &mut c.plane,
            _ => return,
        };
        let planes: Vec<_> = pickable_planes(doc)
            .into_iter()
            .filter(|p| own.is_none_or(|id| p.plane != PlaneRef::Construction(id)))
            .collect();
        let current =
            planes.iter().find(|p| p.plane == *plane).map_or_else(String::new, |p| p.label.clone());
        ui.horizontal(|ui| {
            ui.label(tr("command-plane"));
            egui::ComboBox::from_id_salt("qoob3d-command-plane").selected_text(current).show_ui(
                ui,
                |ui| {
                    for p in &planes {
                        ui.selectable_value(plane, p.plane, &p.label);
                    }
                },
            );
        });
    }

    fn number_row(&mut self, ui: &mut egui::Ui, key: &'static str, decimal_comma: bool) {
        let label = ui.label(tr(key));
        let bad = self.bad.contains(key);
        let text = self.texts.entry(key).or_default();
        let edit = egui::TextEdit::singleline(text).desired_width(110.0).text_color(if bad {
            theme::ERROR
        } else {
            theme::TEXT
        });
        let response = ui.add(edit).labelled_by(label.id);
        if response.changed() {
            let value = if Self::is_unitless(key) {
                expr::eval_number(text, decimal_comma)
            } else {
                expr::eval_length_mm(text, decimal_comma)
            };
            match value {
                Ok(v) if v.is_finite() => {
                    self.bad.remove(key);
                    self.set_number(key, v);
                }
                _ => {
                    self.bad.insert(key);
                }
            }
        }
    }

    fn option_rows(&mut self, ui: &mut egui::Ui) {
        if let FeatureKind::Extrude(e) = &mut self.feature {
            ui.checkbox(&mut e.symmetric, tr("command-symmetric"));
        }
        if let FeatureKind::Coil(c) = &mut self.feature {
            ui.horizontal(|ui| {
                ui.label(tr("command-coil-type"));
                ui.selectable_value(&mut c.kind, CoilKind::Helix, tr("coil-helix"));
                ui.selectable_value(&mut c.kind, CoilKind::Spiral, tr("coil-spiral"));
            });
        }
        let op = match &mut self.feature {
            FeatureKind::Extrude(e) => &mut e.operation,
            FeatureKind::Revolve(r) => &mut r.operation,
            FeatureKind::Loft(l) => &mut l.operation,
            FeatureKind::SpurGear(g) => &mut g.operation,
            FeatureKind::Coil(c) => &mut c.operation,
            _ => return,
        };
        ui.horizontal(|ui| {
            ui.label(tr("command-operation"));
            for o in BodyOp::ALL {
                let key = match o {
                    BodyOp::NewBody => "op-new-body",
                    BodyOp::Join => "op-join",
                    BodyOp::Cut => "op-cut",
                };
                ui.selectable_value(op, o, tr(key));
            }
        });
    }
}

pub fn axis_label(a: OriginAxis) -> String {
    match a {
        OriginAxis::X => tr("origin-x"),
        OriginAxis::Y => tr("origin-y"),
        OriginAxis::Z => tr("origin-z"),
    }
}

fn fill_profile(painter: &egui::Painter, view: &View<'_>, profile: &Profile, color: Color32) {
    let screen: Vec<Option<Pos2>> =
        profile.positions.iter().map(|p| view.to_screen(DVec2::new(p[0], p[1]))).collect();
    if screen.iter().any(Option::is_none) {
        return;
    }
    let mut mesh = Mesh::default();
    for p in screen.into_iter().flatten() {
        mesh.colored_vertex(p, color);
    }
    for t in &profile.triangles {
        mesh.add_triangle(t[0], t[1], t[2]);
    }
    painter.add(Shape::mesh(mesh));
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Screen position inside a profile of a sketch (tests, smoke test): the
/// centroid of its largest triangle.
pub fn profile_screen_pos(
    doc: &Document,
    project: &Project,
    camera: &Camera,
    rect: Rect,
    sketch: FeatureId,
    index: usize,
) -> Option<Pos2> {
    let s = doc.sketch(sketch)?;
    let view = View { camera, rect, frame: doc.plane_frame(s.plane)? };
    let profile = project.profiles(sketch).get(index)?;
    let tri = profile.triangles.iter().max_by(|a, b| {
        let area = |t: &&[u32; 3]| {
            let [p, q, r] = t.map(|i| profile.positions[i as usize]);
            ((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])).abs()
        };
        area(a).total_cmp(&area(b))
    })?;
    let c = tri.iter().fold([0.0; 2], |acc, &i| {
        let p = profile.positions[i as usize];
        [acc[0] + p[0] / 3.0, acc[1] + p[1] / 3.0]
    });
    view.to_screen(DVec2::new(c[0], c[1]))
}

/// Screen position of a sketch point or a line's midpoint (tests).
pub fn entity_screen_pos(
    doc: &Document,
    camera: &Camera,
    rect: Rect,
    sketch: FeatureId,
    entity: EntityId,
) -> Option<Pos2> {
    let s = doc.sketch(sketch)?;
    let view = View { camera, rect, frame: doc.plane_frame(s.plane)? };
    let local = match s.point(entity) {
        Some(p) => [p.x, p.y],
        None => match s.curve(entity)?.kind {
            CurveKind::Line { start, end } => {
                let (a, b) = (s.pos(start), s.pos(end));
                [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]
            }
            _ => return None,
        },
    };
    view.to_screen(DVec2::new(local[0], local[1]))
}
