//! Binary STL import (WA-LV-03, docs/requirements/project-app.md): triangle
//! soup → indexed [`ShapeMesh`].
//!
//! - Corners are welded by exact position (`-0.0` counts as `0.0`), so the
//!   triangles share edges again.
//! - Normals come from the winding, never from the file (many writers store
//!   zeros). The triangles around a corner are grouped by the feature angle
//!   and a group shares the average normal, so flat faces stay flat and
//!   smooth surfaces stay smooth; a corner gets one output vertex per group.
//! - Feature edges: welded edges whose two triangle normals differ by more
//!   than [`FEATURE_ANGLE_DEG`], plus open and non-manifold edges.
//!
//! The declared triangle count is checked against the input length before
//! anything is allocated.

use std::collections::HashMap;

use crate::mesh::{Polylines, ShapeMesh, TriMesh};

/// Triangle normals further apart than this make an edge a feature edge.
pub const FEATURE_ANGLE_DEG: f32 = 30.0;

const HEADER_BYTES: usize = 84;
const TRIANGLE_BYTES: usize = 50;

/// Why an STL could not be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StlError {
    #[error("STL too short for its header: {0} bytes, need 84")]
    NoHeader(usize),
    #[error("STL truncated: {count} triangles declared need {expected} bytes, input has {actual}")]
    Truncated { count: u32, expected: u64, actual: usize },
}

/// Reads a binary STL (also when the 80-byte header starts with "solid").
pub fn read_binary(bytes: &[u8]) -> Result<ShapeMesh, StlError> {
    let Some((header, body)) = bytes.split_first_chunk::<HEADER_BYTES>() else {
        return Err(StlError::NoHeader(bytes.len()));
    };
    let count = u32::from_le_bytes([header[80], header[81], header[82], header[83]]);
    let expected = HEADER_BYTES as u64 + TRIANGLE_BYTES as u64 * u64::from(count);
    if (bytes.len() as u64) < expected {
        return Err(StlError::Truncated { count, expected, actual: bytes.len() });
    }
    let records = &body[..TRIANGLE_BYTES * count as usize];

    // Weld corners by exact position.
    let mut welded: HashMap<[u32; 3], u32> = HashMap::new();
    let mut points: Vec<[f32; 3]> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::with_capacity(count as usize);
    for record in records.as_chunks::<TRIANGLE_BYTES>().0 {
        // Bytes 0..12 hold the stored normal, ignored; 48..50 the attribute.
        let corner = |k: usize| {
            let f = |i: usize| {
                let o = 12 + 12 * k + 4 * i;
                // `+ 0.0` turns -0.0 into 0.0, so both weld together.
                f32::from_le_bytes([record[o], record[o + 1], record[o + 2], record[o + 3]]) + 0.0
            };
            [f(0), f(1), f(2)]
        };
        triangles.push(std::array::from_fn(|k| {
            let p = corner(k);
            *welded.entry(p.map(f32::to_bits)).or_insert_with(|| {
                points.push(p);
                points.len() as u32 - 1
            })
        }));
    }
    drop(welded);

    let normals: Vec<[f32; 3]> = triangles.iter().map(|t| face_normal(&points, *t)).collect();
    let cos_limit = FEATURE_ANGLE_DEG.to_radians().cos();
    let faces = shaded_faces(&points, &triangles, &normals, cos_limit);
    let edges = feature_edges(&points, &triangles, &normals, cos_limit);
    Ok(ShapeMesh { faces, edges })
}

