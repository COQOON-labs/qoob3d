//! `text` of the script API (docs/requirements/agent.md §6.6, AG-SC-38):
//! lettering as a profile, checked on the geometry it makes.
//!
//! The font is DejaVu Sans Bold, the example font of every project (the
//! project folder of these tests is `docs/script-api`). Its numbers, read
//! from the font: 2048 units per em, "H" 1493 units high and its ink from
//! x = 188 to 1526, "I" a rectangle from x = 188 to 573 with the advance
//! 762. At the cap height 8 mm one unit is 8 / 1493 mm.
//!
//! Tolerances: `VOL` = 1e-6 relative (polygons: exact B-reps), `POS` =
//! 1e-6 mm for boxes, as in `tests/script.rs`.

use std::path::{Path, PathBuf};

use qoob3d_core::font::{Align, Setting, lettering};
use qoob3d_kernel::script::{RunOptions, RunOutput, ScriptError, Shape, run};

const VOL: f64 = 1e-6;
const POS: f64 = 1e-6;
/// mm per font unit at the cap height 8 mm.
const UNIT: f64 = 8.0 / 1493.0;
/// The options of most tests.
const F: &str = "let f = #{ font: \"lib/examples/DejaVuSans-Bold.ttf\", height: 8 };";

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/script-api")
}

fn run_in(dir: Option<&Path>, source: &str) -> Result<RunOutput, ScriptError> {
    let options = RunOptions { project_dir: dir.map(Path::to_path_buf), ..RunOptions::default() };
    run(&format!("{F}\n{source}"), &options)
}

fn run_ok(source: &str) -> RunOutput {
    run_in(Some(&examples()), source)
        .unwrap_or_else(|e| panic!("script failed: {e}\n--- script ---\n{source}"))
}

/// The message of the runtime error `source` ends with.
fn message(dir: Option<&Path>, source: &str) -> String {
    match run_in(dir, source) {
        Ok(out) => panic!("script must fail, printed {:?}\n{source}", out.printed),
        Err(ScriptError::Runtime { message, .. }) => message,
        Err(other) => panic!("a runtime error is expected, got {other:?}\n{source}"),
    }
}

fn shape<'a>(out: &'a RunOutput, name: &str) -> &'a Shape {
    &out.shapes.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no shape {name}")).1
}

fn printed(out: &RunOutput, line: usize) -> f64 {
    let text = out.printed.get(line).unwrap_or_else(|| panic!("no line {line}: {:?}", out.printed));
    text.trim().parse().unwrap_or_else(|e| panic!("line {line} {text:?}: {e}"))
}

fn assert_rel(actual: f64, expected: f64, tol: f64, what: &str) {
    let rel = (actual - expected).abs() / expected.abs();
    assert!(rel < tol, "{what}: got {actual}, expected {expected} (rel {rel:e} >= {tol:e})");
}

fn assert_bbox(s: &Shape, min: [f64; 3], max: [f64; 3], what: &str) {
    let (a, b) = s.bbox();
    let close = (0..3).all(|k| (a[k] - min[k]).abs() < POS && (b[k] - max[k]).abs() < POS);
    assert!(close, "bbox of {what}: got {a:?}..{b:?}, expected {min:?}..{max:?}");
}

/// A valid B-rep of `solids` solids and `volume` whose mesh is watertight
/// with the same volume.
fn assert_solid(s: &Shape, solids: usize, volume: f64, what: &str) {
    assert!(s.is_valid(), "{what} is not a valid B-rep");
    assert_eq!(s.solid_count(), solids, "{what}: solids");
    assert_rel(s.volume(), volume, VOL, &format!("volume of {what}"));
    let mesh = s.mesh(0.01).unwrap_or_else(|e| panic!("{what}: mesh failed: {e:?}"));
    let report = mesh.faces.analyze();
    assert!(report.is_watertight(), "{what}: mesh not watertight: {report:?}");
    // Planar faces mesh exactly; f32 positions round to < 1e-5 relative.
    assert_rel(report.volume_mm3, volume, 1e-5, &format!("mesh volume of {what}"));
}

