use super::*;
use eframe::egui::pos2;
use qoob3d_render::ViewPreset;

fn view_for(camera: &Camera, plane: OriginPlane) -> View<'_> {
    View {
        camera,
        rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)),
        frame: plane.frame(),
    }
}

#[test]
fn screen_to_sketch_round_trip_on_every_plane() {
    for (plane, preset) in [
        (OriginPlane::XY, ViewPreset::Iso),
        (OriginPlane::XZ, ViewPreset::Front),
        (OriginPlane::YZ, ViewPreset::Iso),
    ] {
        let mut camera = Camera::default();
        camera.look_from(preset.eye_direction());
        let view = view_for(&camera, plane);
        let p = DVec2::new(12.5, -7.25);
        let s = view.to_screen(p).unwrap();
        let back = view.to_local(s).unwrap();
        assert!(back.distance(p) < 1e-3, "{plane:?}: {back:?}");
        assert!(view.units_per_px(s) > 0.0);
    }
}

#[test]
fn edge_on_plane_gives_no_input() {
    let mut camera = Camera::default();
    camera.look_from(ViewPreset::Front.eye_direction());
    // Looking along −Y at the XY plane: edge-on.
    let view = view_for(&camera, OriginPlane::XY);
    assert!(view.to_local(pos2(400.0, 300.0)).is_none());
}

#[test]
fn plane_centres_pick_their_plane() {
    let camera = Camera::default(); // iso
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
    let a = f64::from(camera.half_height_at_target()) * 0.45;
    let planes = pickable_planes(&qoob3d_core::document::Document::default());
    for plane in OriginPlane::ALL {
        let p = plane_center(&camera, rect, plane).unwrap();
        assert_eq!(
            plane_under(&camera, rect, p, a, &planes),
            Some(PlaneRef::Origin(plane)),
            "{plane:?}"
        );
    }
}
