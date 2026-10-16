//! Application chrome: toolbar (top), browser (left), timeline (bottom),
//! sketch palette. Panels only *report* user intent as [`UiAction`]s; the
//! app applies them. Layout per docs/design/theme.md, texts per ADR-0020.

use eframe::egui::{self, Align, Layout, PointerButton, RichText};
use qoob3d_core::document::{Document, FeatureId, FeatureKind, OriginPlane, PlaneRef};
use qoob3d_kernel::BodyResult;
use qoob3d_sketch::tools::ToolKind;

use crate::commands::{CommandKind, icon};
use crate::i18n::{self, Language, decimal, tr, tr_args};
use crate::navigation::double_clicked_by;
use crate::settings::Settings;
use crate::sketch_ui::plane_label;
use crate::theme;

/// Something the user asked for in a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiAction {
    NewDocument,
    Open,
    Save,
    Undo,
    Redo,
    CreateSketch,
    /// Start a sketch directly on this plane (SK-LC-02).
    SketchOnPlane(PlaneRef),
    /// Open a feature dialog (Extrude, Revolve …).
    Command(CommandKind),
    Export(ExportFormat),
    Tool(ToolKind),
    ToggleConstruction,
    FinishSketch,
    LookAt,
    EditFeature(FeatureId),
    DeleteFeature(FeatureId),
    ToggleVisible(FeatureId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportFormat {
    Stl,
    ThreeMf,
}

/// The state panels need to render.
pub(crate) struct PanelState<'a> {
    pub doc: &'a Document,
    pub sketching: Option<FeatureId>,
    pub picking_plane: bool,
    pub active_tool: Option<ToolKind>,
    pub construction: bool,
    pub undo: Option<&'static str>,
    pub redo: Option<&'static str>,
    pub has_demo_body: bool,
    pub bodies: &'a [BodyResult],
    /// Open feature dialog: kind and the feature it edits.
    pub command: Option<(CommandKind, Option<FeatureId>)>,
    pub feature_error: &'a dyn Fn(FeatureId) -> Option<String>,
}

/// A toolbar group: caption below a row of buttons (Fusion ribbon style).
fn group(ui: &mut egui::Ui, caption_key: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.horizontal(add);
        ui.label(theme::caption(&tr(caption_key)));
    });
}

fn planned(ui: &mut egui::Ui, key: &str, milestone: &str) {
    ui.add_enabled(false, egui::Button::new(tr(key)))
        .on_disabled_hover_text(tr_args("coming-in", &[("milestone", milestone.into())]));
}

fn button(ui: &mut egui::Ui, key: &str, tip_key: &str, out: &mut Vec<UiAction>, action: UiAction) {
    if ui.button(tr(key)).on_hover_text(tr(tip_key)).clicked() {
        out.push(action);
    }
}

fn history_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: Option<&'static str>,
    tip_key: &str,
    out: &mut Vec<UiAction>,
    action: UiAction,
) {
    let response = ui.add_enabled(label.is_some(), egui::Button::new(icon));
    let response = match label {
        Some(l) => response.on_hover_text(tr_args(tip_key, &[("action", tr(l).into())])),
        None => response,
    };
    if response.clicked() {
        out.push(action);
    }
}

