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
mod tests {
    use super::*;
    use qoob3d_core::document::{OriginPlane, PlaneRef};
    use std::f64::consts::PI;

    fn rect(s: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<EntityId> {
        let p = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|q| s.add_point(q));
        (0..4).map(|i| s.add_line(p[i], p[(i + 1) % 4])).collect()
    }

    fn sketch() -> Sketch {
        Sketch::new(PlaneRef::Origin(OriginPlane::XY))
    }

    #[test]
    fn rectangle_is_one_profile() {
        let mut s = sketch();
        let lines = rect(&mut s, 0.0, 0.0, 40.0, 30.0);
        let p = sketch_profiles(&s).unwrap();
        assert_eq!(p.len(), 1);
        assert!((p[0].area_mm2 - 1200.0).abs() < 1e-6, "{}", p[0].area_mm2);
        assert_eq!(p[0].key, lines);
        assert!(p[0].contains([20.0, 15.0]) && !p[0].contains([50.0, 15.0]));
    }

    #[test]
    fn circle_inside_rectangle_gives_ring_and_disc() {
        let mut s = sketch();
        let lines = rect(&mut s, 0.0, 0.0, 40.0, 30.0);
        let c = s.add_point([20.0, 15.0]);
        let circle = s.add_circle(c, 7.0);
        let p = sketch_profiles(&s).unwrap();
        assert_eq!(p.len(), 2, "{:?}", p.iter().map(|x| &x.key).collect::<Vec<_>>());
        let disc = p.iter().find(|x| x.key == vec![circle]).expect("disc");
        let ring = p.iter().find(|x| x.key.len() == 5).expect("ring");
        assert!((disc.area_mm2 - PI * 49.0).abs() < 1e-6);
        assert!((ring.area_mm2 - (1200.0 - PI * 49.0)).abs() < 1e-6);
        assert!(lines.iter().all(|l| ring.key.contains(l)));
        assert!(!ring.contains([20.0, 15.0]), "the hole is not part of the ring");
    }

    #[test]
    fn overlapping_shapes_split_into_regions() {
        let mut s = sketch();
        rect(&mut s, 0.0, 0.0, 20.0, 20.0);
        rect(&mut s, 10.0, 10.0, 30.0, 30.0);
        let p = sketch_profiles(&s).unwrap();
        assert_eq!(p.len(), 3, "two L-shapes and the overlap");
        let total: f64 = p.iter().map(|x| x.area_mm2).sum();
        assert!((total - (400.0 + 400.0 - 100.0)).abs() < 1e-6);
    }

    #[test]
    fn open_contours_and_construction_make_no_profile() {
        let mut s = sketch();
        let a = s.add_point([0.0, 0.0]);
        let b = s.add_point([10.0, 0.0]);
        let c = s.add_point([10.0, 10.0]);
        s.add_line(a, b);
        s.add_line(b, c);
        assert!(sketch_profiles(&s).unwrap().is_empty(), "open polyline");

        let mut s = sketch();
        for id in rect(&mut s, 0.0, 0.0, 10.0, 10.0) {
            s.curve_mut(id).unwrap().construction = true;
        }
        assert!(sketch_profiles(&s).unwrap().is_empty(), "construction rectangle");
        assert!(sketch_profiles(&sketch()).unwrap().is_empty(), "empty sketch");
    }
}
