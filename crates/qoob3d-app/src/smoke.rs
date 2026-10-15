//! `qoob3d --smoke-test`: drives the real app with synthetic input through
//! the regular egui input path, verifies the effect on the camera and exits
//! with 0 (ok) or 1 (failure). A watchdog thread catches hangs and deadlocks,
//! which would otherwise only show up after egui's 10 s debug timeout
//! (ADR-0013).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, TouchPhase, Vec2, pos2,
    vec2,
};
use glam::Vec3;
use qoob3d_core::Aabb;
use qoob3d_render::{Camera, ViewPreset};

/// Abort if no frame was rendered for this long.
const STALL_TIMEOUT: Duration = Duration::from_secs(15);
/// Abort if the whole test takes longer than this.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// Give up waiting for the first kernel result after this many frames.
const MAX_WAIT_FRAMES: u64 = 3000;
/// Extra frames a step may wait for a camera animation to finish.
const MAX_SETTLE_FRAMES: u32 = 600;

pub type SmokeResult = Arc<Mutex<Option<Result<(), String>>>>;

/// What the app exposes to the smoke test after each frame.
pub struct Observation<'a> {
    pub camera: &'a Camera,
    pub mesh_bounds: Option<Aabb>,
    pub viewport_center: Pos2,
    pub viewport_aspect: f32,
    /// Screen position of the ViewCube's top face, if visible.
    pub viewcube_top: Option<Pos2>,
    pub home_button: Option<Pos2>,
    pub last_orbit_pivot: Option<Vec3>,
    pub animating: bool,
    /// Screen position of the XY plane square while picking a plane.
    pub xy_plane: Option<Pos2>,
    /// Screen positions of sketch points (0,0) and (40,30) in sketch mode.
    pub sketch_corners: [Option<Pos2>; 2],
    pub sketching: bool,
    pub picking_plane: bool,
    pub sketch_curves: usize,
    pub sketch_profiles: usize,
    /// The kernel is still computing (profiles not up to date yet).
    pub rebuilding: bool,
    pub features: usize,
    pub commanding: bool,
    pub bodies: usize,
}

/// Screen positions the scripted input aims at (from the previous frame).
#[derive(Debug, Clone, Copy)]
struct Targets {
    center: Pos2,
    viewcube_top: Option<Pos2>,
    home: Option<Pos2>,
    xy_plane: Option<Pos2>,
    corners: [Option<Pos2>; 2],
}

#[derive(Debug, PartialEq, Eq)]
pub enum Control {
    Continue,
    Close,
}