pub(crate) fn toolbar(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    s: &PanelState<'_>,
) -> Vec<UiAction> {
    let mut out = Vec::new();
    ui.horizontal(|ui| {
        ui.label(RichText::new("QOOB3D").strong().size(17.0).color(theme::ACCENT));
        ui.add_space(6.0);
        ui.label(theme::caption(&tr("workspace-design")));
        ui.separator();
        group(ui, "group-file", |ui| {
            button(ui, "file-new", "file-new-tip", &mut out, UiAction::NewDocument);
            button(ui, "file-open", "file-open-tip", &mut out, UiAction::Open);
            button(ui, "file-save", "file-save-tip", &mut out, UiAction::Save);
            history_button(ui, "⟲", s.undo, "undo-tip", &mut out, UiAction::Undo);
            history_button(ui, "⟳", s.redo, "redo-tip", &mut out, UiAction::Redo);
        });
        ui.separator();
        if s.sketching.is_some() {
            group(ui, "group-sketch", |ui| {
                for (kind, key, tip) in [
                    (ToolKind::Line, "tool-line", "tool-line-tip"),
                    (ToolKind::Rectangle, "tool-rectangle", "tool-rectangle-tip"),
                    (
                        ToolKind::CenterRectangle,
                        "tool-center-rectangle",
                        "tool-center-rectangle-tip",
                    ),
                    (ToolKind::Circle, "tool-circle", "tool-circle-tip"),
                    (ToolKind::Arc, "tool-arc", "tool-arc-tip"),
                    (ToolKind::Point, "tool-point", "tool-point-tip"),
                ] {
                    let active = s.active_tool == Some(kind);
                    if ui.selectable_label(active, tr(key)).on_hover_text(tr(tip)).clicked() {
                        out.push(UiAction::Tool(kind));
                    }
                }
                if ui
                    .selectable_label(s.construction, tr("tool-construction"))
                    .on_hover_text(tr("tool-construction-tip"))
                    .clicked()
                {
                    out.push(UiAction::ToggleConstruction);
                }
            });
            ui.separator();
            let finish = egui::Button::new(
                RichText::new(tr("sketch-finish")).strong().color(theme::PANEL_BG_DARK),
            )
            .fill(theme::ACCENT);
            if ui.add(finish).on_hover_text(tr("sketch-finish-tip")).clicked() {
                out.push(UiAction::FinishSketch);
            }
        } else {
            group(ui, "group-sketch", |ui| {
                if ui
                    .selectable_label(s.picking_plane, tr("sketch-create"))
                    .on_hover_text(tr("sketch-create-tip"))
                    .clicked()
                {
                    out.push(UiAction::CreateSketch);
                }
            });
            ui.separator();
            group(ui, "group-create", |ui| {
                for kind in [
                    CommandKind::Extrude,
                    CommandKind::Revolve,
                    CommandKind::Loft,
                    CommandKind::Gear,
                    CommandKind::Coil,
                ] {
                    command_button(ui, s, kind, &mut out);
                }
            });
            ui.separator();
            group(ui, "group-construct", |ui| command_button(ui, s, CommandKind::Plane, &mut out));
            ui.separator();
            group(ui, "group-export", |ui| {
                let has = !s.bodies.is_empty();
                for (key, tip, format) in [
                    ("export-stl", "export-stl-tip", ExportFormat::Stl),
                    ("export-3mf", "export-3mf-tip", ExportFormat::ThreeMf),
                ] {
                    if ui
                        .add_enabled(has, egui::Button::new(tr(key)))
                        .on_hover_text(tr(tip))
                        .on_disabled_hover_text(tr("export-nothing"))
                        .clicked()
                    {
                        out.push(UiAction::Export(format));
                    }
                }
                planned(ui, "export-step", "M6");
            });
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.menu_button(tr("settings"), |ui| settings_menu(ui, settings));
        });
    });
    out
}

fn command_button(
    ui: &mut egui::Ui,
    s: &PanelState<'_>,
    kind: CommandKind,
    out: &mut Vec<UiAction>,
) {
    let id = kind.kind_id();
    let active = s.command.is_some_and(|(k, _)| k == kind);
    let label = format!("{} {}", icon(id), kind.title());
    if ui.selectable_label(active, label).on_hover_text(tr(kind.keys().1)).clicked() {
        out.push(UiAction::Command(kind));
    }
}

