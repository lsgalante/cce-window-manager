// Overview-mode move rules: when a dragged window covers another window, the
// covered window automatically relocates to the vacated side.
//
// The rule is swap-like: dragging the browser rightward onto the system
// interface displaces the system interface to the browser's LEFT — the
// covered window exits toward the side the drag vacated, i.e. opposite the
// dominant axis/sign of the drag delta, abutting the dragged window with
// the desktop-grid gap. Called by the mechanism on every motion event of an
// overview move, so windows scoot out of the way live; it is naturally
// convergent because a displaced window no longer overlaps the dragged one.
//
// Displacement is single-level on purpose: a displaced window may itself
// land on a third window without cascading further. Coordinates are
// virtual-surface content coordinates throughout.

use crate::snap::{self, SnapParams};

/// A window the dragged window may displace, as the mechanism snapshots it.
#[derive(Debug, Clone, Copy)]
pub struct DisplaceCandidate {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Tiled candidates get their displaced position snapped back onto the
    /// grid so they stay tiled.
    pub tiled: bool,
}

/// Covered fraction (of the smaller window) that triggers displacement.
pub const DISPLACE_THRESHOLD: f64 = 0.5;

fn overlap_1d(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// Decide displacements for one motion step of an overview drag. `moved` is
/// the dragged window's current content box (x, y, w, h); `drag_delta` is
/// the cumulative virtual-space delta since the grab started. Returns
/// `(candidate index, new position)` for every candidate the drag covers.
///
/// A candidate is covered when the overlap area exceeds
/// [`DISPLACE_THRESHOLD`] of the smaller of the two windows. It exits toward
/// the side the drag vacated — opposite the dominant axis/sign of
/// `drag_delta` (a rightward drag sends it to the dragged window's left) —
/// abutting the dragged window's content box with `gap` between them. With
/// no meaningful drag delta it falls back to flipping the candidate across
/// the dragged window along their center offset.
pub fn displace(
    moved: (f64, f64, f64, f64),
    drag_delta: (f64, f64),
    candidates: &[DisplaceCandidate],
    p: &SnapParams,
    gap: f64,
) -> Vec<(usize, (f64, f64))> {
    let (mx, my, mw, mh) = moved;
    if mw <= 0.0 || mh <= 0.0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        if c.w <= 0.0 || c.h <= 0.0 {
            continue;
        }
        let overlap = overlap_1d(mx, mx + mw, c.x, c.x + c.w)
            * overlap_1d(my, my + mh, c.y, c.y + c.h);
        let smaller = (mw * mh).min(c.w * c.h);
        if overlap <= smaller * DISPLACE_THRESHOLD {
            continue;
        }

        // Exit toward the vacated side: opposite the drag direction on its
        // dominant axis. A grab with no travel yet (or a degenerate delta)
        // falls back to flipping the candidate across the dragged window
        // along their center offset.
        let (dx, dy) = if drag_delta.0.abs() >= 1.0 || drag_delta.1.abs() >= 1.0 {
            drag_delta
        } else {
            (
                (c.x + c.w / 2.0) - (mx + mw / 2.0),
                (c.y + c.h / 2.0) - (my + mh / 2.0),
            )
        };
        let (mut nx, mut ny) = (c.x, c.y);
        if dx.abs() >= dy.abs() {
            nx = if dx >= 0.0 { mx - gap - c.w } else { mx + mw + gap };
        } else {
            ny = if dy >= 0.0 { my - gap - c.h } else { my + mh + gap };
        }

        // A tiled candidate stays tiled: hard-snap the landing spot to the
        // nearest cell edges, no threshold (its size is already
        // cell-quantized, so aligning the low edges aligns the whole box).
        // Magnetic snapping cannot be widened into a guarantee — targets
        // are one PERIOD apart, so any threshold below (cell + gap)/2
        // leaves a dead band around the midpoint where the exit spot rests
        // mid-cell, and the arrange pass then expands the "tiled" window to
        // every cell the off-grid box touches.
        if c.tiled {
            let (sx, sy) = snap::snap_move_tiled(nx, ny, p);
            nx = sx;
            ny = sy;
        }

        if (nx - c.x).abs() > f64::EPSILON || (ny - c.y).abs() > f64::EPSILON {
            out.push((i, (nx, ny)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SnapParams {
        // cell 512, no gap, fade inset 4: visible cell k spans
        // [512k + 4, 512k + 508].
        SnapParams { cell_size: 512.0, gap_width: 0.0, cell_inset: 4.0, threshold: 24.0 }
    }

    fn cand(x: f64, y: f64, w: f64, h: f64) -> DisplaceCandidate {
        DisplaceCandidate { x, y, w, h, tiled: false }
    }

    #[test]
    fn browser_dragged_right_displaces_neighbor_to_its_left() {
        // Browser (800x600) starts left of the system interface (600x600)
        // and is dragged rightward until it covers most of it.
        let system_interface = cand(1000.0, 100.0, 600.0, 600.0);
        // Browser now at x=900: overlap x [1000, 1600] = 600... fully
        // covering the system interface horizontally is not needed; 60%
        // coverage of the smaller window triggers.
        let moved = (900.0, 100.0, 800.0, 600.0);
        let d = displace(moved, (500.0, 0.0), &[system_interface], &params(), 16.0);
        assert_eq!(d.len(), 1);
        let (idx, (nx, ny)) = d[0];
        assert_eq!(idx, 0);
        // Rightward drag: the system interface exits to the browser's
        // left, abutting with the gap: 900 - 16 - 600.
        assert_eq!((nx, ny), (284.0, 100.0));
    }

    #[test]
    fn approach_from_the_right_displaces_rightward() {
        // Dragged window comes from the right; the covered window exits
        // right — into the vacated space.
        let covered = cand(1000.0, 100.0, 600.0, 600.0);
        // Overlap x [1150, 1600] = 450 of 600 → 75% of the smaller window.
        let moved = (1150.0, 100.0, 800.0, 600.0);
        let d = displace(moved, (-450.0, 0.0), &[covered], &params(), 16.0);
        assert_eq!(d.len(), 1);
        // Leftward drag: the candidate goes to the moved window's RIGHT:
        // 1150 + 800 + 16.
        assert_eq!(d[0].1, (1966.0, 100.0));
    }

    #[test]
    fn vertical_approach_displaces_vertically() {
        let covered = cand(100.0, 800.0, 600.0, 500.0);
        // Dragged from above, covering the top 60% of the candidate.
        let moved = (100.0, 500.0, 600.0, 600.0);
        let d = displace(moved, (0.0, 300.0), &[covered], &params(), 16.0);
        assert_eq!(d.len(), 1);
        // Downward drag: the candidate exits above, into the vacated space:
        // y = 500 - 16 - 500.
        assert_eq!(d[0].1, (100.0, -16.0));
    }

    #[test]
    fn below_threshold_is_untouched() {
        // 40% horizontal overlap of the smaller window: no displacement.
        let covered = cand(1000.0, 100.0, 600.0, 600.0);
        let moved = (640.0, 100.0, 600.0, 600.0); // overlap x = 240 → 40%
        assert!(displace(moved, (300.0, 0.0), &[covered], &params(), 16.0).is_empty());
    }

    #[test]
    fn tiled_candidate_lands_on_cell_edges() {
        // A tiled candidate filling cell 2 exactly: visible content box
        // [1028, 1532] → x=1028, w=504.
        let covered = DisplaceCandidate { x: 1028.0, y: 4.0, w: 504.0, h: 504.0, tiled: true };
        // Dragged window covers it, approaching from the left; its own box
        // is NOT grid-aligned (mid-drag).
        let moved = (700.0, 10.0, 700.0, 500.0);
        let d = displace(moved, (400.0, 6.0), &[covered], &params(), 16.0);
        assert_eq!(d.len(), 1);
        // Raw exit spot 700 - 16 - 504 = 180 snaps onto cell 0's visible
        // box: left edge 180 → 4 (within the widened threshold), y 10 → 4.
        // The candidate stays cell-aligned.
        assert_eq!(d[0].1, (4.0, 4.0));
    }

    #[test]
    fn tiled_exit_in_magnetic_dead_band_still_snaps() {
        // With a gap the grid period is 528, so snap targets are 528 apart
        // and the old widened magnetic snap (threshold 0.45 * cell = 230.4)
        // had a ~67px dead band around the midpoint: a landing spot ~256px
        // from the nearest edge stayed mid-cell, and the arrange pass then
        // grew the "tiled" window to every cell it touched.
        let p = SnapParams { cell_size: 512.0, gap_width: 16.0, cell_inset: 4.0, threshold: 24.0 };
        // Tiled candidate filling cell row 1 exactly: visible box
        // y [532, 1036] → y=532, h=504.
        let covered = DisplaceCandidate { x: 4.0, y: 532.0, w: 504.0, h: 504.0, tiled: true };
        // Dragged DOWN onto it; the mover's mid-drag y is not grid-aligned.
        // Overlap y [780, 1036] = 256 of 504 → ~51% of the smaller window.
        let moved = (4.0, 780.0, 600.0, 600.0);
        let d = displace(moved, (0.0, 300.0), &[covered], &p, 16.0);
        assert_eq!(d.len(), 1);
        // Downward drag: the candidate exits above, abutting the mover:
        // raw y = 780 - 16 - 504 = 260 — 256 from the nearest low target
        // (4), squarely in the old dead band. The hard snap lands it there
        // anyway; x is untouched and already aligned.
        assert_eq!(d[0].1, (4.0, 4.0));
    }

    #[test]
    fn multiple_covered_windows_each_displace() {
        let a = cand(1000.0, 100.0, 400.0, 400.0);
        let b = cand(1000.0, 600.0, 400.0, 400.0);
        // A tall dragged window covering both.
        // Dragged rightward: both exit to the dragged window's left even
        // though their centers are offset vertically from the mover's.
        let moved = (900.0, 50.0, 600.0, 1000.0);
        let d = displace(moved, (400.0, 0.0), &[a, b], &params(), 16.0);
        assert_eq!(d.len(), 2);
        // Both exit left — opposite the rightward drag.
        assert_eq!(d[0], (0, (484.0, 100.0)));
        assert_eq!(d[1], (1, (484.0, 600.0)));
    }
}
