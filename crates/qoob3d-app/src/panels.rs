//! Application chrome: toolbar (top), browser (left), timeline (bottom).
//! Layout per docs/design/theme.md. Features of later milestones are shown
//! disabled with a hint, so the layout is final from M2 on. All texts are
//! translated (ADR-0020).

use eframe::egui::{self, Align, Layout, RichText};

use crate::i18n::{self, Language, tr, tr_args};
use crate::settings::Settings;
use crate::theme;

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

pub fn toolbar(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("QOOB3D").strong().size(17.0).color(theme::ACCENT));
        ui.add_space(6.0);
        ui.label(theme::caption(&tr("workspace-design")));
        ui.separator();
        group(ui, "group-file", |ui| {
            planned(ui, "file-open", "M3");
            planned(ui, "file-save", "M3");
        });
        ui.separator();
        group(ui, "group-sketch", |ui| planned(ui, "sketch-create", "M3"));
        ui.separator();
        group(ui, "group-create", |ui| planned(ui, "create-extrude", "M4"));
        ui.separator();
        group(ui, "group-export", |ui| {
            planned(ui, "export-stl", "M6");
            planned(ui, "export-3mf", "M6");
            planned(ui, "export-step", "M6");
        });

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.menu_button(tr("settings"), |ui| settings_menu(ui, settings));
        });
    });
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
                .add(egui::Button::new("✕").frame(false))
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

/// What the browser shows about the current model.
pub struct BrowserInfo {
    pub has_body: bool,
}

pub fn browser(ui: &mut egui::Ui, info: &BrowserInfo) {
    ui.add_space(4.0);
    ui.label(theme::caption(&tr("browser")));
    ui.add_space(2.0);
    egui::CollapsingHeader::new(tr("browser-untitled")).id_salt("doc").default_open(true).show(
        ui,
        |ui| {
            egui::CollapsingHeader::new(tr("browser-origin"))
                .id_salt("origin")
                .default_open(false)
                .show(ui, |ui| {
                    for key in
                        ["origin-xy", "origin-xz", "origin-yz", "origin-x", "origin-y", "origin-z"]
                    {
                        ui.label(format!("▫ {}", tr(key)));
                    }
                });
            egui::CollapsingHeader::new(tr("browser-bodies"))
                .id_salt("bodies")
                .default_open(true)
                .show(ui, |ui| {
                    if info.has_body {
                        ui.label(format!("◼ {}", tr("body-demo")))
                            .on_hover_text(tr("body-demo-tip"));
                    } else {
                        ui.label(RichText::new(tr("browser-none")).color(theme::TEXT_MUTED));
                    }
                });
            egui::CollapsingHeader::new(tr("browser-sketches"))
                .id_salt("sketches")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(RichText::new(tr("browser-sketches-later")).color(theme::TEXT_MUTED));
                });
        },
    );
}

pub fn timeline(ui: &mut egui::Ui) {
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
        ui.label(RichText::new(tr("timeline-placeholder")).color(theme::TEXT_MUTED));
    });
}
