//! Drawing tools as state machines without any UI (SK-QA-01, ADR-0036).
//!
//! The UI feeds [`ToolEvent`]s in sketch coordinates (already snapped) and
//! draws [`Tool::preview`] and [`Tool::fields`]. Values typed into a field
//! lock that dimension (SK-IN-02); the cursor then only drives the rest.
//! Without a solver (M3a) typed values only set the geometry (SK-IN-05).

use glam::DVec2;
use qoob3d_core::document::{EntityId, Sketch};

use crate::snap::Snap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// Line / polyline (L).
    Line,
    /// Two-point rectangle (R).
    Rectangle,
    /// Rectangle from centre and corner.
    CenterRectangle,
    /// Centre–diameter circle (C).
    Circle,
    /// Centre-point arc (A): centre → start (radius) → end (sweep, CCW).
    Arc,
    Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldUnit {
    Length,
    /// Degrees, counter-clockwise from local +u (SK-IN-06).
    Angle,
}

/// A value shown next to the cursor while drawing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    /// Translation key of the label (ADR-0020).
    pub key: &'static str,
    pub unit: FieldUnit,
    pub value: f64,
    pub locked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolEvent<'a> {
    Move(&'a Snap),
    Click(&'a Snap),
    /// Enter: commit with the current (possibly locked) values.
    Commit,
    /// Double-click or closing a loop: end the current polyline.
    EndChain,
    Escape,
    /// A value typed into field `index` (mm or degrees): locks it.
    SetField {
        index: usize,
        value: f64,
    },
    /// Removes the lock from field `index`.
    Unlock(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    /// The sketch was changed (UI: undo snapshot + rebuild).
    Edited,
    /// The tool wants to be closed (Escape while idle).
    Done,
}

/// Dashed preview geometry in sketch coordinates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preview {
    pub segments: Vec<[DVec2; 2]>,
    pub circles: Vec<(DVec2, f64)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Base {
    Existing(EntityId, DVec2),
    New(DVec2),
}

impl Base {
    fn from_snap(s: &Snap) -> Self {
        match s.entity {
            Some(id) => Base::Existing(id, s.point),
            None => Base::New(s.point),
        }
    }

    fn pos(self) -> DVec2 {
        match self {
            Base::Existing(_, p) | Base::New(p) => p,
        }
    }

    fn materialize(self, sketch: &mut Sketch) -> EntityId {
        match self {
            Base::Existing(id, _) => id,
            Base::New(p) => sketch.add_point(p.to_array()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Idle,
    /// First point placed; `chain_start` = first point of a polyline.
    Placed {
        base: Base,
        chain_start: Option<EntityId>,
    },
    /// Arc: centre (`base`) and start placed, choosing the end.
    Sweeping {
        base: Base,
        start: Base,
    },
}

const EPS: f64 = 1e-6;

#[derive(Debug, Clone)]
pub struct Tool {
    kind: ToolKind,
    /// New curves are construction geometry (X toggles it, SK-TL-16).
    pub construction: bool,
    state: State,
    fields: Vec<Field>,
    cursor: DVec2,
    /// Existing point under the cursor (reused if no field is locked).
    cursor_entity: Option<EntityId>,
}

impl Tool {
    pub fn new(kind: ToolKind) -> Self {
        let field = |key, unit| Field { key, unit, value: 0.0, locked: false };
        let fields = match kind {
            ToolKind::Line => vec![
                field("field-length", FieldUnit::Length),
                field("field-angle", FieldUnit::Angle),
            ],
            ToolKind::Rectangle | ToolKind::CenterRectangle => vec![
                field("field-width", FieldUnit::Length),
                field("field-height", FieldUnit::Length),
            ],
            ToolKind::Circle => vec![field("field-diameter", FieldUnit::Length)],
            ToolKind::Arc => vec![
                field("field-radius", FieldUnit::Length),
                field("field-sweep", FieldUnit::Angle),
            ],
            ToolKind::Point => Vec::new(),
        };
        Self {
            kind,
            construction: false,
            state: State::Idle,
            fields,
            cursor: DVec2::ZERO,
            cursor_entity: None,
        }
    }

    pub fn kind(&self) -> ToolKind {
        self.kind
    }

    /// Base point for alignment snapping (horizontal/vertical lock).
    pub fn anchor(&self) -> Option<DVec2> {
        match self.state {
            State::Placed { base, .. } | State::Sweeping { base, .. } => Some(base.pos()),
            State::Idle => None,
        }
    }

    /// True once the first point is placed (fields are shown then).
    pub fn is_placing(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    /// Fields to show next to the cursor (only while placing).
    /// Arc fields come one at a time (radius, then sweep), so the HUD
    /// always types into the dimension that is being chosen.
    pub fn fields(&self) -> &[Field] {
        if self.is_placing() {
            let o = self.field_offset();
            &self.fields[o..o + self.visible_field_count().min(self.fields.len() - o)]
        } else {
            &[]
        }
    }

    fn field_offset(&self) -> usize {
        match (self.kind, self.state) {
            (ToolKind::Arc, State::Placed { .. }) => 0,
            (ToolKind::Arc, State::Sweeping { .. }) => 1,
            _ => 0,
        }
    }

    fn visible_field_count(&self) -> usize {
        match (self.kind, self.state) {
            (ToolKind::Arc, State::Placed { .. }) => 1,
            _ => self.fields.len(),
        }
    }

    fn any_locked(&self) -> bool {
        self.fields.iter().any(|f| f.locked)
    }

    fn unlock_all(&mut self) {
        for f in &mut self.fields {
            f.locked = false;
        }
    }

    pub fn handle(&mut self, sketch: &mut Sketch, event: ToolEvent<'_>) -> Outcome {
        match event {
            ToolEvent::Move(s) => {
                self.track(s);
                self.update_fields();
                Outcome::Nothing
            }
            ToolEvent::Click(s) => {
                self.track(s);
                self.click(sketch, s)
            }
            ToolEvent::Commit => match self.state {
                State::Placed { .. } | State::Sweeping { .. } => self.commit(sketch),
                State::Idle => Outcome::Nothing,
            },
            ToolEvent::EndChain => {
                self.state = State::Idle;
                self.unlock_all();
                Outcome::Nothing
            }
            ToolEvent::Escape => match self.state {
                State::Placed { .. } | State::Sweeping { .. } => {
                    self.state = State::Idle;
                    self.unlock_all();
                    Outcome::Nothing
                }
                State::Idle => Outcome::Done,
            },
            ToolEvent::SetField { index, value } => {
                if index >= self.visible_field_count() {
                    return Outcome::Nothing;
                }
                let index = index + self.field_offset();
                let valid = match self.fields.get(index).map(|f| f.unit) {
                    Some(FieldUnit::Length) => value.is_finite() && value > EPS,
                    Some(FieldUnit::Angle) => value.is_finite(),
                    None => false,
                };
                if valid {
                    self.fields[index].value = value;
                    self.fields[index].locked = true;
                    self.update_fields();
                }
                Outcome::Nothing
            }
            ToolEvent::Unlock(index) => {
                let index = index + self.field_offset();
                if let Some(f) = self.fields.get_mut(index) {
                    f.locked = false;
                }
                self.update_fields();
                Outcome::Nothing
            }
        }
    }

    fn track(&mut self, s: &Snap) {
        self.cursor = s.point;
        self.cursor_entity = s.entity;
    }

    fn click(&mut self, sketch: &mut Sketch, s: &Snap) -> Outcome {
        match self.state {
            State::Idle => {
                if self.kind == ToolKind::Point {
                    if s.entity.is_some() {
                        return Outcome::Nothing; // a point is already there
                    }
                    sketch.add_point(s.point.to_array());
                    return Outcome::Edited;
                }
                self.state = State::Placed { base: Base::from_snap(s), chain_start: None };
                self.update_fields();
                Outcome::Nothing
            }
            State::Placed { .. } | State::Sweeping { .. } => self.commit(sketch),
        }
    }

    /// Arc end for the current cursor / locked sweep (state `Sweeping`).
    fn arc_end(&self, center: DVec2, start: DVec2) -> (DVec2, f64) {
        let r = start.distance(center);
        let a0 = (start - center).to_angle();
        let sweep = if self.fields[1].locked {
            self.fields[1].value.to_radians().rem_euclid(std::f64::consts::TAU)
        } else {
            ((self.cursor - center).to_angle() - a0).rem_euclid(std::f64::consts::TAU)
        };
        let a = a0 + sweep;
        (center + DVec2::new(a.cos(), a.sin()) * r, sweep)
    }

    /// The point the cursor and the locked fields define.
    fn target(&self) -> DVec2 {
        if let State::Sweeping { base, start } = self.state {
            return self.arc_end(base.pos(), start.pos()).0;
        }
        let State::Placed { base, .. } = self.state else { return self.cursor };
        let b = base.pos();
        let d = self.cursor - b;
        match self.kind {
            ToolKind::Line => {
                let [len, ang] = [self.fields[0], self.fields[1]];
                let length = if len.locked { len.value } else { d.length() };
                let angle = if ang.locked { ang.value.to_radians() } else { d.y.atan2(d.x) };
                b + DVec2::new(angle.cos(), angle.sin()) * length
            }
            ToolKind::Rectangle => {
                let sign = |x: f64| if x < 0.0 { -1.0 } else { 1.0 };
                let w = if self.fields[0].locked { self.fields[0].value * sign(d.x) } else { d.x };
                let h = if self.fields[1].locked { self.fields[1].value * sign(d.y) } else { d.y };
                b + DVec2::new(w, h)
            }
            ToolKind::CenterRectangle => {
                let sign = |x: f64| if x < 0.0 { -1.0 } else { 1.0 };
                let w = if self.fields[0].locked {
                    0.5 * self.fields[0].value * sign(d.x)
                } else {
                    d.x
                };
                let h = if self.fields[1].locked {
                    0.5 * self.fields[1].value * sign(d.y)
                } else {
                    d.y
                };
                b + DVec2::new(w, h)
            }
            ToolKind::Circle => {
                let r = if self.fields[0].locked { 0.5 * self.fields[0].value } else { d.length() };
                let dir = if d.length() > EPS { d.normalize() } else { DVec2::X };
                b + dir * r
            }
            ToolKind::Arc => {
                let r = if self.fields[0].locked { self.fields[0].value } else { d.length() };
                let dir = if d.length() > EPS { d.normalize() } else { DVec2::X };
                b + dir * r
            }
            ToolKind::Point => self.cursor,
        }
    }

    fn update_fields(&mut self) {
        if let State::Sweeping { base, start } = self.state {
            let (_, sweep) = self.arc_end(base.pos(), start.pos());
            if !self.fields[1].locked {
                self.fields[1].value = sweep.to_degrees();
            }
            return;
        }
        let State::Placed { base, .. } = self.state else { return };
        let b = base.pos();
        let t = self.target();
        let d = t - b;
        let values: Vec<f64> = match self.kind {
            ToolKind::Arc => vec![d.length()],
            ToolKind::Line => vec![d.length(), d.y.atan2(d.x).to_degrees()],
            ToolKind::Rectangle => vec![d.x.abs(), d.y.abs()],
            ToolKind::CenterRectangle => vec![2.0 * d.x.abs(), 2.0 * d.y.abs()],
            ToolKind::Circle => vec![2.0 * d.length()],
            ToolKind::Point => Vec::new(),
        };
        for (f, v) in self.fields.iter_mut().zip(values) {
            if !f.locked {
                f.value = v;
            }
        }
    }

    /// Reuse the snapped point only if the cursor alone defines the target.
    fn target_point(&self, sketch: &mut Sketch, t: DVec2) -> EntityId {
        match self.cursor_entity {
            Some(id) if !self.any_locked() => id,
            _ => sketch.add_point(t.to_array()),
        }
    }

    fn commit(&mut self, sketch: &mut Sketch) -> Outcome {
        if let State::Sweeping { base, start } = self.state {
            let (end, sweep) = self.arc_end(base.pos(), start.pos());
            if sweep < 1e-6 {
                return Outcome::Nothing;
            }
            let c = base.materialize(sketch);
            let a = start.materialize(sketch);
            // Reuse a snapped point only if it lies exactly on the circle.
            let r = start.pos().distance(base.pos());
            let e = match self.cursor_entity {
                Some(id)
                    if !self.fields[1].locked
                        && id != a
                        && sketch.point(id).is_some_and(|p| {
                            (DVec2::new(p.x, p.y).distance(base.pos()) - r).abs() < 1e-9
                        }) =>
                {
                    id
                }
                _ => sketch.add_point(end.to_array()),
            };
            let arc = sketch.add_arc(c, a, e);
            set_construction(sketch, arc, self.construction);
            self.state = State::Idle;
            self.unlock_all();
            return Outcome::Edited;
        }
        let State::Placed { base, chain_start } = self.state else { return Outcome::Nothing };
        let b = base.pos();
        let t = self.target();
        let construction = self.construction;
        match self.kind {
            ToolKind::Line => {
                if t.distance(b) < EPS {
                    return Outcome::Nothing;
                }
                let start = base.materialize(sketch);
                let end = self.target_point(sketch, t);
                if start == end {
                    return Outcome::Nothing;
                }
                let line = sketch.add_line(start, end);
                set_construction(sketch, line, construction);
                let first = chain_start.unwrap_or(start);
                // Closing the loop at the first point ends the polyline.
                self.state = if end == first {
                    State::Idle
                } else {
                    State::Placed { base: Base::Existing(end, t), chain_start: Some(first) }
                };
            }
            ToolKind::Rectangle | ToolKind::CenterRectangle => {
                let (c1, c2) =
                    if self.kind == ToolKind::Rectangle { (b, t) } else { (b - (t - b), t) };
                let size = (c2 - c1).abs();
                if size.x < EPS || size.y < EPS {
                    return Outcome::Nothing;
                }
                let p1 = if self.kind == ToolKind::Rectangle {
                    base.materialize(sketch)
                } else {
                    sketch.add_point(c1.to_array())
                };
                let p2 = sketch.add_point([c2.x, c1.y]);
                let p3 = self.target_point(sketch, c2);
                let p4 = sketch.add_point([c1.x, c2.y]);
                for (a, z) in [(p1, p2), (p2, p3), (p3, p4), (p4, p1)] {
                    let line = sketch.add_line(a, z);
                    set_construction(sketch, line, construction);
                }
                self.state = State::Idle;
            }
            ToolKind::Circle => {
                let r = t.distance(b);
                if r < EPS {
                    return Outcome::Nothing;
                }
                let center = base.materialize(sketch);
                let circle = sketch.add_circle(center, r);
                set_construction(sketch, circle, construction);
                self.state = State::Idle;
            }
            ToolKind::Arc => {
                if t.distance(b) < EPS {
                    return Outcome::Nothing;
                }
                // Start point fixed; reuse a snapped point if the radius is free.
                let start = match self.cursor_entity {
                    Some(id) if !self.fields[0].locked => Base::Existing(id, t),
                    _ => Base::New(t),
                };
                self.state = State::Sweeping { base, start };
                self.update_fields();
                return Outcome::Nothing;
            }
            ToolKind::Point => return Outcome::Nothing,
        }
        self.unlock_all();
        self.update_fields();
        Outcome::Edited
    }

    pub fn preview(&self) -> Preview {
        let mut p = Preview::default();
        if let State::Sweeping { base, start } = self.state {
            let (c, a) = (base.pos(), start.pos());
            let (_, sweep) = self.arc_end(c, a);
            let pts = crate::geom::arc_polyline(c, a.distance(c), (a - c).to_angle(), sweep, 48);
            p.segments.extend(pts.windows(2).map(|w| [w[0], w[1]]));
            p.segments.push([c, a]);
            return p;
        }
        let State::Placed { base, .. } = self.state else { return p };
        let b = base.pos();
        let t = self.target();
        match self.kind {
            ToolKind::Line => p.segments.push([b, t]),
            ToolKind::Rectangle | ToolKind::CenterRectangle => {
                let (c1, c2) =
                    if self.kind == ToolKind::Rectangle { (b, t) } else { (b - (t - b), t) };
                let q = [c1, DVec2::new(c2.x, c1.y), c2, DVec2::new(c1.x, c2.y)];
                for i in 0..4 {
                    p.segments.push([q[i], q[(i + 1) % 4]]);
                }
            }
            ToolKind::Circle | ToolKind::Arc => {
                p.circles.push((b, t.distance(b)));
                p.segments.push([b, t]);
            }
            ToolKind::Point => {}
        }
        p
    }
}

fn set_construction(sketch: &mut Sketch, curve: EntityId, construction: bool) {
    if let Some(c) = sketch.curve_mut(curve) {
        c.construction = construction;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snap::SnapKind;
    use qoob3d_core::document::{CurveKind, OriginPlane, PlaneRef};

    fn sketch() -> Sketch {
        Sketch::new(PlaneRef::Origin(OriginPlane::XY))
    }

    fn at(x: f64, y: f64) -> Snap {
        Snap::free(DVec2::new(x, y))
    }

    fn on(id: EntityId, x: f64, y: f64) -> Snap {
        Snap { point: DVec2::new(x, y), kind: SnapKind::Endpoint, entity: Some(id), guides: vec![] }
    }

    fn click(t: &mut Tool, s: &mut Sketch, snap: &Snap) -> Outcome {
        t.handle(s, ToolEvent::Move(snap));
        t.handle(s, ToolEvent::Click(snap))
    }

    #[test]
    fn polyline_shares_points_and_closes() {
        let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Line));
        click(&mut t, &mut s, &at(0.0, 0.0));
        assert_eq!(click(&mut t, &mut s, &at(10.0, 0.0)), Outcome::Edited);
        click(&mut t, &mut s, &at(10.0, 10.0));
        let first = s.points[0].id;
        // Clicking the first point closes the loop and ends the chain.
        assert_eq!(click(&mut t, &mut s, &on(first, 0.0, 0.0)), Outcome::Edited);
        assert_eq!(s.curves.len(), 3);
        assert_eq!(s.points.len(), 3, "corners are shared points");
        assert!(!t.is_placing(), "closed loop ends the chain");
    }

    #[test]
    fn typed_length_and_angle_lock_the_line() {
        let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Line));
        click(&mut t, &mut s, &at(0.0, 0.0));
        t.handle(&mut s, ToolEvent::Move(&at(3.0, 4.0)));
        assert!((t.fields()[0].value - 5.0).abs() < 1e-9, "live length");
        t.handle(&mut s, ToolEvent::SetField { index: 0, value: 20.0 });
        t.handle(&mut s, ToolEvent::SetField { index: 1, value: 90.0 });
        assert_eq!(t.handle(&mut s, ToolEvent::Commit), Outcome::Edited);
        let end = s.points.last().unwrap();
        assert!(end.x.abs() < 1e-9 && (end.y - 20.0).abs() < 1e-9, "{end:?}");
    }

    #[test]
    fn rectangle_two_point_with_typed_size() {
        let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Rectangle));
        click(&mut t, &mut s, &at(5.0, 5.0));
        t.handle(&mut s, ToolEvent::Move(&at(-1.0, 50.0)));
        t.handle(&mut s, ToolEvent::SetField { index: 0, value: 40.0 });
        t.handle(&mut s, ToolEvent::SetField { index: 1, value: 30.0 });
        assert_eq!(t.handle(&mut s, ToolEvent::Commit), Outcome::Edited);
        assert_eq!((s.points.len(), s.curves.len()), (4, 4));
        let xs: Vec<f64> = s.points.iter().map(|p| p.x).collect();
        assert!(xs.contains(&-35.0), "width extends towards the cursor (left): {xs:?}");
        assert!(!t.is_placing(), "tool is ready for the next rectangle");
    }

    #[test]
    fn center_rectangle_and_circle() {
        let (mut s, mut t) = (sketch(), Tool::new(ToolKind::CenterRectangle));
        click(&mut t, &mut s, &at(0.0, 0.0));
        click(&mut t, &mut s, &at(20.0, 10.0));
        let (xmin, xmax) =
            s.points.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
        assert_eq!((xmin, xmax), (-20.0, 20.0));

        let mut t = Tool::new(ToolKind::Circle);
        click(&mut t, &mut s, &at(100.0, 0.0));
        t.handle(&mut s, ToolEvent::SetField { index: 0, value: 14.0 });
        t.handle(&mut s, ToolEvent::Commit);
        let circle = s.curves.last().unwrap();
        assert!(
            matches!(circle.kind, CurveKind::Circle { radius, .. } if (radius - 7.0).abs() < 1e-12)
        );
    }

    #[test]
    fn construction_flag_and_escape() {
        let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Line));
        t.construction = true;
        click(&mut t, &mut s, &at(0.0, 0.0));
        click(&mut t, &mut s, &at(5.0, 0.0));
        assert!(s.curves[0].construction);
        assert_eq!(t.handle(&mut s, ToolEvent::Escape), Outcome::Nothing, "first Esc ends chain");
        assert_eq!(t.handle(&mut s, ToolEvent::Escape), Outcome::Done, "second Esc closes tool");
    }

    #[test]
    fn degenerate_input_is_ignored() {
        let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Rectangle));
        click(&mut t, &mut s, &at(1.0, 1.0));
        assert_eq!(click(&mut t, &mut s, &at(1.0, 9.0)), Outcome::Nothing, "zero width");
        assert!(s.curves.is_empty());
        let mut t = Tool::new(ToolKind::Line);
        assert_eq!(
            t.handle(&mut s, ToolEvent::SetField { index: 0, value: -3.0 }),
            Outcome::Nothing
        );
        assert!(!t.fields.iter().any(|f| f.locked), "negative length rejected");
    }

    #[test]
    fn point_tool_does_not_duplicate() {
        let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Point));
        assert_eq!(click(&mut t, &mut s, &at(3.0, 3.0)), Outcome::Edited);
        let id = s.points[0].id;
        assert_eq!(click(&mut t, &mut s, &on(id, 3.0, 3.0)), Outcome::Nothing);
        assert_eq!(s.points.len(), 1);
    }

    #[test]
    fn arc_from_centre_start_and_typed_sweep() {
        let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Arc));
        click(&mut t, &mut s, &at(0.0, 0.0));
        assert_eq!(click(&mut t, &mut s, &at(10.0, 0.0)), Outcome::Nothing, "start placed");
        assert!(t.is_placing());
        t.handle(&mut s, ToolEvent::Move(&at(0.0, -10.0)));
        assert_eq!(t.fields().len(), 1, "only the sweep is open now");
        t.handle(&mut s, ToolEvent::SetField { index: 0, value: 180.0 });
        assert_eq!(t.handle(&mut s, ToolEvent::Commit), Outcome::Edited);
        let CurveKind::Arc { center, start, end } = s.curves[0].kind else { panic!() };
        assert_eq!(s.pos(center), [0.0, 0.0]);
        assert_eq!(s.pos(start), [10.0, 0.0]);
        let e = s.pos(end);
        assert!((e[0] + 10.0).abs() < 1e-9 && e[1].abs() < 1e-9, "{e:?}");
        assert!(!t.is_placing());
        assert!((s.arc(center, start, end).sweep - std::f64::consts::PI).abs() < 1e-9);
    }

    #[test]
    fn arc_radius_can_be_typed() {
        let (mut s, mut t) = (sketch(), Tool::new(ToolKind::Arc));
        click(&mut t, &mut s, &at(0.0, 0.0));
        t.handle(&mut s, ToolEvent::Move(&at(3.0, 0.0)));
        t.handle(&mut s, ToolEvent::SetField { index: 0, value: 7.5 });
        t.handle(&mut s, ToolEvent::Commit); // Enter places the start
        t.handle(&mut s, ToolEvent::Move(&at(0.0, 9.0)));
        assert_eq!(t.handle(&mut s, ToolEvent::Commit), Outcome::Edited);
        let CurveKind::Arc { center, start, end } = s.curves[0].kind else { panic!() };
        let g = s.arc(center, start, end);
        assert!((g.radius - 7.5).abs() < 1e-12);
        assert!((g.sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "quarter arc");
    }
}
