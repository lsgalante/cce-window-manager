// Directional focus selection: which window receives focus when the user
// moves focus up/down/left/right of the current one.
//
// Inputs are window FOOTPRINTS in virtual-surface coordinates (the same
// space as `WindowSnapshot.virtual_x/y`; y grows downward). The mechanism
// side builds the candidate list (visible, non-status windows) and applies
// the returned index.

use super::api::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    /// The direction a focus action moves in, `None` for non-directional
    /// actions.
    pub fn from_action(action: Action) -> Option<Direction> {
        match action {
            Action::FocusUp => Some(Direction::Up),
            Action::FocusDown => Some(Direction::Down),
            Action::FocusLeft => Some(Direction::Left),
            Action::FocusRight => Some(Direction::Right),
            _ => None,
        }
    }
}

/// A window's footprint on the virtual surface: top-left corner and extent,
/// the extent already multiplied by the window's output scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// The rect seen along `dir`: `(lo, hi)` is its extent on the axis of
    /// travel, increasing in the direction of travel, and `(olo, ohi)` its
    /// extent across it.
    fn along(&self, dir: Direction) -> (f64, f64, f64, f64) {
        let (x0, x1, y0, y1) = (self.x, self.x + self.w, self.y, self.y + self.h);
        match dir {
            Direction::Right => (x0, x1, y0, y1),
            Direction::Left => (-x1, -x0, y0, y1),
            Direction::Down => (y0, y1, x0, x1),
            Direction::Up => (-y1, -y0, x0, x1),
        }
    }
}

/// Weight of off-axis distance in the candidate score, for candidates that
/// do not share a row/column with the focused window: a window slightly
/// ahead but far off to the side loses to one nearly straight ahead.
const ORTHOGONAL_PENALTY: f64 = 2.0;

/// Pick the window to focus when moving in `dir` from `focused`.
///
/// Edges decide, not centers. A candidate must lie ahead: its near edge
/// past the focused window's midpoint on the axis of travel. That excludes
/// a window that merely sticks out past the focused one — a wide window
/// directly above a narrow one has a center to the right of it, but is not
/// "to the right" of it. Candidates whose extent across the axis of travel
/// overlaps the focused window's (same row for left/right, same column for
/// up/down) win over any that do not; within a group the smallest edge gap
/// wins, off-axis gap weighted by `ORTHOGONAL_PENALTY`, then the nearest
/// off-axis center. No wraparound: with no candidate in that direction the
/// focus stays put (`None`).
///
/// With nothing focused, the entry window is the one whose center is
/// furthest on the opposite side (moving right enters at the leftmost
/// window), matching the "focus is entering the surface from off-screen"
/// intuition.
pub fn directional_focus(rects: &[Rect], focused: Option<usize>, dir: Direction) -> Option<usize> {
    if rects.is_empty() {
        return None;
    }

    let Some(focused) = focused else {
        let entry_key = |r: &Rect| {
            let (x, y) = r.center();
            match dir {
                Direction::Right => x,
                Direction::Left => -x,
                Direction::Down => y,
                Direction::Up => -y,
            }
        };
        return rects
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| entry_key(a).total_cmp(&entry_key(b)))
            .map(|(i, _)| i);
    };

    let (flo, fhi, folo, fohi) = rects[focused].along(dir);
    let fmid = (flo + fhi) / 2.0;
    let fomid = (folo + fohi) / 2.0;
    let mut best: Option<(usize, (bool, f64, f64))> = None;
    for (i, r) in rects.iter().enumerate() {
        if i == focused {
            continue;
        }
        let (lo, hi, olo, ohi) = r.along(dir);
        if lo <= fmid {
            continue;
        }
        let in_line = olo < fohi && ohi > folo;
        let primary = (lo - fhi).max(0.0);
        let orthogonal = (olo - fohi).max(folo - ohi).max(0.0);
        let key = (
            !in_line,
            primary + ORTHOGONAL_PENALTY * orthogonal,
            ((olo + ohi) / 2.0 - fomid).abs(),
        );
        if best.is_none_or(|(_, b)| key < b) {
            best = Some((i, key));
        }
    }
    best.map(|(i, _)| i)
}

