//! The CAD functions of the script API (AG-SC-01…14, 18, 21, 22, `docs/script-api.md`):
//! argument checks, placement and the plausibility rule AG-SC-12 in Rust,
//! geometry in OCCT. Numbers may be ints or floats wherever a number is
//! expected; lengths are mm, angles degrees.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use cxx::UniquePtr;
use rhai::{Array, Dynamic, Engine, EvalAltResult, Map, Position};

use super::{RunOutput, Shape, compound, profile, select};
use crate::KernelError;
use crate::occt::ffi;
use crate::occt::{self, occ};

pub(super) type Rr<T> = Result<T, Box<EvalAltResult>>;

/// Payload of a `Geometry` error (AG-SC-03): the API function and why.
#[derive(Debug, Clone)]
pub(super) struct GeometryFault {
    pub(super) function: &'static str,
    pub(super) message: String,
}

impl std::fmt::Display for GeometryFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed: {}", self.function, self.message)
    }
}

/// Results with less volume are empty (AG-SC-12), mm³.
const MIN_VOLUME_MM3: f64 = 1e-6;
/// Relative volume change below which a cut "removed nothing" (AG-SC-12).
const VOLUME_REL_TOL: f64 = 1e-9;

pub(super) const FUSE: u32 = 0;
pub(super) const CUT: u32 = 1;
pub(super) const COMMON: u32 = 2;

/// `kind` of `qoob3d_primitive`.
const BOX: u32 = 0;
const CYLINDER: u32 = 1;
const SPHERE: u32 = 2;
const CONE: u32 = 3;
const TORUS: u32 = 4;

pub(super) const TRANSLATE: u32 = 0;
pub(super) const ROTATE: u32 = 1;
const MIRROR: u32 = 2;
const SCALE: u32 = 3;

/// A `Runtime` error at the position of the calling expression.
pub(super) fn runtime(message: impl Into<String>) -> Box<EvalAltResult> {
    EvalAltResult::ErrorRuntime(message.into().into(), Position::NONE).into()
}

/// A `Geometry` error naming the API function.
pub(super) fn geometry(function: &'static str, message: impl ToString) -> Box<EvalAltResult> {
    let fault = GeometryFault { function, message: message.to_string() };
    EvalAltResult::ErrorRuntime(Dynamic::from(fault), Position::NONE).into()
}

pub(super) fn made(
    function: &'static str,
    r: Result<UniquePtr<ffi::TopoDS_Shape>, cxx::Exception>,
) -> Rr<Shape> {
    occt::shape(r).map(Shape::new).map_err(|e| geometry(function, e))
}

pub(super) fn occt<T>(function: &'static str, r: Result<T, KernelError>) -> Rr<T> {
    r.map_err(|e| geometry(function, e))
}

pub(super) fn num(v: &Dynamic, what: &str) -> Rr<f64> {
    if let Ok(f) = v.as_float() {
        return Ok(f);
    }
    v.as_int().map(|i| i as f64).map_err(|t| runtime(format!("{what} must be a number, got {t}")))
}

pub(super) fn finite(v: &Dynamic, what: &str) -> Rr<f64> {
    let x = num(v, what)?;
    if x.is_finite() { Ok(x) } else { Err(runtime(format!("{what} must be finite, got {x}"))) }
}

pub(super) fn positive(v: &Dynamic, what: &str) -> Rr<f64> {
    let x = num(v, what)?;
    if x > 0.0 && x.is_finite() {
        Ok(x)
    } else {
        Err(runtime(format!("{what} must be a positive number, got {x}")))
    }
}

pub(super) fn non_negative(v: &Dynamic, what: &str) -> Rr<f64> {
    let x = num(v, what)?;
    if x >= 0.0 && x.is_finite() {
        Ok(x)
    } else {
        Err(runtime(format!("{what} must be zero or positive, got {x}")))
    }
}

