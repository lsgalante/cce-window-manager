// Chess-style addressing for desktop-grid squares.
//
// Coordinates in and out are the same virtual-surface CONTENT coordinates
// snap.rs works in, and the grid geometry matches it exactly: cells of
// `cell_w` x `cell_h` every `cell + gap_width` on each axis, each cell
// fading inward by `cell_inset`, so square k's content span on an axis is
// [k*period + inset, k*period + cell - inset]. A window snapped to a
// square therefore has its virtual position equal to that square's origin —
// the two modules must agree or a "tiled" window would not land on a named
// square.
//
// The naming, with the origin square (the one containing canvas 0,0) as A1:
//
//     column:  … -B   -A    A    B    C  …      (letters right, '-' left)
//     row:     … -2   -1    1    2    3  …      (numbers DOWN, '-' up)
//
// There is no row 0 and no bare-letter-less column: the axes step straight
// from -1 to 1, the way chess files/ranks are 1-based. Columns past Z carry
// on Excel-style (AA, AB, …). So A1 is the origin square, -A1 sits left of
// it, A-1 above it, and -A-1 diagonally up-left.

fn grid_period(cell_size: f64, gap_width: f64) -> f64 {
    cell_size + gap_width.max(0.0)
}

fn grid_inset(cell_size: f64, cell_inset: f64) -> f64 {
    // `clamp` panics when min > max, so a sub-2px cell must not produce a
    // negative ceiling (see the matching guard in snap.rs).
    cell_inset.clamp(0.0, (cell_size / 2.0 - 1.0).max(0.0))
}

/// The square index containing a virtual coordinate, on either axis.
pub fn cell_index(v: f64, cell_size: f64, gap_width: f64) -> i32 {
    let p = grid_period(cell_size, gap_width);
    if p <= 0.0 || !v.is_finite() {
        return 0;
    }
    (v / p).floor() as i32
}

