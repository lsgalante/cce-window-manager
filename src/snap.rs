// Magnetic grid snapping for interactive move/resize.
//
// All coordinates are virtual-surface CONTENT coordinates, and snapping acts
// directly on them: the window's own edge lands on the snap target. Borders
// draw OUTSIDE the content box, so a snapped border overhangs its cell into
// the gap rather than being inset to stay within it. This matches the
// Tiled grid-snap convention, where the content fills the covered cells
// edge to edge.
//
// The pull is continuous (see `pull`): an edge within half the threshold
// sits on its target, one in the outer half is drawn toward it by a ramp
// that vanishes at the threshold, so nothing jumps when an edge comes into
// range. Only the hard Tiled snaps (`snap_move_tiled`, `resize_axis_tiled`)
// are steps: a Tiled window covers whole cells and nothing else, so its
// move and resize both land edge-on-cell from any distance.
//
// Targets are the VISIBLE cell edges, not the raw grid lines. The desktop
// grid draws cells of `cell_size` every `cell_size + gap_width`, and each
// cell fades inward by `cell_inset` — so cell k's visible span is
// [k*period + inset, k*period + cell_size - inset]. A left/top edge snaps
// to the former, a right/bottom edge to the latter, letting windows abut
// the cells instead of floating mid-gap.

fn grid_period(cell_size: f64, gap_width: f64) -> f64 {
    cell_size + gap_width.max(0.0)
}

fn grid_inset(cell_size: f64, cell_inset: f64) -> f64 {
    // `f64::clamp` panics when min > max, which a `grid_cell_size` under 2
    // (a config typo) produces — and this runs on every arrange pass, so the
    // panic would take the whole session down.
    cell_inset.clamp(0.0, (cell_size / 2.0 - 1.0).max(0.0))
}

/// Hard grid snap for Tiled windows: the visible outer edges of every
/// cell the span [x1, x2) touches. Returns (low, high) — the content
/// footprint, which fills the covered cells exactly.
pub fn tiled_span(x1: f64, x2: f64, cell_size: f64, gap_width: f64, cell_inset: f64) -> (f64, f64) {
    let p = grid_period(cell_size, gap_width);
    let inset = grid_inset(cell_size, cell_inset);
    let col_min = (x1 / p).floor();
    let col_max = ((x2 / p).ceil() - 1.0).max(col_min);
    (col_min * p + inset, col_max * p + cell_size - inset)
}

#[derive(Debug, Clone, Copy)]
pub struct SnapParams {
    /// Desktop grid cell WIDTH in virtual units (the x-axis cell size).
    pub cell_w: f64,
    /// Desktop grid cell HEIGHT in virtual units (the y-axis cell size).
    pub cell_h: f64,
    /// Gap between cells; each axis's grid period is its cell size + gap.
    pub gap_width: f64,
    /// Visual inset of a cell's edge (the fade inset).
    pub cell_inset: f64,
    /// Snap radius in virtual units; <= 0 disables snapping.
    pub threshold: f64,
}

/// One axis's view of the grid: the cell size along that axis plus the
/// shared gap/inset/threshold. All the target math lives here; x/y code
/// paths differ only in which cell size they carry.
#[derive(Debug, Clone, Copy)]
pub struct AxisSnapParams {
    pub cell_size: f64,
    pub gap_width: f64,
    pub cell_inset: f64,
    pub threshold: f64,
}

impl SnapParams {
    /// The horizontal axis: columns of width `cell_w`.
    pub fn x(&self) -> AxisSnapParams {
        AxisSnapParams {
            cell_size: self.cell_w,
            gap_width: self.gap_width,
            cell_inset: self.cell_inset,
            threshold: self.threshold,
        }
    }

    /// The vertical axis: rows of height `cell_h`.
    pub fn y(&self) -> AxisSnapParams {
        AxisSnapParams {
            cell_size: self.cell_h,
            gap_width: self.gap_width,
            cell_inset: self.cell_inset,
            threshold: self.threshold,
        }
    }

    /// Snapping is aimed in SCREEN space: the configured threshold is the
    /// grab distance at zoom 1, and zooming out must not shrink the felt
    /// target — so the virtual-space threshold grows by 1/zoom. Capped at
    /// 45% of the SMALLER cell dimension so a deep zoom-out can't snap from
    /// half a cell away (targets are one period apart; past the midpoint
    /// snapping would thrash between neighbors).
    pub fn for_zoom(mut self, zoom: f64) -> Self {
        if self.threshold > 0.0 && zoom > 0.0 && zoom.is_finite() {
            self.threshold =
                (self.threshold / zoom).min(self.cell_w.min(self.cell_h) * 0.45);
        }
        self
    }
}

