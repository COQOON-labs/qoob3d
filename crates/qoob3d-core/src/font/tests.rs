//! Lettering (AG-SC-38). The font of the tests is DejaVu Sans Bold, the one
//! `qoob3d init` copies into a project: 2048 units per em, "H" 1493 units
//! high (no capital height in its OS/2 table), advances I 762, space 713,
//! A and V 1585, kerning A V −139. At the height 8 mm of the tests one unit
//! is 8 / 1493 mm.

use super::regions::{Glyph, assemble, cleaned, regions, signed_area};
use super::*;

/// mm per font unit at the cap height 8 mm.
const UNIT: f64 = 8.0 / 1493.0;
const EIGHT: Setting = Setting { height: 8.0, spacing: 0.0, align: Align::Left };

fn dejavu() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/script-api/lib/examples/DejaVuSans-Bold.ttf"
    );
    std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn set(text: &str) -> Lettering {
    lettering(&dejavu(), text, &EIGHT).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

fn width(l: &Lettering) -> f64 {
    let (min, max) = l.bounds();
    max[0] - min[0]
}

fn close(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < 1e-9, "{what}: got {actual}, expected {expected}");
}

fn square(x: f64, y: f64, side: f64) -> Vec<[f64; 2]> {
    vec![[x, y], [x + side, y], [x + side, y + side], [x, y + side]]
}

fn glyph_of(character: char, contours: Vec<Vec<[f64; 2]>>) -> Glyph {
    Glyph { character, contours }
}

// --- polygon geometry -------------------------------------------------------

#[test]
fn the_signed_area_follows_the_direction() {
    let mut s = square(1.0, 2.0, 3.0);
    close(signed_area(&s), 9.0, "counter-clockwise");
    s.reverse();
    close(signed_area(&s), -9.0, "clockwise");
}

#[test]
fn cleaning_drops_repeated_points_and_contours_without_area() {
    let repeated = vec![
        [0.0, 0.0],
        [0.0, 0.0],
        [4.0, 0.0],
        [4.0, 4.0],
        [4.0, 4.0 + 1e-9],
        [0.0, 4.0],
        [0.0, 0.0],
    ];
    assert_eq!(cleaned(repeated, 1e-6), Some(square(0.0, 0.0, 4.0)));
    // Two points, a line walked there and back, a point: no ink.
    assert_eq!(cleaned(vec![[0.0, 0.0], [5.0, 0.0]], 1e-6), None);
    assert_eq!(cleaned(vec![[0.0, 0.0], [5.0, 0.0], [9.0, 0.0], [5.0, 0.0]], 1e-6), None);
    assert_eq!(cleaned(vec![[1.0, 1.0]; 5], 1e-6), None);
    assert_eq!(cleaned(Vec::new(), 1e-6), None);
    // Smaller than the shortest edge: nothing is left of it.
    assert_eq!(cleaned(square(0.0, 0.0, 1e-7), 1e-6), None);
}

#[test]
fn holes_follow_from_nesting_not_from_the_winding() {
    // A 10 square, a 6 square in it and a 2 square in that one, all running
    // the same way: ring, and an island in its hole.
    let contours = vec![square(4.0, 4.0, 2.0), square(0.0, 0.0, 10.0), square(2.0, 2.0, 6.0)];
    let found = regions(contours);
    assert_eq!(found.len(), 2, "{found:?}");
    let island = &found[0];
    let ring = &found[1];
    assert!(island.holes.is_empty());
    close(signed_area(&island.outline), 4.0, "island");
    close(signed_area(&ring.outline), 100.0, "ring outline, counter-clockwise");
    assert_eq!(ring.holes.len(), 1);
    close(signed_area(&ring.holes[0]), -36.0, "ring hole, clockwise");

    // The same with every contour clockwise.
    let clockwise: Vec<_> = [square(0.0, 0.0, 10.0), square(2.0, 2.0, 6.0)]
        .into_iter()
        .map(|mut c| {
            c.reverse();
            c
        })
        .collect();
    let found = regions(clockwise);
    assert_eq!(found.len(), 1);
    close(signed_area(&found[0].outline), 100.0, "outline");
    close(signed_area(&found[0].holes[0]), -36.0, "hole");
}

