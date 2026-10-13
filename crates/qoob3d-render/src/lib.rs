//! QOOB3D viewport renderer (ADR-0007).
//!
//! Renders the 3D scene into an offscreen texture (4x MSAA, reverse-Z depth)
//! that the UI displays as an image. wgpu is used exclusively through
//! `eframe::egui_wgpu::wgpu` so its version always matches egui's.

pub mod camera;
pub mod headless;
mod renderer;

pub use camera::{Camera, ViewPreset};
pub use eframe::egui_wgpu::wgpu;
pub use renderer::{COLOR_FORMAT, ViewportRenderer};
