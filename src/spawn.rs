// Where a window launched from a point on the desktop should land.
//
// A window normally reopens wherever it last was. That is right when you
// summon it from nowhere in particular, and wrong when you asked for it AT a
// place — right-clicking a grid square and picking Terminal says something
// about where you want the terminal. This module answers only that second
// case; the caller decides when it applies.
//
// The rule is: the window covers the square you invoked from, and grows AWAY
// from whatever is already there. It never searches for somewhere else to be,
// so the result stays predictable — the invocation square is always one of
// its corners, and only WHICH corner is in question.

/// A rectangular block of grid squares, inclusive on both corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellBlock {
    pub col0: i32,
    pub row0: i32,
    pub col1: i32,
    pub row1: i32,
}

impl CellBlock {
    pub fn new(col0: i32, row0: i32, col1: i32, row1: i32) -> Self {
        Self {
            col0: col0.min(col1),
            row0: row0.min(row1),
            col1: col0.max(col1),
            row1: row0.max(row1),
        }
    }

    /// Cells covered by both blocks.
    fn overlap_cells(&self, other: &CellBlock) -> i64 {
        let w = (self.col1.min(other.col1) - self.col0.max(other.col0) + 1).max(0) as i64;
        let h = (self.row1.min(other.row1) - self.row0.max(other.row0) + 1).max(0) as i64;
        w * h
    }
}

/// The four ways a block of `cols` x `rows` can hang off one square, in
/// preference order. Top-left first because a window growing right and down
/// from where you clicked is the conventional reading; the others are only
/// reached when that would land on something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Anchor {
    pub const ORDER: [Anchor; 4] =
        [Anchor::TopLeft, Anchor::TopRight, Anchor::BottomLeft, Anchor::BottomRight];

    fn block(self, col: i32, row: i32, cols: i32, rows: i32) -> CellBlock {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let (col0, row0) = match self {
            Anchor::TopLeft => (col, row),
            Anchor::TopRight => (col - cols + 1, row),
            Anchor::BottomLeft => (col, row - rows + 1),
            Anchor::BottomRight => (col - cols + 1, row - rows + 1),
        };
        CellBlock::new(col0, row0, col0 + cols - 1, row0 + rows - 1)
    }
}

/// Where a `cols` x `rows` window invoked at square `(col, row)` should go.
///
/// `occupied` are the blocks already taken by other windows; `viewport` is the
/// block of squares currently on screen, used only to break ties — of two
/// placements that collide with nothing, the one you can see is the better
/// answer. Returns the chosen block; the caller turns it back into
/// coordinates with [`crate::cells::block_rect`].
pub fn place_at_cell(
    col: i32,
    row: i32,
    cols: i32,
    rows: i32,
    occupied: &[CellBlock],
    viewport: Option<CellBlock>,
) -> CellBlock {
    let mut best: Option<(i64, i64, CellBlock)> = None;
    for anchor in Anchor::ORDER {
        let block = anchor.block(col, row, cols, rows);
        let collision: i64 = occupied.iter().map(|o| block.overlap_cells(o)).sum();
        // Negated so that "more visible" sorts the same direction as "less
        // collision" — both smaller-is-better in the comparison below.
        let unseen = match viewport {
            Some(v) => {
                let total = (cols.max(1) as i64) * (rows.max(1) as i64);
                total - block.overlap_cells(&v)
            }
            None => 0,
        };
        let better = match best {
            None => true,
            // Collisions dominate: a placement that lands on another window is
            // worse than one you have to scroll to, however far off-screen.
            Some((bc, bu, _)) => (collision, unseen) < (bc, bu),
        };
        if better {
            best = Some((collision, unseen, block));
        }
        // Nothing beats a clean, fully-visible placement, and the order is a
        // preference order — stop at the first one.
        if collision == 0 && unseen == 0 {
            break;
        }
    }
    best.map(|(_, _, b)| b).unwrap_or_else(|| Anchor::TopLeft.block(col, row, cols, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(c0: i32, r0: i32, c1: i32, r1: i32) -> CellBlock {
        CellBlock::new(c0, r0, c1, r1)
    }

    #[test]
    fn empty_desktop_grows_right_and_down() {
        // Nothing in the way: the invocation square is the top-left corner.
        assert_eq!(place_at_cell(1, 1, 2, 2, &[], None), b(1, 1, 2, 2));
    }

    #[test]
    fn grows_away_from_a_neighbour_on_the_right() {
        // The reported case: the menu is opened one square LEFT of a 2x2
        // window, and a 2x2 window opening there must not land on it — so the
        // invocation square becomes its top-RIGHT corner and it grows left.
        let claude = b(2, 1, 3, 2);
        let placed = place_at_cell(1, 1, 2, 2, &[claude], None);
        assert_eq!(placed, b(0, 1, 1, 2));
        assert_eq!(placed.col1, 1, "invocation square is its right edge");
        assert_eq!(placed.row0, 1, "invocation square is its top edge");
        assert_eq!(placed.overlap_cells(&claude), 0);
    }

    #[test]
    fn boxed_in_below_and_right_it_goes_up_and_left() {
        // Right and below taken. Growing up alone is not enough — a
        // bottom-LEFT anchor still clips the right-hand window by a square —
        // so the only clean corner is up AND left.
        let right = b(2, 1, 3, 2);
        let below = b(1, 2, 2, 3);
        let placed = place_at_cell(1, 1, 2, 2, &[right, below], None);
        assert_eq!(placed, b(0, 0, 1, 1));
        assert_eq!(placed.overlap_cells(&right), 0);
        assert_eq!(placed.overlap_cells(&below), 0);
        assert_eq!(placed.col1, 1, "invocation square is still a corner");
        assert_eq!(placed.row1, 1);
    }

    #[test]
    fn a_visible_placement_beats_an_off_screen_one() {
        // Both anchors are collision-free, but growing left would leave the
        // window off the visible desktop, so it grows right instead.
        let viewport = b(0, 0, 5, 3);
        let placed = place_at_cell(0, 0, 3, 1, &[], Some(viewport));
        assert_eq!(placed, b(0, 0, 2, 0));
    }

    #[test]
    fn collisions_outrank_visibility() {
        // The only on-screen anchor is occupied: take the off-screen one
        // rather than open on top of another window.
        let viewport = b(0, 0, 5, 3);
        let blocker = b(1, 0, 2, 0);
        let placed = place_at_cell(1, 0, 2, 1, &[blocker], Some(viewport));
        assert_eq!(placed, b(0, 0, 1, 0));
    }

    #[test]
    fn a_single_square_window_lands_on_the_square_itself() {
        // 1x1 has the same block under every anchor — the invocation square.
        for occ in [vec![], vec![b(0, 0, 0, 0)]] {
            assert_eq!(place_at_cell(4, -2, 1, 1, &occ, None), b(4, -2, 4, -2));
        }
    }

    #[test]
    fn degenerate_sizes_are_clamped_to_one_square() {
        assert_eq!(place_at_cell(2, 2, 0, -3, &[], None), b(2, 2, 2, 2));
    }
}
