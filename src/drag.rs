//! Pointer drags: where a moved window lands, the size a resize gives it,
//! and which edge a dragged status bar docks to. Pure functions over where
//! the drag began, how far the pointer has gone and how far the camera has
//! panned meanwhile; the compositor's seat op feeds them and applies what
//! they return. Moved here from the compositor's `Seat::op_update` on
//! 2026-10-10, which also lets the arrange snapshot's resize
//! (`get_active_resize_dimensions`) call the same function instead of
//! repeating its arithmetic.

use crate::arrange::StatusEdge;
use crate::snap::{self, SnapParams};

/// The smallest a Floating resize makes either side, in virtual units.
pub const FLOATING_MIN_SIZE: f64 = 50.0;

/// How far a drag has carried, in virtual units: the pointer's travel
/// (`dx`, `dy`, layout px) over the zoom, plus however far the camera
/// panned since the grab (`pan` now, `start_pan` then), so a window held
/// still while the desk edge-pans under it keeps up with the desk.
pub fn virtual_delta(dx: i32, dy: i32, zoom: f64, pan: (f64, f64), start_pan: (f64, f64)) -> (f64, f64) {
    (dx as f64 / zoom + (pan.0 - start_pan.0), dy as f64 / zoom + (pan.1 - start_pan.1))
}

/// Where a moved window's top-left lands: `start` plus `delta`, snapped.
/// A window grabbed Tiled snaps hard to the nearest whole cell (it only
/// ever occupies whole cells); a Floating one gets the magnetic pull onto
/// visible cell edges, which decides at release whether it lands aligned
/// and so becomes Tiled. `start_was_tiled` is what the window was when
/// grabbed: the first motion un-tiles a dragged window so it can follow the
/// pointer, so its live mode is always Floating by now.
pub fn move_to(
    start: (f64, f64),
    delta: (f64, f64),
    size: (f64, f64),
    start_was_tiled: bool,
    sp: &SnapParams,
) -> (f64, f64) {
    let (x, y) = (start.0 + delta.0, start.1 + delta.1);
    if start_was_tiled {
        snap::snap_move_tiled(x, y, sp)
    } else {
        snap::snap_move(x, y, size.0, size.1, sp)
    }
}

/// The offset a group move carries its windows by when grabbed by an image
/// (no window under the pointer): the travel itself, snapped to whole cells
/// when a Tiled window is along — measured on that window, `start` being its
/// top-left, so it lands on cells.
pub fn group_offset(start: (f64, f64), delta: (f64, f64), start_was_tiled: bool, sp: &SnapParams) -> (f64, f64) {
    if start_was_tiled {
        let (sx, sy) = snap::snap_move_tiled(start.0 + delta.0, start.1 + delta.1, sp);
        (sx - start.0, sy - start.1)
    } else {
        delta
    }
}

/// Which edges of a window a resize drags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

/// The size a resize gives a window that began at `start` (top-left) and
/// `start_size`, after the drag has carried `delta`. A window grabbed Tiled
/// snaps hard: the dragged edge lands on a cell edge from any distance and
/// the size stays whole cells, so it is still Tiled on release. A Floating
/// one gets the magnetic pull onto visible cell edges and is never smaller
/// than [`FLOATING_MIN_SIZE`]. The anchored edge is untouched either way.
pub fn resize_to(
    start: (f64, f64),
    start_size: (u32, u32),
    delta: (f64, f64),
    edges: Edges,
    start_was_tiled: bool,
    sp: &SnapParams,
) -> (u32, u32) {
    let (w, h) = (start_size.0 as f64, start_size.1 as f64);
    if start_was_tiled {
        (
            snap::resize_axis_tiled(start.0, w, delta.0, edges.left, edges.right, &sp.x()) as u32,
            snap::resize_axis_tiled(start.1, h, delta.1, edges.top, edges.bottom, &sp.y()) as u32,
        )
    } else {
        (
            snap::resize_axis(start.0, w, delta.0, edges.left, edges.right, FLOATING_MIN_SIZE, &sp.x()) as u32,
            snap::resize_axis(start.1, h, delta.1, edges.top, edges.bottom, FLOATING_MIN_SIZE, &sp.y()) as u32,
        )
    }
}

