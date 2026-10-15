use super::*;

#[test]
fn ndc_round_trip() {
    let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), egui::vec2(400.0, 300.0));
    for p in [Pos2::new(10.0, 20.0), Pos2::new(410.0, 320.0), Pos2::new(123.0, 77.0)] {
        let back = from_ndc(to_ndc(p, rect), rect);
        assert!((back - p).length() < 1e-3, "{p:?} -> {back:?}");
    }
    assert_eq!(to_ndc(rect.center(), rect), Vec2::ZERO);
    assert_eq!(to_ndc(rect.left_top(), rect), Vec2::new(-1.0, 1.0));
}