type Inject = fn(frame: u32, at: &Targets, raw: &mut RawInput);
type Check = fn(before: &Camera, after: &Camera, obs: &Observation<'_>) -> Result<(), String>;

struct Step {
    name: &'static str,
    /// Number of frames the step runs; `inject` sees 0..frames, the check
    /// runs after the last one.
    frames: u32,
    inject: Inject,
    check: Check,
}

pub struct SmokeTest {
    steps: Vec<Step>,
    current: Option<usize>,
    frame_in_step: u32,
    before: Option<Camera>,
    targets: Targets,
    finished: bool,
    frames: Arc<AtomicU64>,
    result: SmokeResult,
}

impl SmokeTest {
    /// Creates the test and starts its watchdog thread.
    pub fn start() -> (Self, SmokeResult) {
        let frames = Arc::new(AtomicU64::new(0));
        let result: SmokeResult = Arc::default();
        spawn_watchdog(frames.clone());
        let test = Self {
            steps: steps().into_iter().chain(sketch_steps()).collect(),
            current: None,
            frame_in_step: 0,
            before: None,
            targets: Targets {
                center: pos2(400.0, 300.0),
                viewcube_top: None,
                home: None,
                xy_plane: None,
                corners: [None, None],
            },
            finished: false,
            frames,
            result: result.clone(),
        };
        (test, result)
    }

    /// Called from `App::raw_input_hook`, before egui processes the frame.
    pub fn inject(&mut self, raw: &mut RawInput) {
        // Deterministic input only: real mouse/keyboard events (e.g. the user
        // moving the pointer over the popped-up window) would race with the
        // script and make the test flaky.
        raw.events.clear();
        if let Some(step) = self.current.and_then(|i| self.steps.get(i)) {
            (step.inject)(self.frame_in_step, &self.targets, raw);
        }
    }

    /// Called at the end of every frame.
    pub fn after_frame(&mut self, obs: &Observation<'_>) -> Control {
        let frame = self.frames.fetch_add(1, Ordering::Relaxed) + 1;
        self.targets = Targets {
            center: obs.viewport_center,
            viewcube_top: obs.viewcube_top,
            home: obs.home_button,
            xy_plane: obs.xy_plane,
            corners: obs.sketch_corners,
        };
        if self.finished {
            // Waiting for the window to close.
            return Control::Close;
        }

        let Some(index) = self.current else {
            if obs.mesh_bounds.is_some() && frame >= 3 {
                println!("smoke: model loaded after {frame} frames");
                self.begin_step(0, obs.camera);
            } else if frame > MAX_WAIT_FRAMES {
                return self.finish(Err(format!("no kernel result after {frame} frames")));
            }
            return Control::Continue;
        };

        self.frame_in_step += 1;
        let step = &self.steps[index];
        if self.frame_in_step < step.frames {
            return Control::Continue;
        }
        // Animated steps: wait (bounded) until the camera has settled, so the
        // test does not depend on the display refresh rate.
        let busy = obs.animating || obs.rebuilding;
        if busy && self.frame_in_step < step.frames + MAX_SETTLE_FRAMES {
            return Control::Continue;
        }

        let before = self.before.expect("step started with a camera snapshot");
        match (step.check)(&before, obs.camera, obs) {
            Ok(()) => println!("smoke: ok    {}", step.name),
            Err(msg) => {
                println!("smoke: FAIL  {} – {msg}", step.name);
                return self.finish(Err(format!("{}: {msg}", step.name)));
            }
        }
        if index + 1 < self.steps.len() {
            self.begin_step(index + 1, obs.camera);
            Control::Continue
        } else {
            self.finish(Ok(()))
        }
    }

    fn begin_step(&mut self, index: usize, camera: &Camera) {
        self.current = Some(index);
        self.frame_in_step = 0;
        self.before = Some(*camera);
    }

    fn finish(&mut self, result: Result<(), String>) -> Control {
        *self.result.lock().expect("smoke result lock poisoned") = Some(result);
        self.current = None;
        self.finished = true;
        Control::Close
    }
}

fn spawn_watchdog(frames: Arc<AtomicU64>) {
    std::thread::Builder::new()
        .name("qoob3d-smoke-watchdog".into())
        .spawn(move || {
            let start = Instant::now();
            let mut last = (0, Instant::now());
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let now = frames.load(Ordering::Relaxed);
                if now != last.0 {
                    last = (now, Instant::now());
                }
                let reason = if last.1.elapsed() > STALL_TIMEOUT {
                    Some(format!("no frame progress for {STALL_TIMEOUT:?} (hang/deadlock?)"))
                } else if start.elapsed() > TOTAL_TIMEOUT {
                    Some(format!("total run time over {TOTAL_TIMEOUT:?}"))
                } else {
                    None
                };
                if let Some(reason) = reason {
                    eprintln!("smoke: FAIL  watchdog – {reason} (frame {now})");
                    std::process::exit(1);
                }
            }
        })
        .expect("failed to spawn smoke watchdog");
}

// ----------------------------------------------------------------- script

