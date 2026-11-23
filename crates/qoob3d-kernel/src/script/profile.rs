//! 2D profiles, extrude/revolve/loft and pipe of the script API (AG-SC-07,
//! AG-SC-08, AG-SC-17), sections (AG-SC-21). An outline keeps its curves and plane; the face is made by the
//! sketch region code (`qoob3d_profile_faces`, ADR-0028) when it is used.
//! Profile algebra (AG-PF-02) combines such faces in the plane z = 0.

use std::f64::consts::{FRAC_PI_2, PI};
use std::rc::Rc;

use rhai::{Array, Dynamic, Engine};

use super::api::{
    COMMON, CUT, FUSE, ROTATE, Rr, TRANSLATE, finite, geometry, made, num, occt, point, positive,
    runtime,
};
use super::{Shape, compound};
use crate::occt::{ffi, occ};

/// `qoob3d_transform`: from the XY plane into a plane frame.
const PLACE: u32 = 4;
/// Profiles with less area are empty, mm².
const MIN_AREA_MM2: f64 = 1e-9;

/// Right-handed local frame of a profile plane: `n = u × v`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    origin: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    n: [f64; 3],
}

impl Frame {
    // The origin planes, with the local axes of the ViewCube faces TOP,
    // FRONT and RIGHT, so a profile is never mirrored in its standard view
    // (ADR-0006).
    const XY: Self = Self::axes([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    const XZ: Self = Self::axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]);
    const YZ: Self = Self::axes([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);

    const fn axes(u: [f64; 3], v: [f64; 3], n: [f64; 3]) -> Self {
        Self { origin: [0.0; 3], u, v, n }
    }

    /// Flat form for FFI: origin, u, v, n.
    fn to_array(self) -> [f64; 12] {
        let mut out = [0.0; 12];
        for (i, v) in [self.origin, self.u, self.v, self.n].iter().enumerate() {
            out[3 * i..3 * i + 3].copy_from_slice(v);
        }
        out
    }
}

/// A region in the local XY plane of `frame`.
#[derive(Clone)]
pub(super) struct Profile {
    source: Source,
    frame: Frame,
}

#[derive(Clone)]
enum Source {
    /// One closed outline; `simple` if it cannot intersect itself (every
    /// outline but `polygon`), so its face needs no region search.
    Outline { curves: Rc<ffi::Qoob3dCurves>, simple: bool },
    /// Result of a profile operation: planar faces in the plane z = 0, with
    /// holes and several regions (AG-PF-02).
    Faces(Shape),
}

impl Profile {
    fn new(lines: Vec<f64>, circles: Vec<f64>, arcs: Vec<f64>, simple: bool) -> Self {
        // Curve ids are consecutive: lines, circles, arcs.
        let ids = |from: usize, n: usize| (from as u32..(from + n) as u32).collect::<Vec<_>>();
        let (nl, nc, na) = (lines.len() / 4, circles.len() / 3, arcs.len() / 5);
        let curves = ffi::Qoob3dCurves {
            line_ids: ids(0, nl),
            circle_ids: ids(nl, nc),
            arc_ids: ids(nl + nc, na),
            lines,
            circles,
            arcs,
        };
        let source = Source::Outline { curves: Rc::new(curves), simple };
        Self { source, frame: Frame::XY }
    }

    /// The faces of the profile in the plane z = 0.
    fn local(&self, function: &'static str) -> Rr<Shape> {
        let c = match &self.source {
            Source::Outline { curves, simple: true } => {
                return made(function, ffi::qoob3d_outline_face(curves));
            }
            Source::Outline { curves, simple: false } => curves,
            Source::Faces(faces) => return Ok(faces.clone()),
        };
        let keys: Vec<u32> =
            c.line_ids.iter().chain(&c.circle_ids).chain(&c.arc_ids).copied().collect();
        let r = ffi::qoob3d_profile_faces(c, &keys, &[0, keys.len() as u32], &Frame::XY.to_array());
        if r.as_ref().is_err_and(|e| e.what().contains("profile not found")) {
            return Err(geometry(function, "the profile is not one simple closed outline"));
        }
        made(function, r)
    }