fn count(v: &Dynamic, what: &str) -> Rr<usize> {
    match v.as_int() {
        Ok(n) if n >= 1 => Ok(n as usize),
        _ => Err(runtime(format!("{what} must be a whole number >= 1, got {v}"))),
    }
}

/// `[x, y, z]` of finite numbers.
pub(super) fn point(v: &Dynamic, what: &str) -> Rr<[f64; 3]> {
    let Some(a) = v.read_lock::<Array>().filter(|a| a.len() == 3) else {
        return Err(runtime(format!("{what} must be an array [x, y, z]")));
    };
    Ok([finite(&a[0], what)?, finite(&a[1], what)?, finite(&a[2], what)?])
}

/// `"x"`, `"y"`, `"z"` or a non-zero vector `[x, y, z]`.
pub(super) fn axis(v: &Dynamic) -> Rr<[f64; 3]> {
    if let Ok(name) = v.clone().into_string() {
        return match name.as_str() {
            "x" => Ok([1.0, 0.0, 0.0]),
            "y" => Ok([0.0, 1.0, 0.0]),
            "z" => Ok([0.0, 0.0, 1.0]),
            other => Err(runtime(format!(
                "axis must be \"x\", \"y\", \"z\" or [x, y, z], got {other:?}"
            ))),
        };
    }
    let d = point(v, "axis")?;
    let length = d.iter().map(|c| c * c).sum::<f64>().sqrt();
    if length > 0.0 {
        Ok(d.map(|c| c / length))
    } else {
        Err(runtime("axis must not be [0, 0, 0]"))
    }
}

fn shape_of(v: &Dynamic, what: &str) -> Rr<Shape> {
    v.clone()
        .try_cast::<Shape>()
        .ok_or_else(|| runtime(format!("{what} must be a shape, got {}", v.type_name())))
}

fn shapes_of(items: &Array, what: &str) -> Rr<Vec<Shape>> {
    items.iter().map(|v| shape_of(v, what)).collect()
}

pub(super) enum Expect {
    /// Volume > [`MIN_VOLUME_MM3`].
    Material,
    /// Less volume than before (cut, shell).
    Less(f64),
    /// At least the largest input (union).
    AtLeast(f64),
    /// A union: at least the largest input, and its box covers the boxes of
    /// all inputs. A union of freeform bodies once dropped a whole input
    /// (a deck lid) while its volume stayed above the largest one.
    Covers { largest: f64, min: [f64; 3], max: [f64; 3] },
}

/// The box around all `shapes` (`None` if one has none).
fn joint_bounds(shapes: &[Shape]) -> Option<([f64; 3], [f64; 3])> {
    shapes.iter().try_fold(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), |(lo, hi), s| {
        let (a, b) = s.bounds().ok()?;
        Some((std::array::from_fn(|i| lo[i].min(a[i])), std::array::from_fn(|i| hi[i].max(b[i]))))
    })
}

/// Whether the box of `result` reaches the box `min`..`max` within a
/// tolerance relative to its size.
fn covers(result: &Shape, min: [f64; 3], max: [f64; 3]) -> Result<(), String> {
    let (lo, hi) = result.bounds().map_err(|e| e.to_string())?;
    let size = (0..3).map(|i| max[i] - min[i]).fold(0.0, f64::max);
    let tol = 1e-6 * size + 1e-3;
    for i in 0..3 {
        if lo[i] > min[i] + tol || hi[i] < max[i] - tol {
            let axis = ["x", "y", "z"][i];
            return Err(format!(
                "the result reaches {axis} {} to {}, its inputs {} to {}: part of an input was lost",
                lo[i], hi[i], min[i], max[i]
            ));
        }
    }
    Ok(())
}