impl AxisSnapParams {
    fn enabled(&self) -> bool {
        self.threshold > 0.0 && self.cell_size > 0.5
    }

    fn period(&self) -> f64 {
        grid_period(self.cell_size, self.gap_width)
    }

    /// Inset clamped so the two visible edges of a cell can't cross.
    fn inset(&self) -> f64 {
        grid_inset(self.cell_size, self.cell_inset)
    }

    /// Nearest visible LEFT/TOP cell edge (k*period + inset) to `v`.
    fn nearest_low_target(&self, v: f64) -> f64 {
        let p = self.period();
        let inset = self.inset();
        ((v - inset) / p).round() * p + inset
    }

    /// Nearest visible RIGHT/BOTTOM cell edge (k*period + cell_size - inset).
    fn nearest_high_target(&self, v: f64) -> f64 {
        let p = self.period();
        let edge = self.cell_size - self.inset();
        ((v - edge) / p).round() * p + edge
    }
}

fn within(delta: f64, p: &AxisSnapParams) -> bool {
    delta.abs() <= p.threshold
}

/// Fraction of the threshold inside which a pulled edge sits ON its target.
const SNAP_HOLD_FRACTION: f64 = 0.5;

/// Magnetic pull toward `target` as a CONTINUOUS function of the distance:
/// inside the hold radius (half the threshold) the edge sits on the target;
/// from there out to the threshold it keeps a fraction of its distance that
/// ramps linearly from 0 to 1, meeting the untouched position exactly at
/// the threshold. Outside, untouched.
///
/// The old rule was a step — anything within the threshold jumped onto the
/// target — and at overview zoom, where the threshold is scaled by 1/zoom
/// to keep its screen size, the jump was up to 24 screen px: an edge being
/// dragged toward a cell edge lurched the moment it came into range. The
/// ramp trades the outer half of the landing zone for a pull that
/// decelerates the edge into the target instead.
fn pull(pos: f64, target: f64, p: &AxisSnapParams) -> f64 {
    let d = pos - target;
    let r_out = p.threshold;
    let r_in = r_out * SNAP_HOLD_FRACTION;
    let a = d.abs();
    if a >= r_out {
        pos
    } else if a <= r_in {
        target
    } else {
        target + d.signum() * (a - r_in) * (r_out / (r_out - r_in))
    }
}

/// True when every content edge of the box lies on a visible cell edge —
/// the geometric definition of `TilingMode::Tiled`. Left/top edges must sit
/// on a low target (`k*period + inset`), right/bottom edges on a high target
/// (`k*period + cell_size - inset`), each within `eps`. Independent of the
/// snap `threshold`: this classifies a resting geometry, it doesn't attract
/// one.
pub fn is_cell_aligned(x: f64, y: f64, w: f64, h: f64, p: &SnapParams, eps: f64) -> bool {
    if p.cell_w <= 0.5 || p.cell_h <= 0.5 || w <= 0.0 || h <= 0.0 {
        return false;
    }
    let (px, py) = (p.x(), p.y());
    (px.nearest_low_target(x) - x).abs() <= eps
        && (px.nearest_high_target(x + w) - (x + w)).abs() <= eps
        && (py.nearest_low_target(y) - y).abs() <= eps
        && (py.nearest_high_target(y + h) - (y + h)).abs() <= eps
}

/// Snap a window position during a move. On each axis the two content edges
/// compete for their nearest visible cell edge; the closer candidate within
/// the threshold wins. `w`/`h` are content sizes.
pub fn snap_move(x: f64, y: f64, w: f64, h: f64, p: &SnapParams) -> (f64, f64) {
    (snap_move_axis(x, w, &p.x()), snap_move_axis(y, h, &p.y()))
}

fn snap_move_axis(pos: f64, len: f64, p: &AxisSnapParams) -> f64 {
    if !p.enabled() {
        return pos;
    }
    let lo = pos;
    let hi = pos + len;
    let lo_delta = p.nearest_low_target(lo) - lo;
    let hi_delta = p.nearest_high_target(hi) - hi;
    if lo_delta.abs() <= hi_delta.abs() && within(lo_delta, p) {
        pull(pos, pos + lo_delta, p)
    } else if within(hi_delta, p) {
        pull(pos, pos + hi_delta, p)
    } else {
        pos
    }
}