fn settings_menu(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.label(theme::caption(&tr("settings-language")));
    for lang in Language::ALL {
        if ui.radio_value(&mut settings.language, lang, lang.native_name()).changed() {
            i18n::set_language(lang);
        }
    }
    ui.separator();
    ui.checkbox(&mut settings.left_drag_orbit, tr("settings-left-drag-orbit"))
        .on_hover_text(tr("settings-left-drag-orbit-tip"));
    ui.checkbox(&mut settings.sketch_look_at, tr("settings-sketch-look-at"));

    // "My printers" (ADR-0023): pinned on top of the printer list.
    ui.separator();
    ui.label(theme::caption(&tr("settings-printers")));
    ui.checkbox(&mut settings.only_favorite_printers, tr("settings-only-favorites"));
    if settings.favorite_printers.is_empty() {
        ui.label(RichText::new(tr("settings-printers-none")).color(theme::TEXT_MUTED));
    }
    let mut remove = None;
    for id in &settings.favorite_printers {
        let Some(profile) = qoob3d_core::PrinterProfile::by_id(id) else { continue };
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new("×").frame(false))
                .on_hover_text(tr("printer-favorite-remove"))
                .clicked()
            {
                remove = Some(id.clone());
            }
            ui.label(profile.name);
        });
    }
    if let Some(id) = remove {
        settings.favorite_printers.retain(|f| *f != id);
    }
    ui.label(RichText::new(tr("settings-printers-hint")).color(theme::TEXT_MUTED).small());
}

pub(crate) fn browser(ui: &mut egui::Ui, s: &PanelState<'_>, title: &str) -> Vec<UiAction> {
    let mut out = Vec::new();
    ui.add_space(4.0);
    ui.label(theme::caption(&tr("browser")));
    ui.add_space(2.0);
    egui::CollapsingHeader::new(format!("📄 {title}")).id_salt("doc").default_open(true).show(
        ui,
        |ui| {
            egui::CollapsingHeader::new(tr("browser-origin"))
                .id_salt("origin")
                .default_open(false)
                .open(s.picking_plane.then_some(true))
                .show(ui, |ui| {
                    for plane in OriginPlane::ALL {
                        let name = tr_args("origin-plane", &[("plane", plane_label(plane).into())]);
                        if ui
                            .add(egui::Button::new(format!("▫ {name}")).frame(false))
                            .on_hover_text(tr("origin-plane-tip"))
                            .clicked()
                        {
                            out.push(UiAction::SketchOnPlane(PlaneRef::Origin(plane)));
                        }
                    }
                    for (id, _) in s.doc.planes() {
                        let Some(f) = s.doc.feature(id) else { continue };
                        if ui
                            .add(
                                egui::Button::new(format!("{} {}", icon("plane"), f.name))
                                    .frame(false),
                            )
                            .on_hover_text(tr("origin-plane-tip"))
                            .clicked()
                        {
                            out.push(UiAction::SketchOnPlane(PlaneRef::Construction(id)));
                        }
                    }
                    for key in ["origin-x", "origin-y", "origin-z"] {
                        ui.label(format!("▫ {}", tr(key)));
                    }
                });
            egui::CollapsingHeader::new(tr("browser-bodies"))
                .id_salt("bodies")
                .default_open(true)
                .show(ui, |ui| {
                    if s.has_demo_body {
                        ui.label(format!("◼ {}", tr("body-demo")))
                            .on_hover_text(tr("body-demo-tip"));
                    } else if s.bodies.is_empty() {
                        ui.label(RichText::new(tr("browser-none")).color(theme::TEXT_MUTED));
                    }
                    for b in s.bodies {
                        let name = tr_args("body-name", &[("n", b.number.into())]);
                        let volume = decimal(b.volume_mm3 / 1000.0, 2);
                        let text = format!("◼ {name}  ·  {volume} cm³");
                        let color = if b.valid { theme::TEXT } else { theme::ERROR };
                        ui.label(RichText::new(text).color(color));
                    }
                });
            egui::CollapsingHeader::new(tr("browser-sketches"))
                .id_salt("sketches")
                .default_open(true)
                .show(ui, |ui| {
                    if s.doc.sketch_count() == 0 {
                        ui.label(RichText::new(tr("browser-none")).color(theme::TEXT_MUTED));
                    }
                    for f in &s.doc.features {
                        let FeatureKind::Sketch(sketch) = &f.kind else { continue };
                        ui.horizontal(|ui| {
                            let eye = if sketch.visible { "👁" } else { "○" };
                            if ui
                                .add(egui::Button::new(eye).frame(false))
                                .on_hover_text(tr("browser-toggle-visibility"))
                                .clicked()
                            {
                                out.push(UiAction::ToggleVisible(f.id));
                            }
                            let active = s.sketching == Some(f.id);
                            let label = ui.selectable_label(active, format!("✏ {}", f.name));
                            // Already editing: a real triple click must not restart it.
                            if !active && double_clicked_by(&label, PointerButton::Primary) {
                                out.push(UiAction::EditFeature(f.id));
                            }
                            label.on_hover_text(tr("feature-edit-tip"));
                        });
                    }
                });
        },
    );
    out
}