/// The plausibility rule AG-SC-12: OCCT can return "valid" results that are
/// empty or unchanged (M0).
pub(super) fn plausible(function: &'static str, result: Shape, expect: &Expect) -> Rr<Shape> {
    let v = result.volume();
    if v.is_nan() || v <= MIN_VOLUME_MM3 {
        return Err(geometry(function, format!("the result has no volume ({v} mm³)")));
    }
    match *expect {
        Expect::Less(before) if v >= before * (1.0 - VOLUME_REL_TOL) => Err(geometry(
            function,
            format!("removed no material ({before} -> {v} mm³); does the tool overlap the body?"),
        )),
        Expect::AtLeast(largest) | Expect::Covers { largest, .. }
            if v < largest * (1.0 - VOLUME_REL_TOL) =>
        {
            Err(geometry(
                function,
                format!("the result ({v} mm³) is smaller than an input ({largest} mm³)"),
            ))
        }
        Expect::Covers { min, max, .. } => {
            covers(&result, min, max).map_err(|m| geometry(function, m))?;
            Ok(result)
        }
        _ => Ok(result),
    }
}

/// Options of the booleans (`union`, `cut`, `common`).
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct BoolOptions {
    /// Geometry closer than this (mm) counts as touching, 0 is exact;
    /// `None` leaves it to [`boolean`].
    pub(super) fuzzy: Option<f64>,
}

impl BoolOptions {
    fn of(map: &Map, function: &str) -> Rr<Self> {
        let fuzzy = options(map, function, &["fuzzy"])?;
        let fuzzy = match fuzzy.get("fuzzy") {
            Some(v) => Some(non_negative(v, &format!("{function}: fuzzy"))?),
            None => None,
        };
        Ok(Self { fuzzy })
    }
}

/// The keys of an options map, checked against the `known` ones.
pub(super) fn options(map: &Map, function: &str, known: &[&str]) -> Rr<Map> {
    if let Some(key) = map.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(runtime(format!(
            "{function}: unknown option {key:?}; known: {}",
            known.join(", ")
        )));
    }
    Ok(map.clone())
}

/// The bool option `key` of an options map (false when it is missing).
pub(super) fn flag(map: &Map, key: &str, function: &str) -> Rr<bool> {
    match map.get(key) {
        None => Ok(false),
        Some(v) => v
            .as_bool()
            .map_err(|t| runtime(format!("{function}: {key} must be true or false, got {t}"))),
    }
}

/// `fuzzy` for `qoob3d_boolean`: the tolerance of freeform inputs, exact
/// for analytic ones.
pub(super) const AUTO_FUZZY: f64 = -1.0;

/// One BOP, checked by the plausibility rule (AG-SC-12, AG-SC-25). Without
/// a `fuzzy` option, cuts and commons run with the tolerance of freeform
/// inputs and unions exactly (measured on an imported hull, see
/// `freeform_fuzzy` in the C++ wrappers). A failed or implausible result
/// is computed again serially, and a union finally with the fuzzy value;
/// the first error is the one reported.
pub(super) fn boolean(
    function: &'static str,
    a: &Shape,
    b: &Shape,
    op: u32,
    expect: &Expect,
    opts: BoolOptions,
) -> Rr<Shape> {
    let run = |fuzzy: f64, parallel: bool| -> Rr<Shape> {
        let result = made(function, ffi::qoob3d_boolean(a.get(), b.get(), op, fuzzy, parallel))?;
        plausible(function, result, expect)
    };
    let fuzzy = opts.fuzzy.unwrap_or(if op == FUSE { 0.0 } else { AUTO_FUZZY });
    let first = match run(fuzzy, true) {
        Ok(r) => return Ok(r),
        Err(e) => e,
    };
    if let Ok(r) = run(fuzzy, false) {
        return Ok(r);
    }
    if opts.fuzzy.is_none()
        && op == FUSE
        && let Ok(r) = run(AUTO_FUZZY, false)
    {
        return Ok(r);
    }
    Err(first)
}

