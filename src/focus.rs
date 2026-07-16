// Directional focus selection: which window receives focus when the user
// moves focus up/down/left/right of the current one.
//
// Inputs are window CENTER points in virtual-surface coordinates (the same
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

/// Weight of off-axis distance in the candidate score: a window slightly
/// ahead but far off to the side loses to one straight ahead.
const ORTHOGONAL_PENALTY: f64 = 2.0;

/// Pick the window to focus when moving in `dir` from `focused`.
///
/// A candidate must lie strictly in the direction of travel (its center's
/// primary-axis delta > 0); among candidates the lowest
/// `primary + ORTHOGONAL_PENALTY * |orthogonal|` wins. No wraparound: with
/// no candidate in that direction the focus stays put (`None`).
///
/// With nothing focused, the entry window is the one furthest on the
/// opposite side (moving right enters at the leftmost window), matching the
/// "focus is entering the surface from off-screen" intuition.
pub fn directional_focus(centers: &[(f64, f64)], focused: Option<usize>, dir: Direction) -> Option<usize> {
    if centers.is_empty() {
        return None;
    }

    let Some(focused) = focused else {
        let entry_key = |&(x, y): &(f64, f64)| match dir {
            Direction::Right => x,
            Direction::Left => -x,
            Direction::Down => y,
            Direction::Up => -y,
        };
        return centers
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| entry_key(a).total_cmp(&entry_key(b)))
            .map(|(i, _)| i);
    };

    let (fx, fy) = centers[focused];
    let mut best: Option<(usize, f64)> = None;
    for (i, &(x, y)) in centers.iter().enumerate() {
        if i == focused {
            continue;
        }
        let (primary, orthogonal) = match dir {
            Direction::Right => (x - fx, y - fy),
            Direction::Left => (fx - x, y - fy),
            Direction::Down => (y - fy, x - fx),
            Direction::Up => (fy - y, x - fx),
        };
        if primary <= 0.0 {
            continue;
        }
        let score = primary + ORTHOGONAL_PENALTY * orthogonal.abs();
        if best.is_none_or(|(_, s)| score < s) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 2x2-ish layout (y grows downward):
    //   0:(100,100)   1:(500,100)
    //   2:(100,500)   3:(520,480)
    const GRID: [(f64, f64); 4] = [(100.0, 100.0), (500.0, 100.0), (100.0, 500.0), (520.0, 480.0)];

    #[test]
    fn moves_along_each_axis() {
        assert_eq!(directional_focus(&GRID, Some(0), Direction::Right), Some(1));
        assert_eq!(directional_focus(&GRID, Some(0), Direction::Down), Some(2));
        assert_eq!(directional_focus(&GRID, Some(3), Direction::Left), Some(2));
        assert_eq!(directional_focus(&GRID, Some(3), Direction::Up), Some(1));
    }

    #[test]
    fn no_candidate_means_no_move() {
        // Nothing is left of column 0 or above row 0.
        assert_eq!(directional_focus(&GRID, Some(0), Direction::Left), None);
        assert_eq!(directional_focus(&GRID, Some(0), Direction::Up), None);
        assert_eq!(directional_focus(&[(0.0, 0.0)], Some(0), Direction::Right), None);
        assert_eq!(directional_focus(&[], None, Direction::Right), None);
    }

    #[test]
    fn orthogonal_penalty_prefers_straight_ahead() {
        // From 0 going right: 1 is straight ahead (400 away); 3 is closer on
        // the diagonal-ish (420, 380) but pays the off-axis penalty:
        // score(1) = 400, score(3) = 420 + 2*380 = 1180.
        assert_eq!(directional_focus(&GRID, Some(0), Direction::Right), Some(1));
    }

    #[test]
    fn unfocused_enters_from_the_opposite_side() {
        assert_eq!(directional_focus(&GRID, None, Direction::Right), Some(0)); // leftmost-ish
        assert_eq!(directional_focus(&GRID, None, Direction::Left), Some(3)); // rightmost
        assert_eq!(directional_focus(&GRID, None, Direction::Down), Some(0)); // topmost
        assert_eq!(directional_focus(&GRID, None, Direction::Up), Some(2)); // bottommost
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
