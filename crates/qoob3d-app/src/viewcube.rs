//! Fusion-style ViewCube (ADR-0015): drawn with egui on top of the 3D
//! viewport. Faces, edges and corners are clickable and turn the camera to
//! look from that direction. An edge or corner is *one* target even though
//! it spans two or three faces. Labels lie in the face plane. The geometry
//! part is pure and unit-tested.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Shape, Stroke,
    epaint::{Tessellator, TextShape},
};
use glam::{Quat, Vec3};

use crate::i18n::{tr, tr_args};
use crate::theme;

/// Width of the edge and corner zones, in cube units (face half-size = 1).
const BAND: f32 = 0.26;
/// Point size the labels are laid out in before being mapped onto a face.
const LABEL_PT: f32 = 14.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CubeFace {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

impl CubeFace {
    fn label_key(self) -> &'static str {
        match self {
            CubeFace::Front => "viewcube-front",
            CubeFace::Back => "viewcube-back",
            CubeFace::Left => "viewcube-left",
            CubeFace::Right => "viewcube-right",
            CubeFace::Top => "viewcube-top",
            CubeFace::Bottom => "viewcube-bottom",
        }
    }

    pub fn label(self) -> String {
        tr(self.label_key())
    }
}

struct FaceDef {
    face: CubeFace,
    normal: Vec3,
    /// Text right direction on the face (seen from outside).
    u: Vec3,
    /// Text up direction on the face.
    v: Vec3,
}

/// Orientation per ADR-0006 (front = −Y).
const FACES: [FaceDef; 6] = [
    FaceDef { face: CubeFace::Front, normal: Vec3::NEG_Y, u: Vec3::X, v: Vec3::Z },
    FaceDef { face: CubeFace::Back, normal: Vec3::Y, u: Vec3::NEG_X, v: Vec3::Z },
    FaceDef { face: CubeFace::Right, normal: Vec3::X, u: Vec3::Y, v: Vec3::Z },
    FaceDef { face: CubeFace::Left, normal: Vec3::NEG_X, u: Vec3::NEG_Y, v: Vec3::Z },
    FaceDef { face: CubeFace::Top, normal: Vec3::Z, u: Vec3::X, v: Vec3::Y },
    FaceDef { face: CubeFace::Bottom, normal: Vec3::NEG_Z, u: Vec3::X, v: Vec3::NEG_Y },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    Face,
    Edge,
    Corner,
}

/// A clickable patch of a visible cube face, projected to the screen. Edge
/// and corner targets consist of several patches with the same `eye_dir`.
#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub quad: [Pos2; 4],
    /// Direction the camera looks *from* when this region is clicked.
    pub eye_dir: Vec3,
    pub kind: RegionKind,
    pub face: CubeFace,
    /// Larger = closer to the viewer.
    pub depth: f32,
}

impl Region {
    /// True if both regions belong to the same click target.
    pub fn same_target(&self, other: &Region) -> bool {
        (self.eye_dir - other.eye_dir).length_squared() < 1e-6
    }
}

/// Orthographic screen projection of the cube for a camera `rotation`.
struct Projector {
    right: Vec3,
    up: Vec3,
    back: Vec3,
    center: Pos2,
    scale: f32,
}

impl Projector {
    fn new(rotation: Quat, center: Pos2, half_size_px: f32) -> Self {
        Self {
            right: rotation * Vec3::X,
            up: rotation * Vec3::Y,
            back: rotation * Vec3::Z,
            center,
            // Projected cube radius is at most √3; keep a little margin.
            scale: half_size_px / 1.75,
        }
    }

    fn project(&self, p: Vec3) -> Pos2 {
        self.center + egui::vec2(p.dot(self.right), -p.dot(self.up)) * self.scale
    }

    fn facing(&self, def: &FaceDef) -> f32 {
        def.normal.dot(self.back)
    }
}

/// All clickable regions of the faces that face the viewer.
pub fn regions(rotation: Quat, center: Pos2, half_size_px: f32) -> Vec<Region> {
    let proj = Projector::new(rotation, center, half_size_px);
    let cuts = [-1.0, -1.0 + BAND, 1.0 - BAND, 1.0];
    let side = |k: usize| match k {
        0 => -1.0,
        2 => 1.0,
        _ => 0.0,
    };
    let mut out = Vec::new();
    for def in FACES.iter().filter(|d| proj.facing(d) > 0.05) {
        for i in 0..3 {
            for j in 0..3 {
                let (u0, u1, v0, v1) = (cuts[i], cuts[i + 1], cuts[j], cuts[j + 1]);
                let corner = |u: f32, v: f32| proj.project(def.normal + def.u * u + def.v * v);
                let (su, sv) = (side(i), side(j));
                let kind = match u8::from(su != 0.0) + u8::from(sv != 0.0) {
                    0 => RegionKind::Face,
                    1 => RegionKind::Edge,
                    _ => RegionKind::Corner,
                };
                let mid = def.normal + def.u * (0.5 * (u0 + u1)) + def.v * (0.5 * (v0 + v1));
                out.push(Region {
                    quad: [corner(u0, v0), corner(u1, v0), corner(u1, v1), corner(u0, v1)],
                    eye_dir: (def.normal + def.u * su + def.v * sv).normalize(),
                    kind,
                    face: def.face,
                    depth: mid.dot(proj.back),
                });
            }
        }
    }
    out
}

/// The region under `pos`, if any (closest to the viewer wins).
pub fn hit(regions: &[Region], pos: Pos2) -> Option<&Region> {
    regions
        .iter()
        .filter(|r| inside_convex(&r.quad, pos))
        .max_by(|a, b| a.depth.total_cmp(&b.depth))
}

