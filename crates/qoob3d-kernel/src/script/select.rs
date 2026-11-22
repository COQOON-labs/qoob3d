//! Edge and face selectors, fillet, chamfer and shell of the script API
//! (AG-SC-09, AG-SC-10, AG-SC-19). A selection remembers its shape; OCCT addresses the
//! elements by their index in `TopExp::MapShapes`.

use rhai::{Dynamic, Engine};

use super::Shape;
use super::api::{
    Expect, Rr, axis, geometry, made, non_negative, num, plausible, point, positive, runtime,
};
use crate::occt::ffi;

/// Kind of seams and degenerate edges in `Qoob3dElements`: never selected.
const SEAM: u32 = 3;
const LINE: u32 = 0;
const CIRCLE: u32 = 1;
/// Positions within this distance count as equal (mm).
const TOL_MM: f64 = 1e-6;

#[derive(Debug, Clone, Copy)]
struct Element {
    /// 1-based index in the OCCT map.
    index: u32,
    kind: u32,
    min: [f64; 3],
    max: [f64; 3],
    dir: [f64; 3],
}

#[derive(Clone)]
pub(super) struct Selection {
    shape: Shape,
    faces: bool,
    items: Vec<Element>,
    /// Height range of the whole shape, for `top()` / `bottom()`.
    z_range: [f64; 2],
}

impl Selection {
    fn of(s: &Shape, faces: bool) -> Rr<Self> {
        let function = if faces { "faces" } else { "edges" };
        let raw = ffi::qoob3d_elements(s.get(), faces).map_err(|e| geometry(function, e.what()))?;
        let items: Vec<Element> = (0..raw.kinds.len())
            .filter(|&i| raw.kinds[i] != SEAM)
            .map(|i| {
                let b = &raw.bounds[6 * i..6 * i + 6];
                let d = &raw.directions[3 * i..3 * i + 3];
                Element {
                    index: i as u32 + 1,
                    kind: raw.kinds[i],
                    min: [b[0], b[1], b[2]],
                    max: [b[3], b[4], b[5]],
                    dir: [d[0], d[1], d[2]],
                }
            })
            .collect();
        let z_range = items.iter().fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], e| {
            [lo.min(e.min[2]), hi.max(e.max[2])]
        });
        Ok(Self { shape: s.clone(), faces, items, z_range })
    }

    fn keep(mut self, f: impl Fn(&Element) -> bool) -> Self {
        self.items.retain(f);
        self
    }

    /// Elements that lie in the horizontal plane `z ± tol`.
    fn at_z(self, z: f64, tol: f64) -> Self {
        self.keep(|e| e.min[2] >= z - tol && e.max[2] <= z + tol)
    }

    fn edges_only(&self, filter: &str) -> Rr<()> {
        if self.faces {
            Err(runtime(format!("{filter}() filters edges, not faces")))
        } else {
            Ok(())
        }
    }

    fn indices(&self) -> Vec<u32> {
        self.items.iter().map(|e| e.index).collect()
    }

    /// Elements whose smallest distance to `p` is at most `r`.
    fn near(mut self, p: [f64; 3], r: f64) -> Rr<Self> {
        let distances =
            ffi::qoob3d_element_distances(self.shape.get(), self.faces, &self.indices(), &p)
                .map_err(|e| geometry("near", e.what()))?;
        let near = self.items.into_iter().zip(distances).filter(|(_, d)| *d <= r + TOL_MM);
        self.items = near.map(|(e, _)| e).collect();
        Ok(self)
    }

    /// Sum of the lengths (edges) or areas (faces); `function` names which.
    fn total(&self, function: &'static str, faces: bool) -> Rr<f64> {
        if self.faces != faces {
            let (what, other) = if faces { ("faces", "edges") } else { ("edges", "faces") };
            return Err(runtime(format!("{function}() sums {what}, not {other}")));
        }
        let sizes = ffi::qoob3d_element_sizes(self.shape.get(), self.faces, &self.indices())
            .map_err(|e| geometry(function, e.what()))?;
        Ok(sizes.iter().sum())
    }
}

fn describe(e: &Element) -> String {
    let kind = match e.kind {
        LINE => "line",
        CIRCLE => "circle",
        _ => "curve",
    };
    let p = |v: [f64; 3]| format!("({:.4}, {:.4}, {:.4})", v[0], v[1], v[2]);
    format!("{kind} within {}..{}", p(e.min), p(e.max))
}

/// Checks shared by fillet, chamfer and shell.
fn checked(function: &str, s: &Shape, sel: &Selection, faces: bool) -> Rr<()> {
    let what = if faces { "faces" } else { "edges" };
    if sel.faces != faces {
        return Err(runtime(format!("{function} needs {what}, use {what}(s)")));
    }
    if !sel.shape.same(s) {
        return Err(runtime(format!("{function}: the {what} belong to another shape")));
    }
    if sel.items.is_empty() && !faces {
        return Err(runtime(format!("{function}: no {what} selected")));
    }
    Ok(())
}