/// Union of all parts as one OCCT operation (AG-SC-05).
fn fuse(function: &'static str, parts: &[Shape], opts: BoolOptions) -> Rr<Shape> {
    let Some((first, rest)) = parts.split_first() else {
        return Err(runtime(format!("{function} needs at least one shape")));
    };
    if rest.is_empty() {
        return Ok(first.clone());
    }
    let tools = occt(function, compound(rest))?;
    let largest = parts.iter().map(Shape::volume).fold(0.0, f64::max);
    let expect = match joint_bounds(parts) {
        Some((min, max)) => Expect::Covers { largest, min, max },
        None => Expect::AtLeast(largest),
    };
    boolean(function, first, &tools, FUSE, &expect, opts)
}

/// `a` minus all tools as one OCCT operation (AG-SC-05).
fn cut(a: &Shape, tools: &[Shape], opts: BoolOptions) -> Rr<Shape> {
    let tool = match tools {
        [] => return Err(runtime("cut needs at least one tool")),
        [one] => one.clone(),
        many => occt("cut", compound(many))?,
    };
    boolean("cut", a, &tool, CUT, &Expect::Less(a.volume()), opts)
}

fn common(a: &Shape, b: &Shape, opts: BoolOptions) -> Rr<Shape> {
    boolean("common", a, b, COMMON, &Expect::Material, opts)
}

fn transform(function: &'static str, s: &Shape, kind: u32, params: &[f64]) -> Rr<Shape> {
    made(function, ffi::qoob3d_transform(s.get(), kind, params))
}

fn primitive(function: &'static str, kind: u32, dims: &[f64]) -> Rr<Shape> {
    made(function, ffi::qoob3d_primitive(kind, dims))
}

/// Cylinder of diameter `d` from z = −`l` up to 0 (AG-SC-13).
fn downward_cylinder(function: &'static str, d: f64, l: f64) -> Rr<Shape> {
    let c = primitive(function, CYLINDER, &[d / 2.0, l])?;
    transform(function, &c, TRANSLATE, &[0.0, 0.0, -l])
}

pub(super) fn register(
    engine: &mut Engine,
    output: &Rc<RefCell<RunOutput>>,
    project: Option<PathBuf>,
) {
    engine.register_type_with_name::<Shape>("Shape");
    // A geometry error caught with `catch (e)` reads as its message, not as
    // the name of the Rust type (AG-SC-03).
    engine.register_type_with_name::<GeometryFault>("GeometryError");
    engine.register_fn("to_string", |f: &mut GeometryFault| f.to_string());
    engine.register_fn("to_debug", |f: &mut GeometryFault| f.to_string());
    register_output(engine, output, project);
    register_solids(engine);
    register_screw_holes(engine);
    register_booleans(engine);
    register_transforms(engine);
    register_measures(engine);
    register_print_analysis(engine);
    profile::register(engine);
    select::register(engine);
    super::solid_ops::register(engine);
}

fn register_output(engine: &mut Engine, output: &Rc<RefCell<RunOutput>>, project: Option<PathBuf>) {
    let shown = Rc::clone(output);
    engine.register_fn("show", move |s: Dynamic, name: Dynamic| -> Rr<()> {
        let s = shape_of(&s, "show: the first argument")?;
        let name = name
            .into_string()
            .map_err(|t| runtime(format!("show: name must be a string, got {t}")))?;
        shown.borrow_mut().shapes.push((name, s));
        Ok(())
    });
    super::imports::register(engine, output, project.clone());
    super::text::register(engine, output, project);
}