/// The polygons of `text` at 8 mm as the font module sets them.
fn set(text: &str, spacing: f64) -> qoob3d_core::font::Lettering {
    let font = std::fs::read(examples().join("lib/examples/DejaVuSans-Bold.ttf")).expect("font");
    let setting = Setting { height: 8.0, spacing, align: Align::Left };
    lettering(&font, text, &setting).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

#[test]
fn ag_sc_38_the_height_is_the_cap_height() {
    let out = run_ok(
        "show(extrude(text(\"H\", f), 2), \"h\");\n\
         let big = #{ font: \"lib/examples/DejaVuSans-Bold.ttf\", height: 12.5 };\n\
         show(extrude(text(\"H\", big), 2), \"big\");\n\
         print(text(\"H\", f).area());",
    );
    // The baseline is y = 0, the ink starts at x = 0 and is 1338 units wide.
    assert_bbox(shape(&out, "h"), [0.0; 3], [1338.0 * UNIT, 8.0, 2.0], "H");
    assert_bbox(shape(&out, "big"), [0.0; 3], [1338.0 * 12.5 / 1493.0, 12.5, 2.0], "H at 12.5");
    // Two stems 385 × 1493 and a bar 568 × 291 units between them.
    let area = (2.0 * 385.0 * 1493.0 + 568.0 * 291.0) * UNIT * UNIT;
    assert_rel(printed(&out, 0), area, VOL, "area of H");
    assert_solid(shape(&out, "h"), 1, area * 2.0, "H");
}

#[test]
fn ag_sc_38_glyphs_follow_each_other_by_their_advance() {
    let out = run_ok(
        "show(extrude(text(\"I\", f), 1), \"one\");\n\
         show(extrude(text(\"II\", f), 1), \"two\");\n\
         let wide = #{ font: \"lib/examples/DejaVuSans-Bold.ttf\", height: 8, spacing: 1.5 };\n\
         show(extrude(text(\"II\", wide), 1), \"wide\");\n\
         show(extrude(text(\"I I\", f), 1), \"gap\");",
    );
    let stem = 385.0 * UNIT;
    assert_bbox(shape(&out, "one"), [0.0; 3], [stem, 8.0, 1.0], "I");
    // The second I starts one advance (762 units) after the first.
    assert_bbox(shape(&out, "two"), [0.0; 3], [762.0 * UNIT + stem, 8.0, 1.0], "II");
    assert_bbox(shape(&out, "wide"), [0.0; 3], [762.0 * UNIT + 1.5 + stem, 8.0, 1.0], "II spaced");
    // A space advances by 713 units and draws nothing.
    assert_bbox(shape(&out, "gap"), [0.0; 3], [(762.0 + 713.0) * UNIT + stem, 8.0, 1.0], "I I");
    assert_solid(shape(&out, "two"), 2, 2.0 * stem * 8.0, "II");
    assert_solid(shape(&out, "gap"), 2, 2.0 * stem * 8.0, "I I");
}

#[test]
fn ag_sc_38_counters_are_holes() {
    let out = run_ok(
        "let o = text(\"O\", f);\n\
         print(o.area());\n\
         print(outline(o).area());\n\
         show(extrude(o, 0.6), \"o\");\n\
         let b = text(\"B8\", f);\n\
         print(b.area());\n\
         print(outline(b).area());\n\
         show(extrude(b, 0.6), \"b8\");",
    );
    let (ink, filled) = (printed(&out, 0), printed(&out, 1));
    // Measured with this font: the counter is 31 % of the letter.
    assert_rel(ink, 38.385_376_65, VOL, "ink of O");
    assert_rel(filled, 55.586_889_94, VOL, "O without its counter");
    assert!(ink < 0.75 * filled, "the counter is missing: {ink} of {filled}");
    assert_solid(shape(&out, "o"), 1, ink * 0.6, "O");
    // A ring: outer and inner wall, top and bottom would be 4 faces if the
    // walls were one face each; here every polygon edge is a face.
    let polygons = set("O", 0.0);
    assert_eq!(polygons.regions[0].holes.len(), 1);
    assert_rel(ink, polygons.area(), VOL, "the area of the polygons");
    // B and 8 have two counters each.
    assert_rel(printed(&out, 2), set("B8", 0.0).area(), VOL, "ink of B8");
    assert!(printed(&out, 2) < 0.9 * printed(&out, 3));
    assert_solid(shape(&out, "b8"), 2, printed(&out, 2) * 0.6, "B8");
}

#[test]
fn ag_sc_38_the_area_of_a_text_is_the_sum_of_its_glyphs() {
    let out = run_ok(
        "let label = text(\"MAI 0102\", f);\n\
         print(label.area());\n\
         let sum = 0.0;\n\
         for c in [\"M\", \"A\", \"I\", \"0\", \"1\", \"0\", \"2\"] { sum += text(c, f).area(); }\n\
         print(sum);\n\
         show(extrude(label, 0.6), \"label\");",
    );
    let area = printed(&out, 0);
    assert_rel(area, printed(&out, 1), VOL, "sum of the glyph areas");
    // Measured with this font at 8 mm.
    assert_rel(area, 216.303_317_36, VOL, "area of MAI 0102");
    assert_solid(shape(&out, "label"), 7, area * 0.6, "MAI 0102");
    // 0 and 2 reach 29 units below the baseline and 27 above the cap height.
    let width = 55.898_191_561;
    assert_bbox(
        shape(&out, "label"),
        [0.0, -29.0 * UNIT, 0.0],
        [width, 1520.0 * UNIT, 0.6],
        "MAI 0102",
    );
}

#[test]
fn ag_sc_38_align_places_the_ink_box() {
    let out = run_ok(
        "let at = |a| extrude(text(\"MAI 0102\", #{ font: f.font, height: 8, align: a }), 1);\n\
         show(at.call(\"left\"), \"left\");\n\
         show(at.call(\"center\"), \"center\");\n\
         show(at.call(\"right\"), \"right\");",
    );
    let (w, low, high) = (55.898_191_561, -29.0 * UNIT, 1520.0 * UNIT);
    assert_bbox(shape(&out, "left"), [0.0, low, 0.0], [w, high, 1.0], "left");
    assert_bbox(shape(&out, "center"), [-w / 2.0, low, 0.0], [w / 2.0, high, 1.0], "center");
    assert_bbox(shape(&out, "right"), [-w, low, 0.0], [0.0, high, 1.0], "right");
    let (min, max) = shape(&out, "center").bbox();
    assert!((min[0] + max[0]).abs() < POS, "the centre of the box is x = 0");
}

#[test]
fn ag_sc_38_ink_that_overlaps_is_one_region() {
    let out = run_ok(
        "let c = text(\"\\u00C7\", f);\n\
         print(c.area());\n\
         print(text(\"C\", f).area());\n\
         show(extrude(c, 1), \"cedilla\");\n\
         let tight = text(\"II\", #{ font: f.font, height: 8, spacing: -3 });\n\
         print(tight.area());\n\
         show(extrude(tight, 1), \"tight\");",
    );
    // The cedilla of this font is a contour that runs into the C: as a sum
    // of polygons the overlap would count twice.
    let polygons = set("\u{C7}", 0.0);
    assert!(polygons.overlapping);
    assert_eq!(polygons.regions.len(), 2);
    let (fused, letter) = (printed(&out, 0), printed(&out, 1));
    assert!(fused > letter + 1.0, "C with cedilla {fused}, C {letter}");
    assert!(fused < polygons.area() - 0.01, "fused {fused}, sum {}", polygons.area());
    assert_solid(shape(&out, "cedilla"), 1, fused, "C with cedilla");
    // Two stems 385 units wide whose origins are 762 units − 3 mm apart:
    // one rectangle.
    let width = 762.0 * UNIT - 3.0 + 385.0 * UNIT;
    assert_rel(printed(&out, 2), width * 8.0, VOL, "two stems that overlap");
    assert_solid(shape(&out, "tight"), 1, width * 8.0, "II, tight");
    assert_bbox(shape(&out, "tight"), [0.0; 3], [width, 8.0, 1.0], "II, tight");
}

#[test]
fn ag_sc_38_a_text_is_a_profile_like_the_others() {
    let out = run_ok(
        "let label = text(\"MAI 0102\", f);\n\
         print(label.area());\n\
         let stencil = rect(70, 14).move(28, 4) - label;\n\
         print(stencil.area());\n\
         let plate = box(70, 14, 2).move(28, 4, 0);\n\
         show(plate - extrude(label, 0.6).move(0, 0, 1.4), \"engraved\");\n\
         show(plate + extrude(label, 0.6).move(0, 0, 2), \"raised\");\n\
         show(extrude(on(\"xz\", label.rotate(90).move(3, 0)), 1), \"upright\");",
    );
    let area = printed(&out, 0);
    assert_rel(printed(&out, 1), 70.0 * 14.0 - area, VOL, "stencil");
    assert_solid(shape(&out, "engraved"), 1, 70.0 * 14.0 * 2.0 - area * 0.6, "engraved plate");
    assert_solid(shape(&out, "raised"), 1, 70.0 * 14.0 * 2.0 + area * 0.6, "raised letters");
    // Turned by 90° the line runs up: on XZ its length is along Z, and the
    // extrusion goes along −Y.
    let (low, high, width) = (-29.0 * UNIT, 1520.0 * UNIT, 55.898_191_561);
    assert_bbox(
        shape(&out, "upright"),
        [3.0 - high, -1.0, 0.0],
        [3.0 - low, 0.0, width],
        "upright text",
    );
    assert_solid(shape(&out, "upright"), 7, area, "upright text");
}

#[test]
fn ag_sc_38_letters_with_diacritics_and_long_lines() {
    // Romanian: comma below, breve, circumflex.
    let word = "\u{218}tiin\u{21B}\u{103} \u{219}i \u{EE}nv\u{103}\u{21B}\u{103}m\u{E2}nt";
    let line = "The quick brown fox jumps over the lazy dog 0123456789";
    let out = run_ok(&format!(
        "let word = text(\"{word}\", f);\n\
         print(word.area());\n\
         show(extrude(word, 0.4), \"word\");\n\
         let line = text(\"{line}\", f);\n\
         print(line.area());\n\
         show(extrude(line, 0.4), \"line\");"
    ));
    for (name, text, line) in [("word", word, 0), ("line", line, 1)] {
        let polygons = set(text, 0.0);
        assert!(!polygons.overlapping, "{name}");
        assert_rel(printed(&out, line), polygons.area(), VOL, &format!("area of {name}"));
        let solids = polygons.regions.len();
        assert_solid(shape(&out, name), solids, printed(&out, line) * 0.4, name);
    }
    // 43 letters and digits, i and j with their dots.
    assert_eq!(shape(&out, "line").solid_count(), 47);
}

#[test]
fn ag_sc_38_the_font_is_an_input_of_the_build() {
    let out = run_ok("text(\"A\", f); text(\"B\", f); text(\"C\", #{ font: f.font, height: 3 });");
    assert_eq!(out.files_read, [PathBuf::from("lib/examples/DejaVuSans-Bold.ttf")]);
    // A file that is no font is read, and recorded, too.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("script-text").join("inputs");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).expect("create project");
    std::fs::write(dir.join("lib/broken.ttf"), b"\x00\x01\x00\x00 no tables").expect("write");
    let options = RunOptions { project_dir: Some(dir), ..RunOptions::default() };
    let source =
        "try { text(\"A\", #{ font: \"lib/broken.ttf\", height: 8 }); } catch (e) { print(e); }";
    let out = run(source, &options).expect("the error is caught");
    assert_eq!(out.files_read, [PathBuf::from("lib/broken.ttf")]);
    assert!(out.printed[0].starts_with("text: lib/broken.ttf: not a TrueType or OpenType font"));
}

#[test]
fn ag_sc_38_a_character_without_a_glyph_is_an_error() {
    let here = examples();
    let fails = |source: &str| message(Some(&here), source);
    // DejaVu has no CJK glyphs: the character is named, no box is drawn.
    assert_eq!(
        fails("text(\"MAI \\u4E2D\", f)"),
        "Runtime error: text: the font has no glyph for '\u{4E2D}' (U+4E2D)"
    );
    assert_eq!(fails("text(\"\", f)"), "Runtime error: text: the string is empty");
    assert!(fails("text(\"   \", f)").contains("text: the string draws nothing"));
    assert!(fails("text(\"MAI\\n0102\", f)").contains("text: the string has a line break"));
    let long = "I".repeat(201);
    assert!(
        fails(&format!("text(\"{long}\", f)"))
            .contains("201 characters, more than the limit of 200")
    );
}

#[test]
fn ag_sc_38_the_font_must_be_a_font_file_of_the_project() {
    let here = examples();
    let fails = |source: &str| message(Some(&here), source);
    let with = |font: &str| format!("text(\"A\", #{{ font: \"{font}\", height: 8 }})");
    assert!(
        fails(&with("lib/examples/none.ttf")).contains("text: font \"lib/examples/none.ttf\": "),
        "a missing file"
    );
    assert_eq!(
        fails(&with("../script-api.md")),
        "Runtime error: text: font \"../script-api.md\" must be a path inside the project"
    );
    assert_eq!(
        fails(&with("/System/Library/Fonts/Helvetica.ttc")),
        "Runtime error: text: font \"/System/Library/Fonts/Helvetica.ttc\" must be a path inside \
         the project"
    );
    assert_eq!(
        fails(&with("lib/examples/block.step")),
        "Runtime error: text: lib/examples/block.step: not a TrueType or OpenType font (Invalid \
         sfnt version 0x49534F2D)"
    );
    assert_eq!(
        message(None, &with("lib/examples/DejaVuSans-Bold.ttf")),
        "Runtime error: text needs a project folder"
    );
}

#[test]
fn ag_sc_38_the_options_are_checked() {
    let here = examples();
    let fails = |source: &str| message(Some(&here), source);
    assert!(fails("text(\"A\")").contains("text needs its options"));
    assert!(fails("text(\"A\", #{ height: 8 })").contains("text: the option font is missing"));
    assert!(fails("text(\"A\", #{ font: f.font })").contains("text: the option height is missing"));
    assert!(fails("text(\"A\", #{ font: 3, height: 8 })").contains("text: font must be the path"));
    for height in ["0", "-2", "\"8\""] {
        let source = format!("text(\"A\", #{{ font: f.font, height: {height} }})");
        assert!(fails(&source).contains("text: height must be"), "{height}");
    }
    assert!(
        fails("text(\"A\", #{ font: f.font, height: 8, spacing: \"wide\" })")
            .contains("text: spacing must be a number")
    );
    assert!(
        fails("text(\"A\", #{ font: f.font, height: 8, align: \"middle\" })")
            .contains("text: align must be \"left\", \"center\" or \"right\", got \"middle\"")
    );
    assert!(
        fails("text(\"A\", #{ font: f.font, height: 8, size: 3 })")
            .contains("text: unknown option \"size\"; known: font, height, spacing, align")
    );
}
