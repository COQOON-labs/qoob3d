//! Viewport camera with Fusion 360 style navigation primitives (ADR-0008).
//!
//! World convention: right-handed, +Z up, millimetres (ADR-0006).

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use qoob3d_core::Aabb;

const MIN_DISTANCE: f32 = 1e-3;
const MAX_DISTANCE: f32 = 1e6;

/// Perspective or parallel projection. Both share the same visible height in
/// the plane through `target`, so switching does not change the framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic,
}

/// An orbit camera looking at `target` from `distance` along the camera's
/// local +Z axis. `rotation` maps camera-local axes (x right, y up,
/// z backwards) into world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    pub rotation: Quat,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    pub projection: Projection,
}

/// Spacing of the visible grid lines for a camera distance, matching the
/// level of detail in the grid shader (`fs_grid`): powers of ten, switching
/// to the next coarser level once the finer lines fade out. Used for
/// sketch grid snapping.
pub fn grid_snap_step(distance: f32) -> f32 {
    let lod = distance.max(1e-3).log10() - 1.0;
    let base = 10f32.powf(lod.floor());
    if lod.fract() < 0.5 { base } else { base * 10.0 }
}

/// A world-space ray; `dir` is normalised.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
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
            projection: Projection::Perspective,
        };
        cam.look_from(ViewPreset::Iso.eye_direction());
        cam
    }
}

impl Camera {
    /// Orients the camera so it looks at `target` from direction `dir`,
    /// keeping world +Z pointing up on screen (top/bottom views: +Y/-Y up).
    pub fn look_from(&mut self, dir: Vec3) {
        self.rotation = Self::rotation_looking_from(dir);
    }