fn register_solids(engine: &mut Engine) {
    engine.register_fn("box", |x: Dynamic, y: Dynamic, z: Dynamic| -> Rr<Shape> {
        let (x, y, z) = (positive(&x, "box: x")?, positive(&y, "box: y")?, positive(&z, "box: z")?);
        primitive("box", BOX, &[-x / 2.0, -y / 2.0, 0.0, x, y, z])
    });
    engine.register_fn("cylinder", |d: Dynamic, h: Dynamic| -> Rr<Shape> {
        let (d, h) = (positive(&d, "cylinder: d")?, positive(&h, "cylinder: h")?);
        primitive("cylinder", CYLINDER, &[d / 2.0, h])
    });
    engine.register_fn("sphere", |d: Dynamic| -> Rr<Shape> {
        primitive("sphere", SPHERE, &[positive(&d, "sphere: d")? / 2.0])
    });
    engine.register_fn("cone", |d1: Dynamic, d2: Dynamic, h: Dynamic| -> Rr<Shape> {
        let (d1, d2) = (non_negative(&d1, "cone: d1")?, non_negative(&d2, "cone: d2")?);
        let h = positive(&h, "cone: h")?;
        if d1 == d2 {
            return Err(runtime("cone: d1 and d2 must differ (equal diameters: cylinder)"));
        }
        primitive("cone", CONE, &[d1 / 2.0, d2 / 2.0, h])
    });
    engine.register_fn("torus", |ring: Dynamic, tube: Dynamic| -> Rr<Shape> {
        let (ring, tube) = (positive(&ring, "torus: d_ring")?, positive(&tube, "torus: d_tube")?);
        if tube >= ring {
            return Err(runtime("torus: d_tube must be smaller than d_ring"));
        }
        primitive("torus", TORUS, &[ring / 2.0, tube / 2.0])
    });
    engine.register_fn("pin", |d: Dynamic, l: Dynamic| -> Rr<Shape> {
        downward_cylinder("pin", positive(&d, "pin: d")?, positive(&l, "pin: l")?)
    });
    engine.register_fn("hole", |d: Dynamic, l: Dynamic, clearance: Dynamic| -> Rr<Shape> {
        let (d, l) = (positive(&d, "hole: d")?, positive(&l, "hole: l")?);
        let c = non_negative(&clearance, "hole: clearance")?;
        downward_cylinder("hole", d + 2.0 * c, l + c)
    });
}

/// `d` and `head_d` of a screw hole tool: the head must be the wider one.
fn hole_and_head(function: &str, d: &Dynamic, head_d: &Dynamic) -> Rr<(f64, f64)> {
    let d = positive(d, &format!("{function}: d"))?;
    let head_d = positive(head_d, &format!("{function}: head_d"))?;
    if head_d > d {
        Ok((d / 2.0, head_d / 2.0))
    } else {
        Err(runtime(format!("{function}: head_d ({head_d}) must be larger than d ({d})")))
    }
}

/// Tool bodies for screw holes (AG-SC-18), downward from z = 0 like `hole`:
/// turned outlines (radius, height), so each is one solid without a boolean.
fn register_screw_holes(engine: &mut Engine) {
    engine.register_fn(
        "countersink",
        |d: Dynamic, head_d: Dynamic, angle: Dynamic, l: Dynamic| -> Rr<Shape> {
            let (r, head) = hole_and_head("countersink", &d, &head_d)?;
            let angle = num(&angle, "countersink: angle_deg")?;
            if !(angle > 0.0 && angle < 180.0) {
                return Err(runtime(format!(
                    "countersink: angle_deg must be between 0 and 180, got {angle}"
                )));
            }
            let l = positive(&l, "countersink: l")?;
            // The wall of the cone leaves the axis by `slope` per mm of depth.
            let slope = (angle / 2.0).to_radians().tan();
            let cone = (head - r) / slope;
            let outline = if cone < l {
                vec![[0.0, -l], [r, -l], [r, -cone], [head, 0.0], [0.0, 0.0]]
            } else {
                // The hole ends inside the cone.
                vec![[0.0, -l], [head - l * slope, -l], [head, 0.0], [0.0, 0.0]]
            };
            profile::turned("countersink", &outline)
        },
    );
    engine.register_fn(
        "counterbore",
        |d: Dynamic, head_d: Dynamic, head_h: Dynamic, l: Dynamic| -> Rr<Shape> {
            let (r, head) = hole_and_head("counterbore", &d, &head_d)?;
            let h = positive(&head_h, "counterbore: head_h")?;
            let l = positive(&l, "counterbore: l")?;
            if h >= l {
                return Err(runtime(format!(
                    "counterbore: head_h ({h}) must be smaller than the depth l ({l})"
                )));
            }
            let outline = [[0.0, -l], [r, -l], [r, -h], [head, -h], [head, 0.0], [0.0, 0.0]];
            profile::turned("counterbore", &outline)
        },
    );
}