#[test]
fn every_hole_goes_to_the_outline_around_it() {
    // Two rings side by side and a plain square; the "B" case is two holes
    // in one outline.
    let contours = vec![
        square(1.0, 1.0, 2.0),
        square(0.0, 0.0, 10.0),
        square(21.0, 1.0, 3.0),
        square(20.0, 0.0, 10.0),
        square(5.0, 5.0, 2.0),
        square(40.0, 0.0, 1.0),
    ];
    let found = regions(contours);
    let holes: Vec<usize> = found.iter().map(|r| r.holes.len()).collect();
    assert_eq!(holes, [2, 1, 0]);
    let ink: f64 = found
        .iter()
        .map(|r| signed_area(&r.outline) + r.holes.iter().map(|h| signed_area(h)).sum::<f64>())
        .sum();
    close(ink, (100.0 - 4.0 - 4.0) + (100.0 - 9.0) + 1.0, "ink");
}

#[test]
fn glyphs_apart_do_not_overlap() {
    let glyphs = vec![
        glyph_of('a', vec![square(0.0, 0.0, 10.0), square(2.0, 2.0, 6.0)]),
        glyph_of('b', vec![square(11.0, 0.0, 10.0)]),
        // Inside the hole of 'a', touching nothing.
        glyph_of('c', vec![square(4.0, 4.0, 2.0)]),
    ];
    let (found, overlapping) = assemble(glyphs).expect("simple contours");
    assert_eq!(found.len(), 3);
    assert!(!overlapping);
}

#[test]
fn crossing_touching_and_covered_glyphs_overlap() {
    let overlap = |second: Vec<[f64; 2]>| {
        let glyphs = vec![glyph_of('a', vec![square(0.0, 0.0, 10.0)]), glyph_of('b', vec![second])];
        assemble(glyphs).expect("simple contours").1
    };
    assert!(overlap(square(5.0, 5.0, 10.0)), "crossing");
    assert!(overlap(square(10.0, 10.0, 5.0)), "touching at a corner");
    assert!(overlap(square(10.0, 2.0, 5.0)), "sharing a piece of an edge");
    assert!(overlap(square(3.0, 3.0, 2.0)), "inside the ink of the other, no edge crossing");
    assert!(overlap(square(-5.0, -5.0, 30.0)), "around the other");
    assert!(!overlap(square(10.5, 0.0, 5.0)), "apart");
    // Two contours of one glyph that cross (a stroke over a bowl).
    let glyph = glyph_of('a', vec![square(0.0, 0.0, 10.0), square(8.0, 4.0, 10.0)]);
    let (found, overlapping) = assemble(vec![glyph]).expect("simple contours");
    assert_eq!(found.len(), 2, "both are ink");
    assert!(overlapping);
}

#[test]
fn a_contour_that_crosses_or_touches_itself_is_refused() {
    let refused = |contour: Vec<[f64; 2]>| {
        let glyphs =
            vec![glyph_of('I', vec![square(-20.0, 0.0, 5.0)]), glyph_of('x', vec![contour])];
        assemble(glyphs).err()
    };
    let bow_tie = vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0], [10.0, 12.0]];
    assert_eq!(refused(bow_tie), Some(FontError::SelfCrossing('x')));
    // A spike: out along an edge and back on it.
    let spike =
        vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [10.0, 15.0], [10.0, 10.0], [0.0, 10.0]];
    assert_eq!(refused(spike), Some(FontError::SelfCrossing('x')));
    // Two loops that meet in one point.
    let pinched = vec![[0.0, 0.0], [4.0, 0.0], [2.0, 2.0], [4.0, 4.0], [0.0, 4.0], [2.0, 2.0]];
    assert_eq!(refused(pinched), Some(FontError::SelfCrossing('x')));
    assert_eq!(refused(square(0.0, 0.0, 10.0)), None);
    assert_eq!(
        FontError::SelfCrossing('x').to_string(),
        "the outline of 'x' (U+0078) crosses or touches itself"
    );
}

