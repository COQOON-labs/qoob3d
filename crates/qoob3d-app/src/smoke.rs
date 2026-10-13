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
use qoob3d_core::Aabb;
use qoob3d_render::Camera;

/// Abort if no frame was rendered for this long.
const STALL_TIMEOUT: Duration = Duration::from_secs(15);
/// Abort if the whole test takes longer than this.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// Give up waiting for the first kernel result after this many frames.
const MAX_WAIT_FRAMES: u64 = 3000;

pub type SmokeResult = Arc<Mutex<Option<Result<(), String>>>>;

/// What the app exposes to the smoke test after each frame.
pub struct Observation<'a> {
    pub camera: &'a Camera,
    pub mesh_bounds: Option<Aabb>,
    pub viewport_center: Pos2,
    pub viewport_aspect: f32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Control {
    Continue,
    Close,
}

type Inject = fn(frame: u32, center: Pos2, raw: &mut RawInput);
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
    center: Pos2,
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
            steps: steps(),
            current: None,
            frame_in_step: 0,
            before: None,
            center: pos2(400.0, 300.0),
            finished: false,
            frames,
            result: result.clone(),
        };
        (test, result)
    }

    /// Called from `App::raw_input_hook`, before egui processes the frame.
    pub fn inject(&mut self, raw: &mut RawInput) {
        if let Some(step) = self.current.and_then(|i| self.steps.get(i)) {
            (step.inject)(self.frame_in_step, self.center, raw);
        }
    }

    /// Called at the end of every frame.
    pub fn after_frame(&mut self, obs: &Observation<'_>) -> Control {
        let frame = self.frames.fetch_add(1, Ordering::Relaxed) + 1;
        self.center = obs.viewport_center;
        if self.finished {
            // Waiting for the window to close.
            return Control::Close;
        }

        let Some(index) = self.current else {
            if obs.mesh_bounds.is_some() && frame >= 3 {
                println!("smoke: Modell geladen nach {frame} Frames");
                self.begin_step(0, obs.camera);
            } else if frame > MAX_WAIT_FRAMES {
                return self.finish(Err(format!("kein Kernel-Ergebnis nach {frame} Frames")));
            }
            return Control::Continue;
        };

        self.frame_in_step += 1;
        let step = &self.steps[index];
        if self.frame_in_step < step.frames {
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
                    Some(format!(
                        "kein Frame-Fortschritt seit {STALL_TIMEOUT:?} (Hänger/Deadlock?)"
                    ))
                } else if start.elapsed() > TOTAL_TIMEOUT {
                    Some(format!("Gesamtlaufzeit über {TOTAL_TIMEOUT:?}"))
                } else {
                    None
                };
                if let Some(reason) = reason {
                    eprintln!("smoke: FAIL  watchdog – {reason} (Frame {now})");
                    std::process::exit(1);
                }
            }
        })
        .expect("failed to spawn smoke watchdog");
}

// ----------------------------------------------------------------- script

fn steps() -> Vec<Step> {
    vec![
        Step {
            name: "Orbit (Shift + mittlere Maustaste)",
            frames: 7,
            inject: |f, c, raw| middle_drag(f, c, raw, Modifiers::SHIFT),
            check: |b, a, _| {
                expect_rotated(b, a, true)?;
                expect_close(b.distance, a.distance, "Distanz")?;
                expect_vec_close(b.target.into(), a.target.into(), "Target")
            },
        },
        Step {
            name: "Pan (mittlere Maustaste)",
            frames: 7,
            inject: |f, c, raw| middle_drag(f, c, raw, Modifiers::NONE),
            check: |b, a, _| {
                expect_rotated(b, a, false)?;
                expect_moved(b, a)
            },
        },
        Step {
            name: "Zoom (Mausrad, 2 Raststufen)",
            frames: 2,
            inject: |f, c, raw| {
                if f == 0 {
                    raw.events.push(Event::PointerMoved(c));
                    raw.events.push(wheel(MouseWheelUnit::Line, vec2(0.0, 2.0), Modifiers::NONE));
                }
            },
            check: |b, a, _| expect_close(a.distance / b.distance, 0.88f32.powi(2), "Zoomfaktor"),
        },
        Step {
            name: "Zoom (Trackpad-Pinch)",
            frames: 2,
            inject: |f, c, raw| {
                if f == 0 {
                    raw.events.push(Event::PointerMoved(c));
                    raw.events.push(Event::Zoom(1.25));
                }
            },
            check: |b, a, _| expect_close(a.distance / b.distance, 0.8, "Zoomfaktor"),
        },
        Step {
            name: "Pan (Trackpad, zwei Finger)",
            frames: 2,
            inject: |f, c, raw| {
                if f == 0 {
                    raw.events.push(Event::PointerMoved(c));
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
            name: "Orbit (Trackpad, Shift + zwei Finger)",
            frames: 2,
            inject: |f, c, raw| {
                if f == 0 {
                    raw.events.push(Event::ModifiersChanged(Modifiers::SHIFT));
                    raw.events.push(Event::PointerMoved(c));
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
            name: "Einpassen (F6)",
            frames: 3,
            inject: |f, c, raw| match f {
                0 => {
                    raw.events.push(Event::PointerMoved(c));
                    raw.events.push(key(Key::F6, true));
                }
                1 => raw.events.push(key(Key::F6, false)),
                _ => {}
            },
            check: |_, a, obs| {
                let bounds = obs.mesh_bounds.ok_or("kein Modell")?;
                let mut expected = *a;
                expected.fit(bounds, obs.viewport_aspect);
                expect_close(a.distance, expected.distance, "Distanz")?;
                expect_vec_close(expected.target.into(), a.target.into(), "Target")
            },
        },
    ]
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
        (true, ..) => Err(format!("Kamera hat sich nicht gedreht ({angle:.5} rad)")),
        (false, ..) => Err(format!("Kamera hat sich unerwartet gedreht ({angle:.5} rad)")),
    }
}

fn expect_moved(before: &Camera, after: &Camera) -> Result<(), String> {
    let moved = (after.target - before.target).length();
    if moved > 1e-3 * before.distance {
        Ok(())
    } else {
        Err(format!("Target hat sich nicht bewegt ({moved:.6} mm)"))
    }
}

fn expect_close(actual: f32, expected: f32, what: &str) -> Result<(), String> {
    if (actual - expected).abs() <= 1e-3 * expected.abs().max(1.0) {
        Ok(())
    } else {
        Err(format!("{what}: erwartet {expected:.5}, ist {actual:.5}"))
    }
}

fn expect_vec_close(expected: [f32; 3], actual: [f32; 3], what: &str) -> Result<(), String> {
    let d = (0..3).map(|i| (expected[i] - actual[i]).powi(2)).sum::<f32>().sqrt();
    if d <= 1e-3 { Ok(()) } else { Err(format!("{what}: erwartet {expected:?}, ist {actual:?}")) }
}
