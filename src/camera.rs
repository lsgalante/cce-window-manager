// Viewport camera policy: the pan/zoom math behind zoom actions, wheel
// zoom, viewport jumps, overview fit, and focus-follow panning.
//
// The desktop camera is (pan_x, pan_y, zoom): a virtual point v appears on
// an output at `(v - pan) * zoom` output-local px, so the viewport shows the
// virtual rect [pan, pan + extent/zoom). Every function here is a pure map
// from one camera to another — the mechanism owns the actual fields (and the
// animation easing toward targets) and applies the results.

/// Camera state, by value. Mechanism copies `desk_pan_x/y`/`desk_zoom` in,
/// writes the result back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
}

pub const ZOOM_MIN: f64 = 0.1;
pub const ZOOM_MAX: f64 = 10.0;
/// Multiplier per keyed ZoomIn/ZoomOut press.
pub const KEYED_ZOOM_STEP: f64 = 1.1;
/// Per-unit wheel-delta zoom base: factor = WHEEL_ZOOM_BASE^(-delta).
pub const WHEEL_ZOOM_BASE: f64 = 1.005;
/// Zoom ≠ 1 within this tolerance still counts as "normal" (not overview).
const OVERVIEW_EPSILON: f64 = 0.001;

/// Overview mode is simply "the camera is zoomed": any zoom meaningfully
/// away from 1.
pub fn is_overview(zoom: f64) -> bool {
    (zoom - 1.0).abs() > OVERVIEW_EPSILON
}

/// One keyed zoom press. `dir` > 0 zooms in, < 0 out, 0 resets to 1.
pub fn keyed_zoom(zoom: f64, dir: f64) -> f64 {
    if dir > 0.0 {
        (zoom * KEYED_ZOOM_STEP).min(ZOOM_MAX)
    } else if dir < 0.0 {
        (zoom / KEYED_ZOOM_STEP).max(ZOOM_MIN)
    } else {
        1.0
    }
}

