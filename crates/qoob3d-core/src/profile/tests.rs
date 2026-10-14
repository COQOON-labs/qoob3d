use super::*;

#[test]
fn point_in_triangulated_region() {
    let square = Profile {
        key: vec![EntityId(1)],
        area_mm2: 100.0,
        positions: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
    };
    assert!(square.contains([5.0, 5.0]));
    assert!(square.contains([0.0, 5.0]), "on the border");
    assert!(!square.contains([11.0, 5.0]));
}
