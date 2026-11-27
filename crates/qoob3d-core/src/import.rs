//! Mesh import (AG-SC-23, ADR-0034): STL (binary and ASCII) and 3MF (core
//! plus the production extension) as an indexed `f64` mesh in mm. The kernel
//! turns it into a polyhedral solid.
//!
//! These readers take foreign bytes: they never panic, check every size
//! against the input before allocating, and refuse more than
//! [`MAX_TRIANGLES`] triangles (counted as read, before collapsed ones are
//! dropped) and 3MF archives that inflate to more than 512 MiB.

mod stl;
mod three_mf;

use std::collections::HashMap;

use crate::mesh::WELD_TOLERANCE_MM;

pub use stl::read_stl;
pub use three_mf::read_3mf;

/// Most triangles an import may have (AG-SC-23): keeps a build inside the
/// worker limits.
pub const MAX_TRIANGLES: usize = 200_000;

/// A welded triangle mesh in mm: corners counter-clockwise seen from outside.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportedMesh {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

impl ImportedMesh {
    /// Signed volume by the divergence theorem, mm³: positive for an outward
    /// winding.
    #[must_use]
    pub fn signed_volume(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.positions[i as usize]);
                let cross = [
                    b[1] * c[2] - b[2] * c[1],
                    b[2] * c[0] - b[0] * c[2],
                    b[0] * c[1] - b[1] * c[0],
                ];
                (a[0] * cross[0] + a[1] * cross[1] + a[2] * cross[2]) / 6.0
            })
            .sum()
    }
}

/// Why a file cannot be imported.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// Malformed, truncated or using something these readers do not read.
    #[error("{0}")]
    Unreadable(String),
    #[error("{0} triangles, more than the limit of {MAX_TRIANGLES}")]
    TooManyTriangles(u64),
}

fn unreadable(message: impl Into<String>) -> ImportError {
    ImportError::Unreadable(message.into())
}

/// Collects triangles, welding corners closer than [`WELD_TOLERANCE_MM`]
/// (the same grid as `TriMesh::welded`) and dropping collapsed triangles.
#[derive(Default)]
struct Welder {
    index: HashMap<[i64; 3], u32>,
    mesh: ImportedMesh,
}

impl Welder {
    fn triangle(&mut self, corners: [[f64; 3]; 3]) -> Result<(), ImportError> {
        if corners.iter().flatten().any(|c| !c.is_finite()) {
            return Err(unreadable(format!(
                "a triangle has a corner that is not finite: {corners:?}"
            )));
        }
        let t = corners.map(|p| self.vertex(p));
        if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
            self.mesh.triangles.push(t);
        }
        Ok(())
    }

    fn vertex(&mut self, p: [f64; 3]) -> u32 {
        let positions = &mut self.mesh.positions;
        *self.index.entry(p.map(|c| (c / WELD_TOLERANCE_MM).round() as i64)).or_insert_with(|| {
            positions.push(p);
            // At most 3 · MAX_TRIANGLES vertices.
            (positions.len() - 1) as u32
        })
    }

    fn finish(self) -> Result<ImportedMesh, ImportError> {
        if self.mesh.triangles.is_empty() {
            return Err(unreadable("the file has no triangles"));
        }
        Ok(self.mesh)
    }
}

/// The triangle count `n` of a file, refused above [`MAX_TRIANGLES`].
fn within_limit(n: u64) -> Result<(), ImportError> {
    if n > MAX_TRIANGLES as u64 { Err(ImportError::TooManyTriangles(n)) } else { Ok(()) }
}
