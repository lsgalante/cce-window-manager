// Magnetic grid snapping for interactive move/resize.
//
// All coordinates are virtual-surface CONTENT coordinates. Snapping is
// border-inclusive: the border's outer edge is what lands on the snap
// target, matching the Maximized grid-snap convention where borders stay
// inside the covered cells.
//
// Targets are the VISIBLE cell edges, not the raw grid lines. The desktop
// grid draws cells of `cell_size` every `cell_size + gap_width`, and each
// cell fades inward by `cell_inset` — so cell k's visible span is
// [k*period + inset, k*period + cell_size - inset]. A left/top edge snaps
// to the former, a right/bottom edge to the latter, letting windows abut
// the cells instead of floating mid-gap.

#[derive(Debug, Clone, Copy)]
pub struct SnapParams {
    /// Desktop grid cell size in virtual units.
    pub cell_size: f64,
    /// Gap between cells; the grid period is `cell_size + gap_width`.
    pub gap_width: f64,
    /// Visual inset of a cell's edge (the fade inset).
    pub cell_inset: f64,
    /// Snap radius in virtual units; <= 0 disables snapping.
    pub threshold: f64,
    /// Server-side border width (border-inclusive alignment).
    pub border_width: f64,
}

impl SnapParams {
    fn enabled(&self) -> bool {
        self.threshold > 0.0 && self.cell_size > 0.5
    }

    fn bw(&self) -> f64 {
        self.border_width.max(0.0)
    }

    fn period(&self) -> f64 {
        self.cell_size + self.gap_width.max(0.0)
    }

    /// Inset clamped so the two visible edges of a cell can't cross.
    fn inset(&self) -> f64 {
        self.cell_inset.clamp(0.0, self.cell_size / 2.0 - 1.0)
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

fn within(delta: f64, p: &SnapParams) -> bool {
    delta.abs() <= p.threshold
}

/// Snap a window position during a move. On each axis the two outer border
/// edges compete for their nearest visible cell edge; the closer candidate
/// within the threshold wins. `w`/`h` are content sizes.
pub fn snap_move(x: f64, y: f64, w: f64, h: f64, p: &SnapParams) -> (f64, f64) {
    (snap_move_axis(x, w, p), snap_move_axis(y, h, p))
}

fn snap_move_axis(pos: f64, len: f64, p: &SnapParams) -> f64 {
    if !p.enabled() {
        return pos;
    }
    let lo = pos - p.bw();
    let hi = pos + len + p.bw();
    let lo_delta = p.nearest_low_target(lo) - lo;
    let hi_delta = p.nearest_high_target(hi) - hi;
    if lo_delta.abs() <= hi_delta.abs() && within(lo_delta, p) {
        pos + lo_delta
    } else if within(hi_delta, p) {
        pos + hi_delta
    } else {
        pos
    }
}

/// Snap the dragged left/top CONTENT edge during a resize: the outer border
/// edge (content - border width) is pulled onto the nearest visible left/top
/// cell edge.
pub fn snap_low_edge(pos: f64, p: &SnapParams) -> f64 {
    if !p.enabled() {
        return pos;
    }
    let outer = pos - p.bw();
    let delta = p.nearest_low_target(outer) - outer;
    if within(delta, p) {
        pos + delta
    } else {
        pos
    }
}

/// Snap the dragged right/bottom CONTENT edge during a resize: the outer
/// border edge (content + border width) is pulled onto the nearest visible
/// right/bottom cell edge.
pub fn snap_high_edge(pos: f64, p: &SnapParams) -> f64 {
    if !p.enabled() {
        return pos;
    }
    let outer = pos + p.bw();
    let delta = p.nearest_high_target(outer) - outer;
    if within(delta, p) {
        pos + delta
    } else {
        pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cell 512, no gap, fade inset 4, border 8: visible cell k spans
    /// [512k + 4, 512k + 508].
    fn params() -> SnapParams {
        SnapParams { cell_size: 512.0, gap_width: 0.0, cell_inset: 4.0, threshold: 24.0, border_width: 8.0 }
    }

    #[test]
    fn resize_low_edge_abuts_visible_cell_edge() {
        // Content left 510 → outer 502 → visible edge 516 (dist 14) →
        // content 524, border spans [516, 524].
        assert_eq!(snap_low_edge(510.0, &params()), 524.0);
        // Far from an edge: unchanged.
        assert_eq!(snap_low_edge(300.0, &params()), 300.0);
    }

    #[test]
    fn resize_high_edge_abuts_visible_cell_edge() {
        // Content right 1000 → outer 1008 → visible edge 1020
        // (2*512 - 4, dist 12) → content 1012, border spans [1012, 1020].
        assert_eq!(snap_high_edge(1000.0, &params()), 1012.0);
    }

    #[test]
    fn gap_width_shifts_the_period() {
        // cell 500 + gap 12 → period 512; cell 1's rect spans [512, 1012],
        // visibly [516, 1008].
        let p = SnapParams { cell_size: 500.0, gap_width: 12.0, ..params() };
        assert_eq!(snap_low_edge(520.0, &p), 524.0); // outer 512 → 516
        assert_eq!(snap_high_edge(996.0, &p), 1000.0); // outer 1004 → 1008
    }

    #[test]
    fn move_snaps_the_closer_edge() {
        // Window content [500, 800]: left outer 492 → low target 516
        // (dist 24); right outer 808 → high target... 1020 (dist 212).
        // Left wins: x = 524.
        let (x, y) = snap_move(500.0, 300.0, 300.0, 100.0, &params());
        assert_eq!(x, 524.0);
        assert_eq!(y, 300.0);

        // Right outer edge 4 away from the visible edge 508 beats left.
        // Content [196, 496]: right outer 504 → 508 (dist 4) → x = 200.
        let (x, _) = snap_move(196.0, 300.0, 300.0, 100.0, &params());
        assert_eq!(x, 200.0);
    }

    #[test]
    fn move_beyond_threshold_is_untouched() {
        let (x, y) = snap_move(100.0, 200.0, 300.0, 100.0, &params());
        assert_eq!((x, y), (100.0, 200.0));
    }

    #[test]
    fn zero_threshold_disables() {
        let p = SnapParams { threshold: 0.0, ..params() };
        assert_eq!(snap_move(510.0, 300.0, 300.0, 100.0, &p), (510.0, 300.0));
        assert_eq!(snap_low_edge(510.0, &p), 510.0);
    }
}
