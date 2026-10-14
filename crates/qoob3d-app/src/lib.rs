//! QOOB3D desktop application. Library part of the `qoob3d` binary, so UI
//! tests (egui_kittest, ADR-0018) can drive the real app.

pub mod app;
pub mod batch;
pub mod cli;
pub mod commands;
pub mod headless_render;
mod history;
pub mod i18n;
mod navigation;
mod panels;
pub mod project;
pub mod settings;
pub mod sketch_ui;
pub mod smoke;
mod theme;
pub mod viewcube;
pub mod viewport;

pub use app::{Mode, QoobApp, StartOptions};