fn register_booleans(engine: &mut Engine) {
    let exact = BoolOptions::default();
    engine.register_fn("+", move |a: Shape, b: Shape| fuse("union", &[a, b], exact));
    engine.register_fn("-", move |a: Shape, b: Shape| cut(&a, &[b], exact));
    engine.register_fn("&", move |a: Shape, b: Shape| common(&a, &b, exact));
    engine.register_fn("union", move |parts: Array| {
        fuse("union", &shapes_of(&parts, "union: element")?, exact)
    });
    engine.register_fn("union", |parts: Array, o: Map| -> Rr<Shape> {
        fuse("union", &shapes_of(&parts, "union: element")?, BoolOptions::of(&o, "union")?)
    });
    engine.register_fn("cut", move |a: Shape, tools: Array| {
        cut(&a, &shapes_of(&tools, "cut: tool")?, exact)
    });
    engine.register_fn("cut", move |a: Shape, tool: Shape| cut(&a, &[tool], exact));
    engine.register_fn("cut", |a: Shape, tools: Array, o: Map| -> Rr<Shape> {
        cut(&a, &shapes_of(&tools, "cut: tool")?, BoolOptions::of(&o, "cut")?)
    });
    engine.register_fn("cut", |a: Shape, tool: Shape, o: Map| -> Rr<Shape> {
        cut(&a, &[tool], BoolOptions::of(&o, "cut")?)
    });
    engine.register_fn("common", move |a: Shape, b: Shape| common(&a, &b, exact));
    engine.register_fn("common", |a: Shape, b: Shape, o: Map| -> Rr<Shape> {
        common(&a, &b, BoolOptions::of(&o, "common")?)
    });
    // Several shapes as one without a boolean: they keep their own volumes.
    engine.register_fn("group", |parts: Array| -> Rr<Shape> {
        let shapes = shapes_of(&parts, "group: element")?;
        if shapes.is_empty() {
            return Err(runtime("group needs at least one shape"));
        }
        occt("group", compound(&shapes))
    });
}

fn register_transforms(engine: &mut Engine) {
    engine.register_fn("move", |s: Shape, x: Dynamic, y: Dynamic, z: Dynamic| -> Rr<Shape> {
        let d = [finite(&x, "move: x")?, finite(&y, "move: y")?, finite(&z, "move: z")?];
        transform("move", &s, TRANSLATE, &d)
    });
    engine.register_fn("rotate", |s: Shape, ax: Dynamic, deg: Dynamic| -> Rr<Shape> {
        let [x, y, z] = axis(&ax)?;
        let angle = finite(&deg, "rotate: deg")?.to_radians();
        transform("rotate", &s, ROTATE, &[x, y, z, angle])
    });
    engine.register_fn("mirror", |s: Shape, plane: &str| -> Rr<Shape> {
        let normal = match plane {
            "xy" => [0.0, 0.0, 1.0],
            "xz" => [0.0, 1.0, 0.0],
            "yz" => [1.0, 0.0, 0.0],
            other => {
                return Err(runtime(format!(
                    "mirror: plane must be \"xy\", \"xz\" or \"yz\", got {other:?}"
                )));
            }
        };
        transform("mirror", &s, MIRROR, &normal)
    });
    engine.register_fn("scale", |s: Shape, f: Dynamic| -> Rr<Shape> {
        transform("scale", &s, SCALE, &[positive(&f, "scale: f")?])
    });
    engine.register_fn("linear", |s: Shape, n: Dynamic, step: Dynamic| -> Rr<Shape> {
        let n = count(&n, "linear: n")?;
        let [dx, dy, dz] = point(&step, "linear: step")?;
        let copies = (0..n)
            .map(|k| {
                let k = k as f64;
                transform("linear", &s, TRANSLATE, &[k * dx, k * dy, k * dz])
            })
            .collect::<Rr<Vec<_>>>()?;
        fuse("linear", &copies, BoolOptions::default())
    });
    engine.register_fn("polar", |s: Shape, n: Dynamic, total: Dynamic| -> Rr<Shape> {
        let n = count(&n, "polar: n")?;
        let total = finite(&total, "polar: deg_total")?;
        // A full turn spreads n copies evenly; an arc puts the last copy at its end.
        let full = (total.abs() - 360.0).abs() < 1e-9;
        let step = if full || n == 1 { total / n as f64 } else { total / (n - 1) as f64 };
        let copies = (0..n)
            .map(|k| {
                transform("polar", &s, ROTATE, &[0.0, 0.0, 1.0, (k as f64 * step).to_radians()])
            })
            .collect::<Rr<Vec<_>>>()?;
        fuse("polar", &copies, BoolOptions::default())
    });
}

