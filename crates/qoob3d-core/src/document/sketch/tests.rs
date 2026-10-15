use super::*;

#[test]
fn origin_frames_are_right_handed_and_match_the_viewcube() {
    for plane in OriginPlane::ALL {
        let f = plane.frame();
        let cross = [
            f.u[1] * f.v[2] - f.u[2] * f.v[1],
            f.u[2] * f.v[0] - f.u[0] * f.v[2],
            f.u[0] * f.v[1] - f.u[1] * f.v[0],
        ];
        assert_eq!(cross, f.n, "{plane:?}");
    }
    assert_eq!(OriginPlane::XZ.frame().n, [0.0, -1.0, 0.0], "front plane faces −Y");
}

#[test]
fn local_world_round_trip_and_ray_hit() {
    let f = OriginPlane::XZ.frame();
    let w = f.to_world([3.0, 4.0]);
    assert_eq!(w, [3.0, 0.0, 4.0]);
    assert_eq!(f.to_local(w), [3.0, 4.0]);
    let hit = f.ray_hit([3.0, -50.0, 4.0], [0.0, 1.0, 0.0]).unwrap();
    assert!((hit[0] - 3.0).abs() < 1e-12 && (hit[1] - 4.0).abs() < 1e-12);
    assert!(f.ray_hit([0.0, -50.0, 0.0], [1.0, 0.0, 0.0]).is_none(), "parallel ray");
}

#[test]
fn shared_points_and_deletion() {
    let mut s = Sketch::new(PlaneRef::Origin(OriginPlane::XY));
    let a = s.add_point([0.0, 0.0]);
    let b = s.add_point([10.0, 0.0]);
    let c = s.add_point([10.0, 10.0]);
    let ab = s.add_line(a, b);
    let bc = s.add_line(b, c);
    assert_eq!(s.curves_at(b).count(), 2);

    s.delete(&[ab]);
    assert!(s.point(a).is_none(), "orphaned start point removed");
    assert!(s.point(b).is_some(), "shared point kept");

    s.delete(&[c]);
    assert!(s.curve(bc).is_none(), "deleting a point deletes its curves");
    assert!(s.points.is_empty());
    assert_eq!(s.next_id, 6, "ids are never reused");
}