/// How far in from an output's corner a status bar dragged along an edge
/// docks to that corner rather than the edge's middle, layout px.
pub const STATUS_CORNER_BAND: f64 = 120.0;

/// The radius of each docking socket — the corners, the middles of the top
/// and bottom edges, the middles of the sides — inside which a dragged
/// status bar docks there outright, layout px.
pub const STATUS_SOCKET_RADIUS: f64 = 60.0;

/// The edge a status bar dragged to the pointer at `p` docks to, among the
/// enabled `outputs` (layout boxes `(x, y, w, h)`); `None` with no output.
///
/// The output is the one nearest the pointer. Inside one of its sockets the
/// bar docks there; otherwise to the output edge the pointer is nearest, at
/// that edge's corner when within [`STATUS_CORNER_BAND`] of it and in its
/// middle otherwise.
pub fn status_edge_at(p: (f64, f64), outputs: &[(f64, f64, f64, f64)]) -> Option<StatusEdge> {
    let (lx, ly) = p;
    let mut best: Option<(f64, f64, f64, f64)> = None;
    let mut best_dist = f64::MAX;
    for &(ox, oy, ow, oh) in outputs {
        let cx = lx.max(ox).min(ox + ow);
        let cy = ly.max(oy).min(oy + oh);
        let dist = (lx - cx).powi(2) + (ly - cy).powi(2);
        if dist < best_dist {
            best_dist = dist;
            best = Some((ox, oy, ow, oh));
        }
    }
    let (ox, oy, ow, oh) = best?;

    let sockets = [
        (StatusEdge::TopLeft, ox + STATUS_SOCKET_RADIUS, oy),
        (StatusEdge::TopCenter, ox + ow / 2.0, oy),
        (StatusEdge::TopRight, ox + ow - STATUS_SOCKET_RADIUS, oy),
        (StatusEdge::BottomLeft, ox + STATUS_SOCKET_RADIUS, oy + oh),
        (StatusEdge::BottomCenter, ox + ow / 2.0, oy + oh),
        (StatusEdge::BottomRight, ox + ow - STATUS_SOCKET_RADIUS, oy + oh),
        (StatusEdge::Left, ox, oy + oh / 2.0),
        (StatusEdge::Right, ox + ow, oy + oh / 2.0),
    ];
    for (edge, cx, cy) in sockets {
        if (lx - cx).powi(2) + (ly - cy).powi(2) <= STATUS_SOCKET_RADIUS * STATUS_SOCKET_RADIUS {
            return Some(edge);
        }
    }

    #[derive(Clone, Copy)]
    enum Side {
        Top,
        Bottom,
        Left,
        Right,
    }
    // The nearest side, the earlier of a tie.
    let mut side = Side::Top;
    let mut min_dist = f64::MAX;
    for (s, d) in [(Side::Top, ly - oy), (Side::Bottom, (oy + oh) - ly), (Side::Left, lx - ox), (Side::Right, (ox + ow) - lx)] {
        if d < min_dist {
            min_dist = d;
            side = s;
        }
    }
    let near_top = ly < oy + STATUS_CORNER_BAND;
    let near_bottom = ly > oy + oh - STATUS_CORNER_BAND;
    let near_left = lx < ox + STATUS_CORNER_BAND;
    let near_right = lx > ox + ow - STATUS_CORNER_BAND;
    Some(match side {
        Side::Top if near_left => StatusEdge::TopLeft,
        Side::Top if near_right => StatusEdge::TopRight,
        Side::Top => StatusEdge::TopCenter,
        Side::Bottom if near_left => StatusEdge::BottomLeft,
        Side::Bottom if near_right => StatusEdge::BottomRight,
        Side::Bottom => StatusEdge::BottomCenter,
        Side::Left if near_top => StatusEdge::TopLeft,
        Side::Left if near_bottom => StatusEdge::BottomLeft,
        Side::Left => StatusEdge::Left,
        Side::Right if near_top => StatusEdge::TopRight,
        Side::Right if near_bottom => StatusEdge::BottomRight,
        Side::Right => StatusEdge::Right,
    })
}

