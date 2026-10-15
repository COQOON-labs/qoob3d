use super::*;

#[test]
fn database_is_large_ids_unique_and_default_exists() {
    assert!(PRINTERS.len() > 300, "only {} printers", PRINTERS.len());
    let mut ids: Vec<_> = PRINTERS.iter().map(|p| p.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), PRINTERS.len(), "duplicate ids");
    assert_eq!(PrinterProfile::default_profile().size(), [256.0, 256.0]);
}

#[test]
fn every_profile_is_plausible() {
    for p in PRINTERS {
        assert!(p.printable_area.len() >= 3, "{p:?}");
        let [w, d] = p.size();
        assert!(w > 10.0 && d > 10.0 && w < 5000.0 && d < 5000.0, "{} {w}×{d}", p.name);
        assert!(p.max_height_mm > 10.0 && p.max_height_mm < 5000.0, "{}", p.name);
    }
}

#[test]
fn bambu_x1_carbon_has_front_left_exclusion() {
    let x1 = PRINTERS.iter().find(|p| p.name == "Bambu Lab X1 Carbon").unwrap();
    assert!(x1.has_exclude_area());
    let bed = x1.bed();
    // Printer origin corner (front left) maps to (-128, -128) in the world.
    let min_x = bed.exclude.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
    let min_y = bed.exclude.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
    assert_eq!([min_x, min_y], [-128.0, -128.0]);
}

#[test]
fn bambu_h2d_has_two_nozzle_areas() {
    let h2d = PRINTERS.iter().find(|p| p.name == "Bambu Lab H2D").unwrap();
    assert!(h2d.has_single_nozzle_zones());
    assert_eq!(h2d.extruder_areas.len(), 2);
    assert_eq!(h2d.size(), [350.0, 320.0]);
}

#[test]
fn a1_mini_is_small() {
    let mini = PRINTERS.iter().find(|p| p.name == "Bambu Lab A1 mini").unwrap();
    assert_eq!(mini.size(), [180.0, 180.0]);
    assert!(!mini.has_exclude_area());
}
