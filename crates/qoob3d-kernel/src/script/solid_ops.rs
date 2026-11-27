//! Operations on whole solids of the script API (AG-SC-25…31,
//! `docs/script-api.md`): what an imported or cut freeform body needs –
//! `defects` names what `BRepCheck_Analyzer` finds, `heal` repairs it –
//! and the operations that came up building a printed kit from such a body:
//! `fill_voids`, `offset`, `split`, `on_bed` and the height map `heights`.

use rhai::{Array, Dynamic, Engine, Map};

use super::Shape;
use super::api::{
    BoolOptions, COMMON, Expect, Rr, boolean, finite, made, occt, plausible, positive, runtime,
};
use crate::occt::ffi;
use crate::occt::occ;

/// Default precision of `heal` in mm: well below a printer's resolution,
/// well above the gaps freeform booleans leave.
const HEAL_TOLERANCE_MM: f64 = 1e-4;
/// `defects` reports at most this many elements.
const MAX_DEFECTS: u32 = 50;
/// A height map has at most this many points.
const MAX_HEIGHT_POINTS: f64 = 1_000_000.0;

const KIND_NAMES: [&str; 6] = ["vertex", "edge", "wire", "face", "shell", "solid"];

fn float_array(values: &[f64]) -> Dynamic {
    Dynamic::from_array(values.iter().copied().map(Dynamic::from_float).collect())
}

/// The defects of a shape as script maps (empty for a valid shape).
fn defects(s: &Shape) -> Rr<Array> {
    let raw = ffi::qoob3d_defects(s.get(), MAX_DEFECTS)
        .map_err(|e| super::api::geometry("defects", e.what()))?;
    Ok((0..raw.indices.len())
        .map(|i| {
            let b = &raw.bounds[6 * i..6 * i + 6];
            let mut m = Map::new();
            let kind = KIND_NAMES.get(raw.kinds[i] as usize).copied().unwrap_or("shape");
            m.insert("kind".into(), kind.into());
            m.insert("index".into(), Dynamic::from_int(i64::from(raw.indices[i])));
            m.insert("problem".into(), raw.statuses[i].clone().into());
            m.insert("min".into(), float_array(&b[..3]));
            m.insert("max".into(), float_array(&b[3..]));
            Dynamic::from_map(m)
        })
        .collect())
}

fn heal(s: &Shape, tolerance: f64) -> Rr<Shape> {
    // A valid shape stays as it is: on a face of an imported boat the
    // repair turned a valid wire into a self-intersecting one.
    if s.is_valid() {
        return Ok(s.clone());
    }
    let healed = made("heal", ffi::qoob3d_heal(s.get(), tolerance))?;
    // Healing must not lose material: at most the tolerance times the area.
    let (before, after) = (s.volume(), healed.volume());
    let allowed = tolerance * s.area().max(0.0) * 2.0 + before.abs() * 1e-6;
    if !(after - before).abs().le(&allowed) {
        return Err(super::api::geometry(
            "heal",
            format!("the volume changed from {before} to {after} mm³; the shape is too damaged"),
        ));
    }
    Ok(healed)
}

/// `[below, above]` the plane through `offset` along the axis of `plane`.
fn split(s: &Shape, plane: &str, offset: f64) -> Rr<Array> {
    let axis = match plane {
        "xy" => 2,
        "xz" => 1,
        "yz" => 0,
        other => {
            return Err(runtime(format!(
                "split: plane must be \"xy\", \"xz\" or \"yz\", got {other:?}"
            )));
        }
    };
    let (min, max) = occt("split", s.bounds())?;
    if offset <= min[axis] || offset >= max[axis] {
        return Err(super::api::geometry(
            "split",
            format!(
                "the plane at {offset} does not cut the shape ({} to {})",
                min[axis], max[axis]
            ),
        ));
    }
    let margin = 1.0 + (0..3).map(|i| max[i] - min[i]).fold(0.0, f64::max);
    let half = |below: bool| -> Rr<Shape> {
        let mut lo: [f64; 3] = std::array::from_fn(|i| min[i] - margin);
        let mut hi: [f64; 3] = std::array::from_fn(|i| max[i] + margin);
        if below {
            hi[axis] = offset;
        } else {
            lo[axis] = offset;
        }
        let size: [f64; 3] = std::array::from_fn(|i| hi[i] - lo[i]);
        let corner_and_size = [lo[0], lo[1], lo[2], size[0], size[1], size[2]];
        let block = made("split", ffi::qoob3d_primitive(0, &corner_and_size))?;
        boolean("split", s, &block, COMMON, &Expect::Material, BoolOptions::default())
    };
    Ok(vec![Dynamic::from(half(true)?), Dynamic::from(half(false)?)])
}

