//! Touchpad gesture policy: how far a swipe in progress leans the desktop,
//! and what a bound action means for navigation. Pure functions over
//! numbers and [`Action`]s; the compositor's gesture handlers feed them the
//! travel and the binds and apply what they return. Moved here from the
//! compositor's `cursor` on 2026-10-10 so the arithmetic is tested beside
//! the rest of the policy.

use crate::api::Action;

/// Whether a swipe bound to `action` moves the view somewhere — a focus or
/// a pan — and so may peek the desktop before it fires.
pub fn action_navigates(action: Action) -> bool {
    use Action::*;
    matches!(action, FocusLeft | FocusRight | FocusUp | FocusDown | PanLeft | PanRight | PanUp | PanDown)
}

/// Whether `action` moves focus in a direction.
pub fn is_directional_focus(action: Action) -> bool {
    matches!(action, Action::FocusLeft | Action::FocusRight | Action::FocusUp | Action::FocusDown)
}

/// The peek the accumulated travel `d` along one axis calls for, in
/// virtual units: proportional and clamped at `threshold`, `peek_px` on
/// screen there, and only toward a direction that has a navigating bind
/// (`neg` / `pos`) — a swipe with nothing bound its way leaves the
/// desktop still.
pub fn swipe_peek_for(d: f64, neg: bool, pos: bool, threshold: f64, peek_px: f64, zoom: f64) -> f64 {
    if (d < 0.0 && neg) || (d > 0.0 && pos) {
        (d / threshold).clamp(-1.0, 1.0) * peek_px / zoom.max(1e-6)
    } else {
        0.0
    }
}

/// Slope (minor over major travel) below which a swipe counts as straight:
/// tan 15°. Within it the lean stays on the dominant axis, since a hand
/// swiping left drifts a little up or down, and leaning with that drift
/// is a wobble, not a direction.
pub const SWIPE_LEAN_STRAIGHT_SLOPE: f64 = 0.268;

/// The lean `[x, y]` (virtual units) the swipe's travel so far calls for.
/// Its size is set by the dominant axis ([`swipe_peek_for`]: proportional,
/// `peek_px` on screen at `threshold`), and its direction follows the
/// fingers: the minor axis leans in proportion to the swipe's slope, so a
/// 45° swipe leans 45°. The slope is measured past the straight band and
/// rescaled, [`SWIPE_LEAN_STRAIGHT_SLOPE`] mapping to 0 and a diagonal to 1,
/// so the lean turns smoothly off the axis rather than jumping at 15°.
/// Each axis only leans toward a direction with a navigating bind
/// (`navigates`: left, right, up, down), and nothing leans at all when the
/// dominant direction has none.
pub fn swipe_lean(dx: f64, dy: f64, navigates: [bool; 4], threshold: f64, peek_px: f64, zoom: f64) -> [f64; 2] {
    let horizontal = dx.abs() >= dy.abs();
    let (major, minor) = if horizontal { (dx, dy) } else { (dy, dx) };
    let (major_nav, minor_nav) = if horizontal {
        ((navigates[0], navigates[1]), (navigates[2], navigates[3]))
    } else {
        ((navigates[2], navigates[3]), (navigates[0], navigates[1]))
    };
    let lean_major = swipe_peek_for(major, major_nav.0, major_nav.1, threshold, peek_px, zoom);
    let minor_allowed = (minor < 0.0 && minor_nav.0) || (minor > 0.0 && minor_nav.1);
    let lean_minor = if lean_major != 0.0 && minor_allowed {
        let slope = minor.abs() / major.abs();
        let turn = ((slope - SWIPE_LEAN_STRAIGHT_SLOPE) / (1.0 - SWIPE_LEAN_STRAIGHT_SLOPE)).clamp(0.0, 1.0);
        lean_major.abs() * turn * minor.signum()
    } else {
        0.0
    };
    if horizontal { [lean_major, lean_minor] } else { [lean_minor, lean_major] }
}