    /// The faces of the profile, placed by `frame`.
    fn face(&self, function: &'static str, frame: &Frame) -> Rr<Shape> {
        let local = self.local(function)?;
        made(function, ffi::qoob3d_transform(local.get(), PLACE, &frame.to_array()))
    }

    /// The profile after a rigid motion in its plane: `map` moves a point,
    /// arcs turn by `turn` (rad). `kind` and `params` say the same to
    /// `qoob3d_transform`.
    fn transformed(
        &self,
        function: &'static str,
        (kind, params): (u32, &[f64]),
        map: impl Fn([f64; 2]) -> [f64; 2],
        turn: f64,
    ) -> Rr<Self> {
        let (c, simple) = match &self.source {
            Source::Outline { curves, simple } => (curves, *simple),
            Source::Faces(faces) => {
                let r = ffi::qoob3d_transform(faces.get(), kind, params);
                return Ok(Self { source: Source::Faces(made(function, r)?), frame: self.frame });
            }
        };
        // Lines are x y pairs; circles and arcs start with their centre.
        let mapped = |values: &[f64], stride: usize| {
            let mut out = values.to_vec();
            for item in out.chunks_exact_mut(stride) {
                [item[0], item[1]] = map([item[0], item[1]]);
            }
            out
        };
        let mut arcs = mapped(&c.arcs, 5);
        for arc in arcs.as_chunks_mut::<5>().0 {
            arc[3] += turn;
            arc[4] += turn;
        }
        let moved = Self::new(mapped(&c.lines, 2), mapped(&c.circles, 3), arcs, simple);
        Ok(Self { frame: self.frame, ..moved })
    }

    /// The profile shifted by `d` in its plane.
    fn moved(&self, d: [f64; 2]) -> Rr<Self> {
        let shift = |[x, y]: [f64; 2]| [x + d[0], y + d[1]];
        self.transformed("move", (TRANSLATE, &[d[0], d[1], 0.0]), shift, 0.0)
    }

    /// The profile turned counter-clockwise about its origin.
    fn rotated(&self, deg: f64) -> Rr<Self> {
        let angle = deg.to_radians();
        let (sin, cos) = angle.sin_cos();
        let turn = |[x, y]: [f64; 2]| [x * cos - y * sin, x * sin + y * cos];
        self.transformed("rotate", (ROTATE, &[0.0, 0.0, 1.0, angle]), turn, angle)
    }

