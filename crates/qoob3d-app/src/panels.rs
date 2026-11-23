//! Application chrome: toolbar (top), browser (left) and the empty state.
//! Panels only *report* user intent as [`UiAction`]s; the app applies them.
//! Layout per docs/design/theme.md, texts per ADR-0020.

use eframe::egui::{self, Align, Layout, RichText};

use crate::client::PartState;
use crate::i18n::{self, Language, tr};
use crate::live::Live;
use crate::settings::Settings;
use crate::theme;

/// Something the user asked for in a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiAction {
    /// Pick a project folder and show it (WA-OP-04).
    OpenProject,
    /// Show/hide a part of the open script project (index into `Live::parts`).
    TogglePart(usize),
}

/// A toolbar group: caption below a row of buttons (Fusion ribbon style).
fn group(ui: &mut egui::Ui, caption_key: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.horizontal(add);
        ui.label(theme::caption(&tr(caption_key)));
    });
}

pub(crate) fn toolbar(ui: &mut egui::Ui, settings: &mut Settings) -> Vec<UiAction> {
    let mut out = Vec::new();
    ui.horizontal(|ui| {
        ui.label(RichText::new("QOOB3D").strong().size(17.0).color(theme::ACCENT));
        ui.add_space(6.0);
        ui.label(theme::caption(&tr("workspace-design")));
        ui.separator();
        group(ui, "group-project", |ui| {
            if ui.button(tr("project-open")).on_hover_text(tr("project-open-tip")).clicked() {
                out.push(UiAction::OpenProject);
            }
        });

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.menu_button(tr("settings"), |ui| settings_menu(ui, settings));
        });
    });
    out
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

/// The parts of the open project, or a hint that none is open.
pub(crate) fn browser(ui: &mut egui::Ui, live: Option<&Live>) -> Vec<UiAction> {
    let mut out = Vec::new();
    ui.add_space(4.0);
    ui.label(theme::caption(&tr("browser")));
    ui.add_space(2.0);
    match live {
        Some(live) => project_parts(ui, live, &mut out),
        None => {
            ui.label(RichText::new(tr("empty-title")).color(theme::TEXT_MUTED));
        }
    }
    out
}

/// Parts of the script project with state and eye (WA-LV-04/05).
fn project_parts(ui: &mut egui::Ui, live: &Live, out: &mut Vec<UiAction>) {
    egui::CollapsingHeader::new(tr("browser-parts")).id_salt("parts").default_open(true).show(
        ui,
        |ui| {
            if !live.reachable() {
                ui.label(RichText::new(tr("live-unreachable")).color(theme::ERROR))
                    .on_hover_text(tr("live-unreachable-tip"));
            }
            if live.parts().is_empty() {
                ui.label(RichText::new(tr("browser-none")).color(theme::TEXT_MUTED));
            }
            for (index, part) in live.parts().iter().enumerate() {
                ui.horizontal(|ui| {
                    let eye = if part.visible { "👁" } else { "○" };
                    if ui
                        .add(egui::Button::new(eye).frame(false))
                        .on_hover_text(tr("browser-toggle-visibility"))
                        .clicked()
                    {
                        out.push(UiAction::TogglePart(index));
                    }
                    let state = tr(match part.state {
                        PartState::Current => "part-state-current",
                        PartState::Outdated => "part-state-outdated",
                        PartState::Failed => "part-state-failed",
                        PartState::NeverBuilt => "part-state-never-built",
                    });
                    let color = match part.state {
                        PartState::Failed => theme::ERROR,
                        PartState::Current => theme::TEXT,
                        PartState::Outdated | PartState::NeverBuilt => theme::TEXT_MUTED,
                    };
                    ui.label(RichText::new(format!("◼ {}  ·  {state}", part.name)).color(color))
                        .on_hover_text(&part.file);
                });
            }
        },
    );
}

/// What the viewport says while no project is open (WA-OP-04): what a
/// project is and the way to open one.
pub(crate) fn empty_state(ctx: &egui::Context, viewport: egui::Rect) -> Vec<UiAction> {
    let mut out = Vec::new();
    egui::Area::new(egui::Id::new("qoob3d-empty-state"))
        .fixed_pos(viewport.center())
        .pivot(egui::Align2::CENTER_CENTER)
        .constrain_to(viewport)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::OVERLAY_BG)
                .stroke(egui::Stroke::new(1.0, theme::STROKE))
                .corner_radius(6)
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_max_width(360.0);
                    ui.vertical_centered(|ui| {
                        ui.heading(tr("empty-title"));
                        ui.add_space(4.0);
                        ui.label(RichText::new(tr("empty-text")).color(theme::TEXT_MUTED));
                        ui.add_space(10.0);
                        let open = egui::Button::new(
                            RichText::new(tr("empty-open")).strong().color(theme::PANEL_BG_DARK),
                        )
                        .fill(theme::ACCENT);
                        if ui.add(open).clicked() {
                            out.push(UiAction::OpenProject);
                        }
                    });
                });
        });
    out
}