/// The shape moved so that it stands on z = 0, centred on the Z axis.
fn on_bed(s: &Shape) -> Rr<Shape> {
    let (min, max) = occt("on_bed", s.bounds())?;
    let d = [-(min[0] + max[0]) / 2.0, -(min[1] + max[1]) / 2.0, -min[2]];
    made("on_bed", ffi::qoob3d_transform(s.get(), super::api::TRANSLATE, &d))
}

fn heights(s: &Shape, from: &Dynamic, to: &Dynamic, step: f64) -> Rr<Array> {
    let corner = |v: &Dynamic, what: &str| -> Rr<[f64; 2]> {
        let Some(a) = v.read_lock::<Array>().filter(|a| a.len() == 2) else {
            return Err(runtime(format!("heights: {what} must be an array [x, y]")));
        };
        Ok([finite(&a[0], what)?, finite(&a[1], what)?])
    };
    let (a, b) = (corner(from, "from")?, corner(to, "to")?);
    let n = |lo: f64, hi: f64| ((hi - lo).abs() / step).floor() + 1.0;
    let (nx, ny) = (n(a[0], b[0]), n(a[1], b[1]));
    if nx * ny > MAX_HEIGHT_POINTS {
        return Err(runtime(format!(
            "heights: {nx} × {ny} points; at most {MAX_HEIGHT_POINTS} (use a larger step)"
        )));
    }
    let (dx, dy) = (step.copysign(b[0] - a[0]), step.copysign(b[1] - a[1]));
    let z = occt(
        "heights",
        occ(ffi::qoob3d_heights(s.get(), a[0], a[1], dx, dy, nx as u32, ny as u32)),
    )?;
    Ok(z.chunks(nx as usize)
        .map(|row| {
            Dynamic::from_array(
                row.iter()
                    .map(|&v| if v.is_nan() { Dynamic::UNIT } else { Dynamic::from_float(v) })
                    .collect(),
            )
        })
        .collect())
}

/// `mesh_report(s)` (AG-SC-34): what a slicer will find in the exported
/// mesh of `s`, tessellated like the project's parts. Lets a check demand
/// what the gate otherwise only warns about (finding 3.6).
pub(super) fn register_mesh_report(engine: &mut Engine, mesh: qoob3d_core::project::Mesh) {
    engine.register_fn("mesh_report", move |s: Shape| -> Rr<Map> {
        let tessellated =
            occt("mesh_report", s.mesh_with_angle(mesh.deflection_mm, mesh.angle_deg))?;
        let report = tessellated.faces.export_report();
        let int = |n: usize| Dynamic::from_int(i64::try_from(n).unwrap_or(i64::MAX));
        let mut m = Map::new();
        m.insert("watertight".into(), Dynamic::from_bool(report.is_watertight()));
        m.insert("triangles".into(), int(report.triangles));
        m.insert("open_edges".into(), int(report.open_edges));
        m.insert("non_manifold_edges".into(), int(report.non_manifold_edges));
        m.insert("flipped_edges".into(), int(report.inconsistent_edges));
        m.insert("volume".into(), Dynamic::from_float(report.volume_mm3));
        Ok(m)
    });
}

pub(super) fn register(engine: &mut Engine) {
    engine.register_fn("defects", |s: Shape| defects(&s));
    engine.register_fn("heal", |s: Shape| heal(&s, HEAL_TOLERANCE_MM));
    engine.register_fn("heal", |s: Shape, tolerance: Dynamic| -> Rr<Shape> {
        heal(&s, positive(&tolerance, "heal: tolerance")?)
    });
    engine.register_fn("fill_voids", |s: Shape| -> Rr<Shape> {
        let filled = made("fill_voids", ffi::qoob3d_fill_voids(s.get()))?;
        let before = s.volume();
        plausible("fill_voids", filled, &Expect::AtLeast(before))
    });
    engine.register_fn("offset", |s: Shape, d: Dynamic| -> Rr<Shape> {
        let d = finite(&d, "offset: d")?;
        if d == 0.0 {
            return Err(runtime("offset: d must not be 0"));
        }
        let grown = made("offset", ffi::qoob3d_offset_shape(s.get(), d))?;
        let expect = if d > 0.0 { Expect::AtLeast(s.volume()) } else { Expect::Less(s.volume()) };
        plausible("offset", grown, &expect)
    });
    engine.register_fn("split", |s: Shape, plane: &str, offset: Dynamic| -> Rr<Array> {
        split(&s, plane, finite(&offset, "split: offset")?)
    });
    engine.register_fn("on_bed", |s: Shape| on_bed(&s));
    engine.register_fn(
        "heights",
        |s: Shape, from: Dynamic, to: Dynamic, step: Dynamic| -> Rr<Array> {
            heights(&s, &from, &to, positive(&step, "heights: step")?)
        },
    );
}
