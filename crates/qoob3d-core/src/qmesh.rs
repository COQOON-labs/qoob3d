//! Mesh transfer format between the project service and the app (WA-QM-01..03,
//! docs/requirements/project-app.md §3, ADR-0053): indexed, so shared corners
//! are stored once. Little endian:
//!
//! `b"QMSH"`, `u32` version 1, `u32` V, T, E, P, positions `f32×3×V`, normals
//! `f32×3×V`, indices `u32×3×T`, edge points `f32×3×E`, polyline starts
//! `u32×(P+1)` (sentinel = E). `face_ids` are not transferred.
//!
//! The reader checks the declared sizes against the input before it
//! allocates, so a corrupt or hostile header never asks for more memory than
//! the input already occupies.

use crate::mesh::{Polylines, ShapeMesh, TriMesh};

const MAGIC: &[u8; 4] = b"QMSH";
const VERSION: u32 = 1;
const HEADER_BYTES: usize = 24;

/// Why a qmesh could not be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QmeshError {
    #[error("not a qmesh (wrong magic)")]
    Magic,
    #[error("unsupported qmesh version {0}")]
    Version(u32),
    #[error("qmesh size mismatch: header declares {expected} bytes, input has {actual}")]
    Size { expected: u64, actual: usize },
    #[error("qmesh triangle index {index} is out of range (vertex count {vertices})")]
    Index { index: u32, vertices: u32 },
    #[error("qmesh polyline starts are not monotonic from 0 to the edge point count")]
    Starts,
}

/// Encodes `mesh` (without `face_ids`).
pub fn write(mesh: &ShapeMesh) -> Vec<u8> {
    let (faces, edges) = (&mesh.faces, &mesh.edges);
    let counts = [
        faces.positions.len(),
        faces.triangle_count(),
        edges.points.len(),
        edges.starts.len().saturating_sub(1),
    ];
    let size = HEADER_BYTES
        + 12 * (faces.positions.len() + faces.normals.len() + edges.points.len())
        + 4 * (faces.indices.len() + edges.starts.len());
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    for c in counts {
        out.extend_from_slice(&(c as u32).to_le_bytes());
    }
    for v in faces.positions.iter().chain(&faces.normals) {
        put_floats(&mut out, v);
    }
    put_words(&mut out, &faces.indices);
    for v in &edges.points {
        put_floats(&mut out, v);
    }
    put_words(&mut out, &edges.starts);
    out
}

fn put_floats(out: &mut Vec<u8>, v: &[f32; 3]) {
    for c in v {
        out.extend_from_slice(&c.to_le_bytes());
    }
}

fn put_words(out: &mut Vec<u8>, words: &[u32]) {
    for w in words {
        out.extend_from_slice(&w.to_le_bytes());
    }
}

/// Decodes and validates a qmesh; never panics on malformed input.
pub fn read(bytes: &[u8]) -> Result<ShapeMesh, QmeshError> {
    let size_error = |expected: u64| QmeshError::Size { expected, actual: bytes.len() };
    let Some((header, body)) = bytes.split_first_chunk::<HEADER_BYTES>() else {
        return Err(size_error(HEADER_BYTES as u64));
    };
    if &header[..4] != MAGIC {
        return Err(QmeshError::Magic);
    }
    let word =
        |i: usize| u32::from_le_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    let version = word(4);
    if version != VERSION {
        return Err(QmeshError::Version(version));
    }
    let [v, t, e, p] = [word(8), word(12), word(16), word(20)].map(u64::from);
    // u64 cannot overflow here: every term is below 2^32 · 24.
    let expected = HEADER_BYTES as u64 + 24 * v + 12 * t + 12 * e + 4 * (p + 1);
    if expected != bytes.len() as u64 {
        return Err(size_error(expected));
    }
    // From here on every count fits the input, so allocations are bounded by it.
    let (positions, rest) = floats(body, v as usize);
    let (normals, rest) = floats(rest, v as usize);
    let (indices, rest) = words(rest, 3 * t as usize);
    let (points, rest) = floats(rest, e as usize);
    let (starts, _) = words(rest, p as usize + 1);

    let vertices = v as u32;
    if let Some(&index) = indices.iter().find(|&&i| i >= vertices) {
        return Err(QmeshError::Index { index, vertices });
    }
    let monotonic = starts.windows(2).all(|w| w[0] <= w[1]);
    if starts.first() != Some(&0) || starts.last() != Some(&(e as u32)) || !monotonic {
        return Err(QmeshError::Starts);
    }
    Ok(ShapeMesh {
        faces: TriMesh { positions, normals, indices, face_ids: Vec::new() },
        edges: Polylines { points, starts },
    })
}

/// `n` points of 12 bytes; the caller has checked the length.
fn floats(bytes: &[u8], n: usize) -> (Vec<[f32; 3]>, &[u8]) {
    let (head, rest) = bytes.split_at(12 * n);
    let out = head
        .as_chunks::<12>()
        .0
        .iter()
        .map(|c| {
            let f = |i: usize| f32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
            [f(0), f(4), f(8)]
        })
        .collect();
    (out, rest)
}

/// `n` words of 4 bytes; the caller has checked the length.
fn words(bytes: &[u8], n: usize) -> (Vec<u32>, &[u8]) {
    let (head, rest) = bytes.split_at(4 * n);
    (head.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect(), rest)
}

#[cfg(test)]
mod tests;
