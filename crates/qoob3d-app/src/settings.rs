//! User settings, persisted by eframe between sessions (ADR-0020, ADR-0021,
//! ADR-0022). Unknown or missing fields fall back to the defaults.

use serde::{Deserialize, Serialize};

use crate::i18n::Language;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    /// [`qoob3d_core::PrinterProfile::id`] of the build plate shown.
    pub printer_id: String,
    /// "My printers": ids pinned to the top of the printer list, in the order added.
    pub favorite_printers: Vec<String>,
    /// Hide all printers except the favourites in the list.
    pub only_favorite_printers: bool,
    pub show_grid: bool,
    pub show_build_volume: bool,
    /// Left-button drag (and therefore macOS three-finger drag) orbits.
    pub left_drag_orbit: bool,
    /// Turn the camera to the sketch plane when a sketch starts (SK-LC-03).
    pub sketch_look_at: bool,
    /// Snapping in sketches (SK-SN-05), grid snapping, profile/point display.
    pub sketch_snap: bool,
    pub sketch_grid_snap: bool,
    pub show_profiles: bool,
    pub show_points: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::English,
            printer_id: qoob3d_core::printer::DEFAULT_PRINTER_ID.to_string(),
            favorite_printers: Vec::new(),
            only_favorite_printers: false,
            show_grid: true,
            show_build_volume: false,
            left_drag_orbit: true,
            sketch_look_at: true,
            sketch_snap: true,
            sketch_grid_snap: true,
            show_profiles: true,
            show_points: false,
        }
    }
}

impl Settings {
    pub const STORAGE_KEY: &str = "qoob3d-settings";
}

#[cfg(test)]
mod tests;