    /// The rotation [`Camera::look_from`] would set, without applying it.
    pub fn rotation_looking_from(dir: Vec3) -> Quat {
        let back = dir.normalize();
        let right = Vec3::Z.cross(back).try_normalize().unwrap_or(Vec3::X);
        let up = back.cross(right);
        Quat::from_mat3(&glam::Mat3::from_cols(right, up, back)).normalize()
    }

    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    /// Viewing direction (from the eye towards the target).
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    pub fn eye(&self) -> Vec3 {
        self.target - self.forward() * self.distance
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.eye(), self.target, self.up())
    }

    /// Reverse-Z projection (depth 1 = near, 0 = far), clip-space Z in
    /// [0, 1] as wgpu expects (ADR-0019).
    pub fn projection(&self, aspect: f32) -> Mat4 {
        match self.projection {
            Projection::Perspective => {
                glam::camera::rh::proj::directx::perspective_infinite_reverse(
                    self.fov_y,
                    aspect,
                    self.near(),
                )
            }
            Projection::Orthographic => {
                let hh = self.half_height_at_target();
                let (near, far) = self.ortho_depth_range();
                // Linear reverse-Z: depth 1 at view distance `near`, 0 at `far`.
                Mat4::from_cols(
                    Vec4::new(1.0 / (hh * aspect), 0.0, 0.0, 0.0),
                    Vec4::new(0.0, 1.0 / hh, 0.0, 0.0),
                    Vec4::new(0.0, 0.0, 1.0 / (far - near), 0.0),
                    Vec4::new(0.0, 0.0, far / (far - near), 1.0),
                )
            }
        }
    }

    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    fn near(&self) -> f32 {
        (self.distance * 1e-3).max(1e-4)
    }

    /// Ortho depth range in view distance; negative near keeps geometry
    /// behind the (virtual) eye visible.
    fn ortho_depth_range(&self) -> (f32, f32) {
        let span = 10.0 * self.distance + 1e4;
        (-span, self.distance + span)
    }

    /// Half height of the visible area in the plane through `target`.
    pub fn half_height_at_target(&self) -> f32 {
        self.distance * (0.5 * self.fov_y).tan()
    }

    /// World point in the plane through `target` that appears at `ndc`.
    fn point_on_target_plane(&self, ndc: Vec2, aspect: f32) -> Vec3 {
        let hh = self.half_height_at_target();
        self.target + self.right() * ndc.x * hh * aspect + self.up() * ndc.y * hh
    }

    /// The ray through `ndc` (normalised device coordinates, y up).
    pub fn ray(&self, ndc: Vec2, aspect: f32) -> Ray {
        let p = self.point_on_target_plane(ndc, aspect);
        match self.projection {
            Projection::Perspective => {
                let origin = self.eye();
                Ray { origin, dir: (p - origin).normalize() }
            }
            Projection::Orthographic => {
                let back = self.ortho_depth_range().0.abs();
                Ray { origin: p - self.forward() * (self.distance + back), dir: self.forward() }
            }
        }
    }

    /// Projects a world point to NDC (x, y in [-1, 1], z = depth). `None` if
    /// the point is behind the camera.
    pub fn project(&self, p: Vec3, aspect: f32) -> Option<Vec3> {
        let clip = self.view_projection(aspect) * p.extend(1.0);
        (clip.w > 1e-6).then(|| clip.truncate() / clip.w)
    }

    /// Turntable orbit around `target`: horizontal motion rotates about world
    /// +Z, vertical motion about the camera's right axis. Angles in radians.
    pub fn orbit(&mut self, yaw: f32, pitch: f32) {
        self.orbit_around(self.target, yaw, pitch);
    }

    /// Like [`Camera::orbit`], but rotating about an arbitrary world pivot
    /// (Fusion orbits around the point under the cursor).
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
        let p = self.point_on_target_plane(ndc, aspect);
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

    /// Smooth transition from `a` to `b` for `t` in [0, 1] (ViewCube,
    /// Home). Rotation is slerped, distance interpolated logarithmically.
    pub fn interpolate(a: &Camera, b: &Camera, t: f32) -> Camera {
        let t = t.clamp(0.0, 1.0);
        Camera {
            target: a.target.lerp(b.target, t),
            distance: (a.distance.ln() + (b.distance.ln() - a.distance.ln()) * t).exp(),
            rotation: a.rotation.slerp(b.rotation, t).normalize(),
            fov_y: b.fov_y,
            projection: b.projection,
        }
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
    fn orbit_around_pivot_keeps_pivot_on_screen() {
        let mut cam = Camera::default();
        let pivot = Vec3::new(10.0, 5.0, 3.0);
        let before = cam.project(pivot, 1.5).unwrap();
        cam.orbit_around(pivot, 0.4, 0.2);
        let after = cam.project(pivot, 1.5).unwrap();
        assert!((before.truncate() - after.truncate()).length() < 1e-3);
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
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let mut cam = Camera { projection, ..Camera::default() };
            let (aspect, ndc) = (1.6, Vec2::new(0.4, -0.3));
            let before = cam.point_on_target_plane(ndc, aspect);
            cam.zoom_at(0.8, ndc, aspect);
            assert!(approx(cam.point_on_target_plane(ndc, aspect), before), "{projection:?}");
        }
    }

    #[test]
    fn ray_hits_what_project_maps_to_ndc() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let cam = Camera { projection, ..Camera::default() };
            let aspect = 1.6;
            let p = Vec3::new(3.0, -4.0, 7.0);
            let ndc = cam.project(p, aspect).unwrap().truncate();
            let ray = cam.ray(ndc, aspect);
            let t = (p - ray.origin).dot(ray.dir);
            assert!(t > 0.0, "{projection:?}");
            assert!((ray.at(t) - p).length() < 1e-2, "{projection:?}: {:?} vs {p}", ray.at(t));
        }
    }

    #[test]
    fn both_projections_use_reverse_z_and_agree_at_target() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let cam = Camera { projection, ..Camera::default() };
            let near = cam.project(cam.target - cam.forward() * 10.0, 1.0).unwrap();
            let far = cam.project(cam.target + cam.forward() * 10.0, 1.0).unwrap();
            assert!(near.z > far.z, "{projection:?}: closer must have larger depth");
            assert!((0.0..=1.0).contains(&near.z) && (0.0..=1.0).contains(&far.z));
        }
        // Same framing: a point at the edge of the target plane maps to the
        // same NDC in both projections.
        let persp = Camera::default();
        let ortho = Camera { projection: Projection::Orthographic, ..persp };
        let p = persp.target + persp.up() * persp.half_height_at_target();
        assert!((persp.project(p, 1.0).unwrap().y - 1.0).abs() < 1e-4);
        assert!((ortho.project(p, 1.0).unwrap().y - 1.0).abs() < 1e-4);
    }

    #[test]
    fn grid_snap_step_follows_zoom() {
        assert_eq!(grid_snap_step(100.0), 10.0);
        assert_eq!(grid_snap_step(20.0), 1.0);
        assert_eq!(grid_snap_step(50.0), 10.0, "fine lines faded → coarser step");
    }

    #[test]
    fn interpolation_hits_endpoints() {
        let a = Camera::default();
        let mut b = Camera { distance: 400.0, target: Vec3::new(5.0, 0.0, 0.0), ..a };
        b.look_from(Vec3::Z);
        assert_eq!(Camera::interpolate(&a, &b, 0.0).target, a.target);
        let end = Camera::interpolate(&a, &b, 1.0);
        assert!(approx(end.eye(), b.eye()));
        let mid = Camera::interpolate(&a, &b, 0.5);
        assert!((mid.distance - 200.0).abs() < 1e-2, "log interpolation: {}", mid.distance);
    }
}
