//! 2D profiles, extrude/revolve/loft and pipe of the script API (AG-SC-07,
//! AG-SC-08). An outline keeps its curves and plane; the face is made by the
//! sketch region code (`qoob3d_profile_faces`, ADR-0028) when it is used.
//! Profile algebra (AG-PF-02) combines such faces in the plane z = 0.

use std::f64::consts::{FRAC_PI_2, PI};
use std::rc::Rc;

use qoob3d_core::document::{OriginPlane, PlaneFrame};
use rhai::{Array, Dynamic, Engine};

use super::api::{
    COMMON, CUT, FUSE, Rr, TRANSLATE, finite, geometry, made, num, occt, point, positive, runtime,
};
use super::{Shape, compound};
use crate::occt::ffi;

/// `qoob3d_transform`: from the XY plane into a plane frame.
const PLACE: u32 = 4;

/// A region in the local XY plane of `frame`.
#[derive(Clone)]
pub(super) struct Profile {
    source: Source,
    frame: PlaneFrame,
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
        Self { source, frame: OriginPlane::XY.frame() }
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
        let r = ffi::qoob3d_profile_faces(
            c,
            &keys,
            &[0, keys.len() as u32],
            &OriginPlane::XY.frame().to_array(),
        );
        if r.as_ref().is_err_and(|e| e.what().contains("profile not found")) {
            return Err(geometry(function, "the profile is not one simple closed outline"));
        }
        made(function, r)
    }

    /// The faces of the profile, placed by `frame`.
    fn face(&self, function: &'static str, frame: &PlaneFrame) -> Rr<Shape> {
        let local = self.local(function)?;
        made(function, ffi::qoob3d_transform(local.get(), PLACE, &frame.to_array()))
    }

    /// The profile shifted by `d` in its plane.
    fn moved(&self, d: [f64; 2]) -> Rr<Self> {
        let (c, simple) = match &self.source {
            Source::Outline { curves, simple } => (curves, *simple),
            Source::Faces(faces) => {
                let r = ffi::qoob3d_transform(faces.get(), TRANSLATE, &[d[0], d[1], 0.0]);
                return Ok(Self { source: Source::Faces(made("move", r)?), frame: self.frame });
            }
        };
        // Lines are x y pairs; circles and arcs start with their centre.
        let shift = |values: &[f64], stride: usize| {
            let mut out = values.to_vec();
            for item in out.chunks_exact_mut(stride) {
                item[0] += d[0];
                item[1] += d[1];
            }
            out
        };
        let moved = Self::new(shift(&c.lines, 2), shift(&c.circles, 3), shift(&c.arcs, 5), simple);
        Ok(Self { frame: self.frame, ..moved })
    }
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

fn plane(name: &str) -> Rr<OriginPlane> {
    match name {
        "xy" => Ok(OriginPlane::XY),
        "xz" => Ok(OriginPlane::XZ),
        "yz" => Ok(OriginPlane::YZ),
        other => Err(runtime(format!("plane must be \"xy\", \"xz\" or \"yz\", got {other:?}"))),
    }
}

fn extrude(p: &Profile, h: f64) -> Rr<Shape> {
    let face = p.face("extrude", &p.frame)?;
    let n = p.frame.n;
    made("extrude", ffi::qoob3d_extrude(face.get(), &n.map(|c| c * h), false))
}

/// The profile's (x, y) become (radius, height): drawn in XZ, turned about Z.
fn revolve(p: &Profile, deg: f64) -> Rr<Shape> {
    let face = p.face("revolve", &OriginPlane::XZ.frame())?;
    let angle = if deg >= 360.0 { 2.0 * PI } else { deg.to_radians() };
    made("revolve", ffi::qoob3d_revolve(face.get(), &[0.0, 0.0, 0.0, 0.0, 0.0, 1.0], angle))
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
    engine.register_fn("polygon", |points: Array| polygon(&points));
    engine.register_fn("slot", |l: Dynamic, w: Dynamic| -> Rr<Profile> {
        let (l, w) = (positive(&l, "slot: l")?, positive(&w, "slot: w")?);
        if l <= w {
            return Err(runtime(format!("slot: l ({l}) must be larger than w ({w})")));
        }
        Ok(slot(l, w))
    });
    engine.register_fn("on", |name: &str, p: Profile| -> Rr<Profile> {
        Ok(Profile { frame: plane(name)?.frame(), ..p })
    });
    engine.register_fn("at_z", |p: Profile, z: Dynamic| -> Rr<Profile> {
        let mut frame = p.frame;
        frame.origin[2] += finite(&z, "at_z: z")?;
        Ok(Profile { frame, ..p })
    });
    engine.register_fn("move", |p: Profile, x: Dynamic, y: Dynamic| -> Rr<Profile> {
        p.moved([finite(&x, "move: x")?, finite(&y, "move: y")?])
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
        revolve(&p, deg)
    });
    engine.register_fn("loft", |sections: Array| loft(&sections));
    engine.register_fn("pipe", |points: Array, d: Dynamic, bend: Dynamic| -> Rr<Shape> {
        pipe(&points, positive(&d, "pipe: d")?, num(&bend, "pipe: bend_radius")?)
    });
}