fn inside_convex(quad: &[Pos2; 4], p: Pos2) -> bool {
    let mut sign = 0.0f32;
    for k in 0..4 {
        let (a, b) = (quad[k], quad[(k + 1) % 4]);
        let cross = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
        if cross.abs() < 1e-6 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// Screen centre of a face (if visible), e.g. for tests.
pub fn face_center(
    rotation: Quat,
    center: Pos2,
    half_size_px: f32,
    face: CubeFace,
) -> Option<Pos2> {
    let proj = Projector::new(rotation, center, half_size_px);
    FACES.iter().find(|f| f.face == face && proj.facing(f) > 0.05).map(|f| proj.project(f.normal))
}

#[derive(Debug, Default)]
pub struct ViewCubeOutput {
    /// Eye direction of the clicked region.
    pub clicked: Option<Vec3>,
    pub home_clicked: bool,
}

/// Draws the cube into `rect` and handles hover/click.
pub fn show(ui: &mut egui::Ui, rect: Rect, rotation: Quat) -> ViewCubeOutput {
    let mut out = ViewCubeOutput::default();
    let center = rect.center();
    let half = 0.5 * rect.width().min(rect.height());
    let regions = regions(rotation, center, half);

    let response = ui.interact(rect, ui.id().with("viewcube"), Sense::click());
    let hovered = response.hover_pos().and_then(|p| hit(&regions, p).copied());
    let response = match hovered {
        Some(r) => {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            response.on_hover_text(match r.kind {
                RegionKind::Face => {
                    tr_args("viewcube-face-tip", &[("face", r.face.label().into())])
                }
                RegionKind::Edge => tr("viewcube-edge-tip"),
                RegionKind::Corner => tr("viewcube-corner-tip"),
            })
        }
        None => response,
    };
    if response.clicked()
        && let Some(region) = hovered
    {
        out.clicked = Some(region.eye_dir);
    }

    // Not clipped to `rect`: labels and the highlight may touch its border.
    let painter = ui.painter().clone();
    let proj = Projector::new(rotation, center, half);
    let outline = Stroke::new(1.0, Color32::from_gray(110));

    // Faces (a convex cube needs no depth sorting).
    for def in FACES.iter().filter(|d| proj.facing(d) > 0.05) {
        let quad: Vec<Pos2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .iter()
            .map(|&(u, v)| proj.project(def.normal + def.u * u + def.v * v))
            .collect();
        let shade = (52.0 + 30.0 * proj.facing(def)) as u8;
        painter.add(Shape::convex_polygon(quad, Color32::from_gray(shade), outline));
    }

    // Highlight the whole target: all patches of an edge or corner.
    if let Some(target) = hovered {
        for r in regions.iter().filter(|r| r.same_target(&target)) {
            painter.add(Shape::convex_polygon(
                r.quad.to_vec(),
                theme::ACCENT.gamma_multiply(0.6),
                Stroke::NONE,
            ));
        }
    }

    // Labels lie in the face plane (sheared like the face itself).
    for def in FACES.iter().filter(|d| proj.facing(d) > 0.2) {
        if let Some(label) = face_label(ui, &proj, def) {
            painter.add(label);
        }
    }

    // Home button (Fusion: house icon at the top left of the cube).
    let home_rect =
        Rect::from_min_size(rect.left_top() + egui::vec2(-6.0, -6.0), egui::vec2(20.0, 20.0));
    let home = ui.interact(home_rect, ui.id().with("viewcube-home"), Sense::click());
    if home.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let home_color = if home.hovered() { theme::ACCENT } else { theme::TEXT_MUTED };
    painter.text(
        home_rect.center(),
        Align2::CENTER_CENTER,
        "🏠",
        FontId::proportional(14.0),
        home_color,
    );
    out.home_clicked = home.on_hover_text(tr("viewcube-home")).clicked();

    out
}

/// The face's label as a text mesh mapped onto the face: laid out flat,
/// tessellated, then every vertex is placed on the (projected) face plane.
fn face_label(ui: &egui::Ui, proj: &Projector, def: &FaceDef) -> Option<Shape> {
    let galley =
        ui.painter().layout_no_wrap(def.face.label(), FontId::proportional(LABEL_PT), theme::TEXT);
    let size = galley.size();
    if size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    // Fit into 65 % of the face width and 30 % of its height (cube units).
    let scale = (1.3 / size.x).min(0.6 / size.y);
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "the method path does not satisfy the higher-ranked closure bound"
    )]
    let font_tex_size = ui.ctx().fonts(|f| f.font_image_size());
    let mut tessellator =
        Tessellator::new(ui.ctx().pixels_per_point(), Default::default(), font_tex_size, vec![]);
    let mut mesh = egui::Mesh::default();
    tessellator.tessellate_text(&TextShape::new(Pos2::ZERO, galley, theme::TEXT), &mut mesh);
    for vertex in &mut mesh.vertices {
        let x = (vertex.pos.x - 0.5 * size.x) * scale;
        let y = -(vertex.pos.y - 0.5 * size.y) * scale;
        vertex.pos = proj.project(def.normal + def.u * x + def.v * y);
    }
    Some(Shape::mesh(mesh))
}

/// Where the home button is, for tests and the smoke test.
pub fn home_button_center(rect: Rect) -> Pos2 {
    rect.left_top() + egui::vec2(4.0, 4.0)
}

#[cfg(test)]
mod tests;
