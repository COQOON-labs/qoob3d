//! Headless UI tests of the real app with egui_kittest + wgpu (ADR-0018).
//!
//! `QOOB3D_SCREENSHOT_DIR=<dir> cargo test -p qoob3d-app --test ui` (or
//! `cargo xtask screenshot`) additionally writes `<dir>/ui.png`, so humans
//! and the coding agent can look at the complete UI.

use std::time::{Duration, Instant};

use eframe::egui::{self, CursorIcon, Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use qoob3d_app::i18n::{self, Language};
use qoob3d_app::viewcube::CubeFace;
use qoob3d_app::{Mode, QoobApp, StartOptions};
use qoob3d_render::Projection;

fn harness(demo: bool) -> Harness<'static, QoobApp> {
    Harness::builder().with_size(egui::vec2(1440.0, 860.0)).wgpu().build_eframe(move |cc| {
        QoobApp::new(cc, StartOptions { demo, ..Default::default() }).expect("app starts")
    })
}

/// The app with the demo body loaded (navigation tests need geometry).
fn app() -> Harness<'static, QoobApp> {
    let mut harness = harness(true);
    let start = Instant::now();
    while !harness.state().has_model() {
        assert!(start.elapsed() < Duration::from_secs(60), "kernel did not deliver a model");
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    // Let the first fit and layout settle.
    harness.run_steps(4);
    harness
}

fn click_at(harness: &mut Harness<'_, QoobApp>, pos: Pos2) {
    harness.event(Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.step();
    }
}