#[expect(clippy::too_many_lines, reason = "predates ADR-0040; split when next changed")]
fn steps() -> Vec<Step> {
    vec![
        Step {
            name: "Orbit around cursor pivot (Shift + middle mouse button)",
            frames: 7,
            inject: |f, at, raw| middle_drag(f, at.center, raw, Modifiers::SHIFT),
            check: |b, a, obs| {
                expect_rotated(b, a, true)?;
                expect_close(b.distance, a.distance, "distance")?;
                // Fusion behaviour: the pivot is the model point under the
                // cursor and stays fixed on screen while orbiting.
                let pivot = obs.last_orbit_pivot.ok_or("no orbit pivot set")?;
                let bounds = obs.mesh_bounds.ok_or("no model")?;
                if (0..3)
                    .any(|k| pivot[k] < bounds.min[k] - 0.01 || pivot[k] > bounds.max[k] + 0.01)
                {
                    return Err(format!("pivot {pivot} is not on the model"));
                }
                let before =
                    b.project(pivot, obs.viewport_aspect).ok_or("pivot behind the camera")?;
                let after =
                    a.project(pivot, obs.viewport_aspect).ok_or("pivot behind the camera")?;
                let moved = (before.truncate() - after.truncate()).length();
                if moved > 1e-3 { Err(format!("pivot moved on screen ({moved})")) } else { Ok(()) }
            },
        },
        Step {
            name: "Pan (middle mouse button)",
            frames: 7,
            inject: |f, at, raw| middle_drag(f, at.center, raw, Modifiers::NONE),
            check: |b, a, _| {
                expect_rotated(b, a, false)?;
                expect_moved(b, a)
            },
        },
        Step {
            name: "Zoom (mouse wheel, 2 notches)",
            frames: 2,
            inject: |f, at, raw| {
                if f == 0 {
                    raw.events.push(Event::PointerMoved(at.center));
                    raw.events.push(wheel(MouseWheelUnit::Line, vec2(0.0, 2.0), Modifiers::NONE));
                }
            },
            check: |b, a, _| expect_close(a.distance / b.distance, 0.88f32.powi(2), "zoom factor"),
        },
        Step {
            name: "Zoom (trackpad pinch)",
            frames: 2,
            inject: |f, at, raw| {
                if f == 0 {
                    raw.events.push(Event::PointerMoved(at.center));
                    raw.events.push(Event::Zoom(1.25));
                }
            },
            check: |b, a, _| expect_close(a.distance / b.distance, 0.8, "zoom factor"),
        },
        Step {
            name: "Pan (trackpad, two fingers)",
            frames: 2,
            inject: |f, at, raw| {
                if f == 0 {
                    raw.events.push(Event::PointerMoved(at.center));
                    raw.events.push(wheel(
                        MouseWheelUnit::Point,
                        vec2(40.0, -20.0),
                        Modifiers::NONE,
                    ));
                }
            },
            check: |b, a, _| {
                expect_rotated(b, a, false)?;
                expect_moved(b, a)
            },
        },
        Step {
            name: "Orbit (trackpad, Shift + two fingers)",
            frames: 2,
            inject: |f, at, raw| {
                if f == 0 {
                    raw.events.push(Event::ModifiersChanged(Modifiers::SHIFT));
                    raw.events.push(Event::PointerMoved(at.center));
                    raw.events.push(wheel(
                        MouseWheelUnit::Point,
                        vec2(30.0, 10.0),
                        Modifiers::SHIFT,
                    ));
                } else {
                    raw.events.push(Event::ModifiersChanged(Modifiers::NONE));
                }
            },
            check: |b, a, _| expect_rotated(b, a, true),
        },
        Step {
            name: "Fit (F6, animated)",
            frames: 4,
            inject: |f, at, raw| match f {
                0 => {
                    raw.events.push(Event::PointerMoved(at.center));
                    raw.events.push(key(Key::F6, true));
                }
                1 => raw.events.push(key(Key::F6, false)),
                _ => {}
            },
            check: |_, a, obs| {
                expect_settled(obs)?;
                let bounds = obs.mesh_bounds.ok_or("no model")?;
                let mut expected = *a;
                expected.fit(bounds, obs.viewport_aspect);
                expect_close(a.distance, expected.distance, "distance")?;
                expect_vec_close(expected.target.into(), a.target.into(), "target")
            },
        },
        Step {
            name: "ViewCube: click TOP",
            frames: 4,
            inject: |f, at, raw| {
                if let Some(p) = at.viewcube_top {
                    left_click(f, p, raw);
                }
            },
            check: |_, a, obs| {
                expect_settled(obs)?;
                let down = a.forward().dot(Vec3::NEG_Z);
                if down > 0.999 {
                    Ok(())
                } else {
                    Err(format!("view direction {} instead of −Z", a.forward()))
                }
            },
        },
        Step {
            name: "Home view",
            frames: 4,
            inject: |f, at, raw| {
                if let Some(p) = at.home {
                    left_click(f, p, raw);
                }
            },
            check: |_, a, obs| {
                expect_settled(obs)?;
                let want = ViewPreset::Iso.eye_direction().normalize();
                let got = (a.eye() - a.target).normalize();
                if got.dot(want) > 0.999 {
                    Ok(())
                } else {
                    Err(format!("view from {got} instead of {want}"))
                }
            },
        },
    ]
}

