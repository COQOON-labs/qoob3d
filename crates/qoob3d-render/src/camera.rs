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
mod tests;
