//! QOOB3D desktop application. Library part of the `qoob3d` binary, so UI
//! tests (egui_kittest, ADR-0018) can drive the real app.

pub mod app;
pub mod cli;
pub mod headless_render;
pub mod i18n;
mod navigation;
mod panels;
pub mod settings;
pub mod smoke;
mod theme;
pub mod viewcube;
pub mod viewport;

pub use app::QoobApp;
