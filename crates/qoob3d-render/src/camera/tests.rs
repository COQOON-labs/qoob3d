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
