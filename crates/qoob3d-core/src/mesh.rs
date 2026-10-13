//! Triangle meshes as produced by the kernel and consumed by the renderer.
//!
//! Units are millimetres, coordinates are right-handed with +Z up (ADR-0006).

use std::collections::HashMap;

/// An indexed triangle mesh with per-vertex normals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Three indices per triangle, counter-clockwise when seen from outside.
    pub indices: Vec<u32>,
}

impl TriMesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Axis-aligned bounding box of all vertices, `None` for an empty mesh.
    pub fn bounds(&self) -> Option<Aabb> {
        let mut iter = self.positions.iter();
        let first = *iter.next()?;
        Some(iter.fold(Aabb { min: first, max: first }, |b, p| Aabb {
            min: std::array::from_fn(|i| b.min[i].min(p[i])),
            max: std::array::from_fn(|i| b.max[i].max(p[i])),
        }))
    }

    /// Topological and volumetric quality check (ADR-0013).
    ///
    /// Vertices closer than [`WELD_TOLERANCE_MM`] are treated as one, because
    /// kernel meshes duplicate vertices along face boundaries.
    pub fn analyze(&self) -> MeshReport {
        let quantize = |p: [f32; 3]| p.map(|c| (f64::from(c) / WELD_TOLERANCE_MM).round() as i64);
        let mut welded: HashMap<[i64; 3], u32> = HashMap::new();
        let remap: Vec<u32> = self
            .positions
            .iter()
            .map(|&p| {
                let next = welded.len() as u32;
                *welded.entry(quantize(p)).or_insert(next)
            })
            .collect();

        // Undirected edge -> (uses, uses in the a<b direction).
        let mut edges: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
        let mut report = MeshReport { triangles: self.triangle_count(), ..Default::default() };
        let mut volume6 = 0.0f64;

        for tri in self.indices.as_chunks::<3>().0 {
            let [a, b, c] = tri.map(|i| self.positions[i as usize].map(f64::from));
            volume6 += dot(a, cross(b, c));

            let w = tri.map(|i| remap[i as usize]);
            if w[0] == w[1] || w[1] == w[2] || w[2] == w[0] {
                report.degenerate_triangles += 1;
                continue;
            }
            for (from, to) in [(w[0], w[1]), (w[1], w[2]), (w[2], w[0])] {
                let entry = edges.entry((from.min(to), from.max(to))).or_default();
                entry.0 += 1;
                entry.1 += u32::from(from < to);
            }
        }

        for &(uses, forward) in edges.values() {
            match uses {
                1 => report.open_edges += 1,
                2 if forward != 1 => report.inconsistent_edges += 1,
                2 => {}
                _ => report.non_manifold_edges += 1,
            }
        }
        report.volume_mm3 = volume6 / 6.0;
        report
    }
}

/// Distance below which two vertices are considered identical by [`TriMesh::analyze`].
pub const WELD_TOLERANCE_MM: f64 = 1e-4;

/// Result of [`TriMesh::analyze`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MeshReport {
    pub triangles: usize,
    /// Edges used by exactly one triangle (holes).
    pub open_edges: usize,
    /// Edges used by more than two triangles.
    pub non_manifold_edges: usize,
    /// Edges whose two triangles traverse them in the same direction
    /// (inconsistent winding / flipped triangle).
    pub inconsistent_edges: usize,
    /// Triangles with two coincident (welded) corners.
    pub degenerate_triangles: usize,
    /// Signed volume; positive for a closed mesh with outward-facing winding.
    pub volume_mm3: f64,
}

