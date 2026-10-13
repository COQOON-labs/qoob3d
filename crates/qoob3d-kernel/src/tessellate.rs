//! Conversion of OCCT shapes into [`TriMesh`]es.

use opencascade::primitives::Shape;
use qoob3d_core::TriMesh;

use crate::KernelError;

pub(crate) fn tessellate(shape: &Shape, tolerance_mm: f64) -> Result<TriMesh, KernelError> {
    let mesh = shape
        .mesh_with_tolerance(tolerance_mm)
        .map_err(|e| KernelError::Tessellation(e.to_string()))?;

    if mesh.normals.len() != mesh.vertices.len() {
        return Err(KernelError::Tessellation(format!(
            "normal count {} does not match vertex count {}",
            mesh.normals.len(),
            mesh.vertices.len()
        )));
    }

    let to_f32 = |v: &occt_glam::DVec3| [v.x as f32, v.y as f32, v.z as f32];
    let mut out = TriMesh {
        positions: mesh.vertices.iter().map(to_f32).collect(),
        normals: mesh.normals.iter().map(to_f32).collect(),
        indices: mesh
            .indices
            .iter()
            .map(|&i| u32::try_from(i).expect("mesh index exceeds u32"))
            .collect(),
    };
    orient_normals_by_winding(&mut out);
    Ok(out)
}

/// ADR-0004: opencascade-rs reports the raw surface normal and ignores the face
/// orientation, so normals of reversed faces point inwards. The triangle
/// winding *is* corrected for orientation, so we flip every vertex normal
/// that disagrees with the (area-weighted) normal of its adjacent triangles.
/// Vertices are never shared between faces, so this is exact per face.
fn orient_normals_by_winding(mesh: &mut TriMesh) {
    let mut geometric = vec![[0.0f32; 3]; mesh.positions.len()];
    for tri in mesh.indices.as_chunks::<3>().0 {
        let [a, b, c] = [0, 1, 2].map(|k| mesh.positions[tri[k] as usize]);
        let n = cross(sub(b, a), sub(c, a));
        for &i in tri {
            let g = &mut geometric[i as usize];
            for k in 0..3 {
                g[k] += n[k];
            }
        }
    }
    for (n, g) in mesh.normals.iter_mut().zip(&geometric) {
        let len = dot(*n, *n).sqrt();
        if len < 1e-6 {
            // Degenerate surface normal: fall back to the geometric one.
            let gl = dot(*g, *g).sqrt().max(f32::MIN_POSITIVE);
            *n = g.map(|v| v / gl);
        } else if dot(*n, *g) < 0.0 {
            *n = n.map(|v| -v);
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Milestone-1 demo geometry: a cube of edge length `size` sitting on the XY
/// plane, centred on the Z axis.
pub(crate) fn demo_cube(size: f64) -> Shape {
    use occt_glam::dvec3;
    let h = size / 2.0;
    Shape::box_from_corners(dvec3(-h, -h, 0.0), dvec3(h, h, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_tessellates_to_twelve_outward_facing_triangles() {
        let mesh = tessellate(&demo_cube(20.0), 0.05).unwrap();
        assert_eq!(mesh.triangle_count(), 12);

        // Slicer-readiness (ADR-0002): closed, manifold, outward winding.
        let report = mesh.analyze();
        assert!(report.is_watertight(), "{report:?}");
        assert!((report.volume_mm3 - 8000.0).abs() < 1e-3, "{report:?}");

        let b = mesh.bounds().unwrap();
        assert_eq!(b.min, [-10.0, -10.0, 0.0]);
        assert_eq!(b.max, [10.0, 10.0, 20.0]);

        // Normals must point away from the cube centre, and the winding must
        // agree with the normals (counter-clockwise seen from outside).
        let c = b.center();
        for tri in mesh.indices.as_chunks::<3>().0 {
            let [a, b2, d] = [0, 1, 2].map(|k| mesh.positions[tri[k] as usize]);
            let n = mesh.normals[tri[0] as usize];
            let to_face = [a[0] - c[0], a[1] - c[1], a[2] - c[2]];
            assert!(dot(n, to_face) > 0.0, "normal {n:?} points inward");

            let e1 = sub(b2, a);
            let e2 = sub(d, a);
            let geo_n = cross(e1, e2);
            assert!(dot(geo_n, n) > 0.0, "winding disagrees with normal {n:?}");
        }
    }
}
