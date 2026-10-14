//! Triangle meshes as produced by the kernel and consumed by the renderer.
//!
//! Units are millimetres, coordinates are right-handed with +Z up (ADR-0006).

use std::collections::HashMap;

/// Tessellated B-rep: shaded faces plus the feature edges drawn on top.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapeMesh {
    pub faces: TriMesh,
    pub edges: Polylines,
}

/// An indexed triangle mesh with per-vertex normals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Three indices per triangle, counter-clockwise when seen from outside.
    pub indices: Vec<u32>,
    /// B-rep face index per triangle (picking, highlighting). Empty if the
    /// mesh does not come from the kernel. Only stable within one rebuild.
    pub face_ids: Vec<u32>,
}

/// A set of polylines in one buffer: polyline `i` spans
/// `points[starts[i]..starts[i + 1]]`; `starts` ends with a sentinel equal to
/// `points.len()`.
#[derive(Debug, Clone, PartialEq)]
pub struct Polylines {
    pub points: Vec<[f32; 3]>,
    pub starts: Vec<u32>,
}

impl Default for Polylines {
    fn default() -> Self {
        Self { points: Vec::new(), starts: vec![0] }
    }
}

impl Polylines {
    pub fn len(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = &[[f32; 3]]> {
        self.starts.windows(2).map(|w| &self.points[w[0] as usize..w[1] as usize])
    }

    /// Line-list vertex pairs (a, b) for all segments, for GPU upload.
    pub fn segments(&self) -> impl Iterator<Item = [[f32; 3]; 2]> + '_ {
        self.iter().flat_map(|line| line.windows(2).map(|w| [w[0], w[1]]))
    }
}

impl ShapeMesh {
    /// One mesh from several (e.g. all bodies for the renderer). Face ids
    /// stay unique: each part's ids are offset past the previous part's.
    pub fn merge<'a>(parts: impl IntoIterator<Item = &'a ShapeMesh>) -> ShapeMesh {
        let mut out = ShapeMesh::default();
        for part in parts {
            let base = out.faces.positions.len() as u32;
            let face_base = out.faces.face_ids.iter().max().map_or(0, |m| m + 1);
            out.faces.positions.extend_from_slice(&part.faces.positions);
            out.faces.normals.extend_from_slice(&part.faces.normals);
            out.faces.indices.extend(part.faces.indices.iter().map(|i| i + base));
            out.faces.face_ids.extend(part.faces.face_ids.iter().map(|f| f + face_base));
            let point_base = out.edges.points.len() as u32;
            out.edges.starts.pop();
            out.edges.points.extend_from_slice(&part.edges.points);
            for w in part.edges.starts.windows(2) {
                out.edges.starts.push(w[0] + point_base);
            }
            out.edges.starts.push(out.edges.points.len() as u32);
        }
        out
    }
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

    /// Shared-vertex form for export (3MF needs indexed, manifold meshes):
    /// vertices within [`WELD_TOLERANCE_MM`] are merged, collapsed triangles
    /// dropped. Returns (positions, triangles).
    pub fn welded(&self) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let quantize = |p: [f32; 3]| p.map(|c| (f64::from(c) / WELD_TOLERANCE_MM).round() as i64);
        let mut index: HashMap<[i64; 3], u32> = HashMap::new();
        let mut positions = Vec::new();
        let remap: Vec<u32> = self
            .positions
            .iter()
            .map(|&p| {
                *index.entry(quantize(p)).or_insert_with(|| {
                    positions.push(p);
                    positions.len() as u32 - 1
                })
            })
            .collect();
        let triangles = self
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| t.map(|i| remap[i as usize]))
            .filter(|t| t[0] != t[1] && t[1] != t[2] && t[2] != t[0])
            .collect();
        (positions, triangles)
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

impl TriMesh {
    /// Closest intersection of a ray with the mesh (both triangle sides),
    /// Möller–Trumbore. Linear in the triangle count; a BVH can come later.
    pub fn raycast(&self, origin: [f32; 3], dir: [f32; 3]) -> Option<RayHit> {
        const EPS: f32 = 1e-9;
        let mut best: Option<RayHit> = None;
        for (triangle, tri) in self.indices.as_chunks::<3>().0.iter().enumerate() {
            let [a, b, c] = tri.map(|i| self.positions[i as usize]);
            let (e1, e2) = (sub(b, a), sub(c, a));
            let p = cross3(dir, e2);
            let det = dot3(e1, p);
            if det.abs() < EPS {
                continue;
            }
            let inv = 1.0 / det;
            let s = sub(origin, a);
            let u = dot3(s, p) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = cross3(s, e1);
            let v = dot3(dir, q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = dot3(e2, q) * inv;
            if t > 0.0 && best.is_none_or(|h| t < h.t) {
                let point = std::array::from_fn(|k| origin[k] + dir[k] * t);
                best = Some(RayHit { t, triangle, point });
            }
        }
        best
    }
}

/// Result of [`TriMesh::raycast`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// Ray parameter: `point = origin + dir * t`.
    pub t: f32,
    /// Triangle index (see [`TriMesh::face_ids`] for the B-rep face).
    pub triangle: usize,
    pub point: [f32; 3],
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
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
#[derive(Debug, Clone, Copy, Default, PartialEq)]
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
mod tests;