/// Pick the window to focus by a free direction rather than one of four:
/// a ray from the focused window's center along `v` (virtual-surface
/// units, y down), and the window whose center lies nearest along it.
///
/// Only centers within `cone_deg` of the ray are candidates, so a swipe
/// toward empty space changes nothing (`None`) rather than reaching for
/// whatever is closest. Among candidates the score is the center's
/// distance divided by cos²θ, θ its angle off the ray: nearer wins, and
/// off-ray costs more the further off it is, so a precise diagonal lands
/// on the diagonal window even when a straight neighbour is a little
/// closer. A candidate centered on the focused window's own center has no
/// direction and is skipped. With nothing focused, or a zero `v`, there
/// is no ray and the answer is `None`; the caller falls back to
/// `directional_focus`, whose entry rule covers the unfocused case.
pub fn vector_focus(rects: &[Rect], focused: Option<usize>, v: (f64, f64), cone_deg: f64) -> Option<usize> {
    let focused = focused?;
    let len = v.0.hypot(v.1);
    if !(len > 0.0) {
        return None;
    }
    let (ux, uy) = (v.0 / len, v.1 / len);
    let cos_cone = cone_deg.to_radians().cos();
    let (fx, fy) = rects[focused].center();
    let mut best: Option<(usize, f64)> = None;
    for (i, r) in rects.iter().enumerate() {
        if i == focused {
            continue;
        }
        let (cx, cy) = r.center();
        let (dx, dy) = (cx - fx, cy - fy);
        let dist = dx.hypot(dy);
        if !(dist > 0.0) {
            continue;
        }
        let cos = (dx * ux + dy * uy) / dist;
        if cos < cos_cone || cos <= 0.0 {
            continue;
        }
        let score = dist / (cos * cos);
        if best.is_none_or(|(_, b)| score < b) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

/// A candidate for the next-visible-focus rule. `eligible` is the
/// mechanism's judgment (mapped, not minimized, not a status bar or
/// background surface).
#[derive(Debug, Clone, Copy)]
pub struct FocusCandidate {
    pub id: super::api::WindowId,
    pub eligible: bool,
}

/// Which window takes focus when the focused one goes away (close,
/// minimize, unmap): the most recently focused eligible window, else — a
/// preserved mechanism quirk — the LAST eligible window in window order,
/// else nothing (focus clears). `history` is most-recent-first.
pub fn next_visible_focus(
    history: &[FocusCandidate],
    windows: &[FocusCandidate],
) -> Option<super::api::WindowId> {
    history
        .iter()
        .find(|c| c.eligible)
        .or_else(|| windows.iter().filter(|c| c.eligible).last())
        .map(|c| c.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::WindowId;
    use crate::slotmap::Key;

    fn fc(index: u32, eligible: bool) -> FocusCandidate {
        FocusCandidate { id: WindowId(Key { generation: 0, index }), eligible }
    }

    #[test]
    fn next_visible_prefers_history_then_last_in_window_order() {
        let history = [fc(3, false), fc(7, true), fc(1, true)];
        let windows = [fc(7, true), fc(3, false), fc(1, true)];
        // Most recent eligible history entry wins.
        assert_eq!(next_visible_focus(&history, &windows), Some(fc(7, true).id));
        // No eligible history: LAST eligible window in window order.
        let history = [fc(3, false)];
        assert_eq!(next_visible_focus(&history, &windows), Some(fc(1, true).id));
        // Nothing eligible anywhere: focus clears.
        let none = [fc(1, false)];
        assert_eq!(next_visible_focus(&history, &none), None);
        assert_eq!(next_visible_focus(&[], &[]), None);
    }

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(x, y, w, h)
    }

    // A 2x2-ish layout of 200x200 windows (y grows downward):
    //   0:(0,0)     1:(400,0)
    //   2:(0,400)   3:(420,380)
    fn grid() -> [Rect; 4] {
        [r(0.0, 0.0, 200.0, 200.0), r(400.0, 0.0, 200.0, 200.0), r(0.0, 400.0, 200.0, 200.0), r(420.0, 380.0, 200.0, 200.0)]
    }

    #[test]
    fn moves_along_each_axis() {
        let g = grid();
        assert_eq!(directional_focus(&g, Some(0), Direction::Right), Some(1));
        assert_eq!(directional_focus(&g, Some(0), Direction::Down), Some(2));
        assert_eq!(directional_focus(&g, Some(3), Direction::Left), Some(2));
        assert_eq!(directional_focus(&g, Some(3), Direction::Up), Some(1));
    }

    #[test]
    fn no_candidate_means_no_move() {
        let g = grid();
        // Nothing is left of column 0 or above row 0.
        assert_eq!(directional_focus(&g, Some(0), Direction::Left), None);
        assert_eq!(directional_focus(&g, Some(0), Direction::Up), None);
        assert_eq!(directional_focus(&[r(0.0, 0.0, 10.0, 10.0)], Some(0), Direction::Right), None);
        assert_eq!(directional_focus(&[], None, Direction::Right), None);
    }

    #[test]
    fn same_row_beats_nearer_off_row() {
        // From 0 going right: 1 shares its row (gap 200); 3 is nearly as
        // close (gap 220) and its center is only 380 down, but it does not
        // overlap row 0 and so loses to any in-line candidate.
        let g = grid();
        assert_eq!(directional_focus(&g, Some(0), Direction::Right), Some(1));
        // With 1 gone, 3 is the only thing ahead and wins.
        let g = [g[0], g[2], g[3]];
        assert_eq!(directional_focus(&g, Some(0), Direction::Right), Some(2));
    }

    #[test]
    fn a_wider_window_above_is_not_to_the_right() {
        // The live layout that motivated edges over centers (virtual px):
        // a one-cell list with a two-cell calendar directly above it and
        // a two-by-two mail window in the next column. The calendar's
        // CENTER is right of the list's (its left edges coincide, it is
        // twice as wide) and nearer than mail's, so a center rule picked
        // it; it is above, not to the right.
        let list = r(0.0, 544.0, 460.0, 532.0);
        let calendar = r(0.0, 0.0, 932.0, 532.0);
        let mail = r(944.0, 0.0, 932.0, 1076.0);
        let wins = [list, calendar, mail];
        assert_eq!(directional_focus(&wins, Some(0), Direction::Right), Some(2));
        assert_eq!(directional_focus(&wins, Some(0), Direction::Up), Some(1));
        assert_eq!(directional_focus(&wins, Some(0), Direction::Left), None);
        assert_eq!(directional_focus(&wins, Some(0), Direction::Down), None);
        // From mail, left: both are ahead and in line; the calendar's edge
        // is 12 px away, the list's 484.
        assert_eq!(directional_focus(&wins, Some(2), Direction::Left), Some(1));
        // From the calendar, right: mail (in line); down: the list.
        assert_eq!(directional_focus(&wins, Some(1), Direction::Right), Some(2));
        assert_eq!(directional_focus(&wins, Some(1), Direction::Down), Some(0));
    }

    #[test]
    fn overlapping_floats_count_only_past_the_midpoint() {
        // A float whose near edge is past the focused window's midpoint is
        // ahead (edge gap 0); one that starts before the midpoint is a
        // stacked window, reachable by FocusNext but not by direction.
        let f = r(0.0, 0.0, 300.0, 300.0);
        let ahead = r(200.0, 50.0, 300.0, 100.0);
        let stacked = r(100.0, 0.0, 300.0, 300.0);
        assert_eq!(directional_focus(&[f, ahead, stacked], Some(0), Direction::Right), Some(1));
        assert_eq!(directional_focus(&[f, stacked], Some(0), Direction::Right), None);
    }

    #[test]
    fn same_column_ties_break_on_the_nearer_center() {
        // Two windows in the next column, both in line with a tall focused
        // window and both at edge gap 0: the one centered nearer wins.
        let f = r(0.0, 0.0, 100.0, 1000.0);
        let far = r(110.0, 0.0, 100.0, 100.0);
        let near = r(110.0, 450.0, 100.0, 100.0);
        assert_eq!(directional_focus(&[f, far, near], Some(0), Direction::Right), Some(2));
    }

    #[test]
    fn unfocused_enters_from_the_opposite_side() {
        let g = grid();
        assert_eq!(directional_focus(&g, None, Direction::Right), Some(0)); // leftmost-ish
        assert_eq!(directional_focus(&g, None, Direction::Left), Some(3)); // rightmost
        assert_eq!(directional_focus(&g, None, Direction::Down), Some(0)); // topmost
        assert_eq!(directional_focus(&g, None, Direction::Up), Some(2)); // bottommost
    }

    #[test]
    fn direction_from_action() {
        assert_eq!(Direction::from_action(Action::FocusUp), Some(Direction::Up));
        assert_eq!(Direction::from_action(Action::FocusDown), Some(Direction::Down));
        assert_eq!(Direction::from_action(Action::FocusLeft), Some(Direction::Left));
        assert_eq!(Direction::from_action(Action::FocusRight), Some(Direction::Right));
        assert_eq!(Direction::from_action(Action::FocusNext), None);
    }

    // Vector focus. A 3x2 grid of 900x800 windows with 40px gaps, focus on
    // the top-left one (center 450,400).
    fn wide_grid() -> Vec<Rect> {
        vec![
            r(0.0, 0.0, 900.0, 800.0),
            r(940.0, 0.0, 900.0, 800.0),
            r(1880.0, 0.0, 900.0, 800.0),
            r(0.0, 840.0, 900.0, 800.0),
            r(940.0, 840.0, 900.0, 800.0),
            r(1880.0, 840.0, 900.0, 800.0),
        ]
    }

    #[test]
    fn vector_straight_takes_the_neighbour() {
        let g = wide_grid();
        assert_eq!(vector_focus(&g, Some(0), (1.0, 0.0), 45.0), Some(1));
        assert_eq!(vector_focus(&g, Some(0), (0.0, 1.0), 45.0), Some(3));
    }

    #[test]
    fn vector_diagonal_takes_the_diagonal_window() {
        // The diagonal neighbour sits about 42° down-right; a swipe that
        // way lands on it, not on the nearer straight neighbours.
        let g = wide_grid();
        assert_eq!(vector_focus(&g, Some(0), (940.0, 840.0), 45.0), Some(4));
        assert_eq!(vector_focus(&g, Some(0), (1.0, 1.0), 45.0), Some(4));
    }

    #[test]
    fn vector_shallow_angle_prefers_the_straight_neighbour() {
        let g = wide_grid();
        assert_eq!(vector_focus(&g, Some(0), (1.0, 0.3), 45.0), Some(1));
    }

    #[test]
    fn vector_toward_nothing_changes_nothing() {
        let g = wide_grid();
        // Up and left of the top-left window there is nothing.
        assert_eq!(vector_focus(&g, Some(0), (-1.0, 0.0), 45.0), None);
        assert_eq!(vector_focus(&g, Some(0), (-1.0, -1.0), 45.0), None);
        // A narrow cone still reaches a window a little off the ray: from
        // the top-right window, a swipe 11° left of straight down finds
        // the one below, while the diagonal one, 37° off, is outside.
        assert_eq!(vector_focus(&g, Some(5), (0.0, -1.0), 45.0), Some(2));
        assert_eq!(vector_focus(&g, Some(2), (-0.2, 1.0), 20.0), Some(5));
        // Aimed into the gap between that diagonal window (42° below
        // left) and the one straight below (90°), a 20° cone reaches
        // neither: both lie about 24° off the ray.
        assert_eq!(vector_focus(&g, Some(2), (-0.4, 0.9), 20.0), None);
    }

    #[test]
    fn vector_needs_a_focus_and_a_direction() {
        let g = wide_grid();
        assert_eq!(vector_focus(&g, None, (1.0, 0.0), 45.0), None);
        assert_eq!(vector_focus(&g, Some(0), (0.0, 0.0), 45.0), None);
    }

    #[test]
    fn vector_skips_a_window_centered_on_the_focus() {
        let rects = [r(0.0, 0.0, 400.0, 400.0), r(100.0, 100.0, 200.0, 200.0), r(600.0, 0.0, 400.0, 400.0)];
        assert_eq!(vector_focus(&rects, Some(0), (1.0, 0.0), 45.0), Some(2));
    }

}
