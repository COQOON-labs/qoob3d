//! Viewport navigation, mapped 1:1 to Autodesk Fusion's default scheme:
//!
//! | Input                           | Action                      |
//! |---------------------------------|-----------------------------|
//! | Middle mouse drag               | Pan                         |
//! | Shift + middle mouse drag       | Orbit                       |
//! | Mouse wheel                     | Zoom towards the cursor     |
//! | Double-click middle mouse / F6  | Fit all                     |
//! | Trackpad: two-finger swipe      | Pan                         |
//! | Trackpad: Shift + two fingers   | Orbit                       |
//! | Trackpad: pinch                 | Zoom towards the cursor     |

use eframe::egui::{self, Event, MouseWheelUnit, PointerButton, Pos2, Rect, Response};
use glam::Vec2;
use qoob3d_render::Camera;

/// Radians of orbit per point of pointer movement.
const ORBIT_SPEED: f32 = 0.006;
/// Distance factor per mouse-wheel line (one notch).
const WHEEL_ZOOM_STEP: f32 = 0.88;

#[derive(Debug, Default)]
pub struct NavOutcome {
    pub fit_requested: bool,
}

/// Applies this frame's viewport input to `camera`.
pub fn handle_input(
    ui: &egui::Ui,
    response: &Response,
    rect: Rect,
    camera: &mut Camera,
) -> NavOutcome {
    let mut outcome = NavOutcome::default();
    let aspect = rect.width() / rect.height().max(1.0);
    let height = rect.height();

    // Query the response *before* `ui.input`: both lock the egui context, and
    // calling a `Response` method inside the input closure deadlocks.
    let middle_dragged = response.dragged_by(PointerButton::Middle);
    let middle_double_clicked = response.double_clicked_by(PointerButton::Middle);
    let hovered = response.hovered();

    ui.input(|i| {
        // --- mouse drags -----------------------------------------------------
        if middle_dragged {
            let d = i.pointer.delta();
            if i.modifiers.shift {
                camera.orbit(d.x * ORBIT_SPEED, d.y * ORBIT_SPEED);
            } else {
                camera.pan(Vec2::new(d.x, d.y), height);
            }
        }
        if middle_double_clicked {
            outcome.fit_requested = true;
        }

        if hovered {
            if i.key_pressed(egui::Key::F6) {
                outcome.fit_requested = true;
            }

            let cursor_ndc = i.pointer.hover_pos().map_or(Vec2::ZERO, |p| to_ndc(p, rect));

            for event in &i.events {
                match event {
                    // Physical mouse wheel: zoom.
                    Event::MouseWheel {
                        unit: MouseWheelUnit::Line | MouseWheelUnit::Page,
                        delta,
                        ..
                    } => {
                        // macOS turns Shift+wheel into horizontal scrolling.
                        let steps = if delta.y != 0.0 { delta.y } else { delta.x };
                        camera.zoom_at(WHEEL_ZOOM_STEP.powf(steps), cursor_ndc, aspect);
                    }
                    // Trackpad swipe: pan, or orbit with Shift.
                    Event::MouseWheel { unit: MouseWheelUnit::Point, delta, modifiers, .. } => {
                        if modifiers.shift {
                            camera.orbit(-delta.x * ORBIT_SPEED, -delta.y * ORBIT_SPEED);
                        } else {
                            camera.pan(Vec2::new(delta.x, delta.y), height);
                        }
                    }
                    // Trackpad pinch (factor > 1 = fingers spread = zoom in).
                    Event::Zoom(factor) if *factor > 0.0 => {
                        camera.zoom_at(1.0 / factor, cursor_ndc, aspect);
                    }
                    _ => {}
                }
            }
        }
    });

    outcome
}

/// Converts a screen position into normalised device coordinates of `rect`.
fn to_ndc(p: Pos2, rect: Rect) -> Vec2 {
    let x = (p.x - rect.left()) / rect.width().max(1.0) * 2.0 - 1.0;
    let y = 1.0 - (p.y - rect.top()) / rect.height().max(1.0) * 2.0;
    Vec2::new(x, y)
}
