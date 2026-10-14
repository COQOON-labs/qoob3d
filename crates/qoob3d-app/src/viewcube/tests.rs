use super::*;
use qoob3d_render::{Camera, ViewPreset};

fn rotation(preset: ViewPreset) -> Quat {
    Camera::rotation_looking_from(preset.eye_direction())
}

const C: Pos2 = Pos2::new(100.0, 100.0);

#[test]
fn front_view_shows_only_front_face_and_center_is_front() {
    let regions = regions(rotation(ViewPreset::Front), C, 50.0);
    assert!(regions.iter().all(|r| r.face == CubeFace::Front));
    let center = hit(&regions, C).unwrap();
    assert_eq!(center.kind, RegionKind::Face);
    assert!((center.eye_dir - Vec3::NEG_Y).length() < 1e-5);
}

#[test]
fn iso_view_shows_three_faces_and_corner_in_the_middle() {
    let regions = regions(rotation(ViewPreset::Iso), C, 50.0);
    let faces: std::collections::HashSet<_> =
        regions.iter().map(|r| format!("{:?}", r.face)).collect();
    assert_eq!(faces.len(), 3);
    for f in ["Top", "Right", "Front"] {
        assert!(faces.contains(f), "{faces:?}");
    }
    let corner = hit(&regions, C).unwrap();
    assert_eq!(corner.kind, RegionKind::Corner);
    assert!((corner.eye_dir - Vec3::new(1.0, -1.0, 1.0).normalize()).length() < 1e-5);
}

#[test]
fn clicking_the_top_face_looks_from_above() {
    let rot = rotation(ViewPreset::Iso);
    let top = face_center(rot, C, 50.0, CubeFace::Top).unwrap();
    let regions = regions(rot, C, 50.0);
    let region = hit(&regions, top).unwrap();
    assert_eq!(region.kind, RegionKind::Face);
    assert!((region.eye_dir - Vec3::Z).length() < 1e-5);
    assert!(face_center(rot, C, 50.0, CubeFace::Bottom).is_none(), "bottom is hidden");
}

#[test]
fn edges_and_corners_are_single_targets_across_faces() {
    let regions = regions(rotation(ViewPreset::Iso), C, 50.0);
    // The front-top edge is visible on two faces, the front-right-top
    // corner on three: each forms one target.
    let count = |dir: Vec3| {
        let probe = Region {
            quad: [Pos2::ZERO; 4],
            eye_dir: dir.normalize(),
            kind: RegionKind::Edge,
            face: CubeFace::Front,
            depth: 0.0,
        };
        regions.iter().filter(|r| r.same_target(&probe)).count()
    };
    assert_eq!(count(Vec3::new(0.0, -1.0, 1.0)), 2, "front-top edge");
    assert_eq!(count(Vec3::new(1.0, -1.0, 1.0)), 3, "front-right-top corner");
    for r in regions.iter().filter(|r| r.kind == RegionKind::Edge) {
        let axes = [r.eye_dir.x, r.eye_dir.y, r.eye_dir.z];
        assert_eq!(axes.iter().filter(|c| c.abs() > 1e-5).count(), 2, "{r:?}");
    }
}

#[test]
fn outside_the_cube_hits_nothing() {
    let regions = regions(rotation(ViewPreset::Iso), C, 50.0);
    assert!(hit(&regions, Pos2::new(0.0, 0.0)).is_none());
}
