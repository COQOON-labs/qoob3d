//! Measures of built shapes for callers outside scripts (`/run`, `/measure`
//! of the service, agent.md AG-SV-05/06): the same OCCT calls as the script
//! functions `area`, `center` and `distance`.

use super::Shape;
use super::VOLUME_RELATIVE_ERROR;
use super::api::COMMON;
use crate::KernelError;
use crate::occt::ffi;
use crate::occt::{self, occ};

impl Shape {
    /// Surface area in mm² (NaN if OCCT cannot compute it).
    #[must_use]
    pub fn area(&self) -> f64 {
        occ(ffi::qoob3d_area(self.get())).unwrap_or(f64::NAN)
    }

    /// Centre of mass in mm (NaN if OCCT cannot compute it).
    #[must_use]
    pub fn center(&self) -> [f64; 3] {
        match occ(ffi::qoob3d_center(self.get())).as_deref() {
            Ok(&[x, y, z]) => [x, y, z],
            _ => [f64::NAN; 3],
        }
    }
}

/// Smallest distance between two shapes in mm; 0 if they touch or overlap.
///
/// # Errors
/// OCCT finds no solution.
pub fn distance(a: &Shape, b: &Shape) -> Result<f64, KernelError> {
    occ(ffi::qoob3d_distance(a.get(), b.get(), f64::INFINITY))
}

/// Volume both shapes share in mm³; 0 if they are disjoint or only touch.
///
/// # Errors
/// The boolean operation failed.
pub fn common_volume(a: &Shape, b: &Shape) -> Result<f64, KernelError> {
    let common = occt::shape(ffi::qoob3d_boolean(a.get(), b.get(), COMMON, 0.0, true))?;
    let volume = occ(ffi::qoob3d_volume(common.get(), VOLUME_RELATIVE_ERROR))?;
    Ok(if volume.is_finite() { volume.max(0.0) } else { 0.0 })
}
