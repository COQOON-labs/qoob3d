//! QOOB3D look: dark, calm, one mint accent (docs/design/theme.md).

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme};

/// The single accent colour: selection, active tool, sketch, focus.
pub const ACCENT: Color32 = Color32::from_rgb(0x3d, 0xe0, 0xb0);
pub const PANEL_BG: Color32 = Color32::from_rgb(0x1d, 0x1f, 0x23);
pub const PANEL_BG_DARK: Color32 = Color32::from_rgb(0x17, 0x19, 0x1c);
pub const STROKE: Color32 = Color32::from_rgb(0x2c, 0x30, 0x36);
pub const TEXT: Color32 = Color32::from_rgb(0xd4, 0xd7, 0xdc);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8a, 0x90, 0x99);
/// Translucent background of overlays inside the viewport.
pub const OVERLAY_BG: Color32 = Color32::from_rgba_premultiplied(0x1d, 0x1f, 0x23, 0xe6);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        let v = &mut style.visuals;
        v.panel_fill = PANEL_BG;
        v.window_fill = PANEL_BG;
        v.extreme_bg_color = PANEL_BG_DARK;
        v.faint_bg_color = Color32::from_rgb(0x22, 0x25, 0x2a);
        v.override_text_color = Some(TEXT);
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT.linear_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.window_stroke = Stroke::new(1.0, STROKE);

        let w = &mut v.widgets;
        w.noninteractive.bg_stroke = Stroke::new(1.0, STROKE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.weak_bg_fill = Color32::from_rgb(0x26, 0x29, 0x2e);
        w.inactive.bg_fill = Color32::from_rgb(0x26, 0x29, 0x2e);
        w.hovered.weak_bg_fill = Color32::from_rgb(0x30, 0x34, 0x3a);
        w.hovered.bg_fill = Color32::from_rgb(0x30, 0x34, 0x3a);
        w.hovered.bg_stroke = Stroke::new(1.0, ACCENT.linear_multiply(0.6));
        w.active.weak_bg_fill = ACCENT.linear_multiply(0.45);
        w.active.bg_fill = ACCENT.linear_multiply(0.45);
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            state.corner_radius = CornerRadius::same(4);
        }

        // CAD UI: labels are not selectable text, so the cursor stays an
        // arrow (cursor policy, docs/design/bedienung.md).
        style.interaction.selectable_labels = false;

        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.text_styles.insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Small, FontId::new(10.5, FontFamily::Proportional));
    });
}

/// Small, letter-spaced section caption (Fusion style "SKIZZE", "BROWSER").
pub fn caption(text: &str) -> egui::RichText {
    egui::RichText::new(text).small().color(TEXT_MUTED)
}
