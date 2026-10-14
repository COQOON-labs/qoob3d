use super::*;

fn close(a: DVec2, b: DVec2) -> bool {
    a.distance(b) < 1e-9
}

#[test]
fn segment_intersections() {
    let x = intersections(
        Shape::Segment(DVec2::new(0.0, 0.0), DVec2::new(10.0, 10.0)),
        Shape::Segment(DVec2::new(0.0, 10.0), DVec2::new(10.0, 0.0)),
    );
    assert_eq!(x.len(), 1);
    assert!(close(x[0], DVec2::new(5.0, 5.0)));
    let parallel = intersections(
        Shape::Segment(DVec2::ZERO, DVec2::X),
        Shape::Segment(DVec2::Y, DVec2::new(1.0, 1.0)),
    );
    assert!(parallel.is_empty());
}

#[test]
fn circle_intersections() {
    let x = intersections(
        Shape::Segment(DVec2::new(-10.0, 0.0), DVec2::new(10.0, 0.0)),
        Shape::Circle(DVec2::ZERO, 5.0),
    );
    assert_eq!(x.len(), 2);
    let y =
        intersections(Shape::Circle(DVec2::ZERO, 5.0), Shape::Circle(DVec2::new(8.0, 0.0), 5.0));
    assert_eq!(y.len(), 2);
    assert!(y.iter().all(|p| (p.x - 4.0).abs() < 1e-9 && (p.y.abs() - 3.0).abs() < 1e-9));
    let tangent =
        intersections(Shape::Circle(DVec2::ZERO, 5.0), Shape::Circle(DVec2::new(10.0, 0.0), 5.0));
    assert_eq!(tangent.len(), 1);
}

#[test]
fn closest_points() {
    assert!(close(
        closest_on_segment(DVec2::new(5.0, 3.0), DVec2::ZERO, DVec2::new(10.0, 0.0)),
        DVec2::new(5.0, 0.0)
    ));
    assert!(close(closest_on_circle(DVec2::new(0.0, 9.0), DVec2::ZERO, 4.0), DVec2::new(0.0, 4.0)));
}