/// A status bar's size docked to `edge`: `bar_h` thick and `length` long,
/// standing up along a side and lying down everywhere else.
pub fn status_bar_size(edge: StatusEdge, bar_h: u32, length: u32) -> (u32, u32) {
    match edge {
        StatusEdge::Left | StatusEdge::Right => (bar_h, length),
        _ => (length, bar_h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp() -> SnapParams {
        SnapParams {
            cell_w: 100.0,
            cell_h: 100.0,
            gap_width: 0.0,
            cell_inset: 0.0,
            threshold: 0.0,
        }
    }

    #[test]
    fn the_delta_counts_the_pan_and_the_zoom() {
        assert_eq!(virtual_delta(50, -20, 0.5, (10.0, 0.0), (0.0, 5.0)), (110.0, -45.0));
    }

    #[test]
    fn a_window_grabbed_floating_follows_the_pointer() {
        // No magnetic pull (threshold 0): it lands where it was carried.
        assert_eq!(move_to((10.0, 20.0), (33.0, -7.0), (200.0, 150.0), false, &sp()), (43.0, 13.0));
        assert_eq!(group_offset((10.0, 20.0), (33.0, -7.0), false, &sp()), (33.0, -7.0));
    }

    #[test]
    fn a_window_grabbed_tiled_lands_on_cells() {
        let (x, y) = move_to((0.0, 0.0), (140.0, 30.0), (200.0, 200.0), true, &sp());
        assert_eq!((x, y), snap::snap_move_tiled(140.0, 30.0, &sp()));
        let (gx, gy) = group_offset((0.0, 0.0), (140.0, 30.0), true, &sp());
        assert_eq!((gx, gy), (x, y));
    }

    #[test]
    fn a_floating_resize_stops_at_the_minimum() {
        let edges = Edges { right: true, bottom: true, ..Edges::default() };
        assert_eq!(resize_to((0.0, 0.0), (300, 200), (-1000.0, 40.0), edges, false, &sp()), (50, 240));
    }

    #[test]
    fn a_status_bar_docks_to_sockets_corners_and_edges() {
        let out = [(0.0, 0.0, 1000.0, 600.0)];
        assert_eq!(status_edge_at((500.0, 10.0), &out), Some(StatusEdge::TopCenter));
        assert_eq!(status_edge_at((70.0, 5.0), &out), Some(StatusEdge::TopLeft));
        assert_eq!(status_edge_at((990.0, 300.0), &out), Some(StatusEdge::Right));
        assert_eq!(status_edge_at((300.0, 590.0), &out), Some(StatusEdge::BottomCenter));
        // Off the side, low: the nearest side is Left, near the bottom.
        assert_eq!(status_edge_at((5.0, 520.0), &out), Some(StatusEdge::BottomLeft));
        assert_eq!(status_edge_at((5.0, 520.0), &[]), None);
        // The nearer output wins.
        let two = [(0.0, 0.0, 1000.0, 600.0), (1000.0, 0.0, 800.0, 600.0)];
        assert_eq!(status_edge_at((1790.0, 300.0), &two), Some(StatusEdge::Right));
    }

    #[test]
    fn a_docked_bar_stands_up_along_a_side() {
        assert_eq!(status_bar_size(StatusEdge::Left, 30, 800), (30, 800));
        assert_eq!(status_bar_size(StatusEdge::TopCenter, 30, 800), (800, 30));
    }
}