    /// Area in mm².
    fn area(&self, function: &'static str) -> Rr<f64> {
        let faces = self.local(function)?;
        occt(function, occ(ffi::qoob3d_area(faces.get())))
    }
}

/// The region within `d` of the profile (`d` > 0) or its points deeper than
/// −`d` inside (AG-SC-17). OCCT may answer a vanished profile with an empty
/// or a degenerate outline: the area must be there and change with `d`.
fn offset(p: &Profile, d: f64) -> Rr<Profile> {
    if d == 0.0 {
        return Ok(p.clone());
    }
    let before = p.area("offset")?;
    let local = p.local("offset")?;
    let faces = made("offset", ffi::qoob3d_profile_offset(local.get(), d))?;
    let result = Profile { source: Source::Faces(faces), frame: p.frame };
    let after = result.area("offset")?;
    let changed = if d > 0.0 { after > before } else { after < before };
    if after.is_nan() || after <= MIN_AREA_MM2 {
        Err(geometry("offset", format!("nothing is left of the profile ({after} mm²)")))
    } else if changed {
        Ok(result)
    } else {
        Err(geometry("offset", format!("implausible area ({before} -> {after} mm²)")))
    }
}

/// The cut of `s` with an origin plane moved to the world coordinate `at`
/// along the axis the plane name leaves out (AG-SC-21).
fn section(s: &Shape, name: &str, at: f64) -> Rr<Profile> {
    let mut frame = plane(name)?;
    frame.origin = frame.n.map(|c| c.abs() * at);
    let faces = made("section", ffi::qoob3d_section(s.get(), &frame.to_array()))?;
    Ok(Profile { source: Source::Faces(faces), frame })
}

/// `first` combined with all of `rest` as one planar operation (AG-PF-02).
fn combine(function: &'static str, first: &Profile, rest: &[Profile], op: u32) -> Rr<Profile> {
    if rest.iter().any(|p| p.frame != first.frame) {
        return Err(runtime(format!("{function}: the profiles must lie in the same plane")));
    }
    let a = first.local(function)?;
    let tools = rest.iter().map(|p| p.local(function)).collect::<Rr<Vec<_>>>()?;
    let b = occt(function, compound(&tools))?;
    let faces = made(function, ffi::qoob3d_profile_boolean(a.get(), b.get(), op))?;
    Ok(Profile { source: Source::Faces(faces), frame: first.frame })
}

fn profiles_of(items: &Array, what: &str) -> Rr<Vec<Profile>> {
    items
        .iter()
        .map(|v| {
            v.clone()
                .try_cast::<Profile>()
                .ok_or_else(|| runtime(format!("{what} must be a profile, got {}", v.type_name())))
        })
        .collect()
}

fn profile_union(parts: &[Profile]) -> Rr<Profile> {
    match parts.split_first() {
        None => Err(runtime("profile_union needs at least one profile")),
        Some((first, [])) => Ok(first.clone()),
        Some((first, rest)) => combine("profile_union", first, rest, FUSE),
    }
}

fn profile_cut(p: &Profile, tools: &[Profile]) -> Rr<Profile> {
    if tools.is_empty() {
        return Err(runtime("profile_cut needs at least one tool"));
    }
    combine("profile_cut", p, tools, CUT)
}

fn rect(w: f64, h: f64) -> Profile {
    let (x, y) = (w / 2.0, h / 2.0);
    let corners = [[-x, -y], [x, -y], [x, y], [-x, y]];
    Profile::new(outline(&corners), Vec::new(), Vec::new(), true)
}

/// Rectangle with corners rounded by quarter circles of radius `r`.
fn rounded_rect(w: f64, h: f64, r: f64) -> Profile {
    // (x, y): centre of the corner arc in the first quadrant.
    let (x, y) = (w / 2.0 - r, h / 2.0 - r);
    let (right, top) = (w / 2.0, h / 2.0);
    let sides =
        [[-x, -top, x, -top], [right, -y, right, y], [x, top, -x, top], [-right, y, -right, -y]];
    // With 2 r = w or h a pair of sides has no length.
    let lines = sides.iter().filter(|s| s[0] != s[2] || s[1] != s[3]).flatten().copied().collect();
    let arcs = [[x, -y], [x, y], [-x, y], [-x, -y]]
        .iter()
        .zip([-1.0, 0.0, 1.0, 2.0])
        .flat_map(|(c, k)| [c[0], c[1], r, k * FRAC_PI_2, (k + 1.0) * FRAC_PI_2])
        .collect();
    Profile::new(lines, Vec::new(), arcs, true)
}

/// Closed polyline through `points` as line segments `x0 y0 x1 y1`.
fn outline(points: &[[f64; 2]]) -> Vec<f64> {
    let n = points.len();
    (0..n)
        .flat_map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            [a[0], a[1], b[0], b[1]]
        })
        .collect()
}

/// Stadium of overall length `l` and width `w`, long axis along X.
fn slot(l: f64, w: f64) -> Profile {
    let (c, r) = ((l - w) / 2.0, w / 2.0);
    let lines = vec![-c, -r, c, -r, c, r, -c, r];
    let arcs = vec![c, 0.0, r, -FRAC_PI_2, FRAC_PI_2, -c, 0.0, r, FRAC_PI_2, 3.0 * FRAC_PI_2];
    Profile::new(lines, Vec::new(), arcs, true)
}