/// The drift (screen px) a lean leaves once the step it previews has
/// fired: the step pans the camera from where the lean left it, and
/// `camera::pan_into_view` corrects each axis on its own, so per axis the
/// part of the lean past the step's own pan that way stays — all of it on
/// an axis the step does not pan — while a lean against the step's pan is
/// undone by it. `lean` and `pan` are both camera moves in screen px. An
/// upper bound: a lean that pushes the window off the far side is panned
/// back, which only leaves less.
pub fn lean_drift(lean: [f64; 2], pan: (f64, f64)) -> f64 {
    let axis = |lean: f64, pan: f64| {
        if pan == 0.0 {
            lean.abs()
        } else if lean * pan > 0.0 {
            (lean.abs() - pan.abs()).max(0.0)
        } else {
            0.0
        }
    };
    axis(lean[0], pan.0).hypot(axis(lean[1], pan.1))
}

/// The `peek_px` a focus swipe's lean should take toward a step that will
/// pan the camera by `pan` (screen px, from the camera with the lean taken
/// out; zero when the window the step lands on is already in view): the
/// full `peek_px`, or as much of it as leaves no more than `near_px` of
/// drift (`lean_drift`) once the step has fired. `shape` is the lean's
/// direction — what [`swipe_lean`] returns, at any `peek_px`.
///
/// A step to a window already on screen moves no camera, so the camera
/// stops where the lean left it: the full peek there was pure drift, the
/// desk swaying by `peek_px` on every switch between two windows in view.
/// Such a window now leans `near_px`, which still answers the fingers (a
/// lean of nothing for an in-view target was tried and retired on
/// 2026-09-22); a window that needs a pan leans up to that pan plus
/// `near_px`, so a straight lean toward one a full peek or more away is
/// the full peek, as before. The bound is on the whole drift, so a
/// diagonal lean toward a window that needs a pan on one axis only is held
/// to `near_px` on the other.
pub fn step_peek_px(peek_px: f64, near_px: f64, shape: [f64; 2], pan: (f64, f64)) -> f64 {
    let major = shape[0].abs().max(shape[1].abs());
    if !(major > 0.0) || !(peek_px > 0.0) {
        return peek_px;
    }
    let near = near_px.max(0.0);
    // The lean at its threshold for a given `peek_px`: `swipe_lean` is
    // linear in it, the major axis reaching it exactly.
    let drift = |peek: f64| lean_drift([shape[0] / major * peek, shape[1] / major * peek], pan);
    if drift(peek_px) <= near {
        return peek_px;
    }
    // The drift only grows with the peek, so bisect for the largest that
    // stays within `near`.
    let (mut lo, mut hi) = (0.0, peek_px);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if drift(mid) <= near {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// The `peek_px` to lean by this event: from `current` toward `target`,
/// but changing by no more than `full_px` per `threshold` of the fingers'
/// travel, `travel` being this event's. The lean is that `peek_px` times
/// the progress toward the threshold, so a size that changed at once —
/// the aim crossing from a window in view to one far off, or to a dead
/// end — moved the camera by the difference times the progress in a
/// single frame, up to ~90 px at `swipe_peek 120`. Held to this rate the
/// camera never moves faster than twice a full lean does, and stops when
/// the fingers do.
pub fn settle_peek(current: f64, target: f64, travel: f64, threshold: f64, full_px: f64) -> f64 {
    let max = travel.abs() / threshold.max(1e-6) * full_px.abs();
    current + (target - current).clamp(-max, max)
}

/// The sense a swipe along `finger_dir` ("left", "right", "up", "down")
/// bound to `action` gives its axis when aiming focus: +1 when the bind
/// focuses the way the fingers moved, -1 when it focuses the opposite way,
/// `None` when it is not a focus on that axis (so the axis does not aim).
pub fn focus_sense(action: Action, finger_dir: &str) -> Option<f64> {
    let focus_dir = match action {
        Action::FocusLeft => "left",
        Action::FocusRight => "right",
        Action::FocusUp => "up",
        Action::FocusDown => "down",
        _ => return None,
    };
    let axis = |d: &str| if d == "left" || d == "right" { 0 } else { 1 };
    if axis(focus_dir) != axis(finger_dir) {
        return None;
    }
    Some(if focus_dir == finger_dir { 1.0 } else { -1.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [bool; 4] = [true; 4];
    const EPS: f64 = 1e-9;

    #[test]
    fn straight_swipe_leans_on_its_axis() {
        // Halfway to the threshold: half the peek, and no drift below 15°.
        assert_eq!(swipe_lean(35.0, 5.0, ALL, 70.0, 120.0, 1.0), [60.0, 0.0]);
        assert_eq!(swipe_lean(-4.0, -35.0, ALL, 70.0, 120.0, 1.0), [0.0, -60.0]);
    }

    #[test]
    fn diagonal_swipe_leans_diagonally() {
        let [x, y] = swipe_lean(35.0, -35.0, ALL, 70.0, 120.0, 1.0);
        assert!((x - 60.0).abs() < EPS && (y + 60.0).abs() < EPS, "{x} {y}");
    }

    #[test]
    fn lean_turns_smoothly_off_the_axis() {
        // Just past the straight band a little; halfway through it, half.
        let slope_mid = (SWIPE_LEAN_STRAIGHT_SLOPE + 1.0) / 2.0;
        let [x, y] = swipe_lean(70.0, 70.0 * slope_mid, ALL, 70.0, 120.0, 1.0);
        assert!((x - 120.0).abs() < EPS && (y - 60.0).abs() < 1e-6, "{x} {y}");
        let [_, y] = swipe_lean(70.0, 70.0 * (SWIPE_LEAN_STRAIGHT_SLOPE + 0.01), ALL, 70.0, 120.0, 1.0);
        assert!(y > 0.0 && y < 3.0, "{y}");
    }

    #[test]
    fn lean_respects_bound_directions() {
        // No up bind: a right-and-up swipe leans only right.
        assert_eq!(swipe_lean(35.0, -35.0, [true, true, false, true], 70.0, 120.0, 1.0), [60.0, 0.0]);
        // Nothing bound right: no lean at all, not even the minor part.
        assert_eq!(swipe_lean(35.0, -30.0, [true, false, true, true], 70.0, 120.0, 1.0), [0.0, 0.0]);
    }

    #[test]
    fn lean_is_clamped_and_zoom_scaled() {
        // Past the threshold the size stops growing; zoomed out, the same
        // screen distance is a longer virtual one.
        let [x, y] = swipe_lean(140.0, 140.0, ALL, 70.0, 120.0, 0.5);
        assert!((x - 240.0).abs() < EPS && (y - 240.0).abs() < EPS, "{x} {y}");
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn step_peek_is_near_for_a_window_in_view() {
        // No pan to make: the lean IS the drift, so it is `near_px` long.
        assert!(close(step_peek_px(120.0, 30.0, [1.0, 0.0], (0.0, 0.0)), 30.0));
        assert!(close(step_peek_px(120.0, 30.0, [0.0, -0.4], (0.0, 0.0)), 30.0));
        // A near lean at or past the full one changes nothing.
        assert_eq!(step_peek_px(60.0, 90.0, [1.0, 0.0], (0.0, 0.0)), 60.0);
    }

    #[test]
    fn step_peek_grows_with_the_pan_and_caps_at_the_full_peek() {
        // A 50 px pan: the lean may run 30 px past it.
        assert!(close(step_peek_px(120.0, 30.0, [1.0, 0.0], (50.0, 0.0)), 80.0));
        // A pan of the full peek or more: the full peek.
        assert_eq!(step_peek_px(120.0, 30.0, [-0.5, 0.0], (-400.0, 0.0)), 120.0);
        // Zero near lean: exactly the pan, never past it.
        assert!(close(step_peek_px(120.0, 0.0, [0.0, 1.0], (0.0, 70.0)), 70.0));
    }

    #[test]
    fn step_peek_against_the_pan_leaves_no_drift() {
        // Leaning right toward a step that pans left: the step undoes the
        // lean on that axis, so it is whole, as before.
        assert_eq!(step_peek_px(120.0, 30.0, [1.0, 0.0], (-300.0, 0.0)), 120.0);
        // Leaning right toward a step that pans only down: the lean is all
        // drift.
        assert!(close(step_peek_px(120.0, 30.0, [1.0, 0.0], (0.0, 300.0)), 30.0));
    }

    #[test]
    fn step_peek_holds_a_diagonal_lean_to_near_off_the_pan() {
        let diag = swipe_lean(70.0, 70.0, ALL, 70.0, 1.0, 1.0);
        // In view: a 45° lean `near_px` long.
        let peek = step_peek_px(120.0, 30.0, diag, (0.0, 0.0));
        let [x, y] = swipe_lean(70.0, 70.0, ALL, 70.0, peek, 1.0);
        assert!(close(x.hypot(y), 30.0), "{x} {y}");
        // A window far to the right only: the x part is previewed, the y
        // part would stay, so the lean is held to 30 down.
        let peek = step_peek_px(120.0, 30.0, diag, (400.0, 0.0));
        let [x, y] = swipe_lean(70.0, 70.0, ALL, 70.0, peek, 1.0);
        assert!(close(y, 30.0) && close(x, 30.0), "{x} {y}");
        assert!(close(lean_drift([x, y], (400.0, 0.0)), 30.0));
        // Far down-right on both axes: the full diagonal lean.
        assert_eq!(step_peek_px(120.0, 30.0, diag, (400.0, 400.0)), 120.0);
    }

    #[test]
    fn step_peek_without_a_lean_is_the_full_peek() {
        assert_eq!(step_peek_px(120.0, 30.0, [0.0, 0.0], (0.0, 0.0)), 120.0);
    }

    #[test]
    fn lean_drift_is_what_the_pan_leaves() {
        // Short of the pan, nothing; past it, the excess; against it,
        // nothing; on an axis it does not pan, all of it.
        assert_eq!(lean_drift([40.0, 0.0], (100.0, 0.0)), 0.0);
        assert_eq!(lean_drift([130.0, 0.0], (100.0, 0.0)), 30.0);
        assert_eq!(lean_drift([-60.0, 0.0], (100.0, 0.0)), 0.0);
        assert_eq!(lean_drift([0.0, -25.0], (100.0, 0.0)), 25.0);
    }

    #[test]
    fn settle_peek_moves_at_the_fingers_pace() {
        // 7 units of a 70 threshold: a tenth of the full 120 at most.
        assert_eq!(settle_peek(30.0, 120.0, 7.0, 70.0, 120.0), 42.0);
        assert_eq!(settle_peek(120.0, 30.0, -7.0, 70.0, 120.0), 108.0);
        // Within reach, it lands; with the fingers still, it holds.
        assert_eq!(settle_peek(30.0, 35.0, 7.0, 70.0, 120.0), 35.0);
        assert_eq!(settle_peek(30.0, 120.0, 0.0, 70.0, 120.0), 30.0);
    }

    #[test]
    fn focus_sense_follows_and_mirrors_and_ignores_other_axes() {
        assert_eq!(focus_sense(Action::FocusLeft, "left"), Some(1.0));
        assert_eq!(focus_sense(Action::FocusRight, "left"), Some(-1.0));
        assert_eq!(focus_sense(Action::FocusUp, "left"), None);
        assert_eq!(focus_sense(Action::PanUp, "up"), None);
    }

    #[test]
    fn only_focus_and_pan_navigate() {
        assert!(action_navigates(Action::PanLeft) && action_navigates(Action::FocusDown));
        assert!(!action_navigates(Action::Overview));
        assert!(is_directional_focus(Action::FocusUp) && !is_directional_focus(Action::PanUp));
    }
}
