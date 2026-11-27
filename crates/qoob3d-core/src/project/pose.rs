//! Print pose of a part (AG-PJ-06): how it lies on the bed for printing,
//! declared with the part (`part("Hull", #{ pose: #{ rotate: ["x", 180] } }, …)`)
//! instead of a second part that only turns the first one. Export writes
//! parts in their print pose; render shows them where they are modelled.
//! A kit of parts is arranged on the bed in rows (AG-SV-09).

use serde::{Deserialize, Serialize};

use crate::mesh::TriMesh;

/// Rotations about origin axes (applied in order), then the part stands on
/// z = 0, centred on the Z axis; `count` copies are printed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintPose {
    /// `(unit axis, angle in degrees)`, right-hand rule.
    pub rotate: Vec<([f64; 3], f64)>,
    pub count: u32,
}

impl Default for PrintPose {
    fn default() -> Self {
        Self { rotate: Vec::new(), count: 1 }
    }
}

/// Rotation matrix (rows) about the unit `axis` by `deg` degrees.
fn rotation(axis: [f64; 3], deg: f64) -> [[f64; 3]; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    let t = 1.0 - c;
    let [x, y, z] = axis;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

fn apply(m: &[[f64; 3]; 3], p: [f32; 3]) -> [f64; 3] {
    let p = p.map(f64::from);
    std::array::from_fn(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2])
}

impl PrintPose {
    /// The mesh turned into this pose and stood on the bed, centred.
    pub fn apply(&self, mesh: &TriMesh) -> TriMesh {
        self.apply_all(&[mesh]).pop().unwrap_or_default()
    }

    /// Several meshes that print as one object (AG-PJ-07) turned into this
    /// pose together: they keep their places relative to each other, and
    /// their joint box stands on the bed, centred.
    pub fn apply_all(&self, meshes: &[&TriMesh]) -> Vec<TriMesh> {
        let mut m = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for &(axis, deg) in &self.rotate {
            let r = rotation(axis, deg);
            m = std::array::from_fn(|i| {
                std::array::from_fn(|j| (0..3).map(|k| r[i][k] * m[k][j]).sum())
            });
        }
        let turned: Vec<Vec<[f64; 3]>> = meshes
            .iter()
            .map(|mesh| mesh.positions.iter().map(|&p| apply(&m, p)).collect())
            .collect();
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in turned.iter().flatten() {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let shift = [-(lo[0] + hi[0]) / 2.0, -(lo[1] + hi[1]) / 2.0, -lo[2]];
        meshes
            .iter()
            .zip(turned)
            .map(|(mesh, turned)| TriMesh {
                positions: turned
                    .iter()
                    .map(|p| std::array::from_fn(|k| (p[k] + shift[k]) as f32))
                    .collect(),
                normals: mesh.normals.iter().map(|&n| apply(&m, n).map(|c| c as f32)).collect(),
                indices: mesh.indices.clone(),
                face_ids: mesh.face_ids.clone(),
            })
            .collect()
    }
}

/// Gap between parts on the bed, mm.
pub const ARRANGE_GAP_MM: f32 = 5.0;

/// Places footprints `sizes` (x, y extents, mm) in rows from the bed's
/// front left corner: a row ends where the next part would pass
/// `bed_width`. Returns the centre of each footprint. Rows that pass the
/// bed's depth continue behind it; the slicer moves them to other plates.
pub fn arrange(sizes: &[[f32; 2]], bed_width: f32) -> Vec<[f32; 2]> {
    let mut centres = Vec::with_capacity(sizes.len());
    let (mut x, mut y, mut row_depth) = (0.0f32, 0.0f32, 0.0f32);
    for &[w, d] in sizes {
        if x > 0.0 && x + w > bed_width {
            x = 0.0;
            y += row_depth + ARRANGE_GAP_MM;
            row_depth = 0.0;
        }
        centres.push([x + w / 2.0, y + d / 2.0]);
        x += w + ARRANGE_GAP_MM;
        row_depth = row_depth.max(d);
    }
    centres
}

#[cfg(test)]
mod tests;
