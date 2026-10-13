//! Viewport camera with Fusion 360 style navigation primitives.
//!
//! World convention: right-handed, +Z up, millimetres.

use glam::{Mat4, Quat, Vec2, Vec3};
use qoob3d_core::Aabb;

const MIN_DISTANCE: f32 = 1e-3;
const MAX_DISTANCE: f32 = 1e6;

/// A perspective orbit camera looking at `target` from `distance` along the
/// camera's local +Z axis. `rotation` maps camera-local axes (x right, y up,
/// z backwards) into world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    pub rotation: Quat,
    /// Vertical field of view in radians.
    pub fov_y: f32,
}

/// Standard views, named like Fusion's ViewCube faces (ADR-0006: front looks
/// along +Y, right looks along -X).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPreset {
    /// Front-right-top corner, Fusion's "Home".
    Iso,
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

impl ViewPreset {
    pub const ALL: [ViewPreset; 7] = [
        ViewPreset::Iso,
        ViewPreset::Front,
        ViewPreset::Back,
        ViewPreset::Left,
        ViewPreset::Right,
        ViewPreset::Top,
        ViewPreset::Bottom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ViewPreset::Iso => "iso",
            ViewPreset::Front => "front",
            ViewPreset::Back => "back",
            ViewPreset::Left => "left",
            ViewPreset::Right => "right",
            ViewPreset::Top => "top",
            ViewPreset::Bottom => "bottom",
        }
    }

    /// Direction from the target towards the eye.
    pub fn eye_direction(self) -> Vec3 {
        match self {
            ViewPreset::Iso => Vec3::new(1.0, -1.0, 0.8),
            ViewPreset::Front => Vec3::NEG_Y,
            ViewPreset::Back => Vec3::Y,
            ViewPreset::Left => Vec3::NEG_X,
            ViewPreset::Right => Vec3::X,
            ViewPreset::Top => Vec3::Z,
            ViewPreset::Bottom => Vec3::NEG_Z,
        }
    }
}

impl Default for Camera {
    /// Fusion's "Home" view: looking at the front-right-top corner.
    fn default() -> Self {
        let mut cam = Self {
            target: Vec3::ZERO,
            distance: 100.0,
            rotation: Quat::IDENTITY,
            fov_y: 30f32.to_radians(),
        };
        cam.look_from(ViewPreset::Iso.eye_direction());
        cam
    }
}

impl Camera {
    /// Orients the camera so it looks at `target` from direction `dir`,
    /// keeping world +Z pointing up on screen (top/bottom views: +Y/-Y up).
    pub fn look_from(&mut self, dir: Vec3) {
        let back = dir.normalize();
        let right = Vec3::Z.cross(back).try_normalize().unwrap_or(Vec3::X);
        let up = back.cross(right);
        self.rotation = Quat::from_mat3(&glam::Mat3::from_cols(right, up, back)).normalize();
    }

    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.rotation * Vec3::Z * self.distance
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.eye(), self.target, self.up())
    }

    /// Reverse-Z infinite perspective projection (depth 1 = near, 0 = far),
    /// clip-space Z in [0, 1] as wgpu expects.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective_infinite_reverse(
            self.fov_y,
            aspect,
            self.near(),
        )
    }

    fn near(&self) -> f32 {
        (self.distance * 1e-3).max(1e-4)
    }

    /// Half height of the visible area in the plane through `target`.
    fn half_height_at_target(&self) -> f32 {
        self.distance * (0.5 * self.fov_y).tan()
    }

    /// Turntable orbit around `target`: horizontal motion rotates about world
    /// +Z, vertical motion about the camera's right axis. Angles in radians.
    pub fn orbit(&mut self, yaw: f32, pitch: f32) {
        self.orbit_around(self.target, yaw, pitch);
    }

    /// Like [`Camera::orbit`], but rotating about an arbitrary world pivot.
    pub fn orbit_around(&mut self, pivot: Vec3, yaw: f32, pitch: f32) {
        let q = Quat::from_axis_angle(self.right(), -pitch) * Quat::from_axis_angle(Vec3::Z, -yaw);
        self.rotation = (q * self.rotation).normalize();
        self.target = pivot + q * (self.target - pivot);
    }

    /// Moves the view so that the content follows the pointer by
    /// `delta_px` (screen pixels, y down) in a viewport `viewport_h_px` tall.
    pub fn pan(&mut self, delta_px: Vec2, viewport_h_px: f32) {
        let world_per_px = 2.0 * self.half_height_at_target() / viewport_h_px.max(1.0);
        self.target += (-self.right() * delta_px.x + self.up() * delta_px.y) * world_per_px;
    }

    /// Zooms by `factor` (< 1 zooms in) towards the point under `ndc`
    /// (normalised device coordinates, y up) at target depth.
    pub fn zoom_at(&mut self, factor: f32, ndc: Vec2, aspect: f32) {
        let new_distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        let k = new_distance / self.distance;
        let hh = self.half_height_at_target();
        let p = self.target + self.right() * ndc.x * hh * aspect + self.up() * ndc.y * hh;
        self.target = p + (self.target - p) * k;
        self.distance = new_distance;
    }

    /// Frames the bounding box so it fills the view (keeps orientation).
    pub fn fit(&mut self, bounds: Aabb, aspect: f32) {
        let radius = bounds.radius().max(1e-3);
        let half_fov = 0.5 * self.fov_y.min(self.fov_y * aspect.min(1.0));
        self.target = Vec3::from(bounds.center());
        self.distance = (radius / half_fov.sin() * 1.15).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn default_view_has_z_up_on_screen() {
        let cam = Camera::default();
        assert!(cam.up().z > 0.5);
        assert!(cam.right().z.abs() < 1e-5);
        assert!(cam.eye().x > 0.0 && cam.eye().y < 0.0 && cam.eye().z > 0.0);
    }

    #[test]
    fn presets_look_along_expected_axes() {
        for preset in ViewPreset::ALL {
            let mut cam = Camera::default();
            cam.look_from(preset.eye_direction());
            let to_eye = (cam.eye() - cam.target).normalize();
            assert!(approx(to_eye, preset.eye_direction().normalize()), "{preset:?}");
            assert!(cam.right().is_normalized() && cam.up().is_normalized(), "{preset:?}");
        }
        let mut top = Camera::default();
        top.look_from(ViewPreset::Top.eye_direction());
        assert!(approx(top.right(), Vec3::X) && approx(top.up(), Vec3::Y));
    }

    #[test]
    fn orbit_keeps_distance_to_target() {
        let mut cam = Camera::default();
        let d = (cam.eye() - cam.target).length();
        cam.orbit(0.7, -0.3);
        assert!(((cam.eye() - cam.target).length() - d).abs() < 1e-3);
    }

    #[test]
    fn zoom_at_center_keeps_target() {
        let mut cam = Camera::default();
        let t = cam.target;
        cam.zoom_at(0.5, Vec2::ZERO, 1.5);
        assert!(approx(cam.target, t));
        assert!((cam.distance - 50.0).abs() < 1e-3);
    }

    #[test]
    fn zoom_keeps_point_under_cursor_fixed() {
        let mut cam = Camera::default();
        let aspect = 1.6;
        let ndc = Vec2::new(0.4, -0.3);
        let world = |c: &Camera| {
            let hh = c.half_height_at_target();
            c.target + c.right() * ndc.x * hh * aspect + c.up() * ndc.y * hh
        };
        let before = world(&cam);
        cam.zoom_at(0.8, ndc, aspect);
        assert!(approx(world(&cam), before));
    }
}