// --- flattening -------------------------------------------------------------

/// Largest distance of `samples` points of the curve from the polyline.
fn deviation(polyline: &[[f64; 2]], curve: impl Fn(f64) -> [f64; 2], samples: usize) -> f64 {
    let to_segment = |p: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
    };
    (0..=samples)
        .map(|i| {
            let p = curve(i as f64 / samples as f64);
            polyline.windows(2).map(|w| to_segment(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max)
}

#[test]
fn a_quadratic_curve_stays_within_the_tolerance() {
    // Second difference (0 − 1000 + 1000, 0 − 2000 + 0): 2000 long, so
    // ceil(sqrt(2000 / (4 · 3))) = 13 segments.
    let mut flat = Flattener::new(3.0, 0.0);
    flat.move_to(0.0, 0.0);
    flat.quad_to(500.0, 1000.0, 1000.0, 0.0);
    let line = &flat.contours[0];
    assert_eq!(line.len(), 14);
    assert_eq!(line[13], [1000.0, 0.0]);
    let curve = |t: f64| [1000.0 * t, 2000.0 * t * (1.0 - t)];
    let off = deviation(line, curve, 1000);
    assert!(off <= 3.0 && off > 1.0, "{off}");
}

#[test]
fn a_cubic_curve_stays_within_the_tolerance() {
    let mut flat = Flattener::new(3.0, 100.0);
    flat.move_to(0.0, 0.0);
    flat.curve_to(0.0, 600.0, 400.0, 1000.0, 1000.0, 1000.0);
    let line = &flat.contours[0];
    // Shifted by the pen position.
    assert_eq!(line[0], [100.0, 0.0]);
    let curve = |t: f64| {
        let s = 1.0 - t;
        [
            100.0 + 3.0 * s * t * t * 400.0 + t * t * t * 1000.0,
            3.0 * s * s * t * 600.0 + 3.0 * s * t * t * 1000.0 + t * t * t * 1000.0,
        ]
    };
    let off = deviation(line, curve, 1000);
    assert!(off <= 3.0, "{off}");
    assert!(line.len() <= 1 + MAX_CURVE_SEGMENTS && line.len() > 6, "{}", line.len());
}

#[test]
fn the_number_of_segments_is_bounded() {
    assert_eq!(segments(0.0, 0.25, 3.0), 1, "a straight curve");
    assert_eq!(segments(48.0, 0.25, 3.0), 2);
    assert_eq!(segments(1e12, 0.25, 3.0), MAX_CURVE_SEGMENTS);
    assert_eq!(segments(f64::INFINITY, 0.75, 3.0), MAX_CURVE_SEGMENTS);
    assert_eq!(segments(f64::NAN, 0.75, 3.0), 1);
    assert_eq!(segments(10.0, 0.25, 0.0), MAX_CURVE_SEGMENTS, "no tolerance");
}

#[test]
fn a_glyph_has_at_most_the_limit_of_points() {
    let mut flat = Flattener::new(1.0, 0.0);
    for i in 0..3 * MAX_GLYPH_POINTS {
        flat.move_to(i as f32, 0.0);
        flat.line_to(i as f32, 1.0);
    }
    assert!(flat.overflow);
    assert_eq!(flat.contours.iter().map(Vec::len).sum::<usize>(), MAX_GLYPH_POINTS);
    assert!(flat.contours.len() <= MAX_GLYPH_POINTS, "no contour without a point");

    let mut flat = Flattener::new(1.0, 0.0);
    flat.line_to(1.0, 1.0);
    flat.quad_to(1.0, 1.0, 2.0, 2.0);
    assert!(flat.contours.is_empty(), "nothing before the first move");
    flat.move_to(0.0, f32::NAN);
    assert!(flat.broken);
}

// --- the font ---------------------------------------------------------------

#[test]
fn the_height_is_the_height_of_a_capital_h() {
    let h = set("H");
    assert_eq!(h.regions.len(), 1);
    assert_eq!(h.bounds(), ([0.0, 0.0], [(1526.0 - 188.0) * UNIT, 8.0]));
    assert!(!h.overlapping);
    // Two stems 385 units wide and a bar 291 high between them: no curve.
    assert_eq!(h.regions[0].outline.len(), 12);
}

#[test]
fn the_same_text_has_the_same_polygons_at_every_height() {
    let small = set("MAI 0102 gq");
    let large = lettering(&dejavu(), "MAI 0102 gq", &Setting { height: 24.0, ..EIGHT })
        .expect("the same text");
    assert_eq!(small.regions.len(), large.regions.len());
    for (a, b) in small.regions.iter().zip(&large.regions) {
        assert_eq!(a.outline.len(), b.outline.len());
        for (p, q) in a.outline.iter().zip(&b.outline) {
            close(3.0 * p[0], q[0], "x");
            close(3.0 * p[1], q[1], "y");
        }
    }
    close(9.0 * small.area(), large.area(), "area");
}

#[test]
fn glyphs_follow_each_other_by_their_advance() {
    let (one, two, three) = (set("I"), set("II"), set("III"));
    close(width(&one), (573.0 - 188.0) * UNIT, "the stem");
    close(width(&two) - width(&one), 762.0 * UNIT, "advance of I");
    close(width(&three) - width(&two), 762.0 * UNIT, "advance of I");
    // A space advances by 713 units and draws nothing.
    let spaced = set("I I");
    assert_eq!(spaced.regions.len(), 2);
    close(width(&spaced) - width(&one), (762.0 + 713.0) * UNIT, "I and space");
    // Leading and trailing spaces do not move the ink of left-aligned text.
    assert_eq!(set("  I ").regions, one.regions);
}

#[test]
fn spacing_is_added_after_every_glyph() {
    let wide = lettering(&dejavu(), "III", &Setting { spacing: 1.5, ..EIGHT }).expect("III");
    close(width(&wide) - width(&set("III")), 3.0, "two gaps of 1.5 mm");
    let tight = lettering(&dejavu(), "II", &Setting { spacing: -1.0, ..EIGHT }).expect("II");
    close(width(&set("II")) - width(&tight), 1.0, "one gap");
    assert!(!tight.overlapping);
}

#[test]
fn pairs_are_kerned_with_the_kern_table() {
    // A and V: 1585 − 139 to the origin of V, whose ink ends at 1575; the
    // ink of A starts at 10.
    close(width(&set("AV")), (1585.0 - 139.0 + 1575.0 - 10.0) * UNIT, "AV");
    // No pair for H H.
    close(width(&set("HH")) - width(&set("H")), 1714.0 * UNIT, "HH");
}

#[test]
fn align_places_the_ink_box() {
    let at = |align| {
        let l = lettering(&dejavu(), " MAI 0102", &Setting { align, ..EIGHT }).expect("text");
        l.bounds()
    };
    let (left, center, right) = (at(Align::Left), at(Align::Center), at(Align::Right));
    let w = left.1[0];
    close(left.0[0], 0.0, "left edge");
    close(center.0[0], -w / 2.0, "centred: left");
    close(center.1[0], w / 2.0, "centred: right");
    close(right.0[0], -w, "right: left");
    close(right.1[0], 0.0, "right edge");
    // The baseline stays.
    assert_eq!((left.0[1], left.1[1]), (center.0[1], center.1[1]));
    assert_eq!(left.0[1], -29.0 * UNIT, "the round letters reach below the baseline");
}

#[test]
fn counters_are_holes() {
    let o = set("O");
    assert_eq!(o.regions.len(), 1);
    assert_eq!(o.regions[0].holes.len(), 1);
    let outline = signed_area(&o.regions[0].outline);
    let hole = signed_area(&o.regions[0].holes[0]);
    assert!(outline > 0.0 && hole < 0.0, "outline {outline}, hole {hole}");
    close(o.area(), outline + hole, "ink");
    // The outline is a little squarer than the ellipse in its box of
    // 1536 × 1549 units.
    let ellipse = std::f64::consts::PI / 4.0 * 1536.0 * 1549.0 * UNIT * UNIT;
    assert!((1.0..1.06).contains(&(outline / ellipse)), "outline {outline}, ellipse {ellipse}");
    assert!(o.area() < 0.75 * outline, "the counter is a good part of it");
    // Moderate point counts: a round letter has a few dozen.
    let points = o.regions[0].outline.len() + o.regions[0].holes[0].len();
    assert!((40..=100).contains(&points), "{points}");

    let holes = |text: &str| set(text).regions.iter().map(|r| r.holes.len()).sum::<usize>();
    assert_eq!([holes("B"), holes("8"), holes("A"), holes("i"), holes("M")], [2, 2, 1, 0, 0]);
    assert_eq!(set("i").regions.len(), 2, "stem and dot");
}

#[test]
fn the_area_of_a_text_is_the_sum_of_its_glyphs() {
    let text = set("MAI 0102");
    assert_eq!(text.regions.len(), 7);
    assert!(!text.overlapping);
    let glyphs: f64 = "MAI0102".chars().map(|c| set(&c.to_string()).area()).sum();
    close(text.area(), glyphs, "sum of the glyphs");
    // Measured with this font; "I" alone is a rectangle 385 × 1493 units.
    assert!((text.area() - 216.303_317).abs() < 1e-6, "{}", text.area());
    close(set("I").area(), 385.0 * UNIT * 8.0, "I");
}

#[test]
fn romanian_diacritics_are_glyphs_of_the_font() {
    let text = set("ȘȚĂÂÎșțăâî");
    // Every letter with its mark; ă and â have a counter, Ă and Â too.
    assert_eq!(text.regions.len(), 20);
    assert_eq!(text.regions.iter().map(|r| r.holes.len()).sum::<usize>(), 4);
    assert!(!text.overlapping);
    // The comma below Ș reaches under the baseline.
    assert!(set("Ș").bounds().0[1] < -2.0);
}

#[test]
fn ink_that_overlaps_is_reported() {
    // The cedilla of this font is a contour that runs into the letter.
    assert!(set("Ç").overlapping);
    assert!(!set("C").overlapping);
    // Stems 2.06 mm wide, 4.08 mm apart: 3 mm tighter they overlap.
    let tight = lettering(&dejavu(), "II", &Setting { spacing: -3.0, ..EIGHT }).expect("II");
    assert!(tight.overlapping);
    assert_eq!(tight.regions.len(), 2);
}

#[test]
fn a_blank_braille_cell_only_advances() {
    // U+2800 is a glyph without an outline and no white space.
    let cells = set("\u{2801}\u{2800}\u{2801}");
    assert_eq!(cells.regions.len(), 2);
    assert!(width(&cells) > 2.0 * width(&set("\u{2801}")));
}

#[test]
fn the_string_is_checked() {
    let font = dejavu();
    let fails = |text: &str| lettering(&font, text, &EIGHT).expect_err(text);
    assert_eq!(fails(""), FontError::Empty);
    assert_eq!(fails("   "), FontError::Blank);
    assert_eq!(fails("\u{2800}"), FontError::Blank);
    assert_eq!(fails("A\nB"), FontError::LineBreak);
    assert_eq!(fails("A\r"), FontError::LineBreak);
    assert_eq!(fails("A\u{2028}B"), FontError::LineBreak);
    assert_eq!(fails(&"I".repeat(MAX_CHARS + 1)), FontError::TooLong(MAX_CHARS + 1));
    assert_eq!(lettering(&font, &"I".repeat(MAX_CHARS), &EIGHT).expect("200").regions.len(), 200);
    // DejaVu has no CJK glyphs and no glyph for a tab.
    assert_eq!(fails("A中"), FontError::MissingGlyph('中'));
    assert_eq!(fails("A中").to_string(), "the font has no glyph for '中' (U+4E2D)");
    assert_eq!(fails("A\tB").to_string(), "the font has no glyph for '\\t' (U+0009)");
}

#[test]
fn the_setting_is_checked() {
    let font = dejavu();
    for height in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let e = lettering(&font, "A", &Setting { height, ..EIGHT }).expect_err("height");
        assert!(matches!(e, FontError::Height(_)), "{height}: {e}");
    }
    for spacing in [f64::NAN, f64::NEG_INFINITY] {
        let e = lettering(&font, "A", &Setting { spacing, ..EIGHT }).expect_err("spacing");
        assert!(matches!(e, FontError::Spacing(_)), "{spacing}: {e}");
    }
    // So small that no edge is left: nothing to draw.
    let e = lettering(&font, "A", &Setting { height: 1e-9, ..EIGHT }).expect_err("tiny");
    assert_eq!(e, FontError::Blank);
}

#[test]
fn bytes_that_are_no_font_are_refused() {
    let fails = |bytes: &[u8]| lettering(bytes, "A", &EIGHT).expect_err("no font");
    assert!(matches!(fails(b""), FontError::NotAFont(_)));
    assert!(matches!(fails(b"ISO-10303-21;\nHEADER;\n"), FontError::NotAFont(_)));
    assert!(fails(&[0; 64]).to_string().starts_with("not a TrueType or OpenType font ("));
    let huge = vec![0; MAX_FONT_BYTES + 1];
    assert_eq!(fails(&huge), FontError::TooLarge(MAX_FONT_BYTES + 1));
    // A TrueType header whose table directory runs past the end, and a
    // complete directory without tables: no font either.
    assert!(matches!(fails(b"\x00\x01\x00\x00 no tables"), FontError::NotAFont(_)));
    assert!(matches!(fails(&[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), FontError::NotAFont(_)));
}

#[test]
fn a_font_without_the_tables_for_metrics_is_refused() {
    // DejaVu with the tag of one required table renamed: `head` (units per
    // em), `hhea` (advances), `maxp` (glyph count) each make it no font.
    for tag in [b"head", b"hhea", b"maxp"] {
        let mut font = dejavu();
        let tables = usize::from(u16::from_be_bytes([font[4], font[5]]));
        let record = (0..tables).map(|i| 12 + 16 * i).find(|at| &font[*at..*at + 4] == tag);
        let at = record.unwrap_or_else(|| panic!("DejaVu has {}", String::from_utf8_lossy(tag)));
        font[at..at + 4].copy_from_slice(b"zzzz");
        let e = lettering(&font, "A", &EIGHT).expect_err("a required table is missing");
        assert!(matches!(e, FontError::NotAFont(_)), "{}: {e}", String::from_utf8_lossy(tag));
    }
}

#[test]
fn a_truncated_font_never_panics() {
    let font = dejavu();
    assert_eq!(&font[..4], [0, 1, 0, 0], "a TrueType font");
    let mut read = 0;
    // The table directory byte by byte, the tables in steps.
    for len in (0..400).chain((400..font.len()).step_by(3989)) {
        read += usize::from(lettering(&font[..len], "MAI 0102 Șg", &EIGHT).is_ok());
    }
    // `glyf` is the largest table and not the last: some prefixes still set.
    assert!(read < 150, "{read} prefixes read");
}

#[test]
fn a_glyph_without_points_but_with_flags_never_panics() {
    // Found by `a_damaged_font_never_panics`: `read-fonts` 0.41 indexed the
    // flags of a simple glyph with no points out of bounds.
    let mut font = dejavu();
    font[256] = 0;
    font[12 + 16 * 9 + 10] = 106;
    let _ = lettering(&font, "MAI 0102 ȘgÇ8", &EIGHT);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(48))]

    /// Damaged bytes anywhere in the font: an error or a text, no panic,
    /// and never more points than the limits allow.
    #[test]
    fn a_damaged_font_never_panics(
        damage in proptest::collection::vec((0usize..705_684, proptest::num::u8::ANY), 1..24),
        table in 0usize..19,
        value in proptest::num::u8::ANY,
    ) {
        let mut font = dejavu();
        for (at, byte) in damage {
            font[at] = byte;
        }
        // One byte of a table record (tag, checksum, offset, length).
        font[12 + 16 * table + usize::from(value % 16)] = value;
        if let Ok(text) = lettering(&font, "MAI 0102 ȘgÇ8", &EIGHT) {
            let points: usize = text.regions.iter().map(|r| r.outline.len()).sum();
            proptest::prop_assert!(points <= MAX_POINTS);
        }
    }
}
