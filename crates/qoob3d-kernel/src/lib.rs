//! QOOB3D geometry kernel (ADR-0004, ADR-0005).
//!
//! All OpenCascade objects live exclusively on one dedicated worker thread
//! (OCCT handles are not `Send`). The rest of the application talks to it via
//! [`Kernel`] using plain-data requests and responses, so the UI never blocks
//! on geometry work and no OCCT type ever crosses this crate's API.

mod model;
mod occt;
mod profiles;
mod tessellate;
mod worker;

pub use model::{BodyId, BodyResult, ExportBody, RebuildResult, SketchResult};
pub use worker::{Kernel, Request, RequestId, Response};

/// Errors reported by the kernel. They are plain data so they can be sent
/// back to the UI thread.
#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum KernelError {
    #[error("tessellation failed: {0}")]
    Tessellation(String),
    /// An OCCT exception, caught in our C++ wrappers (ADR-0004).
    #[error("OpenCascade: {0}")]
    Occt(String),
    #[error("kernel worker thread is not running")]
    WorkerGone,
    /// A modelling problem the user can fix (missing profile, nothing to cut …).
    #[error("{0}")]
    Model(String),
}

/// Chordal deviation used for viewport meshes (mm).
pub const VIEW_TESSELLATION_TOLERANCE_MM: f64 = 0.05;
/// Chordal deviation for STL/3MF export (mm): well below an FDM nozzle.
pub const EXPORT_TESSELLATION_TOLERANCE_MM: f64 = 0.01;