fn register_measures(engine: &mut Engine) {
    engine.register_fn("volume", |s: Shape| s.volume());
    engine.register_fn("area", |s: Shape| occt("area", occ(ffi::qoob3d_area(s.get()))));
    engine.register_fn("mass", |s: Shape, density: Dynamic| -> Rr<f64> {
        // g/cm³ times mm³, 1000 mm³ = 1 cm³.
        Ok(s.volume() * positive(&density, "mass: density_g_cm3")? / 1000.0)
    });
    engine.register_fn("is_valid", |s: Shape| s.is_valid());
    engine.register_fn("solids", |s: Shape| s.solid_count() as i64);
    engine.register_fn("center", |s: Shape| -> Rr<Array> {
        let c = occt("center", occ(ffi::qoob3d_center(s.get())))?;
        Ok(c.into_iter().map(Dynamic::from_float).collect())
    });
    engine.register_fn("bbox", |s: Shape| -> Rr<Map> {
        let (min, max) = occt("bbox", s.bounds())?;
        let array =
            |v: [f64; 3]| Dynamic::from_array(v.into_iter().map(Dynamic::from_float).collect());
        let mut m = Map::new();
        m.insert("min".into(), array(min));
        m.insert("max".into(), array(max));
        m.insert("size".into(), array(std::array::from_fn(|i| max[i] - min[i])));
        Ok(m)
    });
    engine.register_fn("distance", |a: Shape, b: Shape| {
        occt("distance", occ(ffi::qoob3d_distance(a.get(), b.get(), f64::INFINITY)))
    });
    engine.register_fn("ray", |s: Shape, origin: Dynamic, dir: Dynamic| -> Rr<Array> {
        let origin = point(&origin, "ray: origin")?;
        let dir = axis(&dir)?;
        let hits = occt("ray", occ(ffi::qoob3d_ray(s.get(), &origin, &dir)))?;
        Ok(hits
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| Dynamic::from_array(p.iter().copied().map(Dynamic::from_float).collect()))
            .collect())
    });
}

/// First print analysis (AG-SC-22): the shape stands as modelled, bed at its
/// lowest z.
fn register_print_analysis(engine: &mut Engine) {
    engine.register_fn("overhangs", |s: Shape, limit: Dynamic| -> Rr<f64> {
        let deg = num(&limit, "overhangs: max_angle_deg")?;
        if !(0.0..=90.0).contains(&deg) {
            return Err(runtime(format!("overhangs: max_angle_deg must be in [0, 90], got {deg}")));
        }
        occt("overhangs", occ(ffi::qoob3d_overhang_area(s.get(), deg.to_radians())))
    });
    engine.register_fn("bed_area", |s: Shape| occt("bed_area", occ(ffi::qoob3d_bed_area(s.get()))));
}