/// Unnormalized normal (length = 2 × area, so sums weight by area).
fn area_normal(points: &[[f32; 3]], [a, b, c]: [u32; 3]) -> [f32; 3] {
    let [a, b, c] = [a, b, c].map(|i| points[i as usize]);
    let (u, v) = (sub(b, a), sub(c, a));
    [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
}

fn face_normal(points: &[[f32; 3]], t: [u32; 3]) -> [f32; 3] {
    normalize(area_normal(points, t))
}

/// Unit vector; zero stays zero (degenerate triangles).
fn normalize(n: [f32; 3]) -> [f32; 3] {
    let len = dot(n, n).sqrt();
    if len > 0.0 && len.is_finite() { n.map(|c| c / len) } else { [0.0; 3] }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Per-corner normals: the triangles around a corner are grouped once (a
/// triangle joins the first group whose first triangle lies within the
/// feature angle, so the work per corner is valence × groups, not valence²)
/// and every group averages its triangles; one output vertex per group, in
/// order of first use.
fn shaded_faces(
    points: &[[f32; 3]],
    triangles: &[[u32; 3]],
    normals: &[[f32; 3]],
    cos_limit: f32,
) -> TriMesh {
    // Triangle corners around each welded corner, as offsets into one flat
    // list; `slot_of` finds a triangle corner in it again.
    let mut first = vec![0u32; points.len() + 1];
    for &i in triangles.iter().flatten() {
        first[i as usize + 1] += 1;
    }
    for k in 1..first.len() {
        first[k] += first[k - 1];
    }
    let mut fill = first.clone();
    let mut around = vec![0u32; 3 * triangles.len()];
    let mut slot_of = vec![0u32; 3 * triangles.len()];
    for (t, tri) in triangles.iter().enumerate() {
        for (k, &i) in tri.iter().enumerate() {
            around[fill[i as usize] as usize] = t as u32;
            slot_of[3 * t + k] = fill[i as usize];
            fill[i as usize] += 1;
        }
    }

    // Groups of all corners in one list: (first triangle, summed normal).
    let mut groups: Vec<(u32, [f32; 3])> = Vec::new();
    let mut group_of = vec![0u32; around.len()];
    for corner in first.windows(2) {
        let start = groups.len();
        for slot in corner[0] as usize..corner[1] as usize {
            let t = around[slot] as usize;
            let found = groups[start..]
                .iter()
                .position(|(seed, _)| dot(normals[*seed as usize], normals[t]) >= cos_limit);
            let group = start
                + found.unwrap_or_else(|| {
                    groups.push((t as u32, [0.0; 3]));
                    groups.len() - 1 - start
                });
            let n = area_normal(points, triangles[t]);
            groups[group].1 = std::array::from_fn(|k| groups[group].1[k] + n[k]);
            group_of[slot] = group as u32;
        }
    }

    let mut out =
        TriMesh { indices: Vec::with_capacity(3 * triangles.len()), ..TriMesh::default() };
    let mut vertex: Vec<Option<u32>> = vec![None; groups.len()];
    for (&i, &slot) in triangles.iter().flatten().zip(&slot_of) {
        let group = group_of[slot as usize] as usize;
        let index = *vertex[group].get_or_insert_with(|| {
            out.positions.push(points[i as usize]);
            out.normals.push(normalize(groups[group].1));
            out.positions.len() as u32 - 1
        });
        out.indices.push(index);
    }
    out
}

/// Welded edges used by one triangle, by more than two, or by two whose
/// normals differ by more than the feature angle; as two-point polylines in
/// order of first appearance.
fn feature_edges(
    points: &[[f32; 3]],
    triangles: &[[u32; 3]],
    normals: &[[f32; 3]],
    cos_limit: f32,
) -> Polylines {
    // Edge → (first triangle, number of uses, sharp).
    let mut edges: HashMap<(u32, u32), (u32, u32, bool)> = HashMap::new();
    let mut order: Vec<(u32, u32)> = Vec::new();
    for (t, &[a, b, c]) in triangles.iter().enumerate() {
        for (from, to) in [(a, b), (b, c), (c, a)] {
            if from == to {
                continue;
            }
            let key = (from.min(to), from.max(to));
            let entry = edges.entry(key).or_insert_with(|| {
                order.push(key);
                (t as u32, 0, false)
            });
            entry.1 += 1;
            if entry.1 == 2 {
                entry.2 = dot(normals[entry.0 as usize], normals[t]) < cos_limit;
            }
        }
    }
    let mut out = Polylines::default();
    for key in order {
        let (_, uses, sharp) = edges[&key];
        if uses != 2 || sharp {
            out.points.extend([points[key.0 as usize], points[key.1 as usize]]);
            out.starts.push(out.points.len() as u32);
        }
    }
    out
}

#[cfg(test)]
mod tests;
