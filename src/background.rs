// Background drawing policy: the per-frame geometry of the infinite desktop
// grid. `grid_frame` maps a GridSpec + camera + output extent to a concrete
// drawing plan — where the grid tree sits (the modulo shift that makes the
// grid infinite), how big the backdrop is, and whether/how the cells draw
// (density fade, safety caps, zoom-scaled extents). The mechanism owns the
// scene rects, the reuse pool, and scenefx's fade-inset wire encoding.

use crate::api::{GridSpec, Rgba};
use crate::camera::Camera;

/// A camera zoom as the background math consumes it: NaN and non-positive
/// values fall back to 1.
pub fn sanitized_zoom(zoom: f64) -> f64 {
    if zoom.is_nan() || zoom <= 0.0 { 1.0 } else { zoom }
}

/// One frame's grid drawing plan, in output-local px unless noted.
#[derive(Debug, Clone, PartialEq)]
pub struct GridFrame {
    /// Grid-tree translation in layout px (includes the output's own
    /// offset): the pan shift folded modulo one period, minus one period so
    /// the tree always overhangs the top-left edge. None when the zoomed
    /// period rounds to zero — the tree keeps its last position.
    pub tree_pos: Option<(i32, i32)>,
    /// The zoomed grid period (cell + gap), rounded to px.
    pub period_px: i32,
    /// Backdrop (gap color) extent: viewport plus one period, so pan shifts
    /// never expose the edge.
    pub backdrop_w: i32,
    pub backdrop_h: i32,
    /// None when the cells are fully density-faded, the period degenerates,
    /// or the cell count exceeds the safety caps — backdrop only.
    pub cells: Option<GridCells>,
}

/// The repeated cell lattice: draw a cell at (col * period_px,
/// row * period_px) for col in 0..=cols, row in 0..=rows, tree-local.
#[derive(Debug, Clone, PartialEq)]
pub struct GridCells {
    pub cell_px: i32,
    pub cols: i32,
    pub rows: i32,
    /// Cell color with the density fade already applied to alpha.
    pub color: Rgba,
    pub corner_radius_px: i32,
    /// Zoom-scaled fade inset, capped at 45% of the cell so cells can't
    /// blur out entirely when far zoomed out; 0 disables the fade.
    pub fade_inset_px: i32,
}