/// Snap the dragged left/top CONTENT edge during a resize onto the nearest
/// visible left/top cell edge. Takes the axis view: `p.x()` when dragging a
/// left edge, `p.y()` for a top edge.
pub fn snap_low_edge(pos: f64, p: &AxisSnapParams) -> f64 {
    if !p.enabled() {
        return pos;
    }
    pull(pos, p.nearest_low_target(pos), p)
}

/// Snap the dragged right/bottom CONTENT edge during a resize onto the
/// nearest visible right/bottom cell edge. Takes the axis view like
/// [`snap_low_edge`].
pub fn snap_high_edge(pos: f64, p: &AxisSnapParams) -> f64 {
    if !p.enabled() {
        return pos;
    }
    pull(pos, p.nearest_high_target(pos), p)
}

/// Hard grid snap for MOVING a `Tiled` window: both low edges land on the
/// nearest cell start, with no threshold, so the window can only ever come to
/// rest covering whole squares. `snap_move`'s magnetic pull is for Floating
/// windows deciding whether to tile; once a window IS tiled, sitting between
/// squares is not a state it is allowed to reach — a drag that ended mid-cell
/// used to leave the window aligned on screen (the arrange pass re-snaps a
/// Tiled window's rendered box every frame) while its virtual position was
/// off-grid, so `is_cell_aligned` failed at op_end and the window silently
/// demoted to Floating and jumped.
///
/// Deliberately not gated on `enabled()`: the snap threshold is a grab
/// distance for magnetic snapping, while a Tiled window fills whole cells by
/// definition (that is what `tiled_span` renders), so disabling magnetic
/// snapping must not strand it off-grid.
pub fn snap_move_tiled(x: f64, y: f64, p: &SnapParams) -> (f64, f64) {
    let nx = if p.cell_w > 0.5 { p.x().nearest_low_target(x) } else { x };
    let ny = if p.cell_h > 0.5 { p.y().nearest_low_target(y) } else { y };
    (nx, ny)
}

/// One axis of an interactive resize: the dragged content edge (low =
/// left/top, high = right/bottom) follows the pointer delta and snaps to the
/// visible cell edges; the opposite edge stays anchored. Returns the new
/// content length, at least `min_len`. Takes the axis view (`p.x()` for
/// width, `p.y()` for height). The single source of this math — both the
/// seat op and the arrange snapshot derive sizes from it, so the snapped
/// result can't be overridden by an unsnapped recomputation.
pub fn resize_axis(
    start_pos: f64,
    start_len: f64,
    delta: f64,
    dragging_low: bool,
    dragging_high: bool,
    min_len: f64,
    p: &AxisSnapParams,
) -> f64 {
    if dragging_low {
        let low = snap_low_edge(start_pos + delta, p);
        ((start_pos + start_len) - low).max(min_len)
    } else if dragging_high {
        let high = snap_high_edge(start_pos + start_len + delta, p);
        (high - start_pos).max(min_len)
    } else {
        start_len
    }
}