impl MeshReport {
    /// Closed, 2-manifold and consistently oriented: what a slicer needs.
    pub fn is_watertight(&self) -> bool {
        self.triangles > 0
            && self.open_edges == 0
            && self.non_manifold_edges == 0
            && self.inconsistent_edges == 0
            && self.degenerate_triangles == 0
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub fn center(&self) -> [f32; 3] {
        std::array::from_fn(|i| 0.5 * (self.min[i] + self.max[i]))
    }

    /// Radius of the bounding sphere around [`Aabb::center`].
    pub fn radius(&self) -> f32 {
        let d: [f32; 3] = std::array::from_fn(|i| self.max[i] - self.min[i]);
        0.5 * (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_of_empty_mesh_is_none() {
        assert_eq!(TriMesh::default().bounds(), None);
    }

    #[test]
    fn bounds_cover_all_positions() {
        let mesh =
            TriMesh { positions: vec![[0.0, -1.0, 2.0], [3.0, 4.0, -5.0]], ..Default::default() };
        let b = mesh.bounds().unwrap();
        assert_eq!(b.min, [0.0, -1.0, -5.0]);
        assert_eq!(b.max, [3.0, 4.0, 2.0]);
        assert_eq!(b.center(), [1.5, 1.5, -1.5]);
    }

    /// Cube `[0, s]^3` built like kernel output: 4 separate vertices per face.
    fn split_face_cube(s: f32) -> TriMesh {
        let faces: [[[f32; 3]; 4]; 6] = [
            [[0., 0., s], [s, 0., s], [s, s, s], [0., s, s]],
            [[0., 0., 0.], [0., s, 0.], [s, s, 0.], [s, 0., 0.]],
            [[s, 0., 0.], [s, s, 0.], [s, s, s], [s, 0., s]],
            [[0., 0., 0.], [0., 0., s], [0., s, s], [0., s, 0.]],
            [[0., s, 0.], [0., s, s], [s, s, s], [s, s, 0.]],
            [[0., 0., 0.], [s, 0., 0.], [s, 0., s], [0., 0., s]],
        ];
        let mut mesh = TriMesh::default();
        for quad in faces {
            let base = mesh.positions.len() as u32;
            mesh.positions.extend(quad);
            mesh.normals.extend([[0.0; 3]; 4]);
            mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        mesh
    }

    #[test]
    fn closed_cube_is_watertight_with_exact_volume() {
        let r = split_face_cube(20.0).analyze();
        assert!(r.is_watertight(), "{r:?}");
        assert_eq!(r.triangles, 12);
        assert!((r.volume_mm3 - 8000.0).abs() < 1e-6, "{r:?}");
    }

    #[test]
    fn missing_triangle_is_reported_as_open() {
        let mut mesh = split_face_cube(10.0);
        mesh.indices.truncate(mesh.indices.len() - 3);
        let r = mesh.analyze();
        assert!(!r.is_watertight());
        assert_eq!(r.open_edges, 3);
    }

    #[test]
    fn flipped_triangle_is_reported_as_inconsistent() {
        let mut mesh = split_face_cube(10.0);
        mesh.indices.swap(0, 1);
        let r = mesh.analyze();
        assert!(!r.is_watertight());
        assert!(r.inconsistent_edges > 0, "{r:?}");
    }

    #[test]
    fn inside_out_cube_has_negative_volume() {
        let mut mesh = split_face_cube(10.0);
        for tri in mesh.indices.as_chunks_mut::<3>().0 {
            tri.swap(1, 2);
        }
        let r = mesh.analyze();
        assert!(r.is_watertight(), "consistently inverted is still closed: {r:?}");
        assert!((r.volume_mm3 + 1000.0).abs() < 1e-6);
    }

    #[test]
    fn empty_mesh_is_not_watertight() {
        assert!(!TriMesh::default().analyze().is_watertight());
    }

    proptest::proptest! {
        /// Translating and scaling never changes topology; volume scales with s³.
        #[test]
        fn analysis_is_invariant_under_translation_and_scale(
            s in 0.5f32..200.0,
            t in proptest::array::uniform3(-500.0f32..500.0),
        ) {
            let mut mesh = split_face_cube(s);
            for p in &mut mesh.positions {
                for k in 0..3 { p[k] += t[k]; }
            }
            let r = mesh.analyze();
            proptest::prop_assert!(r.is_watertight(), "{:?}", r);
            let expected = f64::from(s).powi(3);
            proptest::prop_assert!((r.volume_mm3 - expected).abs() / expected < 1e-3, "{:?}", r);
        }
    }
}
