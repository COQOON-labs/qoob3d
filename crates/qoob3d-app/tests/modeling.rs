//! End-to-end modelling through the real UI (ADR-0018, ADR-0033): every
//! reference shape is built the way a user builds it – plane, sketch tools
//! with typed values, feature dialog, OK – and the resulting body is checked
//! against its analytic volume. One test also exports 3MF and checks the file.
//!
//! The same shapes exist as files in `models/` (generated from
//! `qoob3d_core::document::examples`), checked headless by `cargo xtask models`.

use std::f64::consts::PI;
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use qoob3d_app::{Mode, QoobApp, StartOptions};
use qoob3d_core::document::{CurveKind, EntityId, FeatureId, FeatureKind};

type H = Harness<'static, QoobApp>;

fn harness() -> H {
    // Real frame timing (the kittest default of 0.25 s per step makes every
    // double click too slow).
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 860.0))
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| QoobApp::new(cc, StartOptions::default()).expect("app starts"));
    h.run_steps(3);
    h
}

fn click_at(h: &mut H, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    // Keep consecutive clicks apart (no accidental double click).
    h.run_steps(2);
}

fn double_click_at(h: &mut H, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false, true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(2);
}

fn wait_until(h: &mut H, what: &str, done: impl Fn(&QoobApp) -> bool) {
    let start = Instant::now();
    while !done(h.state()) {
        assert!(start.elapsed() < Duration::from_secs(60), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn settle(h: &mut H) {
    wait_until(h, "camera animation", |a| !a.viewport().is_animating());
    h.run_steps(2);
}

fn idle(h: &mut H) {
    wait_until(h, "kernel", |a| !a.project().is_rebuilding());
    h.run_steps(2);
}

fn key(h: &mut H, key: egui::Key) {
    h.key_press(key);
    h.run_steps(1);
}

fn type_text(h: &mut H, text: &str) {
    h.event(Event::Text(text.into()));
    h.run_steps(1);
}

/// Clicks a button and lets new windows lay out (an egui area is measured
/// in its first frame and placed in the next).
fn button(h: &mut H, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(4);
}

/// Replaces the text of a dialog field (select all, type).
fn set_field(h: &mut H, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    type_text(h, value);
    h.key_press(egui::Key::Tab); // leave the field
    h.run_steps(2);
}

fn sketch_at(h: &H, p: [f64; 2]) -> Pos2 {
    h.state().sketch_screen_pos(p).expect("sketching")
}

fn move_to(h: &mut H, p: [f64; 2]) {
    let s = sketch_at(h, p);
    h.event(Event::PointerMoved(s));
    h.run_steps(1);
}

/// Browser click on an origin or construction plane: starts a sketch there.
fn sketch_on(h: &mut H, plane_label: &str) -> FeatureId {
    if h.query_all_by_label(plane_label).next().is_none() {
        button(h, "📁 Origin"); // the browser folder is collapsed by default
    }
    // Construction planes also have a timeline chip: take the browser entry.
    let pos = h
        .get_all_by_label(plane_label)
        .map(|n| n.rect().center())
        .min_by(|a, b| a.x.total_cmp(&b.x))
        .expect("plane in the browser");
    click_at(h, pos);
    settle(h);
    match h.state().mode() {
        Mode::Sketch(s) => s.feature,
        _ => panic!("clicking {plane_label} should start a sketch"),
    }
}

/// Esc (end tool) + Enter (finish sketch).
fn finish_sketch(h: &mut H) {
    key(h, egui::Key::Escape);
    key(h, egui::Key::Enter);
    assert!(matches!(h.state().mode(), Mode::Model), "sketch finished");
    idle(h);
}

fn ok(h: &mut H) {
    h.run_steps(2);
    let complete = h.state().command().is_some_and(qoob3d_app::commands::Command::is_complete);
    assert!(complete, "dialog incomplete before OK: {:?}", h.state().command().map(|c| &c.feature));
    button(h, "OK");
    assert!(matches!(h.state().mode(), Mode::Model), "dialog closed");
    idle(h);
}

fn total_volume(h: &H) -> f64 {
    h.state().bodies().iter().map(|b| b.volume_mm3).sum()
}

fn assert_volume(h: &H, expected: f64, rel: f64) {
    let v = total_volume(h);
    assert!((v - expected).abs() / expected < rel, "volume {v:.3} mm³, expected {expected:.3}");
    for b in h.state().bodies() {
        assert!(b.valid, "body {} invalid", b.number);
        assert!(b.mesh.faces.analyze().is_watertight(), "body {} not watertight", b.number);
    }
}

fn last_curve_points(h: &H, sketch: FeatureId) -> Vec<EntityId> {
    let s = h.state().project().doc().sketch(sketch).expect("sketch exists").clone();
    s.points.iter().map(|p| p.id).collect()
}

/// Quader: rectangle 40 × 25 typed, Extrude 10 (profile preselected), then
/// export as 3MF.
#[test]
fn box_from_rectangle_extrude_and_3mf_export() {
    let mut h = harness();
    sketch_on(&mut h, "▫ XY Plane");
    key(&mut h, egui::Key::R);
    let origin = sketch_at(&h, [0.0, 0.0]);
    click_at(&mut h, origin);
    move_to(&mut h, [10.0, 10.0]);
    type_text(&mut h, "40");
    key(&mut h, egui::Key::Tab);
    type_text(&mut h, "25");
    key(&mut h, egui::Key::Enter);
    finish_sketch(&mut h);

    key(&mut h, egui::Key::E);
    assert!(h.state().command().is_some(), "E opens Extrude");
    set_field(&mut h, "Distance", "10");
    ok(&mut h);
    assert_eq!(h.state().bodies().len(), 1);
    assert_volume(&h, 40.0 * 25.0 * 10.0, 1e-9);
    // The consumed sketch is hidden (Fusion).
    assert!(!h.state().project().doc().sketches().next().unwrap().1.visible);

    let path = std::env::temp_dir().join(format!("qoob3d-e2e-{}.3mf", std::process::id()));
    h.state_mut().export_to(path.clone());
    wait_until(&mut h, "export", |a| !a.is_exporting());
    let bytes = std::fs::read(&path).expect("3MF written");
    let files = qoob3d_core::export::unzip_stored(&bytes).expect("valid zip");
    let model = String::from_utf8(files[2].1.clone()).unwrap();
    assert_eq!(model.matches("<vertex ").count(), 8, "a box has 8 shared corners");
    // The real consumer, if installed: Bambu Studio must load it (ADR-0033).
    let slicer = std::path::Path::new("/Applications/BambuStudio.app/Contents/MacOS/BambuStudio");
    if slicer.exists() {
        let out = std::process::Command::new(slicer)
            .arg("--info")
            .arg(&path)
            .current_dir(std::env::temp_dir()) // it writes result.json there
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(!text.contains("failed"), "Bambu Studio rejects the export:\n{text}");
        assert!(text.contains("number_of_facets = 12"), "{text}");
        assert!(text.contains("volume = 10000.0"), "{text}");
    }
    std::fs::remove_file(path).ok();
}

/// Zylinder: circle Ø20 typed, Extrude 30.
#[test]
fn cylinder_from_circle_and_extrude() {
    let mut h = harness();
    sketch_on(&mut h, "▫ XY Plane");
    key(&mut h, egui::Key::C);
    let origin = sketch_at(&h, [0.0, 0.0]);
    click_at(&mut h, origin);
    move_to(&mut h, [5.0, 0.0]);
    type_text(&mut h, "20");
    key(&mut h, egui::Key::Enter);
    finish_sketch(&mut h);
    button(&mut h, "⬆ Extrude");
    set_field(&mut h, "Distance", "30");
    ok(&mut h);
    assert_volume(&h, PI * 100.0 * 30.0, 1e-9);
}

/// Kugel: on XZ an arc (A) r = 15 over 180°, a line closing it, Revolve
/// around the X axis.
#[test]
fn sphere_from_arc_and_revolve() {
    let mut h = harness();
    let sketch = sketch_on(&mut h, "▫ XZ Plane");
    key(&mut h, egui::Key::A);
    let origin = sketch_at(&h, [0.0, 0.0]);
    click_at(&mut h, origin);
    move_to(&mut h, [6.0, 0.0]);
    type_text(&mut h, "15");
    key(&mut h, egui::Key::Enter); // radius → start on +X
    move_to(&mut h, [0.0, 6.0]);
    type_text(&mut h, "180");
    key(&mut h, egui::Key::Enter); // sweep → end on −X
    key(&mut h, egui::Key::Escape);
    key(&mut h, egui::Key::L);
    for p in [[-15.0, 0.0], [15.0, 0.0]] {
        let s = sketch_at(&h, p);
        click_at(&mut h, s);
    }
    key(&mut h, egui::Key::Escape); // end the line chain; finish_sketch ends the tool
    let s = h.state().project().doc().sketch(sketch).unwrap();
    assert!(s.curves.iter().any(|c| matches!(c.kind, CurveKind::Arc { .. })));
    assert_eq!(s.points.len(), 3, "the line reuses the arc end points");
    finish_sketch(&mut h);
    assert_eq!(h.state().project().profiles(sketch).len(), 1, "half disc");

    button(&mut h, "🔄 Revolve");
    button(&mut h, "X Axis");
    ok(&mut h);
    assert_volume(&h, 4.0 / 3.0 * PI * 15f64.powi(3), 1e-6);
}

/// Pyramide: square 30 × 30, Offset Plane 25 above XY, a point on it,
/// Loft square → point.
#[test]
fn pyramid_from_square_offset_plane_point_and_loft() {
    let mut h = harness();
    let base = sketch_on(&mut h, "▫ XY Plane");
    key(&mut h, egui::Key::R);
    let origin = sketch_at(&h, [0.0, 0.0]);
    click_at(&mut h, origin);
    move_to(&mut h, [10.0, 10.0]);
    type_text(&mut h, "30");
    key(&mut h, egui::Key::Tab);
    type_text(&mut h, "30");
    key(&mut h, egui::Key::Enter);
    finish_sketch(&mut h);

    button(&mut h, "▫ Offset Plane");
    set_field(&mut h, "Offset", "25");
    ok(&mut h);

    let tip = sketch_on(&mut h, "▫ Plane1");
    button(&mut h, "• Point");
    let p = sketch_at(&h, [15.0, 15.0]);
    click_at(&mut h, p);
    let apex = last_curve_points(&h, tip)[0];
    finish_sketch(&mut h);

    button(&mut h, "⛶ Loft");
    let profile = h.state().profile_screen_pos(base, 0).unwrap();
    click_at(&mut h, profile);
    let point = h.state().entity_screen_pos(tip, apex).unwrap();
    click_at(&mut h, point);
    ok(&mut h);
    // V = a²·h/3 wherever the apex lies on the plane.
    assert_volume(&h, 30.0 * 30.0 * 25.0 / 3.0, 1e-6);
}

/// Spur Gear with 24 teeth, defaults otherwise.
#[test]
fn spur_gear_from_dialog() {
    let mut h = harness();
    button(&mut h, "⚙ Spur Gear");
    set_field(&mut h, "Teeth", "24");
    ok(&mut h);
    let doc = h.state().project().doc().clone();
    let FeatureKind::SpurGear(g) = &doc.features[0].kind else { panic!("gear feature") };
    assert_eq!(g.teeth, 24);
    let area = qoob3d_core::generate::gear_outline(g).area() - PI * (0.5 * g.bore).powi(2);
    assert_volume(&h, area * g.thickness, 0.01);
}

/// Coil as helix, then edited (timeline double-click) into a flat
/// spiral – the body follows the parameters.
#[test]
fn coil_then_edited_into_a_flat_spiral() {
    let mut h = harness();
    button(&mut h, "🌀 Coil");
    ok(&mut h);
    let wire = |h: &H| {
        let doc = h.state().project().doc().clone();
        let FeatureKind::Coil(c) = &doc.features[0].kind else { panic!("coil feature") };
        PI * (0.5 * c.section).powi(2) * qoob3d_core::generate::coil_length(c)
    };
    let expected = wire(&h);
    assert_volume(&h, expected, 0.01);

    let chip = h.get_by_label("🌀 Coil1").rect().center();
    double_click_at(&mut h, chip);
    assert!(h.state().command().is_some_and(|c| c.editing.is_some()), "editing Coil1");
    button(&mut h, "Flat spiral");
    ok(&mut h);
    let doc = h.state().project().doc().clone();
    assert!(matches!(&doc.features[0].kind,
        FeatureKind::Coil(c) if c.kind == qoob3d_core::document::CoilKind::Spiral));
    let expected = wire(&h);
    assert_volume(&h, expected, 0.01);
    let bounds = h.state().bodies()[0].mesh.faces.bounds().unwrap();
    assert!(bounds.max[2] - bounds.min[2] < 2.5, "flat: only the wire thickness");
}

/// A neutral click: empty viewport area in model mode, which does nothing
/// (no command, no selection). The left middle of the viewport is far from
/// the ViewCube (top right), the navigation bar (bottom centre), the browser
/// and the timeline, so the double click that follows lands well beyond
/// egui's `max_click_dist` (6 pt) from it.
fn neutral_click(h: &mut H) {
    let rect = h.state().viewport().layout().expect("viewport laid out").rect;
    click_at(h, rect.left_center() + egui::vec2(60.0, 0.0));
    assert!(matches!(h.state().mode(), Mode::Model), "the neutral click changes nothing");
    assert!(h.state().command().is_none(), "the neutral click opens nothing");
}

/// KO-FT-09: a double click on a timeline chip edits the feature, even when
/// the user clicked somewhere else just before (e.g. OK, then quickly the
/// chip). Regression for the flaky `coil_then_edited_into_a_flat_spiral`.
///
/// Timing (step_dt = 1/60 s): the neutral click releases at frame t0,
/// `click_at` adds 2 frames, `double_click_at` needs 3 frames to the first
/// release and 2 more to the second, so the second release is ~7 frames
/// ≈ 0.12 s after t0. That is below egui's triple-click window
/// (2 × `max_double_click_delay` = 0.6 s), so egui reports the second
/// release as click count 3 although the neutral click was far away; a
/// double click on the chip must still edit it.
#[test]
fn timeline_double_click_edits_right_after_a_click_elsewhere() {
    let mut h = harness();
    button(&mut h, "🌀 Coil");
    ok(&mut h);
    assert!(h.state().command().is_none(), "dialog closed");

    neutral_click(&mut h);
    let chip = h.get_by_label("🌀 Coil1").rect().center();
    double_click_at(&mut h, chip);
    assert!(
        h.state().command().is_some_and(|c| c.editing.is_some()),
        "double click on the Coil1 chip right after another click must edit Coil1"
    );
}

/// SK-LC-06: a double click on a sketch in the browser edits it, even when
/// the user clicked somewhere else just before. Same timing as
/// `timeline_double_click_edits_right_after_a_click_elsewhere`.
#[test]
fn browser_double_click_edits_sketch_right_after_a_click_elsewhere() {
    let mut h = harness();
    let sketch = sketch_on(&mut h, "▫ XY Plane");
    key(&mut h, egui::Key::C);
    let origin = sketch_at(&h, [0.0, 0.0]);
    click_at(&mut h, origin);
    move_to(&mut h, [5.0, 0.0]);
    type_text(&mut h, "20");
    key(&mut h, egui::Key::Enter);
    finish_sketch(&mut h);
    settle(&mut h); // the camera returns from the sketch view

    let name = h.state().project().doc().feature(sketch).expect("sketch exists").name.clone();
    // The timeline chip has the same label: take the browser entry (left).
    let entry = h
        .get_all_by_label(&format!("✏ {name}"))
        .map(|n| n.rect().center())
        .min_by(|a, b| a.x.total_cmp(&b.x))
        .expect("sketch in the browser");

    neutral_click(&mut h);
    double_click_at(&mut h, entry);
    assert!(
        matches!(h.state().mode(), Mode::Sketch(s) if s.feature == sketch),
        "double click on the browser entry right after another click must edit {name}"
    );
}

/// `cargo xtask screenshot`: the Extrude dialog with its live preview and
/// a finished multi-body model, for visual review.
#[test]
fn screenshot_when_requested() {
    let Some(dir) = std::env::var_os("QOOB3D_SCREENSHOT_DIR") else {
        return;
    };
    let save = |h: &mut H, name: &str| {
        let image = h.render().expect("render");
        let path = std::path::Path::new(&dir).join(name);
        image.save(&path).expect("write screenshot");
        println!("screenshot: {}", path.display());
    };
    let mut h = harness();
    sketch_on(&mut h, "▫ XY Plane");
    key(&mut h, egui::Key::R);
    let origin = sketch_at(&h, [-20.0, -15.0]);
    click_at(&mut h, origin);
    move_to(&mut h, [0.0, 0.0]);
    type_text(&mut h, "40");
    key(&mut h, egui::Key::Tab);
    type_text(&mut h, "30");
    key(&mut h, egui::Key::Enter);
    key(&mut h, egui::Key::C);
    let c = sketch_at(&h, [0.0, 0.0]);
    click_at(&mut h, c);
    move_to(&mut h, [3.0, 0.0]);
    type_text(&mut h, "12");
    key(&mut h, egui::Key::Enter);
    finish_sketch(&mut h);
    let home = h.state().viewport().home_button().expect("home button");
    click_at(&mut h, home);
    settle(&mut h);
    button(&mut h, "⬆ Extrude");
    let ring = h.state().project().doc().features[0].id;
    let p = h.state().profile_screen_pos(ring, 0).unwrap();
    click_at(&mut h, p);
    set_field(&mut h, "Distance", "8");
    idle(&mut h);
    save(&mut h, "command-extrude.png");
    ok(&mut h);

    button(&mut h, "⚙ Spur Gear");
    set_field(&mut h, "Thickness", "5");
    idle(&mut h);
    ok(&mut h);
    key(&mut h, egui::Key::F6);
    settle(&mut h);
    save(&mut h, "model.png");
}