pub(crate) fn timeline(ui: &mut egui::Ui, s: &PanelState<'_>) -> Vec<UiAction> {
    let mut out = Vec::new();
    ui.horizontal_centered(|ui| {
        for (icon, key) in [
            ("⏮", "timeline-start"),
            ("⏴", "timeline-back"),
            ("⏵", "timeline-forward"),
            ("⏭", "timeline-end"),
        ] {
            ui.add_enabled(false, egui::Button::new(icon)).on_disabled_hover_text(tr(key));
        }
        ui.separator();
        if s.doc.features.is_empty() {
            ui.label(RichText::new(tr("timeline-placeholder")).color(theme::TEXT_MUTED));
        }
        for f in &s.doc.features {
            let active = s.sketching == Some(f.id)
                || s.command.is_some_and(|(_, editing)| editing == Some(f.id));
            let error = (s.feature_error)(f.id);
            let mut text = RichText::new(format!("{} {}", icon(f.kind.kind_id()), f.name));
            if error.is_some() {
                text = text.color(theme::ERROR);
            }
            let chip = ui.selectable_label(active, text);
            // Already editing: a real triple click must not restart it.
            if !active && double_clicked_by(&chip, PointerButton::Primary) {
                out.push(UiAction::EditFeature(f.id));
            }
            chip.context_menu(|ui| {
                if ui.button(tr("feature-edit")).clicked() {
                    out.push(UiAction::EditFeature(f.id));
                    ui.close();
                }
                if ui.button(tr("feature-delete")).clicked() {
                    out.push(UiAction::DeleteFeature(f.id));
                    ui.close();
                }
            });
            match &error {
                Some(e) => chip.on_hover_text(e.as_str()),
                None => chip.on_hover_text(tr("feature-edit-tip")),
            };
        }
    });
    out
}

/// Sketch palette (SK-UI-04), floating at the right edge of the viewport.
pub(crate) fn sketch_palette(
    ctx: &egui::Context,
    viewport: egui::Rect,
    settings: &mut Settings,
    construction: bool,
) -> Vec<UiAction> {
    let mut out = Vec::new();
    egui::Area::new(egui::Id::new("qoob3d-sketch-palette"))
        .fixed_pos(viewport.right_top() + egui::vec2(-16.0, 170.0))
        .pivot(egui::Align2::RIGHT_TOP)
        .constrain_to(viewport)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::OVERLAY_BG)
                .stroke(egui::Stroke::new(1.0, theme::STROKE))
                .corner_radius(6)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(190.0);
                    ui.label(theme::caption(&tr("palette")));
                    if ui.selectable_label(construction, tr("tool-construction")).clicked() {
                        out.push(UiAction::ToggleConstruction);
                    }
                    if ui.button(tr("palette-look-at")).clicked() {
                        out.push(UiAction::LookAt);
                    }
                    ui.checkbox(&mut settings.show_grid, tr("nav-grid"));
                    ui.checkbox(&mut settings.sketch_snap, tr("palette-snap"))
                        .on_hover_text(tr("palette-snap-tip"));
                    ui.checkbox(&mut settings.sketch_grid_snap, tr("palette-grid-snap"));
                    ui.checkbox(&mut settings.show_profiles, tr("palette-profiles"));
                    ui.checkbox(&mut settings.show_points, tr("palette-points"));
                    ui.separator();
                    let finish = egui::Button::new(
                        RichText::new(tr("sketch-finish")).strong().color(theme::PANEL_BG_DARK),
                    )
                    .fill(theme::ACCENT);
                    if ui.add(finish).clicked() {
                        out.push(UiAction::FinishSketch);
                    }
                });
        });
    out
}
