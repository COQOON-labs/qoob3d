//! QOOB3D desktop application. Library part of the `qoob3d` binary, so UI
//! tests (egui_kittest, ADR-0013) can drive the real app.

pub mod app;
pub mod cli;
pub mod headless_render;
pub mod i18n;
pub mod live;
mod navigation;
mod panels;
pub mod project_render;
pub mod service_cli;
pub mod settings;
pub mod smoke;
mod theme;
pub mod viewcube;
pub mod viewport;

pub use app::{QoobApp, StartOptions};