/// Bijective base-26 (1 -> A, 26 -> Z, 27 -> AA). `n` must be >= 1.
fn letters(mut n: i32) -> String {
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push((b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    out.iter().rev().collect()
}

fn letters_to_index(s: &str) -> Option<i32> {
    if s.is_empty() {
        return None;
    }
    let mut n: i32 = 0;
    for c in s.chars() {
        let d = match c {
            'A'..='Z' => c as i32 - 'A' as i32 + 1,
            'a'..='z' => c as i32 - 'a' as i32 + 1,
            _ => return None,
        };
        n = n.checked_mul(26)?.checked_add(d)?;
    }
    Some(n)
}

/// Column label: 0 -> "A", 25 -> "Z", 26 -> "AA"; -1 -> "-A", -2 -> "-B".
pub fn column_label(col: i32) -> String {
    if col >= 0 {
        letters(col + 1)
    } else {
        format!("-{}", letters(-col))
    }
}

/// Row label: 0 -> "1", 1 -> "2"; -1 -> "-1", -9 -> "-9". No row 0.
pub fn row_label(row: i32) -> String {
    if row >= 0 {
        (row + 1).to_string()
    } else {
        row.to_string()
    }
}

/// Full square name, e.g. (2, -9) -> "C-9".
pub fn square_label(col: i32, row: i32) -> String {
    format!("{}{}", column_label(col), row_label(row))
}

/// Parse a square name back to (col, row). Case-insensitive; the leading '-'
/// belongs to the column, the inner '-' to the row ("-A-9" = col -1, row -9).
pub fn parse_square(s: &str) -> Option<(i32, i32)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (col_neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let split = rest.find(|c: char| c == '-' || c.is_ascii_digit())?;
    let (letters_part, row_part) = rest.split_at(split);
    let mag = letters_to_index(letters_part)?;
    let col = if col_neg { -mag } else { mag - 1 };

    let row_val: i32 = row_part.parse().ok()?;
    if row_val == 0 {
        return None; // there is no row 0
    }
    let row = if row_val > 0 { row_val - 1 } else { row_val };
    Some((col, row))
}

/// Content rect (x, y, w, h) of one square — what a window snapped to it fills.
pub fn square_rect(
    col: i32,
    row: i32,
    cell_w: f64,
    cell_h: f64,
    gap_width: f64,
    cell_inset: f64,
) -> (f64, f64, f64, f64) {
    block_rect(col, row, col, row, cell_w, cell_h, gap_width, cell_inset)
}

/// Content rect of a whole block of squares, inclusive of both corners.
/// A Tiled window covering the block fills exactly this.
pub fn block_rect(
    col0: i32,
    row0: i32,
    col1: i32,
    row1: i32,
    cell_w: f64,
    cell_h: f64,
    gap_width: f64,
    cell_inset: f64,
) -> (f64, f64, f64, f64) {
    let px = grid_period(cell_w, gap_width);
    let py = grid_period(cell_h, gap_width);
    let inset_x = grid_inset(cell_w, cell_inset);
    let inset_y = grid_inset(cell_h, cell_inset);
    let (cl, cr) = (col0.min(col1), col0.max(col1));
    let (rt, rb) = (row0.min(row1), row0.max(row1));
    let x = cl as f64 * px + inset_x;
    let y = rt as f64 * py + inset_y;
    let w = (cr - cl) as f64 * px + cell_w - 2.0 * inset_x;
    let h = (rb - rt) as f64 * py + cell_h - 2.0 * inset_y;
    (x, y, w, h)
}

/// The block of squares a window's content box covers, as
/// (col_min, row_min, col_max, row_max), corners inclusive. The high edges
/// are taken just inside the box so a window flush against a cell's right
/// edge does not claim the next column.
pub fn window_span(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cell_w: f64,
    cell_h: f64,
    gap_width: f64,
) -> (i32, i32, i32, i32) {
    let col0 = cell_index(x, cell_w, gap_width);
    let row0 = cell_index(y, cell_h, gap_width);
    let col1 = cell_index(x + w.max(1.0) - 1.0, cell_w, gap_width).max(col0);
    let row1 = cell_index(y + h.max(1.0) - 1.0, cell_h, gap_width).max(row0);
    (col0, row0, col1, row1)
}

/// Human-readable span: one square ("C-9") or a block ("C-9:D-8").
pub fn span_label(col0: i32, row0: i32, col1: i32, row1: i32) -> String {
    if col0 == col1 && row0 == row1 {
        square_label(col0, row0)
    } else {
        format!(
            "{}:{}",
            square_label(col0.min(col1), row0.min(row1)),
            square_label(col0.max(col1), row0.max(row1))
        )
    }
}

/// The square span of a window, already labelled.
pub fn window_span_label(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cell_w: f64,
    cell_h: f64,
    gap_width: f64,
) -> String {
    let (c0, r0, c1, r1) = window_span(x, y, w, h, cell_w, cell_h, gap_width);
    span_label(c0, r0, c1, r1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The live desktop's geometry: 512px cells, 16px gaps, 4px fade inset.
    const CELL: f64 = 512.0;
    const GAP: f64 = 16.0;
    const INSET: f64 = 4.0;
    // period = 528

    #[test]
    fn origin_square_is_a1() {
        assert_eq!(square_label(0, 0), "A1");
        // Its content origin is the inset corner, not the raw grid line.
        let (x, y, w, h) = square_rect(0, 0, CELL, CELL, GAP, INSET);
        assert_eq!((x, y), (4.0, 4.0));
        assert_eq!((w, h), (504.0, 504.0));
    }

    #[test]
    fn neighbours_of_the_origin_skip_zero() {
        assert_eq!(square_label(-1, 0), "-A1"); // left
        assert_eq!(square_label(0, -1), "A-1"); // above
        assert_eq!(square_label(-1, -1), "-A-1"); // up-left
        assert_eq!(square_label(1, 1), "B2"); // down-right
    }

    #[test]
    fn columns_run_past_z() {
        assert_eq!(column_label(25), "Z");
        assert_eq!(column_label(26), "AA");
        assert_eq!(column_label(27), "AB");
        assert_eq!(column_label(-26), "-Z");
        assert_eq!(column_label(-27), "-AA");
    }

    #[test]
    fn live_desktop_window_lands_on_c_minus_9() {
        // claude-desktop sits at virtual (1060, -4748) — exactly the content
        // origin of column 2, row -9 (2*528+4, -9*528+4).
        let col = cell_index(1060.0, CELL, GAP);
        let row = cell_index(-4748.0, CELL, GAP);
        assert_eq!((col, row), (2, -9));
        assert_eq!(square_label(col, row), "C-9");
        let (x, y, _, _) = square_rect(col, row, CELL, CELL, GAP, INSET);
        assert_eq!((x, y), (1060.0, -4748.0));
    }

    #[test]
    fn label_parse_roundtrip() {
        for &(c, r) in &[
            (0, 0),
            (2, -9),
            (-1, 0),
            (0, -1),
            (-1, -1),
            (25, 41),
            (26, -100),
            (-27, 7),
        ] {
            let s = square_label(c, r);
            assert_eq!(parse_square(&s), Some((c, r)), "roundtrip failed for {s}");
        }
    }

    #[test]
    fn parse_is_lenient_but_rejects_nonsense() {
        assert_eq!(parse_square("c-9"), Some((2, -9)));
        assert_eq!(parse_square("  B2 "), Some((1, 1)));
        assert_eq!(parse_square("A0"), None); // no row 0
        assert_eq!(parse_square("A"), None); // no row at all
        assert_eq!(parse_square("9"), None); // no column
        assert_eq!(parse_square(""), None);
        assert_eq!(parse_square("A1B"), None);
    }

    #[test]
    fn window_span_covers_only_the_squares_it_fills() {
        // A window filling exactly one square claims one square.
        let (x, y, w, h) = square_rect(2, -9, CELL, CELL, GAP, INSET);
        assert_eq!(window_span(x, y, w, h, CELL, CELL, GAP), (2, -9, 2, -9));
        assert_eq!(window_span_label(x, y, w, h, CELL, CELL, GAP), "C-9");

        // A 2x1 block claims exactly two columns, not three.
        let (x, y, w, h) = block_rect(2, -9, 3, -9, CELL, CELL, GAP, INSET);
        assert_eq!(window_span(x, y, w, h, CELL, CELL, GAP), (2, -9, 3, -9));
        assert_eq!(window_span_label(x, y, w, h, CELL, CELL, GAP), "C-9:D-9");
    }

    #[test]
    fn block_rect_matches_the_tiled_snap_footprint() {
        // cells.rs and snap.rs must agree: a block's rect is what tiled_span
        // returns for a box spanning those cells.
        let (x, y, w, h) = block_rect(2, -9, 3, -8, CELL, CELL, GAP, INSET);
        let (lo_x, hi_x) = crate::snap::tiled_span(x, x + w, CELL, GAP, INSET);
        let (lo_y, hi_y) = crate::snap::tiled_span(y, y + h, CELL, GAP, INSET);
        assert_eq!((lo_x, hi_x - lo_x), (x, w));
        assert_eq!((lo_y, hi_y - lo_y), (y, h));
    }

    #[test]
    fn rectangular_cells_use_per_axis_periods() {
        // 512-wide, 256-tall cells: columns step 528, rows step 272.
        let (x, y, w, h) = square_rect(1, 2, CELL, 256.0, GAP, INSET);
        assert_eq!((x, y), (532.0, 548.0));
        assert_eq!((w, h), (504.0, 248.0));
        assert_eq!(window_span(x, y, w, h, CELL, 256.0, GAP), (1, 2, 1, 2));
        assert_eq!(cell_index(547.0, 256.0, GAP), 2);
    }

    #[test]
    fn degenerate_geometry_does_not_panic() {
        assert_eq!(cell_index(f64::NAN, CELL, GAP), 0);
        assert_eq!(cell_index(10.0, 0.0, 0.0), 0);
        let _ = square_rect(0, 0, 0.0, 0.0, 0.0, 0.0);
    }
}
