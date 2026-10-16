//! Viewport navigation, mapped 1:1 to Autodesk Fusion's default scheme
//! (ADR-0008, docs/design/controls.md):
//!
//! | Input                           | Action                                   |
//! |---------------------------------|------------------------------------------|
//! | Middle mouse drag               | Pan                                      |
//! | Shift + middle mouse drag       | Orbit around the point under the cursor  |
//! | Mouse wheel                     | Zoom towards the cursor                  |
//! | Double-click middle mouse / F6  | Fit all                                  |
//! | Trackpad: two-finger swipe      | Pan                                      |
//! | Trackpad: Shift + two fingers   | Orbit around the point under the cursor  |
//! | Trackpad: pinch                 | Zoom towards the cursor                  |
//! | Left drag / three-finger drag   | Orbit (setting, default on; ADR-0022)    |

use eframe::egui::{self, CursorIcon, Event, MouseWheelUnit, PointerButton, Pos2, Rect, Response};
use glam::{Vec2, Vec3};
use qoob3d_render::{Camera, Ray};

/// Radians of orbit per point of pointer movement.
const ORBIT_SPEED: f32 = 0.006;
/// Distance factor per mouse-wheel line (one notch).
const WHEEL_ZOOM_STEP: f32 = 0.88;
/// A trackpad orbit gesture ends after this pause (seconds).
const TRACKPAD_GESTURE_GAP: f64 = 0.25;

/// Navigation state that lives across frames.
#[derive(Debug, Default)]
pub(crate) struct NavState {
    /// Pivot of the orbit in progress (mouse or trackpad).
    pub orbit_pivot: Option<Vec3>,
    last_trackpad_orbit: f64,
}

#[derive(Debug, Default)]
pub(crate) struct NavOutcome {
    pub fit_requested: bool,
    /// The user moved the camera this frame (cancels camera animations).
    pub interacted: bool,
}

/// Applies this frame's viewport input to `camera`. `pick` returns the
/// surface point hit by a ray, used as orbit pivot (Fusion behaviour).
/// `left_drag_orbit`: a left-button drag (macOS three-finger drag) orbits.
pub(crate) fn handle_input(
    ui: &egui::Ui,
    response: &Response,
    rect: Rect,
    camera: &mut Camera,
    state: &mut NavState,
    pick: &dyn Fn(Ray) -> Option<Vec3>,
    left_drag_orbit: bool,
) -> NavOutcome {
    let mut out = NavOutcome::default();
    let aspect = rect.width() / rect.height().max(1.0);
    let height = rect.height();

    // Query the response *before* `ui.input`: both lock the egui context, and
    // calling a `Response` method inside the input closure deadlocks.
    let middle_dragged = response.dragged_by(PointerButton::Middle);
    let left_orbit = left_drag_orbit && response.dragged_by(PointerButton::Primary);
    let middle_double_clicked = double_clicked_by(response, PointerButton::Middle);
    let hovered = response.hovered();

    let (delta, shift, pointer, now, f6, events) = ui.input(|i| {
        (
            i.pointer.delta(),
            i.modifiers.shift,
            i.pointer.latest_pos(),
            i.time,
            i.key_pressed(egui::Key::F6),
            if hovered { i.events.clone() } else { Vec::new() },
        )
    });
    let cursor_ndc = pointer.map_or(Vec2::ZERO, |p| to_ndc(p, rect));
    let pivot_under_cursor =
        |camera: &Camera| pick(camera.ray(cursor_ndc, aspect)).unwrap_or(camera.target);

    // --- mouse drags --------------------------------------------------------
    if middle_dragged || left_orbit {
        out.interacted = true;
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        if shift || left_orbit {
            let pivot = *state.orbit_pivot.get_or_insert_with(|| pivot_under_cursor(camera));
            camera.orbit_around(pivot, delta.x * ORBIT_SPEED, delta.y * ORBIT_SPEED);
        } else {
            camera.pan(Vec2::new(delta.x, delta.y), height);
        }
    } else if now - state.last_trackpad_orbit > TRACKPAD_GESTURE_GAP {
        state.orbit_pivot = None;
    }
    if middle_double_clicked || (hovered && f6) {
        out.fit_requested = true;
    }

    // --- wheel, trackpad, pinch ---------------------------------------------
    for event in &events {
        match event {
            // Physical mouse wheel: zoom.
            Event::MouseWheel {
                unit: MouseWheelUnit::Line | MouseWheelUnit::Page, delta, ..
            } => {
                // macOS turns Shift+wheel into horizontal scrolling.
                let steps = if delta.y == 0.0 { delta.x } else { delta.y };
                camera.zoom_at(WHEEL_ZOOM_STEP.powf(steps), cursor_ndc, aspect);
                out.interacted = true;
            }
            // Trackpad swipe: pan, or orbit with Shift.
            Event::MouseWheel { unit: MouseWheelUnit::Point, delta, modifiers, .. } => {
                if modifiers.shift {
                    if now - state.last_trackpad_orbit > TRACKPAD_GESTURE_GAP {
                        state.orbit_pivot = Some(pivot_under_cursor(camera));
                    }
                    state.last_trackpad_orbit = now;
                    let pivot = state.orbit_pivot.unwrap_or(camera.target);
                    camera.orbit_around(pivot, -delta.x * ORBIT_SPEED, -delta.y * ORBIT_SPEED);
                } else {
                    camera.pan(Vec2::new(delta.x, delta.y), height);
                }
                out.interacted = true;
            }
            // Trackpad pinch (factor > 1 = fingers spread = zoom in).
            Event::Zoom(factor) if *factor > 0.0 => {
                camera.zoom_at(1.0 / factor, cursor_ndc, aspect);
                out.interacted = true;
            }
            _ => {}
        }
    }

    out
}

/// A double click on `response` by `button`, also when egui reports it as a
/// triple click. egui 0.36 counts a release as click 3 when the click before
/// the previous one is less than 2 × `max_double_click_delay` ago, but checks
/// the distance only to the previous click: a click elsewhere followed
/// quickly by a double click yields counts 1, 3 and `double_clicked_by` never
/// fires. Callers must tolerate a real triple click firing a second time.
pub(crate) fn double_clicked_by(response: &Response, button: PointerButton) -> bool {
    response.double_clicked_by(button) || response.triple_clicked_by(button)
}

/// Converts a screen position into normalised device coordinates of `rect`.
pub(crate) fn to_ndc(p: Pos2, rect: Rect) -> Vec2 {
    let x = (p.x - rect.left()) / rect.width().max(1.0) * 2.0 - 1.0;
    let y = 1.0 - (p.y - rect.top()) / rect.height().max(1.0) * 2.0;
    Vec2::new(x, y)
}

/// Inverse of [`to_ndc`].
pub(crate) fn from_ndc(ndc: Vec2, rect: Rect) -> Pos2 {
    Pos2::new(
        rect.left() + (ndc.x + 1.0) * 0.5 * rect.width(),
        rect.top() + (1.0 - ndc.y) * 0.5 * rect.height(),
    )
}

#[cfg(test)]
mod tests;
