//! Conversion of OCCT shapes into [`ShapeMesh`]es via our own C++ mesher
//! (`cpp/qoob3d_occt.cpp`, ADR-0014): face ids per triangle, correctly
//! oriented normals and feature edges that lie exactly on the faces.

use opencascade::primitives::Shape;
use qoob3d_core::{Polylines, ShapeMesh, TriMesh};

use crate::KernelError;
use crate::occt::ffi;

/// Angular deflection (radians) used together with the linear deflection.
const ANGULAR_DEFLECTION: f64 = 0.35;

pub(crate) fn tessellate(
    shape: &Shape,
    linear_deflection_mm: f64,
) -> Result<ShapeMesh, KernelError> {
    tessellate_raw(shape.as_ffi(), linear_deflection_mm)
}

pub(crate) fn tessellate_raw(
    shape: &ffi::TopoDS_Shape,
    linear_deflection_mm: f64,
) -> Result<ShapeMesh, KernelError> {
    let raw = ffi::qoob3d_tessellate(shape, linear_deflection_mm, ANGULAR_DEFLECTION)
        .map_err(|e| KernelError::Occt(e.what().to_string()))?;

    let triples = |v: &[f32]| v.as_chunks::<3>().0.to_vec();
    let mesh = ShapeMesh {
        faces: TriMesh {
            positions: triples(&raw.positions),
            normals: triples(&raw.normals),
            indices: raw.indices,
            face_ids: raw.face_ids,
        },
        edges: Polylines { points: triples(&raw.edge_points), starts: raw.edge_starts },
    };
    validate(&mesh)?;
    Ok(mesh)
}

/// Cheap structural checks so a broken FFI result never reaches the GPU.
fn validate(mesh: &ShapeMesh) -> Result<(), KernelError> {
    let f = &mesh.faces;
    let err = |msg: String| Err(KernelError::Tessellation(msg));
    if f.normals.len() != f.positions.len() {
        return err(format!("{} normals for {} vertices", f.normals.len(), f.positions.len()));
    }
    if !f.indices.len().is_multiple_of(3) || f.face_ids.len() != f.indices.len() / 3 {
        return err(format!("{} indices, {} face ids", f.indices.len(), f.face_ids.len()));
    }
    if let Some(&i) = f.indices.iter().find(|&&i| i as usize >= f.positions.len()) {
        return err(format!("index {i} out of range"));
    }
    let e = &mesh.edges;
    let sorted = e.starts.windows(2).all(|w| w[0] <= w[1]);
    if !sorted || e.starts.last().copied() != Some(e.points.len() as u32) {
        return err("edge polyline offsets are inconsistent".into());
    }
    Ok(())
}

/// Placeholder geometry until the document model exists (M3): a 40×30×15 mm
/// block on the XY plane with a Ø14 mm through hole – exercises planar and
/// curved faces, seams and booleans.
pub(crate) fn demo_part() -> Shape {
    use occt_glam::{DVec3, dvec3};
    let block = Shape::box_from_corners(dvec3(-20.0, -15.0, 0.0), dvec3(20.0, 15.0, 15.0));
    let hole = Shape::cylinder(dvec3(0.0, 0.0, -1.0), 7.0, DVec3::Z, 17.0);
    block.subtract(&hole).shape
}

#[cfg(test)]
mod tests;