/// Continuous wheel zoom: scroll up (negative delta) zooms in.
pub fn wheel_zoom(zoom: f64, delta: f64) -> f64 {
    (zoom * WHEEL_ZOOM_BASE.powf(-delta)).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Continuous pinch zoom. libinput reports `scale` as the absolute finger
/// spread relative to the gesture's begin (not a per-event delta), so the
/// whole gesture maps off the zoom captured at pinch begin — never the
/// current zoom, which would compound every update into runaway growth.
pub fn pinch_zoom(start_zoom: f64, scale: f64) -> f64 {
    (start_zoom * scale).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Change zoom while keeping the virtual point under an output-local anchor
/// (`ax`, `ay` px from the output's top-left) fixed on screen — the wheel
/// zooms about the cursor, keyed zooms about the viewport center.
pub fn zoom_about_anchor(cam: Camera, ax: f64, ay: f64, new_zoom: f64) -> Camera {
    let new_zoom = new_zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    Camera {
        pan_x: cam.pan_x + ax * (1.0 / cam.zoom - 1.0 / new_zoom),
        pan_y: cam.pan_y + ay * (1.0 / cam.zoom - 1.0 / new_zoom),
        zoom: new_zoom,
    }
}

/// The camera that centers virtual point (`cx`, `cy`) in a viewport of
/// `vw` x `vh` output px at the given zoom.
pub fn center_on(cx: f64, cy: f64, vw: f64, vh: f64, zoom: f64) -> Camera {
    Camera {
        pan_x: cx - (vw / 2.0) / zoom,
        pan_y: cy - (vh / 2.0) / zoom,
        zoom,
    }
}

/// Virtual top-left that puts a `win`-long window in the middle of the
/// viewport, on one axis. The mirror of [`center_on`]: that moves the camera
/// to a window, this moves a window to the camera.
///
/// The viewport shows `[pan, pan + extent/zoom)`, so its virtual midpoint is
/// `pan + extent/(2*zoom)` and the window starts half its own length before
/// it. `extent` is the output's length in px; `win` is virtual (unscaled),
/// because a window's stored geometry is virtual and zoom is applied when it
/// is drawn.
///
/// Session modals place with this so they open where the user is currently
/// looking rather than wherever they last sat — on a panning desktop a
/// remembered position is usually off-view by the time the window reopens.
pub fn centered_window_origin(pan: f64, extent: f64, zoom: f64, win: f64) -> f64 {
    pan + (extent / zoom - win) / 2.0
}

/// Anchor-stable zoom-pan interpolation between two cameras at progress
/// `p` ∈ [0, 1]: zoom log-lerps, and pan is DERIVED from the unique world
/// point that maps to the same screen position under both cameras — so the
/// whole transition reads as a single zoom about a stationary anchor
/// instead of a sideways slide-while-zooming (independent pan/zoom lerp
/// keeps no point fixed; every pixel bows along a curve). Endpoints are
/// exact. Near-equal zooms have no anchor (it runs to infinity), so that
/// case degrades to a straight pan at constant zoom.
pub fn anchored_interp(start: Camera, end: Camera, p: f64) -> Camera {
    let z0 = start.zoom.max(1e-9);
    let z1 = end.zoom.max(1e-9);
    let zoom = (z0.ln() + (z1.ln() - z0.ln()) * p).exp();
    if (z1 / z0).ln().abs() < 1e-6 {
        return Camera {
            pan_x: start.pan_x + (end.pan_x - start.pan_x) * p,
            pan_y: start.pan_y + (end.pan_y - start.pan_y) * p,
            zoom,
        };
    }
    // Per axis: the fixed point q solves (q - pan0)·z0 = (q - pan1)·z1;
    // its constant screen coordinate is a = (q - pan0)·z0, and the pan at
    // any zoom follows from holding q at a.
    let axis = |pan0: f64, pan1: f64| -> f64 {
        let q = (pan0 * z0 - pan1 * z1) / (z0 - z1);
        let a = (q - pan0) * z0;
        q - a / zoom
    };
    Camera {
        pan_x: axis(start.pan_x, end.pan_x),
        pan_y: axis(start.pan_y, end.pan_y),
        zoom,
    }
}

/// Fraction of a virtual-space window rect visible in the viewport, 0.0–1.0.
///
/// Its one caller is [`recalled_origin`], which restores a remembered
/// floating window where it was only if at least [`RESTORE_VISIBLE_MIN`] of
/// it would show. It fed the focus-follow decision too until 2026-09-12,
/// when [`pan_into_view`] replaced "below a threshold, centre it" with the
/// minimal pan that brings a window fully into view — focus asks how far a
/// window is out of view, not how much of it is in.
pub fn visible_fraction(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cam: Camera,
    vw: f64,
    vh: f64,
) -> f64 {
    if w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let v_right = cam.pan_x + vw / cam.zoom;
    let v_bottom = cam.pan_y + vh / cam.zoom;
    let i_w = (x + w).min(v_right) - x.max(cam.pan_x);
    let i_h = (y + h).min(v_bottom) - y.max(cam.pan_y);
    (i_w.max(0.0) * i_h.max(0.0)) / (w * h)
}

/// Breathing room a window lands with when the camera moves for it, output
/// px — enough for the hover/border band, so the grab surface comes along
/// with the content.
const VIEW_MARGIN: f64 = 24.0;

/// The camera a focus change moves to: the MINIMAL pan that brings a window
/// fully into view, or `None` when it already is (a window parked exactly
/// flush at an edge is left alone — only a window actually crossing the
/// viewport bound moves the camera). Each axis is handled independently;
/// the corrected edge lands `VIEW_MARGIN` in from the viewport. A window too
/// large to fit prioritizes its top-left edge.
///
/// This is the whole focus-follow rule, for a window half off the edge and
/// for one a screen away alike. It used to apply only to a window already
/// three-quarters visible, and anything less got centered — so focusing the
/// window immediately to the right swung the desktop over and parked it in
/// the middle, throwing away the spatial relationship the user had just
/// navigated by. The camera should move as little as the request demands:
/// the window arrives at the edge it was behind, and everything else on
/// screen stays where the eye left it.
pub fn pan_into_view(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cam: Camera,
    vw: f64,
    vh: f64,
) -> Option<Camera> {
    // Screen-space rect of the window under the current camera.
    let l = (x - cam.pan_x) * cam.zoom;
    let t = (y - cam.pan_y) * cam.zoom;
    let r = l + w * cam.zoom;
    let b = t + h * cam.zoom;

    // Per axis: the screen-px shift applied to the WINDOW (camera moves the
    // opposite way). Nothing happens unless the window actually crosses the
    // viewport bounds on that axis.
    let axis_shift = |low: f64, high: f64, extent: f64| -> f64 {
        let mut d = 0.0;
        if high > extent {
            d = (extent - VIEW_MARGIN) - high;
        }
        if low + d < 0.0 {
            // Clipped low (or over-corrected by the high fix / oversized
            // window): top-left priority.
            d = VIEW_MARGIN - low;
        }
        d
    };
    let dx = axis_shift(l, r, vw);
    let dy = axis_shift(t, b, vh);

    if dx == 0.0 && dy == 0.0 {
        return None;
    }
    Some(Camera {
        pan_x: cam.pan_x - dx / cam.zoom,
        pan_y: cam.pan_y - dy / cam.zoom,
        zoom: cam.zoom,
    })
}

/// A remembered floating window whose position would show LESS than this
/// fraction of it is recalled into view instead of restored where it was.
pub const RESTORE_VISIBLE_MIN: f64 = 0.25;

/// Whether a remembered window rect is ON THE DESK: it overlaps the tiled
/// windows' bounding box inflated by one viewport on every side (virtual
/// units, so at zoom 1 a viewport is `vw` x `vh`). `None` for the desk means
/// there are no tiled windows to be beside, and nothing is on the desk.
///
/// A floating window parked beside a tiled column, or one screen past the
/// desk's edge, is at most one pan away from content the user navigates
/// by — it is placed, not lost. Only a window with no tiled neighbour
/// within a screen has nothing on the desk to say where it is.
pub fn on_tiled_desk(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    desk: Option<(f64, f64, f64, f64)>,
    vw: f64,
    vh: f64,
) -> bool {
    let Some((min_x, min_y, max_x, max_y)) = desk else { return false };
    if w <= 0.0 || h <= 0.0 || max_x <= min_x || max_y <= min_y {
        return false;
    }
    x < max_x + vw && x + w > min_x - vw && y < max_y + vh && y + h > min_y - vh
}

/// Where a remembered FLOATING window should reopen: `None` to keep its
/// remembered origin, or the origin that centers it in the current view.
///
/// On a panning desktop a remembered position is often off-view by the
/// time the window reopens — the camera was somewhere else when the
/// session was saved, or has moved since. Tiled windows are part of the
/// grid and belong wherever the grid puts them, so this is for floating
/// windows only: an Inkscape start screen restored a screen above the
/// viewport is not "remembered", it is lost, with nothing on screen to say
/// it exists. A window that would still be mostly visible keeps its spot —
/// a floating window deliberately tucked at an edge stays tucked.
///
/// So does a window ON THE DESK (`on_tiled_desk` against `desk`, the tiled
/// windows' bounding box): a data editor parked beside the leftmost tiled
/// column was recalled into the middle of the view every login because the
/// camera had been left two screens to the right at logout. Off-view is
/// not lost when the tiled desk is right there to pan along; the recall is
/// for a window with no neighbour at all.
pub fn recalled_origin(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    cam: Camera,
    vw: f64,
    vh: f64,
    desk: Option<(f64, f64, f64, f64)>,
) -> Option<(f64, f64)> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    if visible_fraction(x, y, w, h, cam, vw, vh) >= RESTORE_VISIBLE_MIN {
        return None;
    }
    if on_tiled_desk(x, y, w, h, desk, vw, vh) {
        return None;
    }
    let zoom = cam.zoom.max(0.01);
    Some((
        centered_window_origin(cam.pan_x, vw, zoom, w),
        centered_window_origin(cam.pan_y, vh, zoom, h),
    ))
}

/// Margin kept around the fitted bounds when entering overview, output px.
const OVERVIEW_MARGIN: f64 = 100.0;
/// The margin never shrinks the usable viewport below this, output px.
const OVERVIEW_MIN_AVAIL: f64 = 200.0;
/// Overview fit only zooms OUT (cap 1.0), and never further than this.
const OVERVIEW_ZOOM_MIN: f64 = 0.05;

/// Entering overview: fit the virtual bounding box [min_x, max_x] x
/// [min_y, max_y] into the viewport with a margin, centered. Zoom is capped
/// at 1 — a desktop smaller than the screen is centered, not magnified.
pub fn fit_bounds(
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    vw: f64,
    vh: f64,
) -> Camera {
    let box_w = max_x - min_x;
    let box_h = max_y - min_y;
    let avail_w = (vw - 2.0 * OVERVIEW_MARGIN).max(OVERVIEW_MIN_AVAIL);
    let avail_h = (vh - 2.0 * OVERVIEW_MARGIN).max(OVERVIEW_MIN_AVAIL);
    let zoom = (avail_w / box_w.max(1.0))
        .min(avail_h / box_h.max(1.0))
        .min(1.0)
        .max(OVERVIEW_ZOOM_MIN);
    center_on(min_x + box_w / 2.0, min_y + box_h / 2.0, vw, vh, zoom)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VW: f64 = 1920.0;
    const VH: f64 = 1080.0;

    fn cam(pan_x: f64, pan_y: f64, zoom: f64) -> Camera {
        Camera { pan_x, pan_y, zoom }
    }

    #[test]
    fn a_remembered_floating_window_off_view_is_recalled_to_center() {
        // Camera at (-4708, -3196), zoom 1: the view spans y -3196..-2116.
        // A 700x666 window remembered at y=-4422 ends at -3756 — a whole
        // screen above. It comes back centered in the view.
        let c = cam(-4708.0, -3196.0, 1.0);
        let got = recalled_origin(-3272.0, -4422.0, 700.0, 666.0, c, VW, VH, None);
        assert_eq!(got, Some((-4708.0 + (VW - 700.0) / 2.0, -3196.0 + (VH - 666.0) / 2.0)));
    }

    #[test]
    fn a_remembered_window_beside_the_tiled_desk_keeps_its_spot() {
        // The 2026-09-14 login, at output scale 2 (1920x1200 logical):
        // camera (-5928, -3244), the data editor 952x904 remembered at
        // (-8461, -3564) — two and a half screens left, 0% visible — and
        // the leftmost tiled column at x=-7380, 129 px to its right.
        let (vw, vh) = (1920.0, 1200.0);
        let c = cam(-5928.0, -3244.0, 1.0);
        let desk = Some((-7380.0, -4380.0, -2984.0, -2076.0));
        assert_eq!(recalled_origin(-8461.0, -3564.0, 952.0, 904.0, c, vw, vh, desk), None);
        // Without a tiled desk the same window is lost, and recalled.
        assert!(recalled_origin(-8461.0, -3564.0, 952.0, 904.0, c, vw, vh, None).is_some());
        // More than a viewport past the desk's edge: nothing to be beside.
        assert!(recalled_origin(-7380.0 - vw - 952.0 - 1.0, -3564.0, 952.0, 904.0, c, vw, vh, desk).is_some());
        // Exactly one viewport past still counts — the pan that reaches
        // the desk's edge shows it.
        assert_eq!(recalled_origin(-7380.0 - vw - 952.0 + 1.0, -3564.0, 952.0, 904.0, c, vw, vh, desk), None);
    }

    #[test]
    fn on_tiled_desk_is_the_inflated_bounding_box() {
        let desk = Some((0.0, 0.0, 1000.0, 1000.0));
        // Inside, overlapping, and within a viewport of every side.
        assert!(on_tiled_desk(100.0, 100.0, 200.0, 200.0, desk, VW, VH));
        assert!(on_tiled_desk(-VW - 100.0, 0.0, 200.0, 200.0, desk, VW, VH));
        assert!(on_tiled_desk(0.0, 1000.0 + VH - 1.0, 200.0, 200.0, desk, VW, VH));
        // Past the inflated box on either axis.
        assert!(!on_tiled_desk(-VW - 200.0, 0.0, 200.0, 200.0, desk, VW, VH));
        assert!(!on_tiled_desk(0.0, 1000.0 + VH, 200.0, 200.0, desk, VW, VH));
        // No desk, a sizeless window, or a degenerate desk: never on it.
        assert!(!on_tiled_desk(100.0, 100.0, 200.0, 200.0, None, VW, VH));
        assert!(!on_tiled_desk(100.0, 100.0, 0.0, 0.0, desk, VW, VH));
        assert!(!on_tiled_desk(100.0, 100.0, 200.0, 200.0, Some((5.0, 5.0, 5.0, 5.0)), VW, VH));
    }

    #[test]
    fn a_remembered_window_mostly_in_view_keeps_its_spot() {
        let c = cam(0.0, 0.0, 1.0);
        // Fully visible.
        assert_eq!(recalled_origin(100.0, 100.0, 700.0, 666.0, c, VW, VH, None), None);
        // Half off the right edge: 50% visible, above the quarter floor.
        assert_eq!(recalled_origin(VW - 350.0, 100.0, 700.0, 666.0, c, VW, VH, None), None);
        // Only a sliver (10%) on screen: recalled.
        assert!(recalled_origin(VW - 70.0, 100.0, 700.0, 666.0, c, VW, VH, None).is_some());
    }

    #[test]
    fn recall_centers_under_the_current_zoom() {
        // Zoomed out to 0.5 the view covers twice the virtual extent.
        let c = cam(1000.0, 1000.0, 0.5);
        let got = recalled_origin(-9000.0, -9000.0, 400.0, 300.0, c, VW, VH, None);
        assert_eq!(got, Some((1000.0 + (VW / 0.5 - 400.0) / 2.0, 1000.0 + (VH / 0.5 - 300.0) / 2.0)));
        // A sizeless window has nothing to place.
        assert_eq!(recalled_origin(-9000.0, -9000.0, 0.0, 0.0, c, VW, VH, None), None);
    }

    #[test]
    fn overview_is_any_meaningful_zoom() {
        assert!(!is_overview(1.0));
        assert!(!is_overview(1.0005));
        assert!(is_overview(1.1));
        assert!(is_overview(0.5));
    }

    #[test]
    fn keyed_zoom_steps_and_clamps() {
        assert_eq!(keyed_zoom(1.0, 1.0), 1.1);
        assert_eq!(keyed_zoom(1.1, -1.0), 1.0);
        assert_eq!(keyed_zoom(9.99, 1.0), ZOOM_MAX);
        assert_eq!(keyed_zoom(0.10001, -1.0), ZOOM_MIN);
        assert_eq!(keyed_zoom(3.7, 0.0), 1.0);
    }

    #[test]
    fn centered_window_origin_puts_the_window_mid_viewport() {
        // Zoom 1: a 640-wide window in a 1920 viewport starts 640 in, and
        // the whole thing shifts with the pan.
        assert_eq!(centered_window_origin(0.0, VW, 1.0, 640.0), 640.0);
        assert_eq!(centered_window_origin(5000.0, VW, 1.0, 640.0), 5640.0);
        // Vertical axis is the same call.
        assert_eq!(centered_window_origin(0.0, VH, 1.0, 400.0), 340.0);

        // Zoomed out to 0.5 the viewport covers 3840 virtual px, so the same
        // window centers further from the pan origin — the point of dividing
        // the extent by zoom rather than scaling the window.
        assert_eq!(centered_window_origin(0.0, VW, 0.5, 640.0), 1600.0);
        // Zoomed in 2x it covers only 960, so the window sits nearer.
        assert_eq!(centered_window_origin(0.0, VW, 2.0, 640.0), 160.0);

        // Round-trip against the projection the module documents:
        // screen = (virtual - pan) * zoom. The window's screen midpoint must
        // land on the viewport's screen midpoint at any camera.
        for &(pan, zoom, win) in &[(0.0, 1.0, 640.0), (1234.5, 0.75, 500.0), (-800.0, 1.6, 900.0)] {
            let v = centered_window_origin(pan, VW, zoom, win);
            let screen_mid = (v - pan) * zoom + (win * zoom) / 2.0;
            assert!((screen_mid - VW / 2.0).abs() < 1e-9, "pan={pan} zoom={zoom}");
        }

        // A window wider than the viewport overhangs symmetrically (negative
        // origin) rather than being clamped — centering, not fitting.
        assert_eq!(centered_window_origin(0.0, VW, 1.0, 2920.0), -500.0);
    }

    #[test]
    fn pinch_zoom_maps_off_the_begin_zoom_and_clamps() {
        // Absolute-scale semantics: spreading to 2x from zoom 1.5 lands on
        // 3.0 no matter how many intermediate updates arrived.
        assert_eq!(pinch_zoom(1.5, 2.0), 3.0);
        assert_eq!(pinch_zoom(1.5, 1.0), 1.5); // begin-scale identity
        assert_eq!(pinch_zoom(1.0, 0.5), 0.5);
        // Clamped at both ends.
        assert_eq!(pinch_zoom(8.0, 4.0), ZOOM_MAX);
        assert_eq!(pinch_zoom(0.4, 0.1), ZOOM_MIN);
    }

    #[test]
    fn zoom_about_anchor_pins_the_anchored_point() {
        // Virtual point under the anchor before == after. Anchor (960, 540),
        // camera (100, 50, 1): virtual point = pan + anchor/zoom.
        let c0 = cam(100.0, 50.0, 1.0);
        let (ax, ay) = (960.0, 540.0);
        let before = (c0.pan_x + ax / c0.zoom, c0.pan_y + ay / c0.zoom);
        let c1 = zoom_about_anchor(c0, ax, ay, 2.0);
        let after = (c1.pan_x + ax / c1.zoom, c1.pan_y + ay / c1.zoom);
        assert!((before.0 - after.0).abs() < 1e-9);
        assert!((before.1 - after.1).abs() < 1e-9);
        assert_eq!(c1.zoom, 2.0);
    }

    #[test]
    fn center_on_round_trips_through_visibility() {
        // A 400x300 window centered by center_on is fully visible.
        let c = center_on(200.0, 150.0, VW, VH, 1.0);
        assert_eq!(visible_fraction(0.0, 0.0, 400.0, 300.0, c, VW, VH), 1.0);
    }

    #[test]
    fn visible_fraction_partial_and_none() {
        // Viewport [0,1920)x[0,1080): a 200-wide window half off the left
        // edge is half visible; one fully outside is 0.
        let c = cam(0.0, 0.0, 1.0);
        assert_eq!(visible_fraction(-100.0, 0.0, 200.0, 100.0, c, VW, VH), 0.5);
        assert_eq!(visible_fraction(-500.0, 0.0, 200.0, 100.0, c, VW, VH), 0.0);
        // Zoom 2 halves the visible virtual extent: a window spanning
        // [0, 1920) virtual is only half on screen.
        let z = cam(0.0, 0.0, 2.0);
        assert_eq!(visible_fraction(0.0, 0.0, 1920.0, 100.0, z, VW, VH), 0.5);
    }

    #[test]
    fn fit_bounds_fits_and_centers() {
        // 3440x1880 bounds into 1920x1080: avail 1720x880, zoom limited by
        // height 880/1880; the bounds' center lands at the viewport center.
        let c = fit_bounds(0.0, 0.0, 3440.0, 1880.0, VW, VH);
        assert!((c.zoom - 880.0 / 1880.0).abs() < 1e-9);
        assert!((c.pan_x + (VW / 2.0) / c.zoom - 1720.0).abs() < 1e-9);
        // Tiny bounds: zoom caps at 1, no magnification.
        let c = fit_bounds(0.0, 0.0, 100.0, 100.0, VW, VH);
        assert_eq!(c.zoom, 1.0);
    }

    #[test]
    fn anchored_interp_endpoints_are_exact() {
        let s = cam(100.0, 50.0, 1.0);
        let e = cam(-400.0, -90.0, 0.5);
        let a0 = anchored_interp(s, e, 0.0);
        let a1 = anchored_interp(s, e, 1.0);
        assert!((a0.pan_x - s.pan_x).abs() < 1e-9 && (a0.zoom - s.zoom).abs() < 1e-12);
        assert!((a1.pan_x - e.pan_x).abs() < 1e-6 && (a1.pan_y - e.pan_y).abs() < 1e-6);
        assert!((a1.zoom - e.zoom).abs() < 1e-9);
    }

    #[test]
    fn anchored_interp_keeps_the_fixed_point_stationary() {
        let s = cam(200.0, -80.0, 1.0);
        let e = cam(-350.0, 140.0, 0.4);
        // The per-axis fixed point and its screen coordinate under start.
        let qx = (s.pan_x * s.zoom - e.pan_x * e.zoom) / (s.zoom - e.zoom);
        let qy = (s.pan_y * s.zoom - e.pan_y * e.zoom) / (s.zoom - e.zoom);
        let ax = (qx - s.pan_x) * s.zoom;
        let ay = (qy - s.pan_y) * s.zoom;
        for i in 0..=10 {
            let c = anchored_interp(s, e, i as f64 / 10.0);
            assert!(((qx - c.pan_x) * c.zoom - ax).abs() < 1e-6, "p={}", i);
            assert!(((qy - c.pan_y) * c.zoom - ay).abs() < 1e-6, "p={}", i);
        }
    }

    #[test]
    fn anchored_interp_zoom_about_viewport_center_stays_centered() {
        // start/end share their viewport center: the anchor IS that center,
        // which must stay put the whole way (1920x1080 viewport).
        let s = cam(0.0, 0.0, 1.0);
        let cx = 960.0;
        let cy = 540.0;
        let e = center_on(cx, cy, 1920.0, 1080.0, 0.5);
        for i in 0..=10 {
            let c = anchored_interp(s, e, i as f64 / 10.0);
            let sx = (cx - c.pan_x) * c.zoom;
            let sy = (cy - c.pan_y) * c.zoom;
            assert!((sx - 960.0).abs() < 1e-6 && (sy - 540.0).abs() < 1e-6, "p={}", i);
        }
    }

    #[test]
    fn anchored_interp_equal_zoom_is_straight_pan() {
        let s = cam(0.0, 0.0, 1.0);
        let e = cam(500.0, -300.0, 1.0);
        let c = anchored_interp(s, e, 0.5);
        assert!((c.pan_x - 250.0).abs() < 1e-9 && (c.pan_y + 150.0).abs() < 1e-9);
        assert_eq!(c.zoom, 1.0);
    }

    #[test]
    fn pan_leaves_fully_visible_windows_alone() {
        let c = cam(0.0, 0.0, 1.0);
        // Comfortably inside, and flush at the origin edge: both untouched.
        assert!(pan_into_view(500.0, 300.0, 400.0, 300.0, c, VW, VH).is_none());
        assert!(pan_into_view(0.0, 0.0, 400.0, 300.0, c, VW, VH).is_none());
    }

    #[test]
    fn pan_slides_clipped_bottom_edge_on_screen() {
        // 1080-tall viewport; a 300-tall window at y=900 hangs 120px off the
        // bottom. The pan shifts the camera down so the bottom lands 24px in:
        // window bottom 1200 → 1056, a pan_y increase of 144.
        let c = cam(0.0, 0.0, 1.0);
        let n = pan_into_view(100.0, 900.0, 400.0, 300.0, c, VW, VH).unwrap();
        assert_eq!(n.pan_x, 0.0);
        assert_eq!(n.pan_y, 144.0);
    }

    #[test]
    fn pan_left_clip_lands_with_margin() {
        // Window 80px off the left edge: lands at screen x = 24.
        let c = cam(0.0, 0.0, 1.0);
        let n = pan_into_view(-80.0, 100.0, 400.0, 300.0, c, VW, VH).unwrap();
        assert_eq!(n.pan_x, -104.0);
        assert_eq!(n.pan_y, 0.0);
    }

    #[test]
    fn pan_oversized_window_prefers_top_left() {
        // Taller than the viewport and clipped both ways: the top edge wins,
        // landing at margin.
        let c = cam(0.0, 0.0, 1.0);
        let n = pan_into_view(100.0, -50.0, 400.0, 2000.0, c, VW, VH).unwrap();
        assert_eq!(n.pan_y, -74.0);
    }

    #[test]
    fn a_window_fully_offscreen_to_the_right_is_brought_to_the_near_edge() {
        // The reported case: the focused window fills the view and the next
        // one sits entirely off the right edge. Focusing it must pan just
        // far enough to show it — NOT center it.
        let c = cam(0.0, 0.0, 1.0);
        let n = pan_into_view(2000.0, 100.0, 400.0, 300.0, c, VW, VH).unwrap();
        // Its right edge (2400) lands VIEW_MARGIN in from the 1920 viewport:
        // a pan of 2400 - (1920 - 24) = 504.
        assert_eq!(n.pan_x, 504.0);
        assert_eq!(n.pan_y, 0.0);
        // Fully visible afterwards, and hard against the edge it came from:
        // centering would have put it at pan_x = 2200 - 960 = 1240.
        let moved = cam(n.pan_x, n.pan_y, 1.0);
        assert_eq!(visible_fraction(2000.0, 100.0, 400.0, 300.0, moved, VW, VH), 1.0);
        assert!(n.pan_x < 1240.0, "minimal pan, not a recentre");
    }

    #[test]
    fn a_window_fully_offscreen_to_the_left_lands_at_the_left_margin() {
        // The mirror: its left edge lands VIEW_MARGIN in, so the camera
        // stops as soon as the window is whole.
        let c = cam(0.0, 0.0, 1.0);
        let n = pan_into_view(-900.0, 100.0, 400.0, 300.0, c, VW, VH).unwrap();
        assert_eq!(n.pan_x, -924.0);
        let moved = cam(n.pan_x, n.pan_y, 1.0);
        assert_eq!(visible_fraction(-900.0, 100.0, 400.0, 300.0, moved, VW, VH), 1.0);
    }

    #[test]
    fn a_distant_window_moves_the_camera_no_further_than_it_must() {
        // Two windows the same size, one twice as far away: the camera moves
        // exactly the extra distance, not to two different centres.
        let c = cam(0.0, 0.0, 1.0);
        let near = pan_into_view(2000.0, 0.0, 400.0, 300.0, c, VW, VH).unwrap();
        let far = pan_into_view(3000.0, 0.0, 400.0, 300.0, c, VW, VH).unwrap();
        assert_eq!(far.pan_x - near.pan_x, 1000.0);
    }

    #[test]
    fn pan_respects_zoom() {
        // At zoom 0.5, a window at virtual x=3900 (screen 1950) pokes 30px
        // past the 1920 edge... screen shift -54 → pan shift +108 virtual.
        let c = cam(0.0, 0.0, 0.5);
        let n = pan_into_view(3700.0, 100.0, 200.0, 200.0, c, VW, VH).unwrap();
        // screen right = (3700-0)*0.5 + 200*0.5 = 1950; overhang 30 + 24 margin.
        assert!((n.pan_x - 108.0).abs() < 1e-9);
    }
}