pub fn grid_frame(
    spec: &GridSpec,
    cam: Camera,
    viewport_w: i32,
    viewport_h: i32,
    output_x: i32,
    output_y: i32,
) -> GridFrame {
    let zoom = sanitized_zoom(cam.zoom);
    let cell_size = spec.cell_size.max(5.0);
    let gap = spec.gap_width.max(0.0);
    let period = cell_size + gap;

    // Fade the cells out as they shrink (period under 30 screen px) to
    // prevent visual noise and pathological cell counts when zooming.
    let period_pixels = period * zoom;
    let density_fade = if period_pixels < 15.0 {
        0.0
    } else if period_pixels < 30.0 {
        (period_pixels - 15.0) / 15.0
    } else {
        1.0
    };
    let period_px = period_pixels.round() as i32;

    // Modulo shift for infinite scrolling; .floor() matches the truncation
    // direction of window coordinates.
    let tree_pos = if period_px > 0 {
        let origin_x = ((-cam.pan_x) * zoom).floor() as i32;
        let origin_y = ((-cam.pan_y) * zoom).floor() as i32;
        Some((
            output_x + origin_x.rem_euclid(period_px) - period_px,
            output_y + origin_y.rem_euclid(period_px) - period_px,
        ))
    } else {
        None
    };

    let cells = if period_px > 0 && density_fade > 0.0 {
        let cols = (viewport_w as f64 / period_px as f64).ceil() as i32 + 1;
        let rows = (viewport_h as f64 / period_px as f64).ceil() as i32 + 1;
        let cell_px = (cell_size * zoom).round() as i32;
        if cols > 0 && rows > 0 && cols <= 1000 && rows <= 1000 && cols * rows <= 20000 && cell_px > 0 {
            let mut color = spec.cell_color;
            color.0[3] *= density_fade as f32;
            let max_inset = (cell_px as f64 * 0.45).floor() as i32;
            let fade_inset_px = ((spec.cell_fade_inset as f64 * zoom).round() as i32)
                .min(max_inset)
                .max(0);
            Some(GridCells {
                cell_px,
                cols,
                rows,
                color,
                corner_radius_px: (spec.cell_corner_radius as f64 * zoom).round() as i32,
                fade_inset_px,
            })
        } else {
            None
        }
    } else {
        None
    };

    GridFrame {
        tree_pos,
        period_px,
        backdrop_w: viewport_w + period_px,
        backdrop_h: viewport_h + period_px,
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::GridFadeMode;

    const VW: i32 = 1920;
    const VH: i32 = 1080;

    fn spec() -> GridSpec {
        GridSpec {
            gap_color: Rgba([0.0, 0.0, 0.0, 1.0]),
            cell_color: Rgba([0.05, 0.05, 0.05, 0.6]),
            cell_size: 100.0,
            gap_width: 10.0,
            cell_corner_radius: 8,
            cell_fade_inset: 4,
            fade_mode: GridFadeMode::Linear,
        }
    }

    fn cam(pan_x: f64, pan_y: f64, zoom: f64) -> Camera {
        Camera { pan_x, pan_y, zoom }
    }

    #[test]
    fn unpanned_grid_overhangs_one_period() {
        let f = grid_frame(&spec(), cam(0.0, 0.0, 1.0), VW, VH, 100, 50);
        assert_eq!(f.period_px, 110);
        assert_eq!(f.tree_pos, Some((100 - 110, 50 - 110)));
        assert_eq!((f.backdrop_w, f.backdrop_h), (VW + 110, VH + 110));
        let cells = f.cells.unwrap();
        // ceil(1920/110)+1 = 19, ceil(1080/110)+1 = 11.
        assert_eq!((cells.cols, cells.rows), (19, 11));
        assert_eq!(cells.cell_px, 100);
        assert_eq!(cells.fade_inset_px, 4);
        assert_eq!(cells.corner_radius_px, 8);
        // Full density: color untouched.
        assert_eq!(cells.color, Rgba([0.05, 0.05, 0.05, 0.6]));
    }

    #[test]
    fn pan_folds_modulo_one_period() {
        // Pan 30 right: origin -30, rem_euclid(110) = 80, minus a period.
        let f = grid_frame(&spec(), cam(30.0, 0.0, 1.0), VW, VH, 0, 0);
        assert_eq!(f.tree_pos, Some((-30, -110)));
        // A full period of pan lands back where it started.
        let g = grid_frame(&spec(), cam(140.0, 0.0, 1.0), VW, VH, 0, 0);
        assert_eq!(g.tree_pos, Some((-30, -110)));
    }

    #[test]
    fn density_fade_scales_alpha_then_kills_cells() {
        // Zoom 0.2: period 110 * 0.2 = 22 px — mid-ramp (22-15)/15.
        let f = grid_frame(&spec(), cam(0.0, 0.0, 0.2), VW, VH, 0, 0);
        let cells = f.cells.unwrap();
        assert!((cells.color.0[3] - 0.6 * (7.0 / 15.0) as f32).abs() < 1e-6);
        // Zoom 0.1: period 11 px — fully faded, backdrop only.
        let f = grid_frame(&spec(), cam(0.0, 0.0, 0.1), VW, VH, 0, 0);
        assert!(f.cells.is_none());
        assert_eq!(f.period_px, 11);
        assert!(f.tree_pos.is_some());
    }

    #[test]
    fn fade_inset_caps_at_45_percent_of_cell() {
        let mut s = spec();
        s.cell_fade_inset = 60;
        let f = grid_frame(&s, cam(0.0, 0.0, 1.0), VW, VH, 0, 0);
        assert_eq!(f.cells.unwrap().fade_inset_px, 45);
    }

    #[test]
    fn degenerate_zoom_and_period_are_safe() {
        // NaN zoom falls back to 1.
        let f = grid_frame(&spec(), cam(0.0, 0.0, f64::NAN), VW, VH, 0, 0);
        assert_eq!(f.period_px, 110);
        assert!(f.cells.is_some());
        // A period that rounds to zero: no tree move, no cells, backdrop
        // stays viewport-sized.
        let f = grid_frame(&spec(), cam(0.0, 0.0, 0.001), VW, VH, 0, 0);
        assert_eq!(f.period_px, 0);
        assert!(f.tree_pos.is_none());
        assert!(f.cells.is_none());
        assert_eq!((f.backdrop_w, f.backdrop_h), (VW, VH));
    }
}
