//! Sketch profiles via OCCT General Fuse (ADR-0028).

use qoob3d_core::document::{CurveKind, EntityId, Sketch};
use qoob3d_core::profile::Profile;

use crate::KernelError;
use crate::occt::ffi;

/// Curves shorter / smaller than this are ignored (degenerate input).
const MIN_SIZE_MM: f64 = 1e-6;

/// Non-construction curves of a sketch in the FFI layout, plus their extent
/// (max |coordinate|) for tolerances. Degenerate curves are skipped.
pub(crate) fn flatten(sketch: &Sketch) -> (ffi::Qoob3dCurves, f64) {
    let mut c = ffi::Qoob3dCurves {
        lines: vec![],
        line_ids: vec![],
        circles: vec![],
        circle_ids: vec![],
        arcs: vec![],
        arc_ids: vec![],
    };
    let mut extent = 0.0f64;
    for curve in sketch.curves.iter().filter(|c| !c.construction) {
        match curve.kind {
            CurveKind::Line { start, end } => {
                let (a, b) = (sketch.pos(start), sketch.pos(end));
                if (a[0] - b[0]).hypot(a[1] - b[1]) < MIN_SIZE_MM {
                    continue;
                }
                c.lines.extend([a[0], a[1], b[0], b[1]]);
                c.line_ids.push(curve.id.0);
                extent = extent.max(a[0].abs()).max(a[1].abs()).max(b[0].abs()).max(b[1].abs());
            }
            CurveKind::Circle { center, radius } => {
                if radius < MIN_SIZE_MM {
                    continue;
                }
                let p = sketch.pos(center);
                c.circles.extend([p[0], p[1], radius]);
                c.circle_ids.push(curve.id.0);
                extent = extent.max(p[0].abs() + radius).max(p[1].abs() + radius);
            }
            CurveKind::Arc { center, start, end } => {
                let g = sketch.arc(center, start, end);
                if g.radius < MIN_SIZE_MM {
                    continue;
                }
                c.arcs.extend([g.center[0], g.center[1], g.radius, g.start_angle, g.end_angle()]);
                c.arc_ids.push(curve.id.0);
                extent = extent.max(g.center[0].abs() + g.radius).max(g.center[1].abs() + g.radius);
            }
        }
    }
    (c, extent)
}

/// Closed regions of a sketch; construction geometry is ignored (SK-PR-02).
pub(crate) fn sketch_profiles(sketch: &Sketch) -> Result<Vec<Profile>, KernelError> {
    let (curves, extent) = flatten(sketch);
    if curves.line_ids.is_empty() && curves.circle_ids.is_empty() && curves.arc_ids.is_empty() {
        return Ok(Vec::new());
    }
    // Display tessellation: fine enough for smooth circles at any size.
    let deflection = (extent * 1e-3).clamp(1e-3, 0.5);
    let raw = ffi::qoob3d_sketch_regions(&curves, deflection)
        .map_err(|e| KernelError::Occt(e.what().to_string()))?;

    let regions = raw.areas.len();
    let consistent = raw.vertex_starts.len() == regions + 1
        && raw.index_starts.len() == regions + 1
        && raw.key_starts.len() == regions + 1;
    if !consistent {
        return Err(KernelError::Tessellation("inconsistent region buffers".into()));
    }
    let mut out = Vec::with_capacity(regions);
    for r in 0..regions {
        let (v0, v1) = (raw.vertex_starts[r] as usize, raw.vertex_starts[r + 1] as usize);
        let (i0, i1) = (raw.index_starts[r] as usize, raw.index_starts[r + 1] as usize);
        let (k0, k1) = (raw.key_starts[r] as usize, raw.key_starts[r + 1] as usize);
        let positions: Vec<[f64; 2]> = raw.positions[2 * v0..2 * v1].as_chunks::<2>().0.to_vec();
        let triangles: Vec<[u32; 3]> = raw.indices[i0..i1].as_chunks::<3>().0.to_vec();
        if triangles.iter().flatten().any(|&i| i as usize >= positions.len()) {
            return Err(KernelError::Tessellation("profile index out of range".into()));
        }
        out.push(Profile {
            key: raw.key_ids[k0..k1].iter().map(|&id| EntityId(id)).collect(),
            area_mm2: raw.areas[r],
            positions,
            triangles,
        });
    }
    // Deterministic order for stable UI and tests.
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

#[cfg(test)]
mod tests;