fn polygon(points: &Array) -> Rr<Profile> {
    let corners = points
        .iter()
        .map(|p| match p.read_lock::<Array>() {
            Some(xy) if xy.len() == 2 => {
                Ok([finite(&xy[0], "polygon: x")?, finite(&xy[1], "polygon: y")?])
            }
            _ => Err(runtime("polygon: every point must be [x, y]")),
        })
        .collect::<Rr<Vec<_>>>()?;
    if corners.len() < 3 {
        return Err(runtime(format!("polygon needs at least 3 points, got {}", corners.len())));
    }
    let n = corners.len();
    if (0..n).any(|i| corners[i] == corners[(i + 1) % n]) {
        return Err(runtime("polygon: two consecutive points are equal"));
    }
    Ok(Profile::new(outline(&corners), Vec::new(), Vec::new(), false))
}

fn plane(name: &str) -> Rr<Frame> {
    match name {
        "xy" => Ok(Frame::XY),
        "xz" => Ok(Frame::XZ),
        "yz" => Ok(Frame::YZ),
        other => Err(runtime(format!("plane must be \"xy\", \"xz\" or \"yz\", got {other:?}"))),
    }
}

fn extrude(p: &Profile, h: f64) -> Rr<Shape> {
    let face = p.face("extrude", &p.frame)?;
    let n = p.frame.n;
    made("extrude", ffi::qoob3d_extrude(face.get(), &n.map(|c| c * h), false))
}

/// The profile's (x, y) become (radius, height): drawn in XZ, turned about Z.
fn revolve(function: &'static str, p: &Profile, deg: f64) -> Rr<Shape> {
    let face = p.face(function, &Frame::XZ)?;
    let angle = if deg >= 360.0 { 2.0 * PI } else { deg.to_radians() };
    made(function, ffi::qoob3d_revolve(face.get(), &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0], angle))
}

/// Solid of revolution about Z of the closed polyline `points` (radius,
/// height), which runs counter-clockwise and does not cross itself.
pub(super) fn turned(function: &'static str, points: &[[f64; 2]]) -> Rr<Shape> {
    revolve(function, &Profile::new(outline(points), Vec::new(), Vec::new(), true), 360.0)
}

fn loft(sections: &Array) -> Rr<Shape> {
    let profiles = profiles_of(sections, "loft: every section")?;
    // The loft runs through outer wires only: holes would be lost silently.
    if profiles.iter().any(|p| matches!(p.source, Source::Faces(_))) {
        return Err(runtime("loft: every section must be one outline, not a combined profile"));
    }
    if profiles.len() < 2 {
        return Err(runtime(format!("loft needs at least 2 profiles, got {}", profiles.len())));
    }
    let faces = profiles.iter().map(|p| p.face("loft", &p.frame)).collect::<Rr<Vec<_>>>()?;
    let all = occt("loft", compound(&faces))?;
    made("loft", ffi::qoob3d_loft(all.get(), &[], &[]))
}

fn pipe(points: &Array, d: f64, bend: f64) -> Rr<Shape> {
    if points.len() < 2 {
        return Err(runtime(format!("pipe needs at least 2 points, got {}", points.len())));
    }
    // M0: the kernel does not notice a self-intersecting bend.
    if bend.is_nan() || bend <= d / 2.0 {
        return Err(runtime(format!(
            "pipe: bend_radius {bend} must be larger than d/2 = {}",
            d / 2.0
        )));
    }
    let flat = points.iter().map(|p| point(p, "pipe: point")).collect::<Rr<Vec<_>>>()?.concat();
    made("pipe", ffi::qoob3d_pipe(&flat, d / 2.0, bend))
}

