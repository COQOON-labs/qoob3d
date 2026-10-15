use super::*;

#[test]
fn parses_all_point_formats() {
    let a = polygon(&serde_json::json!(["0x0", "256x0", "256x256"])).unwrap();
    let b = polygon(&serde_json::json!("0x0,256x0,256x256")).unwrap();
    let c = polygon(&serde_json::json!(["0x0,256x0", "256x256"])).unwrap();
    assert_eq!(a, b);
    assert_eq!(a, c);
    assert_eq!(a[2], [256.0, 256.0]);
    assert!(parse_points("1x").is_err());
}

#[test]
fn slugs_are_stable() {
    assert_eq!(slug("Bambu Lab X1 Carbon"), "bambu-lab-x1-carbon");
    assert_eq!(slug("Troodon 2.0 - Klipper"), "troodon-2-0-klipper");
}

#[test]
fn polygon_area() {
    assert_eq!(area(&[[0.0, 0.0], [2.0, 0.0], [2.0, 3.0], [0.0, 3.0]]), 6.0);
}
