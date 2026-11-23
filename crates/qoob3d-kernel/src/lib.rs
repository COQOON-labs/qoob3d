//! QOOB3D geometry kernel (ADR-0004, ADR-0053): the script API on
//! OpenCascade.
//!
//! OCCT shapes are not `Send`: a script runs, builds and measures its shapes
//! on the thread that called [`script::run`]. The application runs scripts in
//! worker processes, so a crash or an endless script never takes the service
//! or the app down. No OCCT type crosses this crate's API.

mod occt;
pub mod script;
mod tessellate;

/// Errors reported by the kernel, as plain data.
#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum KernelError {
    #[error("tessellation failed: {0}")]
    Tessellation(String),
    /// An OCCT exception, caught in our C++ wrappers (ADR-0004).
    #[error("OpenCascade: {0}")]
    Occt(String),
    /// Reading or writing a file failed.
    #[error("file: {0}")]
    Io(String),
}

/// Chordal deviation (mm) for meshes outside a project, such as the app's demo
/// body. Project parts use the `[mesh]` settings of `qoob3d.toml` (ADR-0034).
pub const VIEW_TESSELLATION_TOLERANCE_MM: f64 = 0.05;
