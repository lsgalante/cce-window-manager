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
}
