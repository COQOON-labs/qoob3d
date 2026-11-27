use super::*;

#[test]
fn colors_are_six_hex_digits_after_a_hash_or_a_basic_name() {
    assert_eq!(parse_color("#C0392B"), Some([0xC0, 0x39, 0x2B]));
    assert_eq!(parse_color("#c0392b"), Some([0xC0, 0x39, 0x2B]));
    assert_eq!(parse_color("white"), Some([0xFF; 3]));
    assert_eq!(parse_color("Grey"), parse_color("gray"));
    for bad in ["C0392B", "#C0392", "#C0392BFF", "#GG0000", "RAL 9016", "reddish", "", "#+1+2+3"] {
        assert_eq!(parse_color(bad), None, "{bad:?}");
    }
}

/// No two footprints on one plate overlap, and all keep the gap to the edge.
fn assert_packed(sizes: &[[f32; 2]], placed: &[Placement], bed: [f32; 2]) {
    for (i, (s, p)) in sizes.iter().zip(placed).enumerate() {
        if !p.fits {
            continue;
        }
        let (lo, hi) = (
            [p.centre[0] - s[0] / 2.0, p.centre[1] - s[1] / 2.0],
            [p.centre[0] + s[0] / 2.0, p.centre[1] + s[1] / 2.0],
        );
        for k in 0..2 {
            assert!(lo[k] >= ARRANGE_GAP_MM - 1e-3, "{i} leaves the bed: {lo:?}");
            assert!(hi[k] <= bed[k] - ARRANGE_GAP_MM + 1e-3, "{i} leaves the bed: {hi:?}");
        }
        for (j, (t, q)) in sizes.iter().zip(placed).enumerate().skip(i + 1) {
            if q.plate != p.plate {
                continue;
            }
            let apart = (0..2).any(|k| {
                (p.centre[k] - q.centre[k]).abs() >= (s[k] + t[k]) / 2.0 + ARRANGE_GAP_MM - 1e-3
            });
            assert!(apart, "{i} and {j} are closer than the gap on plate {}", p.plate);
        }
    }
}

#[test]
fn a_kit_larger_than_one_bed_spreads_over_plates() {
    // The six hull pieces of the Damen kit and some small parts.
    let sizes = [
        [204.0, 108.0],
        [200.0, 108.0],
        [188.0, 86.0],
        [198.0, 104.0],
        [192.0, 103.0],
        [187.0, 76.0],
        [14.0, 14.0],
        [14.0, 14.0],
        [71.0, 43.0],
    ];
    let bed = [256.0, 256.0];
    let placed = plates(&sizes, &[0; 9], bed, [0.0; 2]);
    assert_packed(&sizes, &placed, bed);
    let count = placed.iter().map(|p| p.plate).max().map_or(0, |m| m + 1);
    assert_eq!(count, 3, "two hull pieces per plate: {placed:?}");
    assert!(placed.iter().all(|p| p.fits));
}

#[test]
fn groups_never_share_a_plate_and_keep_their_order() {
    let sizes = [[20.0, 20.0]; 5];
    let groups = [7, 3, 7, 3, 9];
    let placed = plates(&sizes, &groups, [256.0, 256.0], [0.0; 2]);
    assert_packed(&sizes, &placed, [256.0, 256.0]);
    let plate_of = |g: usize| -> Vec<usize> {
        let mut p: Vec<usize> =
            groups.iter().zip(&placed).filter(|(x, _)| **x == g).map(|(_, p)| p.plate).collect();
        p.dedup();
        p
    };
    assert_eq!((plate_of(7), plate_of(3), plate_of(9)), (vec![0], vec![1], vec![2]));
}

#[test]
fn a_footprint_larger_than_the_bed_gets_a_plate_of_its_own_and_is_reported() {
    let sizes = [[300.0, 50.0], [20.0, 20.0]];
    let placed = plates(&sizes, &[0, 0], [256.0, 256.0], [0.0; 2]);
    assert!(!placed[0].fits);
    assert_eq!(placed[0].centre, [128.0, 128.0]);
    assert!(placed[1].fits);
    assert_ne!(placed[0].plate, placed[1].plate);
}

#[test]
fn a_plate_takes_rows_until_its_depth_is_used() {
    // 246 mm usable: two rows of 100 mm depth and a gap fit, a third does not.
    let sizes = [[200.0, 100.0]; 3];
    let placed = plates(&sizes, &[0; 3], [256.0, 256.0], [0.0; 2]);
    assert_eq!(placed.iter().map(|p| p.plate).collect::<Vec<_>>(), [0, 0, 1]);
    assert_eq!(placed[1].centre, [105.0, 160.0]);
}

/// The purge zone at the front left corner of a Bambu Lab bed (18 × 28 mm):
/// Bambu Studio refuses to slice an object whose outline reaches into it.
#[test]
fn footprints_keep_out_of_the_empty_corner() {
    let corner = [18.0, 28.0];
    let overlaps = |s: [f32; 2], p: &Placement| {
        p.centre[0] - s[0] / 2.0 < corner[0] && p.centre[1] - s[1] / 2.0 < corner[1]
    };
    // A row of small parts starts beside the zone; the next row is behind it.
    let sizes = [[60.0, 40.0], [60.0, 40.0], [60.0, 40.0], [60.0, 40.0], [60.0, 20.0]];
    let placed = plates(&sizes, &[0; 5], [256.0, 256.0], corner);
    assert_packed(&sizes, &placed, [256.0, 256.0]);
    assert_eq!(placed[0].centre, [5.0 + 18.0 + 30.0, 25.0]);
    assert!(sizes.iter().zip(&placed).all(|(s, p)| !overlaps(*s, p)), "{placed:?}");
    // 240 mm wide: no room beside the zone, so the row lies behind it.
    let wide = [[240.0, 50.0]];
    let placed = plates(&wide, &[0], [256.0, 256.0], corner);
    assert_eq!((placed[0].centre, placed[0].fits), ([125.0, 5.0 + 28.0 + 25.0], true));
    // 240 × 240 has no room behind the zone either: reported, not hidden.
    let placed = plates(&[[240.0, 240.0]], &[0], [256.0, 256.0], corner);
    assert!(!placed[0].fits);
}