/// Hard grid snap for RESIZING a `Tiled` window: the dragged content edge
/// lands on the nearest visible cell edge of its kind (a left/top edge on a
/// cell start, a right/bottom edge on a cell end) from any distance, the
/// opposite edge stays anchored, and the result spans at least one whole
/// cell — so the window only ever covers whole squares, the way
/// `snap_move_tiled` guarantees for a move, and is still Tiled at op_end
/// instead of demoting to Floating on the first free resize. Not gated on
/// `enabled()` for the same reason as the move. A degenerate cell size
/// falls back to the magnetic resize rather than dividing by ~zero.
pub fn resize_axis_tiled(
    start_pos: f64,
    start_len: f64,
    delta: f64,
    dragging_low: bool,
    dragging_high: bool,
    p: &AxisSnapParams,
) -> f64 {
    if p.cell_size <= 0.5 {
        return resize_axis(start_pos, start_len, delta, dragging_low, dragging_high, 50.0, p);
    }
    // One visible cell: the anchored edge is on a cell edge, so this floor
    // is exactly "the dragged edge stops at the anchor's own cell".
    let one_cell = (p.cell_size - 2.0 * p.inset()).max(1.0);
    if dragging_low {
        let anchor = start_pos + start_len;
        let low = p.nearest_low_target(start_pos + delta);
        (anchor - low).max(one_cell)
    } else if dragging_high {
        let high = p.nearest_high_target(start_pos + start_len + delta);
        (high - start_pos).max(one_cell)
    } else {
        start_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Square 512 cells, no gap, fade inset 4: visible cell k spans
    /// `[512k + 4, 512k + 508]`. Border width is irrelevant to snapping now —
    /// content edges land on the targets and the border overhangs outward.
    fn params() -> SnapParams {
        SnapParams {
            cell_w: 512.0,
            cell_h: 512.0,
            gap_width: 0.0,
            cell_inset: 4.0,
            threshold: 24.0,
        }
    }

    #[test]
    fn resize_low_edge_abuts_visible_cell_edge() {
        // Content left 510 → visible edge 516 (dist 6, inside the 12 hold
        // radius) → content 516.
        assert_eq!(snap_low_edge(510.0, &params().x()), 516.0);
        // Far from an edge: unchanged.
        assert_eq!(snap_low_edge(300.0, &params().x()), 300.0);
    }

    #[test]
    fn resize_high_edge_abuts_visible_cell_edge() {
        // Content right 1010 → visible edge 1020 (2*512 - 4, dist 10) → 1020.
        assert_eq!(snap_high_edge(1010.0, &params().x()), 1020.0);
        // At dist 20 the edge is in the ramp: it keeps (20 - 12) * 2 = 16 of
        // its distance → 1004.
        assert_eq!(snap_high_edge(1000.0, &params().x()), 1004.0);
    }

    #[test]
    fn gap_width_shifts_the_period() {
        // cell 500 + gap 12 → period 512; cell 1's rect spans [512, 1012],
        // visibly [516, 1008].
        let p = SnapParams { cell_w: 500.0, cell_h: 500.0, gap_width: 12.0, ..params() };
        assert_eq!(snap_low_edge(520.0, &p.x()), 516.0);
        assert_eq!(snap_high_edge(996.0, &p.x()), 1008.0);
    }

    #[test]
    fn rectangular_cells_snap_each_axis_to_its_own_size() {
        // 512-wide, 256-tall cells, no gap, inset 4: x targets every 512,
        // y targets every 256 — row 1's visible top edge is 260.
        let p = SnapParams { cell_h: 256.0, ..params() };
        let (x, y) = snap_move(510.0, 250.0, 300.0, 100.0, &p);
        assert_eq!((x, y), (516.0, 260.0));
        // The hard tiled snap uses per-axis periods the same way.
        assert_eq!(snap_move_tiled(300.0, 300.0, &p), (516.0, 260.0));
        // A box filling one 504x248 visible cell is aligned, as is a
        // two-row 504-tall box (2*256 - 8); a height off the row grid is
        // not.
        assert!(is_cell_aligned(4.0, 4.0, 504.0, 248.0, &p, 1.0));
        assert!(is_cell_aligned(4.0, 4.0, 504.0, 504.0, &p, 1.0));
        assert!(!is_cell_aligned(4.0, 4.0, 504.0, 400.0, &p, 1.0));
    }

    #[test]
    fn move_snaps_the_closer_edge() {
        // Window content [506, 806]: left 506 → low target 516 (dist 10);
        // right 806 → high target 1020 (dist 214). Left wins: x = 516.
        let (x, y) = snap_move(506.0, 300.0, 300.0, 100.0, &params());
        assert_eq!(x, 516.0);
        assert_eq!(y, 300.0);

        // Right content edge 4 past the visible edge 508 beats left.
        // Content [212, 512]: right 512 → 508 (dist 4) → x = 208.
        let (x, _) = snap_move(212.0, 300.0, 300.0, 100.0, &params());
        assert_eq!(x, 208.0);
    }

    #[test]
    fn move_beyond_threshold_is_untouched() {
        let (x, y) = snap_move(100.0, 200.0, 300.0, 100.0, &params());
        assert_eq!((x, y), (100.0, 200.0));
    }

    #[test]
    fn resize_axis_snaps_the_dragged_edge_only() {
        // Window [600, 900), dragging the left edge to 510: visible edge 516
        // → content 516; anchored right edge 900 keeps the width at 384.
        assert_eq!(resize_axis(600.0, 300.0, -90.0, true, false, 50.0, &params().x()), 384.0);
        // Dragging the right edge to 1010: visible edge 1020 → width 420.
        assert_eq!(resize_axis(600.0, 300.0, 110.0, false, true, 50.0, &params().x()), 420.0);
        // To 1000 (dist 20, in the ramp): the edge is pulled to 1004 → 404.
        assert_eq!(resize_axis(600.0, 300.0, 100.0, false, true, 50.0, &params().x()), 404.0);
        // Not dragging this axis: length unchanged.
        assert_eq!(resize_axis(600.0, 300.0, 100.0, false, false, 50.0, &params().x()), 300.0);
        // Minimum clamps.
        assert_eq!(resize_axis(600.0, 300.0, 290.0, true, false, 50.0, &params().x()), 50.0);
    }

    #[test]
    fn tiled_span_covers_touched_visible_cells() {
        // period 100 (no gap), inset 0: legacy behavior — bare cell lines.
        assert_eq!(tiled_span(150.0, 250.0, 100.0, 0.0, 0.0), (100.0, 300.0));
        // period 110 (gap 10), inset 5: cells 1-2 visibly span [115, 315].
        assert_eq!(tiled_span(150.0, 250.0, 100.0, 10.0, 5.0), (115.0, 315.0));
        // Span ending exactly on a period boundary doesn't touch the next cell.
        assert_eq!(tiled_span(150.0, 220.0, 100.0, 10.0, 5.0), (115.0, 205.0));
    }

    #[test]
    fn zoomed_out_threshold_holds_screen_size() {
        // Threshold 24 at zoom 0.5 → 48 virtual = the same 24 screen px.
        let p = params().for_zoom(0.5);
        assert_eq!(p.threshold, 48.0);
        // Deep zoom-out caps at 45% of the cell (512 → 230.4).
        let p = params().for_zoom(0.05);
        assert!((p.threshold - 230.4).abs() < 1e-9);
        // Zoom 1 unchanged; zoomed in shrinks (still 24 screen px).
        assert_eq!(params().for_zoom(1.0).threshold, 24.0);
        assert_eq!(params().for_zoom(2.0).threshold, 12.0);
        // Disabled stays disabled.
        let p = SnapParams { threshold: 0.0, ..params() }.for_zoom(0.5);
        assert_eq!(p.threshold, 0.0);
    }

    #[test]
    fn cell_aligned_needs_all_four_edges() {
        // cell 512, inset 4: cell 0 visibly spans [4, 508], cells 0-1 [4, 1020].
        let p = params();
        assert!(is_cell_aligned(4.0, 4.0, 504.0, 504.0, &p, 1.0));
        // Two-cell-wide span.
        assert!(is_cell_aligned(4.0, 4.0, 1016.0, 504.0, &p, 1.0));
        // One edge off-grid fails.
        assert!(!is_cell_aligned(10.0, 4.0, 504.0, 504.0, &p, 1.0)); // left off
        assert!(!is_cell_aligned(4.0, 4.0, 500.0, 504.0, &p, 1.0)); // right off
        assert!(!is_cell_aligned(4.0, 4.0, 504.0, 512.0, &p, 1.0)); // bottom off
        // Alignment ignores the snap threshold.
        let p = SnapParams { threshold: 0.0, ..params() };
        assert!(is_cell_aligned(4.0, 4.0, 504.0, 504.0, &p, 1.0));
        // Degenerate boxes are never tiled.
        assert!(!is_cell_aligned(4.0, 4.0, 0.0, 504.0, &params(), 1.0));
    }

    #[test]
    fn pull_is_continuous_and_monotonic() {
        // Target 516, threshold 24, hold radius 12. Approaching from the
        // left: untouched at the threshold, then drawn in without a jump.
        let p = params().x();
        assert_eq!(snap_low_edge(492.0, &p), 492.0); // dist 24: at the threshold
        assert_eq!(snap_low_edge(504.0, &p), 516.0); // dist 12: on target
        assert_eq!(snap_low_edge(498.0, &p), 504.0); // dist 18: halfway in
        let mut prev = snap_low_edge(490.0, &p);
        let mut max_step: f64 = 0.0;
        for i in 1..=60 {
            let pos = 490.0 + i as f64 * 0.5;
            let out = snap_low_edge(pos, &p);
            assert!(out >= prev, "pull went backwards at {pos}");
            max_step = max_step.max(out - prev);
            prev = out;
        }
        // Half-unit pointer steps never move the edge more than a unit —
        // the ramp's gain is 2 — where the old step rule jumped 24 at once.
        assert!(max_step <= 1.0 + 1e-9, "max step {max_step}");
        // Symmetric from the right.
        assert_eq!(snap_low_edge(540.0, &p), 540.0);
        assert_eq!(snap_low_edge(534.0, &p), 528.0);
        assert_eq!(snap_low_edge(528.0, &p), 516.0);
    }

    #[test]
    fn zero_threshold_disables() {
        let p = SnapParams { threshold: 0.0, ..params() };
        assert_eq!(snap_move(510.0, 300.0, 300.0, 100.0, &p), (510.0, 300.0));
        assert_eq!(snap_low_edge(510.0, &p.x()), 510.0);
    }

    #[test]
    fn tiled_move_snaps_hard_from_any_distance() {
        // cell 512, no gap, inset 4: cell starts are 4, 516, 1028 …
        let p = params();
        // Well beyond the 24px magnetic threshold, where snap_move gives up.
        assert_eq!(snap_move(200.0, 200.0, 504.0, 504.0, &p), (200.0, 200.0));
        // The tiled snap still lands on the nearest cell start (cell 0 at 4).
        assert_eq!(snap_move_tiled(200.0, 200.0, &p), (4.0, 4.0));
        // Past the midpoint it commits to the next cell instead (cell 1 at 516).
        assert_eq!(snap_move_tiled(300.0, 300.0, &p), (516.0, 516.0));
        // Negative canvas coordinates snap the same way (cell -1 at -508).
        assert_eq!(snap_move_tiled(-400.0, -400.0, &p), (-508.0, -508.0));
    }

    #[test]
    fn tiled_move_result_is_always_cell_aligned() {
        // The point of the hard snap: whatever the drag ends on, the window
        // is still Tiled at op_end instead of silently demoting to Floating.
        let p = params();
        for start in [0.0, 37.0, 260.0, 700.0, -13.0, -900.0] {
            let (x, y) = snap_move_tiled(start, start, &p);
            assert!(
                is_cell_aligned(x, y, 504.0, 504.0, &p, 1.0),
                "drag ending at {start} left the window off-grid at ({x}, {y})"
            );
        }
    }

    #[test]
    fn tiled_resize_snaps_hard_to_whole_cells() {
        // cell 512, no gap, inset 4: cell k visibly spans [512k+4, 512k+508].
        let p = params().x();
        // Two-cell window [4, 1020): dragging the right edge in by 400 puts
        // it at 620, nearest cell end 508 → one cell wide (504).
        assert_eq!(resize_axis_tiled(4.0, 1016.0, -400.0, false, true, &p), 504.0);
        // Out by 300 → 1320, nearest cell end 1532 → three cells (1528).
        assert_eq!(resize_axis_tiled(4.0, 1016.0, 300.0, false, true, &p), 1528.0);
        // Dragging the left edge to 304: nearest cell start 516 → one cell,
        // the right edge anchored at 1020.
        assert_eq!(resize_axis_tiled(4.0, 1016.0, 300.0, true, false, &p), 504.0);
        // Past the anchor's own cell the size floors at one cell.
        assert_eq!(resize_axis_tiled(4.0, 1016.0, -900.0, false, true, &p), 504.0);
        assert_eq!(resize_axis_tiled(4.0, 1016.0, 1000.0, true, false, &p), 504.0);
        // Not dragging this axis: unchanged.
        assert_eq!(resize_axis_tiled(4.0, 1016.0, 300.0, false, false, &p), 1016.0);
        // Every result keeps the window cell-aligned.
        for d in [-900.0, -400.0, -10.0, 0.0, 130.0, 300.0, 700.0] {
            let w = resize_axis_tiled(4.0, 1016.0, d, false, true, &p);
            assert!(is_cell_aligned(4.0, 4.0, w, 504.0, &params(), 1e-9), "delta {d} → width {w}");
        }
        // The threshold plays no part.
        let off = SnapParams { threshold: 0.0, ..params() }.x();
        assert_eq!(resize_axis_tiled(4.0, 1016.0, -400.0, false, true, &off), 504.0);
    }

    #[test]
    fn tiled_move_ignores_a_disabled_threshold() {
        // Magnetic snapping off must not strand a tiled window between cells.
        let p = SnapParams { threshold: 0.0, ..params() };
        assert_eq!(snap_move_tiled(300.0, 300.0, &p), (516.0, 516.0));
        // A degenerate cell size is left alone rather than dividing by ~zero.
        let p = SnapParams { cell_w: 0.0, cell_h: 0.0, ..params() };
        assert_eq!(snap_move_tiled(300.0, 300.0, &p), (300.0, 300.0));
    }
}