/// Steps until camera animations are done.
fn settle(harness: &mut Harness<'_, QoobApp>) {
    for _ in 0..400 {
        if !harness.state().viewport().is_animating() {
            return;
        }
        harness.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("camera animation did not finish");
}

#[test]
fn layout_has_toolbar_browser_timeline_and_navigation_bar() {
    let harness = app();
    // English is the base language (ADR-0020).
    for label in [
        "QOOB3D",
        "DESIGN",
        "✏ Create Sketch",
        "⬆ Extrude",
        "BROWSER",
        "📁 Bodies",
        "◼ Demo part",
        "⛶ Fit",
        "Perspective",
        "Orthographic",
        "Grid",
        "Build volume",
        "⚙ Settings",
    ] {
        harness.get_by_label(label);
    }
}

#[test]
fn projection_toggle_switches_camera() {
    let mut harness = app();
    harness.get_by_label("Orthographic").click();
    harness.run_steps(2);
    assert_eq!(harness.state().viewport().camera.projection, Projection::Orthographic);
    harness.get_by_label("Perspective").click();
    harness.run_steps(2);
    assert_eq!(harness.state().viewport().camera.projection, Projection::Perspective);
}

#[test]
fn viewcube_top_face_turns_camera_to_look_down() {
    let mut harness = app();
    let top = harness
        .state()
        .viewport()
        .viewcube_face(CubeFace::Top)
        .expect("top face visible in home view");
    click_at(&mut harness, top);
    settle(&mut harness);
    let forward = harness.state().viewport().camera.forward();
    assert!(forward.z < -0.999, "looking along {forward}");
}

/// Regression test for ADR-0019: egui must show the viewport texture with
/// the same brightness as the offscreen render (a wrong view format made
/// the top face ~110 instead of ~175 in sRGB).
#[test]
fn viewport_is_not_gamma_darkened() {
    let mut harness = app();
    let image = harness.render().expect("render UI");
    let state = harness.state();
    let layout = state.viewport().layout().expect("viewport laid out");
    let bounds = state.model_bounds().expect("model loaded");
    // A point on the top face next to a corner (the centre is the hole).
    let p = glam::Vec3::new(bounds.min[0] + 5.0, bounds.min[1] + 5.0, bounds.max[2]);
    let aspect = layout.rect.width() / layout.rect.height();
    let ndc = state.viewport().camera.project(p, aspect).expect("in front of camera");
    let scale = image.width() as f32 / 1440.0;
    let x = (layout.rect.left() + (ndc.x + 1.0) * 0.5 * layout.rect.width()) * scale;
    let y = (layout.rect.top() + (1.0 - ndc.y) * 0.5 * layout.rect.height()) * scale;
    let px = image.get_pixel(x as u32, y as u32).0;
    let luma = 0.2126 * f32::from(px[0]) + 0.7152 * f32::from(px[1]) + 0.0722 * f32::from(px[2]);
    assert!(luma > 140.0, "top face too dark in the UI: {px:?} (luma {luma:.0})");
}

fn cursor_at(harness: &mut Harness<'_, QoobApp>, pos: Pos2) -> CursorIcon {
    harness.event(Event::PointerMoved(pos));
    harness.run_steps(2);
    harness.output().platform_output.cursor_icon
}

/// Cursor policy (docs/design/controls.md): arrow everywhere, pointing hand
/// only on the ViewCube; never a text cursor on labels.
#[test]
fn cursors_follow_the_policy() {
    let mut harness = app();
    let top = harness.state().viewport().viewcube_face(CubeFace::Top).expect("top face visible");
    assert_eq!(cursor_at(&mut harness, top), CursorIcon::PointingHand, "over the ViewCube");

    let layout = harness.state().viewport().layout().expect("viewport");
    let empty = layout.rect.left_top() + egui::vec2(40.0, 120.0);
    assert_eq!(cursor_at(&mut harness, empty), CursorIcon::Default, "in the viewport");

    let label = harness.get_by_label("BROWSER").rect().center();
    assert_eq!(cursor_at(&mut harness, label), CursorIcon::Default, "over a label");
}

#[test]
fn language_can_be_switched_to_german() {
    let mut harness = app();
    harness.get_by_label("⚙ Settings").click();
    harness.run_steps(2);
    harness.get_by_label("Deutsch").click();
    harness.run_steps(3);
    let german = harness.state().settings().language;
    assert_eq!(german, Language::German);
    // The language is per UI thread, so parallel tests are unaffected.
    assert_eq!(i18n::language(), Language::German);
    assert_eq!(CubeFace::Top.label(), "OBEN");
    harness.get_by_label("⛶ Einpassen");
}

#[test]
fn left_drag_orbits_like_three_finger_drag() {
    let mut harness = app();
    assert!(harness.state().settings().left_drag_orbit, "default on");
    let layout = harness.state().viewport().layout().expect("viewport");
    let start = layout.rect.center();
    let before = harness.state().viewport().camera.rotation;
    harness.event(Event::PointerMoved(start));
    harness.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    for k in 1..=4 {
        harness.event(Event::PointerMoved(start + egui::vec2(20.0 * k as f32, 0.0)));
        harness.step();
    }
    harness.event(Event::PointerButton {
        pos: start + egui::vec2(80.0, 0.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    let after = harness.state().viewport().camera.rotation;
    assert!(before.angle_between(after) > 0.05, "camera did not orbit");
}

fn open_printer_picker(harness: &mut Harness<'_, QoobApp>) {
    let current = harness.state().settings().printer_id.clone();
    let name = qoob3d_core::PrinterProfile::by_id(&current).expect("known printer").name;
    harness.get_by_value(name).click();
    harness.run_steps(2);
}

/// Like a user: click into the search field (must not close the popup), type,
/// pick a printer (closes the popup).
fn search_and_pick(harness: &mut Harness<'_, QoobApp>, query: &str, label: &str) {
    open_printer_picker(harness);
    let search = harness.get_by_role(egui::accesskit::Role::TextInput);
    assert!(search.is_focused(), "search field is focused when the list opens");
    search.click();
    harness.run_steps(2);
    let search = harness
        .query_by_role(egui::accesskit::Role::TextInput)
        .expect("clicking the search field closed the printer list");
    search.type_text(query);
    harness.run_steps(2);
    harness.get_by_label(label).click();
    harness.run_steps(2);
}

#[test]
fn printer_can_be_selected_via_search() {
    let mut harness = app();
    search_and_pick(&mut harness, "A1 mini", "Bambu Lab A1 mini");
    assert_eq!(harness.state().settings().printer_id, "bambu-lab-a1-mini");
    assert!(
        harness.query_by_role(egui::accesskit::Role::TextInput).is_none(),
        "selecting a printer closes the list"
    );
}

#[test]
fn favourite_printers_are_listed_first_and_others_can_be_hidden() {
    let mut harness = app();
    open_printer_picker(&mut harness);
    harness.get_by_role(egui::accesskit::Role::TextInput).click();
    harness.run_steps(2);
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text("H2D Pro");
    harness.run_steps(2);
    harness.get_by_label("☆").click(); // star the only match
    harness.run_steps(2);
    assert_eq!(harness.state().settings().favorite_printers, ["bambu-lab-h2d-pro"]);

    // Favourites section on top, then everything else.
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    open_printer_picker(&mut harness);
    harness.get_by_label("MY PRINTERS");
    harness.get_by_label("ALL PRINTERS");

    // Settings: show only my printers.
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.get_by_label("⚙ Settings").click();
    harness.run_steps(2);
    harness.get_by_label("Show only my printers in the list").click();
    harness.run_steps(2);
    assert!(harness.state().settings().only_favorite_printers);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    open_printer_picker(&mut harness);
    assert!(
        harness.state().viewport().printer_search().is_empty(),
        "search resets when the list opens"
    );
    harness.get_by_label_contains("Bambu Lab H2D Pro");
    assert!(harness.query_by_label("ALL PRINTERS").is_none(), "others are hidden");
    assert!(harness.query_by_label("Bambu Lab A1 mini").is_none(), "others are hidden");
}

/// F6 fits the model (animated) after the user zoomed in.
#[test]
fn f6_fits_the_model() {
    let mut harness = app();
    let layout = harness.state().viewport().layout().expect("viewport");
    let fitted = harness.state().viewport().camera.distance;
    harness.event(Event::PointerMoved(layout.rect.center()));
    harness.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 3.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(2);
    assert!(harness.state().viewport().camera.distance < 0.9 * fitted, "zoom did not apply");
    harness.key_press(egui::Key::F6);
    harness.run_steps(2);
    settle(&mut harness);
    let after = harness.state().viewport().camera.distance;
    assert!((after - fitted).abs() < 1e-3 * fitted, "F6 did not fit: {after} vs {fitted}");
}

#[test]
fn screenshot_when_requested() {
    let Some(dir) = std::env::var_os("QOOB3D_SCREENSHOT_DIR") else {
        return; // only on demand: `cargo xtask screenshot`
    };
    let mut harness = app();
    let image = harness.render().expect("render UI");
    let path = std::path::Path::new(&dir).join("ui.png");
    image.save(&path).expect("write screenshot");
    println!("screenshot: {}", path.display());

    // Printer list with one favourite pinned on top.
    open_printer_picker(&mut harness);
    harness.get_by_role(egui::accesskit::Role::TextInput).type_text("A1 mini");
    harness.run_steps(2);
    harness.get_by_label("☆").click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    open_printer_picker(&mut harness);
    let image = harness.render().expect("render printer list");
    let path = std::path::Path::new(&dir).join("printer-list.png");
    image.save(&path).expect("write screenshot");
    println!("screenshot: {}", path.display());

    // Plane pick with the cursor over XY.
    let mut harness = harness_empty();
    harness.key_press(egui::Key::S);
    harness.run_steps(2);
    let xy = harness.state().plane_screen_pos(qoob3d_core::document::OriginPlane::XY).unwrap();
    harness.event(Event::PointerMoved(xy));
    harness.run_steps(2);
    save_screenshot(&mut harness, &dir, "sketch-plane-pick.png");

    // Sketch: a closed rectangle, a circle, a construction line and a
    // rectangle in progress with the value HUD.
    click_at(&mut harness, xy);
    settle(&mut harness);
    let at = |h: &Harness<'_, QoobApp>, p: [f64; 2]| h.state().sketch_screen_pos(p).unwrap();
    harness.key_press(egui::Key::R);
    harness.run_steps(1);
    for p in [[-20.0, -10.0], [10.0, 12.0]] {
        let s = at(&harness, p);
        click_at(&mut harness, s);
    }
    harness.key_press(egui::Key::C);
    harness.run_steps(1);
    for p in [[-5.0, 1.0], [1.0, 1.0]] {
        let s = at(&harness, p);
        click_at(&mut harness, s);
    }
    harness.key_press(egui::Key::Escape);
    harness.run_steps(1);
    harness.key_press(egui::Key::L);
    harness.run_steps(1);
    harness.key_press(egui::Key::X);
    harness.run_steps(1);
    for p in [[-25.0, 0.0], [15.0, 0.0]] {
        let s = at(&harness, p);
        click_at(&mut harness, s);
    }
    harness.key_press(egui::Key::Escape);
    harness.run_steps(1);
    harness.key_press(egui::Key::R);
    harness.run_steps(1);
    let s = at(&harness, [14.0, 4.0]);
    click_at(&mut harness, s);
    let s = at(&harness, [19.0, 9.0]);
    harness.event(Event::PointerMoved(s));
    harness.run_steps(1);
    type_text(&mut harness, "8");
    wait_until(&mut harness, "profiles", |a| !a.project().is_rebuilding());
    harness.run_steps(2);
    save_screenshot(&mut harness, &dir, "sketch.png");
}

fn save_screenshot(harness: &mut Harness<'_, QoobApp>, dir: &std::ffi::OsStr, name: &str) {
    let image = harness.render().expect("render");
    let path = std::path::Path::new(dir).join(name);
    image.save(&path).expect("write screenshot");
    println!("screenshot: {}", path.display());
}

fn harness_empty() -> Harness<'static, QoobApp> {
    let mut harness = harness(false);
    harness.run_steps(3);
    harness
}

// ------------------------------------------------------------------ sketching

fn wait_until(harness: &mut Harness<'_, QoobApp>, what: &str, done: impl Fn(&QoobApp) -> bool) {
    let start = Instant::now();
    while !done(harness.state()) {
        assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}");
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn sketch_curves(app: &QoobApp) -> usize {
    match app.mode() {
        Mode::Sketch(s) => app.project().doc().sketch(s.feature).map_or(0, |k| k.curves.len()),
        _ => 0,
    }
}

fn profiles(app: &QoobApp) -> usize {
    match app.mode() {
        Mode::Sketch(s) => app.project().profiles(s.feature).len(),
        _ => 0,
    }
}

fn type_text(harness: &mut Harness<'_, QoobApp>, text: &str) {
    harness.event(Event::Text(text.into()));
    harness.run_steps(1);
}

/// SK-QA-04: create a sketch on XY, draw a rectangle with typed size, get a
/// profile, finish – then undo/redo the whole sketch as one step.
#[test]
fn sketch_rectangle_with_typed_values_end_to_end() {
    let mut harness = harness(false);
    harness.run_steps(3);
    harness.key_press(egui::Key::S);
    harness.run_steps(2);
    assert!(matches!(harness.state().mode(), Mode::PickPlane), "S starts the plane pick");
    let xy = harness.state().plane_screen_pos(qoob3d_core::document::OriginPlane::XY).unwrap();
    click_at(&mut harness, xy);
    settle(&mut harness);
    assert!(matches!(harness.state().mode(), Mode::Sketch(_)), "picking a plane starts sketching");

    harness.key_press(egui::Key::R);
    harness.run_steps(1);
    let origin = harness.state().sketch_screen_pos([0.0, 0.0]).unwrap();
    click_at(&mut harness, origin);
    // Move the cursor into the +x/+y quadrant, then type the size.
    let toward = harness.state().sketch_screen_pos([10.0, 10.0]).unwrap();
    harness.event(Event::PointerMoved(toward));
    harness.run_steps(1);
    type_text(&mut harness, "40");
    harness.key_press(egui::Key::Tab);
    harness.run_steps(1);
    type_text(&mut harness, "25");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert_eq!(sketch_curves(harness.state()), 4);

    let app = harness.state();
    let Mode::Sketch(session) = app.mode() else { panic!("still sketching") };
    let sketch = app.project().doc().sketch(session.feature).unwrap();
    let max = sketch.points.iter().fold([f64::MIN; 2], |m, p| [m[0].max(p.x), m[1].max(p.y)]);
    assert!((max[0] - 40.0).abs() < 1e-9 && (max[1] - 25.0).abs() < 1e-9, "{max:?}");

    wait_until(&mut harness, "profile", |a| profiles(a) == 1);

    harness.key_press(egui::Key::Escape); // end the tool
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter); // finish sketch
    harness.run_steps(2);
    assert!(matches!(harness.state().mode(), Mode::Model));
    assert_eq!(harness.get_all_by_label("✏ Sketch1").count(), 2, "browser and timeline");

    // The whole session is one undo step.
    harness.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert!(harness.state().project().doc().features.is_empty(), "undo removes the sketch");
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(harness.state().project().doc().features.len(), 1, "redo restores it");
}

/// A line chain that returns to its start point closes and forms a profile.
#[test]
fn polyline_closes_into_a_profile() {
    let mut harness = harness(false);
    harness.run_steps(3);
    harness.get_by_label("✏ Create Sketch").click();
    harness.run_steps(2);
    let xy = harness.state().plane_screen_pos(qoob3d_core::document::OriginPlane::XY).unwrap();
    click_at(&mut harness, xy);
    settle(&mut harness);
    harness.get_by_label("∕ Line").click();
    harness.run_steps(1);
    // Small enough to stay clear of the sketch palette on the right.
    for p in [[0.0, 0.0], [15.0, 0.0], [15.0, 10.0], [0.0, 0.0]] {
        let s = harness.state().sketch_screen_pos(p).unwrap();
        click_at(&mut harness, s);
    }
    assert_eq!(sketch_curves(harness.state()), 3);
    wait_until(&mut harness, "triangle profile", |a| profiles(a) == 1);
}

/// Every `.rs` file below `dir`, recursively, except unit test
/// files named `tests.rs` (ADR-0039): production submodules can hold UI
/// texts too.
fn ui_source_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("source directory readable") {
            let path = entry.expect("directory entry readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.file_name().is_some_and(|n| n != "tests.rs")
            {
                files.push(path);
            }
        }
    }
    files
}

// ADR-0020/ADR-0039: the glyph test scans production submodules at any depth
// (`src/a/b.rs`, `src/a/b/c.rs`) and skips only files named `tests.rs`
// (also nested ones); non-`.rs` files are no sources.
#[test]
fn ui_source_scan_includes_submodules_and_skips_unit_tests() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ui_source_scan");
    let _ = std::fs::remove_dir_all(&dir);
    for file in [
        "lib.rs",
        "tests.rs",
        "panel.rs",
        "panel/tests.rs",
        "panel/dialog.rs",
        "panel/dialog/deep.rs",
        "panel/dialog/tests.rs",
        "panel/notes.md",
        "panel/my_tests.rs",
    ] {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    let mut found: Vec<String> = ui_source_files(&dir)
        .iter()
        .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    found.sort();
    assert_eq!(
        found,
        ["lib.rs", "panel.rs", "panel/dialog.rs", "panel/dialog/deep.rs", "panel/my_tests.rs"]
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every non-ASCII character in UI texts (translations and code literals)
/// must exist in the bundled fonts, otherwise egui draws "□".
#[test]
fn every_ui_glyph_exists_in_the_fonts() {
    let harness = harness(false);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut texts = Vec::new();
    for dir in ["locales/en", "locales/de"] {
        texts.push(std::fs::read_to_string(root.join(dir).join("main.ftl")).unwrap());
    }
    let sources = ui_source_files(&root.join("src"));
    // Sanity check of the scan: a known top-level module is included, no
    // unit test file is.
    assert!(sources.iter().any(|p| p.ends_with("src/app.rs")), "{sources:?}");
    assert!(!sources.iter().any(|p| p.ends_with("tests.rs")), "{sources:?}");
    for path in &sources {
        let source = std::fs::read_to_string(path).unwrap();
        // String literals only; comments may use any character.
        for line in source.lines().filter(|l| !l.trim_start().starts_with("//")) {
            texts.extend(line.split('"').skip(1).step_by(2).map(str::to_string));
        }
    }
    // `Fonts::has_glyph` reports false negatives (epaint TODO), so compare
    // the rendered glyph with the replacement glyph instead.
    let uv = |c: char| {
        harness.ctx.fonts_mut(|f| {
            let galley = f.layout_no_wrap(
                c.to_string(),
                egui::FontId::proportional(14.0),
                egui::Color32::WHITE,
            );
            galley.rows[0].glyphs[0].uv_rect
        })
    };
    let replacement = uv('\u{10FFFD}');
    let mut missing: Vec<char> = texts
        .iter()
        .flat_map(|t| t.chars())
        .filter(|c| !c.is_ascii() && uv(*c) == replacement)
        .collect();
    missing.sort_unstable();
    missing.dedup();
    assert!(missing.is_empty(), "glyphs missing in the UI font: {missing:?}");
}