pub(super) fn register(engine: &mut Engine) {
    engine.register_type_with_name::<Profile>("Profile");
    engine.register_fn("rect", |w: Dynamic, h: Dynamic| -> Rr<Profile> {
        Ok(rect(positive(&w, "rect: w")?, positive(&h, "rect: h")?))
    });
    engine.register_fn("circle", |d: Dynamic| -> Rr<Profile> {
        let r = positive(&d, "circle: d")? / 2.0;
        Ok(Profile::new(Vec::new(), vec![0.0, 0.0, r], Vec::new(), true))
    });
    engine.register_fn("rounded_rect", |w: Dynamic, h: Dynamic, r: Dynamic| -> Rr<Profile> {
        let (w, h) = (positive(&w, "rounded_rect: w")?, positive(&h, "rounded_rect: h")?);
        let r = positive(&r, "rounded_rect: r")?;
        if 2.0 * r > w.min(h) {
            return Err(runtime(format!(
                "rounded_rect: r ({r}) must be at most half the shorter side ({})",
                w.min(h) / 2.0
            )));
        }
        Ok(rounded_rect(w, h, r))
    });
    engine.register_fn("polygon", |points: Array| polygon(&points));
    engine.register_fn("slot", |l: Dynamic, w: Dynamic| -> Rr<Profile> {
        let (l, w) = (positive(&l, "slot: l")?, positive(&w, "slot: w")?);
        if l <= w {
            return Err(runtime(format!("slot: l ({l}) must be larger than w ({w})")));
        }
        Ok(slot(l, w))
    });
    engine.register_fn("on", |name: &str, p: Profile| -> Rr<Profile> {
        Ok(Profile { frame: plane(name)?, ..p })
    });
    engine.register_fn("at_z", |p: Profile, z: Dynamic| -> Rr<Profile> {
        let mut frame = p.frame;
        frame.origin[2] += finite(&z, "at_z: z")?;
        Ok(Profile { frame, ..p })
    });
    engine.register_fn("move", |p: Profile, x: Dynamic, y: Dynamic| -> Rr<Profile> {
        p.moved([finite(&x, "move: x")?, finite(&y, "move: y")?])
    });
    engine.register_fn("rotate", |p: Profile, deg: Dynamic| -> Rr<Profile> {
        p.rotated(finite(&deg, "rotate: deg")?)
    });
    engine.register_fn("offset", |p: Profile, d: Dynamic| -> Rr<Profile> {
        offset(&p, finite(&d, "offset: d")?)
    });
    engine.register_fn("area", |p: Profile| p.area("area"));
    engine.register_fn("section", |s: Shape, name: &str, at: Dynamic| -> Rr<Profile> {
        section(&s, name, finite(&at, "section: offset")?)
    });
    engine.register_fn("+", |a: Profile, b: Profile| profile_union(&[a, b]));
    engine.register_fn("-", |a: Profile, b: Profile| profile_cut(&a, &[b]));
    engine.register_fn("&", |a: Profile, b: Profile| combine("profile_common", &a, &[b], COMMON));
    engine.register_fn("profile_union", |parts: Array| {
        profile_union(&profiles_of(&parts, "profile_union: element")?)
    });
    engine.register_fn("profile_cut", |p: Profile, tools: Array| {
        profile_cut(&p, &profiles_of(&tools, "profile_cut: tool")?)
    });
    engine.register_fn("extrude", |p: Profile, h: Dynamic| -> Rr<Shape> {
        let h = finite(&h, "extrude: h")?;
        if h == 0.0 {
            return Err(runtime("extrude: h must not be 0"));
        }
        extrude(&p, h)
    });
    engine.register_fn("revolve", |p: Profile, deg: Dynamic| -> Rr<Shape> {
        let deg = num(&deg, "revolve: deg")?;
        if deg.is_nan() || deg <= 0.0 || deg > 360.0 {
            return Err(runtime(format!("revolve: deg must be in (0, 360], got {deg}")));
        }
        revolve("revolve", &p, deg)
    });
    engine.register_fn("loft", |sections: Array| loft(&sections));
    engine.register_fn("pipe", |points: Array, d: Dynamic, bend: Dynamic| -> Rr<Shape> {
        pipe(&points, positive(&d, "pipe: d")?, num(&bend, "pipe: bend_radius")?)
    });
}