/// Fillet or chamfer; a failure names the first edge that fails alone (M0).
fn edge_feature(function: &'static str, s: &Shape, sel: &Selection, size: f64) -> Rr<Shape> {
    checked(function, s, sel, false)?;
    let chamfer = function == "chamfer";
    match ffi::qoob3d_fillet(s.get(), &sel.indices(), size, chamfer) {
        Ok(r) => plausible(function, made(function, Ok(r))?, Expect::Material),
        Err(all) => {
            let n = sel.items.len();
            let first = sel.items.iter().enumerate().find_map(|(k, e)| {
                ffi::qoob3d_fillet(s.get(), &[e.index], size, chamfer).err().map(|err| {
                    format!(
                        "size {size} fails on edge {} of {n}, {}: {}",
                        k + 1,
                        describe(e),
                        err.what()
                    )
                })
            });
            let message = first.unwrap_or_else(|| {
                format!("each of the {n} edges works alone, together they fail: {}", all.what())
            });
            Err(geometry(function, message))
        }
    }
}

fn shell(s: &Shape, thickness: f64, open: &Selection) -> Rr<Shape> {
    checked("shell", s, open, true)?;
    let result = made("shell", ffi::qoob3d_shell(s.get(), &open.indices(), thickness))?;
    plausible("shell", result, Expect::Less(s.volume()))
}

pub(super) fn register(engine: &mut Engine) {
    engine.register_type_with_name::<Selection>("Selection");
    engine.register_fn("edges", |s: Shape| Selection::of(&s, false));
    engine.register_fn("all_edges", |s: Shape| Selection::of(&s, false));
    engine.register_fn("faces", |s: Shape| Selection::of(&s, true));
    engine.register_fn("parallel", |sel: Selection, ax: Dynamic| -> Rr<Selection> {
        sel.edges_only("parallel")?;
        let a = axis(&ax)?;
        let dot = |d: [f64; 3]| (d[0] * a[0] + d[1] * a[1] + d[2] * a[2]).abs();
        Ok(sel.keep(|e| e.kind == LINE && dot(e.dir) > 1.0 - 1e-9))
    });
    engine.register_fn("lines", |sel: Selection| -> Rr<Selection> {
        sel.edges_only("lines")?;
        Ok(sel.keep(|e| e.kind == LINE))
    });
    engine.register_fn("circles", |sel: Selection| -> Rr<Selection> {
        sel.edges_only("circles")?;
        Ok(sel.keep(|e| e.kind == CIRCLE))
    });
    engine.register_fn("top", |sel: Selection| {
        let z = sel.z_range[1];
        sel.at_z(z, TOL_MM)
    });
    engine.register_fn("bottom", |sel: Selection| {
        let z = sel.z_range[0];
        sel.at_z(z, TOL_MM)
    });
    engine.register_fn("at_z", |sel: Selection, z: Dynamic, tol: Dynamic| -> Rr<Selection> {
        let z = num(&z, "at_z: z")?;
        let tol = num(&tol, "at_z: tol")?;
        if tol.is_nan() || tol < 0.0 {
            return Err(runtime(format!("at_z: tol must be zero or positive, got {tol}")));
        }
        Ok(sel.at_z(z, tol))
    });
    engine.register_fn("in_box", |sel: Selection, min: Dynamic, max: Dynamic| -> Rr<Selection> {
        let (lo, hi) = (point(&min, "in_box: min")?, point(&max, "in_box: max")?);
        Ok(sel.keep(|e| (0..3).all(|i| e.min[i] >= lo[i] - TOL_MM && e.max[i] <= hi[i] + TOL_MM)))
    });
    engine.register_fn("near", |sel: Selection, p: Dynamic, r: Dynamic| -> Rr<Selection> {
        sel.near(point(&p, "near: point")?, non_negative(&r, "near: r")?)
    });
    engine.register_fn("len", |sel: Selection| sel.items.len() as i64);
    engine.register_fn("length", |sel: Selection| sel.total("length", false));
    engine.register_fn("area", |sel: Selection| sel.total("area", true));
    engine.register_fn("fillet", |s: Shape, sel: Selection, r: Dynamic| -> Rr<Shape> {
        edge_feature("fillet", &s, &sel, positive(&r, "fillet: r")?)
    });
    engine.register_fn("chamfer", |s: Shape, sel: Selection, d: Dynamic| -> Rr<Shape> {
        edge_feature("chamfer", &s, &sel, positive(&d, "chamfer: d")?)
    });
    engine.register_fn("shell", |s: Shape, t: Dynamic, open: Selection| -> Rr<Shape> {
        shell(&s, positive(&t, "shell: thickness")?, &open)
    });
}
