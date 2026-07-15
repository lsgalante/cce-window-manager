// Magnetic grid snapping for interactive move/resize.
//
// All coordinates are virtual-surface CONTENT coordinates. Snapping is
// border-inclusive: the border's outer edge is what lands on a grid line
// (content inset by the border width), matching the Maximized grid-snap
// convention where borders stay inside the covered cells.

#[derive(Debug, Clone, Copy)]
pub struct SnapParams {
    /// Desktop grid cell size in virtual units.
    pub grid_scale: f64,
    /// Snap radius in virtual units; <= 0 disables snapping.
    pub threshold: f64,
    /// Server-side border width (border-inclusive alignment).
    pub border_width: f64,
}

impl SnapParams {
    fn enabled(&self) -> bool {
        self.threshold > 0.0 && self.grid_scale > 0.5
    }

    fn bw(&self) -> f64 {
        self.border_width.max(0.0)
    }
}

fn nearest_line(v: f64, scale: f64) -> f64 {
    (v / scale).round() * scale
}

/// Snap an outer-edge coordinate to the nearest grid line if it is within
/// the threshold; otherwise return it unchanged.
fn snap_outer(v: f64, p: &SnapParams) -> f64 {
    let target = nearest_line(v, p.grid_scale);
    if (target - v).abs() <= p.threshold {
        target
    } else {
        v
    }
}

/// Snap a window position during a move. On each axis the two outer border
/// edges compete for their nearest grid line; the closer candidate within
/// the threshold wins. `w`/`h` are content sizes.
pub fn snap_move(x: f64, y: f64, w: f64, h: f64, p: &SnapParams) -> (f64, f64) {
    (snap_move_axis(x, w, p), snap_move_axis(y, h, p))
}

fn snap_move_axis(pos: f64, len: f64, p: &SnapParams) -> f64 {
    if !p.enabled() {
        return pos;
    }
    let lo = pos - p.bw();
    let hi = pos + len + p.bw();
    let lo_delta = nearest_line(lo, p.grid_scale) - lo;
    let hi_delta = nearest_line(hi, p.grid_scale) - hi;
    if lo_delta.abs() <= hi_delta.abs() && lo_delta.abs() <= p.threshold {
        pos + lo_delta
    } else if hi_delta.abs() <= p.threshold {
        pos + hi_delta
    } else {
        pos
    }
}

/// Snap the dragged left/top CONTENT edge during a resize: the outer border
/// edge (content - border width) is pulled onto the grid line.
pub fn snap_low_edge(pos: f64, p: &SnapParams) -> f64 {
    if !p.enabled() {
        return pos;
    }
    snap_outer(pos - p.bw(), p) + p.bw()
}

/// Snap the dragged right/bottom CONTENT edge during a resize: the outer
/// border edge (content + border width) is pulled onto the grid line.
pub fn snap_high_edge(pos: f64, p: &SnapParams) -> f64 {
    if !p.enabled() {
        return pos;
    }
    snap_outer(pos + p.bw(), p) - p.bw()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SnapParams {
        SnapParams { grid_scale: 512.0, threshold: 24.0, border_width: 8.0 }
    }

    #[test]
    fn move_snaps_low_outer_edge_within_threshold() {
        // Left outer edge at 492, 20 away from the 512 line: snaps on.
        let (x, y) = snap_move(500.0, 300.0, 300.0, 100.0, &params());
        assert_eq!(x, 520.0); // outer edge = 520 - 8 = 512
        assert_eq!(y, 300.0); // outer edges 292/408 are far from any line
    }

    #[test]
    fn move_prefers_the_closer_edge() {
        // Right outer edge at 508 (4 from 512) beats left at 208 (208 from 0).
        let (x, _) = snap_move(200.0, 300.0, 300.0, 100.0, &params());
        assert_eq!(x, 204.0); // right outer = 204 + 300 + 8 = 512
    }

    #[test]
    fn move_beyond_threshold_is_untouched() {
        let (x, y) = snap_move(100.0, 200.0, 300.0, 100.0, &params());
        assert_eq!((x, y), (100.0, 200.0));
    }

    #[test]
    fn resize_edges_snap_border_inclusive() {
        // Content left 500 → outer 492 → line 512 → content 520.
        assert_eq!(snap_low_edge(500.0, &params()), 520.0);
        // Content right 1000 → outer 1008 → line 1024 → content 1016.
        assert_eq!(snap_high_edge(1000.0, &params()), 1016.0);
        // Far from a line: unchanged.
        assert_eq!(snap_low_edge(300.0, &params()), 300.0);
    }

    #[test]
    fn zero_threshold_disables() {
        let p = SnapParams { threshold: 0.0, ..params() };
        assert_eq!(snap_move(500.0, 300.0, 300.0, 100.0, &p), (500.0, 300.0));
        assert_eq!(snap_low_edge(500.0, &p), 500.0);
    }
}