/// The sketch workflow on top of the navigation steps (SK-QA-05).
#[expect(clippy::too_many_lines, reason = "predates ADR-0040; split when next changed")]
fn sketch_steps() -> Vec<Step> {
    vec![
        Step {
            name: "Create sketch (S, click plane XY)",
            frames: 8,
            inject: |f, at, raw| match f {
                0 => raw.events.push(key(Key::S, true)),
                1 => raw.events.push(key(Key::S, false)),
                2..=4 => {
                    if let Some(p) = at.xy_plane {
                        left_click(f - 2, p, raw);
                    }
                }
                _ => {}
            },
            check: |_, _, obs| {
                if obs.sketching && obs.features == 1 {
                    Ok(())
                } else {
                    Err(format!(
                        "sketch mode {} (plane picking {}), features {}",
                        obs.sketching, obs.picking_plane, obs.features
                    ))
                }
            },
        },
        Step {
            name: "Draw rectangle (R, two clicks)",
            frames: 10,
            inject: |f, at, raw| match f {
                0 => raw.events.push(key(Key::R, true)),
                1 => raw.events.push(key(Key::R, false)),
                2..=4 => {
                    if let Some(p) = at.corners[0] {
                        left_click(f - 2, p, raw);
                    }
                }
                5..=7 => {
                    if let Some(p) = at.corners[1] {
                        left_click(f - 5, p, raw);
                    }
                }
                _ => {}
            },
            check: |_, _, obs| {
                if obs.sketch_curves == 4 {
                    Ok(())
                } else {
                    Err(format!("{} curves instead of 4", obs.sketch_curves))
                }
            },
        },
        Step {
            name: "Profile found by the kernel",
            frames: 2,
            inject: |_, _, _| {},
            check: |_, _, obs| {
                if obs.sketch_profiles == 1 {
                    Ok(())
                } else {
                    Err(format!("{} profiles instead of 1", obs.sketch_profiles))
                }
            },
        },
        Step {
            name: "Finish sketch (Esc, Enter)",
            frames: 5,
            inject: |f, _, raw| match f {
                0 => raw.events.push(key(Key::Escape, true)),
                1 => raw.events.push(key(Key::Escape, false)),
                2 => raw.events.push(key(Key::Enter, true)),
                3 => raw.events.push(key(Key::Enter, false)),
                _ => {}
            },
            check: |_, _, obs| {
                if !obs.sketching && obs.features == 1 {
                    Ok(())
                } else {
                    Err(format!("sketch mode {}, features {}", obs.sketching, obs.features))
                }
            },
        },
        Step {
            name: "Extrude (E, profile preselected, Enter)",
            frames: 8,
            inject: |f, _, raw| match f {
                0 => raw.events.push(key(Key::E, true)),
                1 => raw.events.push(key(Key::E, false)),
                5 => raw.events.push(key(Key::Enter, true)),
                6 => raw.events.push(key(Key::Enter, false)),
                _ => {}
            },
            check: |_, _, obs| {
                if !obs.commanding && obs.features == 2 {
                    Ok(())
                } else {
                    Err(format!("dialog open {}, features {}", obs.commanding, obs.features))
                }
            },
        },
        Step {
            name: "Body computed by the kernel",
            frames: 2,
            inject: |_, _, _| {},
            check: |_, _, obs| {
                if obs.bodies == 1 {
                    Ok(())
                } else {
                    Err(format!("{} bodies instead of 1", obs.bodies))
                }
            },
        },
    ]
}

fn left_click(f: u32, p: Pos2, raw: &mut RawInput) {
    let button = |pressed| Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    match f {
        0 => raw.events.push(Event::PointerMoved(p)),
        1 => raw.events.push(button(true)),
        2 => raw.events.push(button(false)),
        _ => {}
    }
}

fn expect_settled(obs: &Observation<'_>) -> Result<(), String> {
    if obs.animating { Err("camera animation still running".into()) } else { Ok(()) }
}

/// Press middle button, move in four steps of (25, 10) pt, release.
fn middle_drag(f: u32, c: Pos2, raw: &mut RawInput, modifiers: Modifiers) {
    raw.events.push(Event::ModifiersChanged(if f <= 5 { modifiers } else { Modifiers::NONE }));
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Middle,
        pressed,
        modifiers,
    };
    match f {
        0 => {
            raw.events.push(Event::PointerMoved(c));
            raw.events.push(button(c, true));
        }
        1..=4 => raw.events.push(Event::PointerMoved(c + vec2(25.0, 10.0) * f as f32)),
        5 => raw.events.push(button(c + vec2(100.0, 40.0), false)),
        _ => {}
    }
}

fn wheel(unit: MouseWheelUnit, delta: Vec2, modifiers: Modifiers) -> Event {
    Event::MouseWheel { unit, delta, phase: TouchPhase::Move, modifiers }
}

fn key(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE }
}

fn expect_rotated(before: &Camera, after: &Camera, should: bool) -> Result<(), String> {
    let angle = before.rotation.angle_between(after.rotation);
    match (should, angle > 0.05, angle < 1e-4) {
        (true, true, _) | (false, _, true) => Ok(()),
        (true, ..) => Err(format!("camera did not rotate ({angle:.5} rad)")),
        (false, ..) => Err(format!("camera rotated unexpectedly ({angle:.5} rad)")),
    }
}

fn expect_moved(before: &Camera, after: &Camera) -> Result<(), String> {
    let moved = (after.target - before.target).length();
    if moved > 1e-3 * before.distance {
        Ok(())
    } else {
        Err(format!("target did not move ({moved:.6} mm)"))
    }
}

fn expect_close(actual: f32, expected: f32, what: &str) -> Result<(), String> {
    if (actual - expected).abs() <= 1e-3 * expected.abs().max(1.0) {
        Ok(())
    } else {
        Err(format!("{what}: expected {expected:.5}, got {actual:.5}"))
    }
}

fn expect_vec_close(expected: [f32; 3], actual: [f32; 3], what: &str) -> Result<(), String> {
    let d = (0..3).map(|i| (expected[i] - actual[i]).powi(2)).sum::<f32>().sqrt();
    if d <= 1e-3 { Ok(()) } else { Err(format!("{what}: expected {expected:?}, got {actual:?}")) }
}
