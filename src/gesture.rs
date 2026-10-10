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
