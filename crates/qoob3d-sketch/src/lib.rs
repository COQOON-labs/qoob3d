//! QOOB3D sketch logic (ADR-0036): drawing tools as state machines,
//! snapping/inference, hit tests and editing – pure, without UI, GPU or
//! kernel, so everything is unit-testable (SK-QA-01). Requirements:
//! `docs/requirements/sketches.md`.

pub mod edit;
pub mod geom;
pub mod pick;
pub mod snap;
pub mod tools;

pub use glam::DVec2;
