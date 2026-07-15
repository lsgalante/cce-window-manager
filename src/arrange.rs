// Pure layout computation extracted from `WindowManager::arrange_views()`.
//
// Functions here take plain-data snapshots and return placement plans; the
// mechanism side (`window_manager.rs`) builds the snapshots from FFI state and
// applies the plans to the scene graph. `arrange()` is the whole frame in one
// call — the future `Policy::arrange` entry point — composed from the
// per-section functions below it.

use super::api::{DecorationSpec, Rect, WindowRole};
use super::tiling::TilingMode;

/// Which screen edge/region a status-bar window docks to.
/// Set from the app_id suffix or config; `Unspecified` resolves to `TopLeft`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEdge {
    Unspecified,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    Left,
    Right,
}

/// Snapshot of one status-bar window, in `self.windows` iteration order.
/// Windows being interactively dragged are excluded before layout.
pub struct StatusBarItem {
    pub app_id: String,
    pub edge: StatusEdge,
    /// max(box_geom.width, box_geom.height) — the bar's previous major length.
    pub prev_len: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatusBarPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub struct StatusBarLayoutParams {
    /// The output's layout box.
    pub output: Rect,
    pub bar_height: u32,
    pub hide_mode: bool,
    /// Pixels of bar left peeking when hide_mode pushes top bars offscreen.
    pub hide_mode_preview: i32,
}

/// The usable (tileable) area of an output: the layout box, shrunk by the
/// layer-shell non-exclusive area and by one bar-height per screen edge that
/// has a status bar docked to it. Top bars reserve no space in hide mode.
///
/// `non_exclusive` is relative to the output box; a zero-sized rect means no
/// layer-shell exclusion. `status_edges` holds the raw edge of every live
/// status-bar window (`Unspecified` resolves to `TopLeft`).
pub fn compute_usable_area(
    output: Rect,
    non_exclusive: Rect,
    bar_height: i32,
    status_hide_mode: bool,
    status_edges: &[StatusEdge],
) -> Rect {
    let mut usable = output;

    if non_exclusive.width > 0 && non_exclusive.height > 0 {
        usable.x = output.x + non_exclusive.x;
        usable.y = output.y + non_exclusive.y;
        usable.width = non_exclusive.width;
        usable.height = non_exclusive.height;
    }

    let mut has_top = false;
    let mut has_bottom = false;
    let mut has_left = false;
    let mut has_right = false;

    for &edge in status_edges {
        let edge = if edge == StatusEdge::Unspecified { StatusEdge::TopLeft } else { edge };
        match edge {
            StatusEdge::Unspecified | StatusEdge::TopLeft | StatusEdge::TopCenter | StatusEdge::TopRight => {
                if !status_hide_mode {
                    has_top = true;
                }
            }
            StatusEdge::BottomLeft | StatusEdge::BottomCenter | StatusEdge::BottomRight => {
                has_bottom = true;
            }
            StatusEdge::Left => {
                has_left = true;
            }
            StatusEdge::Right => {
                has_right = true;
            }
        }
    }

    if has_top {
        usable.y += bar_height;
        usable.height -= bar_height;
    }
    if has_bottom {
        usable.height -= bar_height;
    }
    if has_left {
        usable.x += bar_height;
        usable.width -= bar_height;
    }
    if has_right {
        usable.width -= bar_height;
    }

    usable
}

/// How `arrange_views` treats a window this frame. `Background`/`StatusBar`
/// windows get fixed geometry regardless of visibility; `Hidden` windows are
/// disabled in the scene; `Overlay` windows get the overlay slot unless they
/// are mid-drag (then they arrange as `Normal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowClass {
    Background,
    StatusBar,
    Hidden,
    Overlay,
    Normal,
}

pub fn classify_window(
    role: WindowRole,
    minimized: bool,
    closing_or_init: bool,
    mode: TilingMode,
    is_moving: bool,
) -> WindowClass {
    match role {
        WindowRole::Background => WindowClass::Background,
        WindowRole::StatusBar => WindowClass::StatusBar,
        _ => {
            if minimized || closing_or_init {
                WindowClass::Hidden
            } else if mode == TilingMode::Overlay && !is_moving {
                WindowClass::Overlay
            } else {
                WindowClass::Normal
            }
        }
    }
}

/// Overlay windows keep 16px clear above for decorations.
const OVERLAY_DEC_H: i32 = 16;

/// Windows further than this many pixels outside the output are culled.
const OFFSCREEN_MARGIN: f64 = 50.0;

pub const OVERLAY_UNFOCUSED_OPACITY: f32 = 0.85;
pub const NORMAL_UNFOCUSED_OPACITY: f32 = 0.90;

pub fn window_opacity(is_focused: bool, opacity_enabled: bool, unfocused: f32) -> f32 {
    if is_focused || !opacity_enabled { 1.0 } else { unfocused }
}

/// Per-output placement context: the physical box, the usable area from
/// `compute_usable_area`, and the desktop viewport (pan/zoom).
pub struct PlacementCtx {
    pub phys: Rect,
    pub usable: Rect,
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
}

impl PlacementCtx {
    fn virtual_to_screen(&self, vx: f64, vy: f64) -> (i32, i32) {
        (
            self.phys.x + ((vx - self.pan_x) * self.zoom) as i32,
            self.phys.y + ((vy - self.pan_y) * self.zoom) as i32,
        )
    }

    fn is_offscreen(&self, x: i32, y: i32, scaled_w: f64, scaled_h: f64) -> bool {
        let viewport_w = self.phys.width as f64;
        let viewport_h = self.phys.height as f64;
        (x as f64 + scaled_w + OFFSCREEN_MARGIN) < self.phys.x as f64
            || (x as f64 - OFFSCREEN_MARGIN) > (self.phys.x as f64 + viewport_w)
            || (y as f64 + scaled_h + OFFSCREEN_MARGIN) < self.phys.y as f64
            || (y as f64 - OFFSCREEN_MARGIN) > (self.phys.y as f64 + viewport_h)
    }
}

pub struct OverlaySnapshot {
    pub box_geom: Rect,
    pub min_width: i32,
    pub is_cloud: bool,
    pub ssd: bool,
    pub decorations_size: (i32, i32),
}

pub struct OverlayParams {
    pub overlay_width: i32,
    pub border_gap: i32,
    /// Server-side border width; the fresh overlay slot is border-inclusive.
    pub border_width: i32,
    pub position_right: bool,
    pub cloud_position_default: Option<[i32; 2]>,
}

pub struct OverlayPlacement {
    pub pos: (i32, i32),
    /// Persistent geometry to store back on the window, if placement chose it.
    pub box_geom_write: Option<Rect>,
    pub virtual_pos: (f64, f64),
    pub size: (u32, u32),
}

/// Place the primary overlay window: fresh windows get the configured overlay
/// slot (left or right edge, full usable height); cloud windows snap to their
/// configured default position; anything else keeps its stored geometry.
///
/// The fresh slot is border-inclusive: since borders draw outside the content
/// box, the content is inset by the border width so slot + border stays inside
/// the configured gaps. Stored geometry is treated like a floating window (the
/// border extends beyond it).
pub fn place_overlay_window(
    snap: &OverlaySnapshot,
    p: &OverlayParams,
    ctx: &PlacementCtx,
) -> OverlayPlacement {
    let bw = p.border_width.max(0);
    let g = p.border_gap;
    let dec_h = std::cmp::max(bw, OVERLAY_DEC_H);

    let mut sp_x = snap.box_geom.x;
    let mut sp_y = snap.box_geom.y;
    let mut sp_w = snap.box_geom.width;
    let mut sp_h = snap.box_geom.height;
    let mut box_geom_write = None;

    if sp_w == 0 || sp_h == 0 {
        sp_w = if snap.min_width > 32 {
            std::cmp::max(p.overlay_width, snap.min_width)
        } else {
            p.overlay_width
        };
        sp_h = (ctx.usable.height - dec_h - 2 * g - 2 * bw).max(1);

        sp_x = if p.position_right {
            ctx.usable.x + ctx.usable.width - sp_w - g - bw
        } else {
            ctx.usable.x + g + bw
        };
        sp_y = ctx.usable.y + dec_h + g + bw;

        box_geom_write = Some(Rect { x: sp_x, y: sp_y, width: sp_w, height: sp_h });
    } else if snap.is_cloud {
        if let Some(pos) = p.cloud_position_default {
            sp_x = ctx.usable.x + pos[0];
            sp_y = ctx.usable.y + pos[1];
            box_geom_write = Some(Rect { x: sp_x, y: sp_y, width: sp_w, height: sp_h });
        }
    }

    let vx = ctx.pan_x + (sp_x - ctx.phys.x) as f64 / ctx.zoom;
    let vy = ctx.pan_y + (sp_y - ctx.phys.y) as f64 / ctx.zoom;

    let mut target_w = sp_w;
    let mut target_h = sp_h;
    if !snap.ssd {
        let (dec_w, dec_h) = snap.decorations_size;
        target_w = (sp_w - dec_w).max(1);
        target_h = (sp_h - dec_h).max(1);
    }

    OverlayPlacement {
        pos: (sp_x, sp_y),
        box_geom_write,
        virtual_pos: (vx, vy),
        size: (target_w as u32, target_h as u32),
    }
}

/// State-machine step for entering/leaving Maximized mode. `Enter` tells the
/// mechanism to save the given restore size (plus the window's current virtual
/// position) and set `was_maximized`; `Exit` restores the saved geometry.
/// Leaving with an invalid saved size does nothing (`was_maximized` stays set).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaximizedTransition {
    Enter { width: i32, height: i32 },
    Exit { width: i32, height: i32, virtual_x: f64, virtual_y: f64 },
}

pub fn maximized_transition(
    is_maximized_mode: bool,
    was_maximized: bool,
    box_size: (i32, i32),
    min_size: (i32, i32),
    saved_size: (i32, i32),
    saved_virtual: (f64, f64),
) -> Option<MaximizedTransition> {
    if is_maximized_mode && !was_maximized {
        let mut w = box_size.0;
        let mut h = box_size.1;
        if w <= 0 {
            w = if min_size.0 > 32 { min_size.0 } else { 800 };
        }
        if h <= 0 {
            h = if min_size.1 > 32 { min_size.1 } else { 600 };
        }
        Some(MaximizedTransition::Enter { width: w, height: h })
    } else if !is_maximized_mode && was_maximized {
        if saved_size.0 > 0 && saved_size.1 > 0 {
            Some(MaximizedTransition::Exit {
                width: saved_size.0,
                height: saved_size.1,
                virtual_x: saved_virtual.0,
                virtual_y: saved_virtual.1,
            })
        } else {
            None
        }
    } else {
        None
    }
}

pub struct NormalSnapshot {
    pub mode: TilingMode,
    pub box_geom: Rect,
    pub min_size: (i32, i32),
    pub virtual_pos: (f64, f64),
    /// Live interactive-resize dimensions, if a resize op is in progress.
    pub active_resize: Option<(u32, u32)>,
    pub is_cloud: bool,
    pub saved_maximized_size: (i32, i32),
    pub saved_maximized_virtual: (f64, f64),
}

pub struct NormalParams {
    pub gap_right: i32,
    pub gap_top: i32,
    /// Server-side border width; docked and grid-snapped placements are
    /// border-inclusive.
    pub border_width: i32,
    pub cloud_position_default: Option<[i32; 2]>,
    pub desktop_grid_scale: f64,
    /// Desktop grid gap between cells (the grid period is scale + gap).
    pub desktop_gap_width: f64,
    /// Visual inset of a cell's edge (the fade inset); Maximized windows
    /// snap to the visible cell edges like interactive snapping does.
    pub desktop_cell_inset: f64,
}

pub struct NormalPlacement {
    pub pos: (i32, i32),
    pub scale: f64,
    pub size: (u32, u32),
    /// Fullscreen windows report all edges tiled to the client.
    pub tiled_all_edges: bool,
    /// Offscreen-culling result; `None` leaves the window's flag untouched.
    pub hidden: Option<bool>,
    /// Maximized grid-snap moves the window's virtual position.
    pub virtual_write: Option<(f64, f64)>,
}

/// Place a non-overlay window according to its tiling mode: `Popup` docks to
/// the usable area's top-right (or the cloud default position), `Fullscreen`
/// covers the physical output, `Maximized` snaps to cover every desktop-grid
/// cell its saved geometry touches, and everything else pans on the virtual
/// surface under the current viewport.
pub fn place_normal_window(
    snap: &NormalSnapshot,
    p: &NormalParams,
    ctx: &PlacementCtx,
) -> NormalPlacement {
    match snap.mode {
        TilingMode::Popup => {
            let fw = if snap.box_geom.width > 0 {
                snap.box_geom.width
            } else if snap.min_size.0 > 32 {
                snap.min_size.0
            } else {
                360
            };
            let fh = if snap.box_geom.height > 0 {
                snap.box_geom.height
            } else if snap.min_size.1 > 32 {
                snap.min_size.1
            } else {
                100
            };

            // The dock slot is border-inclusive: inset so the border stays
            // within the configured gaps. Explicit cloud positions are taken
            // as content positions verbatim.
            let bw = p.border_width.max(0);
            let (fx, fy) = if snap.is_cloud && p.cloud_position_default.is_some() {
                let pos = p.cloud_position_default.unwrap();
                (ctx.usable.x + pos[0], ctx.usable.y + pos[1])
            } else {
                (ctx.usable.x + ctx.usable.width - fw - p.gap_right - bw, ctx.usable.y + p.gap_top + bw)
            };

            NormalPlacement {
                pos: (fx, fy),
                scale: 1.0,
                size: (fw as u32, fh as u32),
                tiled_all_edges: false,
                hidden: None,
                virtual_write: None,
            }
        }
        TilingMode::Fullscreen => NormalPlacement {
            pos: (ctx.phys.x, ctx.phys.y),
            scale: 1.0,
            size: (ctx.phys.width as u32, ctx.phys.height as u32),
            tiled_all_edges: true,
            hidden: None,
            virtual_write: None,
        },
        TilingMode::Maximized => {
            // Cover the VISIBLE edges of every desktop-grid cell the saved
            // geometry touches — the same cell-edge geometry as interactive
            // grid snapping (snap::maximized_span).
            let x1 = snap.saved_maximized_virtual.0;
            let y1 = snap.saved_maximized_virtual.1;
            let x2 = x1 + snap.saved_maximized_size.0 as f64;
            let y2 = y1 + snap.saved_maximized_size.1 as f64;

            let (low_x, high_x) = crate::snap::maximized_span(
                x1, x2, p.desktop_grid_scale, p.desktop_gap_width, p.desktop_cell_inset,
            );
            let (low_y, high_y) = crate::snap::maximized_span(
                y1, y2, p.desktop_grid_scale, p.desktop_gap_width, p.desktop_cell_inset,
            );

            // The span is border-inclusive: the content is inset so the
            // border stays inside the covered cells. Idempotent across
            // frames because it re-derives from the saved geometry.
            let bw = p.border_width.max(0) as f64;
            let content_x = low_x + bw;
            let content_y = low_y + bw;
            let fw = (high_x - low_x - 2.0 * bw).max(1.0);
            let fh = (high_y - low_y - 2.0 * bw).max(1.0);

            let (final_x, final_y) = ctx.virtual_to_screen(content_x, content_y);

            NormalPlacement {
                pos: (final_x, final_y),
                scale: ctx.zoom,
                size: (fw as u32, fh as u32),
                tiled_all_edges: false,
                hidden: Some(ctx.is_offscreen(final_x, final_y, fw * ctx.zoom, fh * ctx.zoom)),
                virtual_write: Some((content_x, content_y)),
            }
        }
        _ => {
            // Regular pannable window on the virtual surface.
            let fw = if let Some(resize_size) = snap.active_resize {
                resize_size.0 as i32
            } else if snap.box_geom.width > 0 {
                snap.box_geom.width
            } else if snap.min_size.0 > 32 {
                snap.min_size.0
            } else {
                800
            };
            let fh = if let Some(resize_size) = snap.active_resize {
                resize_size.1 as i32
            } else if snap.box_geom.height > 0 {
                snap.box_geom.height
            } else if snap.min_size.1 > 32 {
                snap.min_size.1
            } else {
                600
            };

            let (final_x, final_y) = ctx.virtual_to_screen(snap.virtual_pos.0, snap.virtual_pos.1);

            NormalPlacement {
                pos: (final_x, final_y),
                scale: ctx.zoom,
                size: (fw as u32, fh as u32),
                tiled_all_edges: false,
                hidden: Some(ctx.is_offscreen(final_x, final_y, fw as f64 * ctx.zoom, fh as f64 * ctx.zoom)),
                virtual_write: None,
            }
        }
    }
}

const SPACING: i32 = 12;
const MARGIN: i32 = 12;

const LEFT_ORDER: &[&str] = &["viewport", "window"];
const RIGHT_ORDER: &[&str] = &["tray", "cpu", "memory", "brightness", "volume", "battery", "clock"];

fn left_sort_key(app_id: &str) -> usize {
    let name = app_id.strip_prefix("cce-status-interface-left-")
        .or_else(|| app_id.strip_prefix("cce-status-left-"))
        .unwrap_or(app_id);
    LEFT_ORDER.iter().position(|&m| m == name).unwrap_or(99)
}

fn right_sort_key(app_id: &str) -> usize {
    let name = app_id.strip_prefix("cce-status-interface-right-")
        .or_else(|| app_id.strip_prefix("cce-status-right-"))
        .unwrap_or(app_id);
    RIGHT_ORDER.iter().position(|&m| m == name).unwrap_or(99)
}

/// Bar length to use: previous major length, or 100 for a fresh bar.
fn bar_len(prev_len: i32) -> u32 {
    if prev_len > 0 { prev_len as u32 } else { 100 }
}

/// Lay out status-bar windows on one output: horizontal groups on the top and
/// bottom edges (left/center/right within each), vertical stacks on the left
/// and right edges, and full-width bars across the top.
///
/// Returns one placement per item, index-aligned with `items`.
pub fn layout_status_bars(
    items: &[StatusBarItem],
    p: &StatusBarLayoutParams,
) -> Vec<Option<StatusBarPlacement>> {
    let wlr_box = p.output;
    let bar_h = p.bar_height;
    let spacing = SPACING;
    let margin = MARGIN;

    let mut top_left: Vec<usize> = Vec::new();
    let mut top_center: Vec<usize> = Vec::new();
    let mut top_right: Vec<usize> = Vec::new();
    let mut bottom_left: Vec<usize> = Vec::new();
    let mut bottom_center: Vec<usize> = Vec::new();
    let mut bottom_right: Vec<usize> = Vec::new();
    let mut left_side: Vec<usize> = Vec::new();
    let mut right_side: Vec<usize> = Vec::new();
    let mut full_top: Vec<usize> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        let edge = if item.edge == StatusEdge::Unspecified {
            StatusEdge::TopLeft
        } else {
            item.edge
        };
        match edge {
            StatusEdge::TopLeft => top_left.push(idx),
            StatusEdge::TopCenter => top_center.push(idx),
            StatusEdge::TopRight => top_right.push(idx),
            StatusEdge::BottomLeft => bottom_left.push(idx),
            StatusEdge::BottomCenter => bottom_center.push(idx),
            StatusEdge::BottomRight => bottom_right.push(idx),
            StatusEdge::Left => left_side.push(idx),
            StatusEdge::Right => right_side.push(idx),
            _ => full_top.push(idx),
        }
    }

    log::info!("[ArrangeStatus] top_left_len={}, top_center_len={}, top_right_len={}, left_side_len={}", top_left.len(), top_center.len(), top_right.len(), left_side.len());

    let sort_left = |list: &mut Vec<usize>| {
        list.sort_by_key(|&i| left_sort_key(&items[i].app_id));
    };
    let sort_right = |list: &mut Vec<usize>| {
        list.sort_by_key(|&i| right_sort_key(&items[i].app_id));
    };

    sort_left(&mut top_left);
    sort_left(&mut top_center);
    sort_right(&mut top_right);
    sort_left(&mut bottom_left);
    sort_left(&mut bottom_center);
    sort_right(&mut bottom_right);

    let mut placements: Vec<Option<StatusBarPlacement>> = vec![None; items.len()];

    // 1. Top Edge
    let status_y_top = if p.hide_mode {
        wlr_box.y - (bar_h as i32 - p.hide_mode_preview)
    } else {
        wlr_box.y
    };

    let mut top_right_width_needed = 0;
    for &i in &top_right {
        let w = bar_len(items[i].prev_len);
        top_right_width_needed += w as i32 + spacing;
    }
    let top_right_boundary = wlr_box.x + wlr_box.width - margin - top_right_width_needed;

    // 1a. Left Group (TopLeft / nw)
    let mut cur_left_x = wlr_box.x + margin;
    for &i in &top_left {
        let mut w = bar_len(items[i].prev_len);
        let max_allowed_w = top_right_boundary - cur_left_x - spacing;
        if w as i32 > max_allowed_w {
            w = std::cmp::max(max_allowed_w, 20) as u32;
        }
        log::info!("[TopLeftLayout] app_id={} x={}, w={}", items[i].app_id, cur_left_x, w);
        placements[i] = Some(StatusBarPlacement { x: cur_left_x, y: status_y_top, width: w, height: bar_h });
        cur_left_x += w as i32 + spacing;
    }

    // 1b. Center Group (TopCenter / n)
    let mut top_center_width_needed = 0;
    for &i in &top_center {
        let w = bar_len(items[i].prev_len);
        top_center_width_needed += w as i32 + spacing;
    }
    if top_center_width_needed > 0 {
        top_center_width_needed -= spacing;
    }
    let center_start_x = wlr_box.x + (wlr_box.width - top_center_width_needed) / 2;
    let mut cur_center_x = std::cmp::max(center_start_x, cur_left_x + spacing);

    for &i in &top_center {
        let mut w = bar_len(items[i].prev_len);
        let max_allowed_w = top_right_boundary - cur_center_x - spacing;
        if w as i32 > max_allowed_w {
            w = std::cmp::max(max_allowed_w, 20) as u32;
        }
        log::info!("[TopCenterLayout] app_id={} x={}, w={}", items[i].app_id, cur_center_x, w);
        placements[i] = Some(StatusBarPlacement { x: cur_center_x, y: status_y_top, width: w, height: bar_h });
        cur_center_x += w as i32 + spacing;
    }

    // 1c. Right Group (TopRight / ne)
    let mut cur_right_x = wlr_box.x + wlr_box.width - margin;
    for &i in top_right.iter().rev() {
        let w = bar_len(items[i].prev_len);
        let x = cur_right_x - w as i32;
        placements[i] = Some(StatusBarPlacement { x, y: status_y_top, width: w, height: bar_h });
        cur_right_x = x - spacing;
    }

    for &i in &full_top {
        placements[i] = Some(StatusBarPlacement { x: wlr_box.x, y: status_y_top, width: wlr_box.width as u32, height: bar_h });
    }

    // 2. Bottom Edge
    let status_y_bottom = wlr_box.y + wlr_box.height - bar_h as i32;

    let mut bottom_right_width_needed = 0;
    for &i in &bottom_right {
        let w = bar_len(items[i].prev_len);
        bottom_right_width_needed += w as i32 + spacing;
    }
    let bottom_right_boundary = wlr_box.x + wlr_box.width - margin - bottom_right_width_needed;

    // 2a. Left Group (BottomLeft / sw)
    let mut cur_left_x = wlr_box.x + margin;
    for &i in &bottom_left {
        let mut w = bar_len(items[i].prev_len);
        let max_allowed_w = bottom_right_boundary - cur_left_x - spacing;
        if w as i32 > max_allowed_w {
            w = std::cmp::max(max_allowed_w, 20) as u32;
        }
        placements[i] = Some(StatusBarPlacement { x: cur_left_x, y: status_y_bottom, width: w, height: bar_h });
        cur_left_x += w as i32 + spacing;
    }

    // 2b. Center Group (BottomCenter / s)
    let mut bottom_center_width_needed = 0;
    for &i in &bottom_center {
        let w = bar_len(items[i].prev_len);
        bottom_center_width_needed += w as i32 + spacing;
    }
    if bottom_center_width_needed > 0 {
        bottom_center_width_needed -= spacing;
    }
    let center_start_x = wlr_box.x + (wlr_box.width - bottom_center_width_needed) / 2;
    let mut cur_center_x = std::cmp::max(center_start_x, cur_left_x + spacing);

    for &i in &bottom_center {
        let mut w = bar_len(items[i].prev_len);
        let max_allowed_w = bottom_right_boundary - cur_center_x - spacing;
        if w as i32 > max_allowed_w {
            w = std::cmp::max(max_allowed_w, 20) as u32;
        }
        placements[i] = Some(StatusBarPlacement { x: cur_center_x, y: status_y_bottom, width: w, height: bar_h });
        cur_center_x += w as i32 + spacing;
    }

    // 2c. Right Group (BottomRight / se)
    let mut cur_right_x = wlr_box.x + wlr_box.width - margin;
    for &i in bottom_right.iter().rev() {
        let w = bar_len(items[i].prev_len);
        let x = cur_right_x - w as i32;
        placements[i] = Some(StatusBarPlacement { x, y: status_y_bottom, width: w, height: bar_h });
        cur_right_x = x - spacing;
    }

    // 3. Left Edge (Vertical stacking)
    let mut left_total_height = 0;
    for &i in &left_side {
        let actual_h = bar_len(items[i].prev_len);
        left_total_height += actual_h as i32;
    }
    if !left_side.is_empty() {
        left_total_height += (left_side.len() as i32 - 1) * spacing;
    }
    let mut cur_left_y = wlr_box.y + (wlr_box.height - left_total_height) / 2;

    for &i in &left_side {
        let actual_h = bar_len(items[i].prev_len);
        placements[i] = Some(StatusBarPlacement { x: wlr_box.x, y: cur_left_y, width: bar_h, height: actual_h });
        cur_left_y += actual_h as i32 + spacing;
    }

    // 4. Right Edge (Vertical stacking)
    let mut right_total_height = 0;
    for &i in &right_side {
        let actual_h = bar_len(items[i].prev_len);
        right_total_height += actual_h as i32;
    }
    if !right_side.is_empty() {
        right_total_height += (right_side.len() as i32 - 1) * spacing;
    }
    let mut cur_right_y = wlr_box.y + (wlr_box.height - right_total_height) / 2;

    for &i in &right_side {
        let actual_h = bar_len(items[i].prev_len);
        placements[i] = Some(StatusBarPlacement { x: wlr_box.x + wlr_box.width - bar_h as i32, y: cur_right_y, width: bar_h, height: actual_h });
        cur_right_y += actual_h as i32 + spacing;
    }

    placements
}

/// One enabled output, as `arrange` needs it.
#[derive(Debug, Clone, Copy)]
pub struct OutputSnapshot {
    /// The output's layout box.
    pub layout_box: Rect,
    /// Layer-shell non-exclusive area, relative to the layout box; a
    /// zero-sized rect means no exclusion.
    pub non_exclusive: Rect,
}

/// Everything `arrange` reads about one window. Built once per frame by the
/// mechanism side; seat-dependent answers (`being_moved`, `active_resize`)
/// are captured here so the pure pass never queries mid-computation.
#[derive(Debug, Clone)]
pub struct WindowSnapshot {
    pub app_id: Option<String>,
    /// Only used verbatim in log lines.
    pub title: Option<String>,
    pub role: WindowRole,
    pub minimized: bool,
    /// Window state is Closing or Init.
    pub closing_or_init: bool,
    /// Resolved tiling mode (`get_mode_for_window`).
    pub mode: TilingMode,
    /// Mode-rule SSD override; pre-gated on `!mode_locked`.
    pub rule_ssd: Option<bool>,
    /// A seat is interactively moving this window.
    pub being_moved: bool,
    pub status_edge: StatusEdge,
    pub is_focused: bool,
    pub box_geom: Rect,
    /// (min_width, min_height) size hints.
    pub min_size: (i32, i32),
    pub virtual_pos: (f64, f64),
    pub active_resize: Option<(u32, u32)>,
    /// Current `wm_requested.ssd`.
    pub ssd: bool,
    /// Raw decoration measurement (`measure_decorations`), not gated on
    /// `ssd` — placement applies it only when the effective SSD is off.
    pub decorations_size: (i32, i32),
    pub was_maximized: bool,
    pub saved_maximized_size: (i32, i32),
    pub saved_maximized_virtual: (f64, f64),
}

/// Frame-wide inputs: config knobs plus the desktop viewport.
pub struct ArrangeParams {
    pub bar_height: i32,
    pub status_hide_mode: bool,
    pub hide_mode_preview: i32,
    /// `status_background_blur > 0.001`.
    pub status_blur: bool,
    pub window_blur: bool,
    /// `layout.window_opacity` — unfocused windows dim when set.
    pub opacity_enabled: bool,
    /// The configured server-side decoration, applied to every placed window.
    pub decoration: DecorationSpec,
    /// Border color for the focused window (falls back to the unfocused
    /// color in config when unset).
    pub border_color_focused: crate::api::Rgba,
    pub overlay: OverlayParams,
    pub normal: NormalParams,
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
}

/// Write instructions for one window. `None` leaves the field untouched, so
/// the mechanism apply loop is a flat sequence of `if let Some` writes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowPlan {
    /// Enable/disable the window's scene tree.
    pub scene_enabled: Option<bool>,
    pub hidden: Option<bool>,
    pub tiling_mode: Option<TilingMode>,
    /// `wm_requested.tiled` edge bitmask.
    pub tiled: Option<u32>,
    pub ssd: Option<bool>,
    pub scale: Option<f64>,
    /// `rendering_requested.{x,y}`.
    pub pos: Option<(i32, i32)>,
    /// `wm_requested.dimensions` and `.bounds`.
    pub size: Option<(u32, u32)>,
    /// Persistent geometry write-back.
    pub box_geom: Option<Rect>,
    pub virtual_pos: Option<(f64, f64)>,
    pub blur: Option<bool>,
    pub decoration: Option<DecorationSpec>,
    pub opacity: Option<f32>,
    pub was_maximized: Option<bool>,
    /// Maximized-enter save: (restore size, restore virtual position).
    pub saved_maximized: Option<((i32, i32), (f64, f64))>,
}

pub struct ArrangePlan {
    /// Index-aligned with the input snapshots.
    pub windows: Vec<WindowPlan>,
    /// Whether each output's fallback background rect should be shown
    /// (false while a wallpaper window exists).
    pub background_rect_enabled: bool,
}

fn is_cloud_app(app_id: Option<&str>) -> bool {
    app_id.map_or(false, |id| id.starts_with("cce-cloud"))
}

/// The decoration one placed window gets: the configured spec with the
/// focused border color swapped in, and no border at all when fullscreen.
fn decoration_for(p: &ArrangeParams, is_focused: bool, mode: TilingMode) -> DecorationSpec {
    if mode == TilingMode::Fullscreen {
        return DecorationSpec {
            border_width: 0,
            corner_radius: 0,
            ..p.decoration
        };
    }
    DecorationSpec {
        border_color: if is_focused {
            p.border_color_focused
        } else {
            p.decoration.border_color
        },
        ..p.decoration
    }
}

/// The whole arrange pass as one pure function: classify every window, place
/// overlays/normals/status bars per output, and return per-window write
/// instructions.
///
/// The output loop is last-wins, like the mechanism loop it replaces: every
/// output pass re-plans every window, so with several outputs the final plan
/// reflects the last one. State that arranging itself evolves (box geometry,
/// virtual position, SSD overrides, the maximized save/restore machine) is
/// tracked on a working copy of the snapshots so later sections and later
/// output passes read what earlier ones wrote — exactly as the mutating
/// original did.
pub fn arrange(
    windows: &[WindowSnapshot],
    outputs: &[OutputSnapshot],
    p: &ArrangeParams,
) -> ArrangePlan {
    let mut state: Vec<WindowSnapshot> = windows.to_vec();
    let mut plan: Vec<WindowPlan> = vec![WindowPlan::default(); windows.len()];

    let has_wallpaper = state.iter().any(|w| w.role == WindowRole::Background);

    for out in outputs {
        let phys = out.layout_box;

        let status_edges: Vec<StatusEdge> = state
            .iter()
            .filter(|w| w.role == WindowRole::StatusBar)
            .map(|w| w.status_edge)
            .collect();
        let usable = compute_usable_area(
            phys,
            out.non_exclusive,
            p.bar_height,
            p.status_hide_mode,
            &status_edges,
        );
        let ctx = PlacementCtx {
            phys,
            usable,
            pan_x: p.pan_x,
            pan_y: p.pan_y,
            zoom: p.zoom,
        };

        let mut overlay_windows: Vec<usize> = Vec::new();
        let mut normal_windows: Vec<usize> = Vec::new();

        for (i, w) in state.iter_mut().enumerate() {
            let class = classify_window(w.role, w.minimized, w.closing_or_init, w.mode, w.being_moved);
            let wp = &mut plan[i];
            match class {
                WindowClass::Background => {
                    wp.tiling_mode = Some(TilingMode::Status);
                    wp.tiled = Some(0);
                    wp.ssd = Some(false);
                    w.ssd = false;
                    wp.scale = Some(1.0);
                    wp.scene_enabled = Some(true);
                    wp.hidden = Some(false);
                    wp.blur = Some(false);
                    wp.pos = Some((phys.x, phys.y));
                    wp.size = Some((phys.width as u32, phys.height as u32));
                }
                WindowClass::StatusBar => {
                    wp.tiling_mode = Some(TilingMode::Status);
                    wp.tiled = Some(0);
                    wp.ssd = Some(false);
                    w.ssd = false;
                    wp.scale = Some(1.0);
                    wp.scene_enabled = Some(true);
                    wp.hidden = Some(false);
                    wp.blur = Some(p.status_blur);
                }
                WindowClass::Hidden => {
                    wp.scene_enabled = Some(false);
                    wp.hidden = Some(true);
                }
                WindowClass::Overlay | WindowClass::Normal => {
                    wp.scene_enabled = Some(true);
                    wp.hidden = Some(false);
                    wp.tiling_mode = Some(w.mode);
                    if let Some(rule_ssd) = w.rule_ssd {
                        wp.ssd = Some(rule_ssd);
                        w.ssd = rule_ssd;
                    }
                    if class == WindowClass::Overlay {
                        overlay_windows.push(i);
                    } else {
                        normal_windows.push(i);
                    }
                }
            }
        }

        for (sp_idx, &i) in overlay_windows.iter().enumerate() {
            if sp_idx == 0 {
                let w = &state[i];
                let placement = place_overlay_window(
                    &OverlaySnapshot {
                        box_geom: w.box_geom,
                        min_width: w.min_size.0,
                        is_cloud: is_cloud_app(w.app_id.as_deref()),
                        ssd: w.ssd,
                        decorations_size: w.decorations_size,
                    },
                    &p.overlay,
                    &ctx,
                );

                if let Some(bg) = placement.box_geom_write {
                    state[i].box_geom = bg;
                }
                state[i].virtual_pos = placement.virtual_pos;

                let wp = &mut plan[i];
                if let Some(bg) = placement.box_geom_write {
                    wp.box_geom = Some(bg);
                }
                wp.pos = Some(placement.pos);
                wp.scale = Some(1.0);
                wp.virtual_pos = Some(placement.virtual_pos);
                wp.size = Some(placement.size);
                wp.tiled = Some(1 | 2 | 4 | 8);
                wp.decoration = Some(decoration_for(p, state[i].is_focused, state[i].mode));
                wp.blur = Some(p.window_blur);
                wp.opacity = Some(window_opacity(
                    state[i].is_focused,
                    p.opacity_enabled,
                    OVERLAY_UNFOCUSED_OPACITY,
                ));
            } else {
                normal_windows.push(i);
            }
        }

        // Manage entering/exiting Maximized state for normal windows.
        for &i in &normal_windows {
            let w = &state[i];
            let transition = maximized_transition(
                w.mode == TilingMode::Maximized,
                w.was_maximized,
                (w.box_geom.width, w.box_geom.height),
                w.min_size,
                w.saved_maximized_size,
                w.saved_maximized_virtual,
            );
            match transition {
                Some(MaximizedTransition::Enter { width, height }) => {
                    let w = &mut state[i];
                    w.saved_maximized_size = (width, height);
                    w.saved_maximized_virtual = w.virtual_pos;
                    w.was_maximized = true;
                    let wp = &mut plan[i];
                    wp.saved_maximized = Some(((width, height), w.saved_maximized_virtual));
                    wp.was_maximized = Some(true);
                    log::info!("[Maximized] Saved window {:?} geometry: {}x{} at ({}, {})",
                        w.title.as_deref().unwrap_or(""),
                        width, height,
                        w.saved_maximized_virtual.0, w.saved_maximized_virtual.1
                    );
                }
                Some(MaximizedTransition::Exit { width, height, virtual_x, virtual_y }) => {
                    let w = &mut state[i];
                    w.box_geom.width = width;
                    w.box_geom.height = height;
                    w.virtual_pos = (virtual_x, virtual_y);
                    w.was_maximized = false;
                    let wp = &mut plan[i];
                    wp.box_geom = Some(w.box_geom);
                    wp.virtual_pos = Some((virtual_x, virtual_y));
                    wp.was_maximized = Some(false);
                    wp.size = Some((width as u32, height as u32));
                    log::info!("[Maximized] Restored window {:?} geometry: {}x{} at ({}, {})",
                        w.title.as_deref().unwrap_or(""),
                        width, height, virtual_x, virtual_y
                    );
                }
                None => {}
            }
        }

        // Arrange normal windows on the virtual surface.
        for &i in &normal_windows {
            let w = &state[i];
            let mode = w.mode;
            let placement = place_normal_window(
                &NormalSnapshot {
                    mode: w.mode,
                    box_geom: w.box_geom,
                    min_size: w.min_size,
                    virtual_pos: w.virtual_pos,
                    active_resize: w.active_resize,
                    is_cloud: is_cloud_app(w.app_id.as_deref()),
                    saved_maximized_size: w.saved_maximized_size,
                    saved_maximized_virtual: w.saved_maximized_virtual,
                },
                &p.normal,
                &ctx,
            );

            let is_focused = w.is_focused;
            if let Some(v) = placement.virtual_write {
                state[i].virtual_pos = v;
            }

            let wp = &mut plan[i];
            wp.pos = Some(placement.pos);
            wp.scale = Some(placement.scale);
            if let Some(v) = placement.virtual_write {
                wp.virtual_pos = Some(v);
            }
            if let Some(hidden) = placement.hidden {
                wp.hidden = Some(hidden);
            }
            wp.size = Some(placement.size);
            if placement.tiled_all_edges {
                wp.tiled = Some(1 | 2 | 4 | 8);
            }
            wp.decoration = Some(decoration_for(p, is_focused, mode));
            wp.blur = Some(p.window_blur);
            wp.opacity = Some(window_opacity(is_focused, p.opacity_enabled, NORMAL_UNFOCUSED_OPACITY));
        }

        // Position status bar windows on this output, excluding any being
        // interactively dragged.
        let mut status_items: Vec<StatusBarItem> = Vec::new();
        let mut status_idxs: Vec<usize> = Vec::new();
        for (i, w) in state.iter().enumerate() {
            if w.closing_or_init {
                continue;
            }
            if let Some(app_id) = &w.app_id {
                if app_id.starts_with("cce-status") {
                    if w.being_moved {
                        continue;
                    }
                    log::info!("[ArrangeStatus] app_id={} status_edge={:?}", app_id, w.status_edge);
                    status_items.push(StatusBarItem {
                        app_id: app_id.clone(),
                        edge: w.status_edge,
                        prev_len: std::cmp::max(w.box_geom.width, w.box_geom.height),
                    });
                    status_idxs.push(i);
                }
            }
        }

        let placements = layout_status_bars(
            &status_items,
            &StatusBarLayoutParams {
                output: phys,
                bar_height: p.bar_height as u32,
                hide_mode: p.status_hide_mode,
                hide_mode_preview: p.hide_mode_preview,
            },
        );

        for (&i, placement) in status_idxs.iter().zip(placements.iter()) {
            if let Some(pl) = placement {
                plan[i].pos = Some((pl.x, pl.y));
                plan[i].size = Some((pl.width, pl.height));
            }
        }
    }

    ArrangePlan {
        windows: plan,
        background_rect_enabled: !has_wallpaper,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> StatusBarLayoutParams {
        StatusBarLayoutParams {
            output: Rect { x: 0, y: 0, width: 1920, height: 1080 },
            bar_height: 30,
            hide_mode: false,
            hide_mode_preview: 5,
        }
    }

    fn item(app_id: &str, edge: StatusEdge, prev_len: i32) -> StatusBarItem {
        StatusBarItem { app_id: app_id.to_string(), edge, prev_len }
    }

    #[test]
    fn top_groups_flow_from_edges() {
        let items = vec![
            item("cce-status-left-viewport", StatusEdge::TopLeft, 0),
            item("cce-status-left-window", StatusEdge::TopLeft, 0),
            item("cce-status-right-clock", StatusEdge::TopRight, 200),
        ];
        let p = layout_status_bars(&items, &params());
        // Left group flows right from the margin; fresh bars default to width 100.
        assert_eq!(p[0], Some(StatusBarPlacement { x: 12, y: 0, width: 100, height: 30 }));
        assert_eq!(p[1], Some(StatusBarPlacement { x: 124, y: 0, width: 100, height: 30 }));
        // Right group is placed from the right edge inward.
        assert_eq!(p[2], Some(StatusBarPlacement { x: 1908 - 200, y: 0, width: 200, height: 30 }));
    }

    #[test]
    fn right_group_sorted_by_module_order() {
        let items = vec![
            item("cce-status-right-clock", StatusEdge::TopRight, 100),
            item("cce-status-right-battery", StatusEdge::TopRight, 100),
        ];
        let p = layout_status_bars(&items, &params());
        // RIGHT_ORDER puts battery before clock left-to-right, so clock hugs the edge.
        assert_eq!(p[0].unwrap().x, 1808);
        assert_eq!(p[1].unwrap().x, 1808 - 12 - 100);
    }

    #[test]
    fn hide_mode_pushes_top_bars_offscreen_with_preview() {
        let mut prm = params();
        prm.hide_mode = true;
        let items = vec![item("cce-status-left-viewport", StatusEdge::Unspecified, 0)];
        let p = layout_status_bars(&items, &prm);
        // Unspecified resolves to TopLeft; y = 0 - (30 - 5).
        assert_eq!(p[0].unwrap().y, -25);
    }

    #[test]
    fn usable_area_reserves_bar_edges() {
        let output = Rect { x: 0, y: 0, width: 1920, height: 1080 };
        let no_excl = Rect { x: 0, y: 0, width: 0, height: 0 };

        // No bars, no exclusion: the full output.
        assert_eq!(compute_usable_area(output, no_excl, 30, false, &[]), output);

        // Top + left bars each reserve one bar-height.
        let edges = [StatusEdge::TopLeft, StatusEdge::Left];
        assert_eq!(
            compute_usable_area(output, no_excl, 30, false, &edges),
            Rect { x: 30, y: 30, width: 1890, height: 1050 }
        );

        // Hide mode releases the top reservation but not the others.
        assert_eq!(
            compute_usable_area(output, no_excl, 30, true, &edges),
            Rect { x: 30, y: 0, width: 1890, height: 1080 }
        );

        // Layer-shell non-exclusive area applies before bar reservations.
        let excl = Rect { x: 10, y: 20, width: 1900, height: 1040 };
        assert_eq!(
            compute_usable_area(output, excl, 30, false, &[StatusEdge::BottomCenter]),
            Rect { x: 10, y: 20, width: 1900, height: 1010 }
        );
    }

    #[test]
    fn window_roles_from_app_id() {
        assert_eq!(WindowRole::from_app_id(Some("cce-wallpaper")), WindowRole::Background);
        assert_eq!(WindowRole::from_app_id(Some("cce-status-interface-right-clock")), WindowRole::StatusBar);
        assert_eq!(WindowRole::from_app_id(Some("firefox")), WindowRole::Normal);
        assert_eq!(WindowRole::from_app_id(None), WindowRole::Normal);
    }

    #[test]
    fn classification_precedence() {
        // Role wins over visibility: a minimized wallpaper still arranges as background.
        assert_eq!(
            classify_window(WindowRole::Background, true, true, TilingMode::Floating, false),
            WindowClass::Background
        );
        assert_eq!(
            classify_window(WindowRole::StatusBar, false, false, TilingMode::Floating, false),
            WindowClass::StatusBar
        );
        // Minimized or closing/init normal windows are hidden.
        assert_eq!(
            classify_window(WindowRole::Normal, true, false, TilingMode::Grid, false),
            WindowClass::Hidden
        );
        // Overlay mode gets the overlay slot — unless mid-drag.
        assert_eq!(
            classify_window(WindowRole::Normal, false, false, TilingMode::Overlay, false),
            WindowClass::Overlay
        );
        assert_eq!(
            classify_window(WindowRole::Normal, false, false, TilingMode::Overlay, true),
            WindowClass::Normal
        );
        assert_eq!(
            classify_window(WindowRole::Normal, false, false, TilingMode::Cascade, false),
            WindowClass::Normal
        );
    }

    fn ctx() -> PlacementCtx {
        PlacementCtx {
            phys: Rect { x: 0, y: 0, width: 1920, height: 1080 },
            usable: Rect { x: 0, y: 30, width: 1920, height: 1050 },
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
        }
    }

    #[test]
    fn fresh_overlay_gets_configured_slot() {
        let placement = place_overlay_window(
            &OverlaySnapshot {
                box_geom: Rect { x: 0, y: 0, width: 0, height: 0 },
                min_width: 0,
                is_cloud: false,
                ssd: true,
                decorations_size: (0, 16),
            },
            &OverlayParams {
                overlay_width: 400,
                border_gap: 8,
                border_width: 0,
                position_right: true,
                cloud_position_default: None,
            },
            &ctx(),
        );
        // Right slot: x = usable right edge - width - gap; height fills the
        // usable area minus the 16px decoration strip and both gaps.
        assert_eq!(placement.pos, (1512, 54));
        assert_eq!(placement.size, (400, 1018));
        assert_eq!(placement.box_geom_write, Some(Rect { x: 1512, y: 54, width: 400, height: 1018 }));
        assert_eq!(placement.virtual_pos, (1512.0, 54.0));
    }

    #[test]
    fn fresh_overlay_slot_insets_by_border_width() {
        let placement = place_overlay_window(
            &OverlaySnapshot {
                box_geom: Rect { x: 0, y: 0, width: 0, height: 0 },
                min_width: 0,
                is_cloud: false,
                ssd: true,
                decorations_size: (0, 16),
            },
            &OverlayParams {
                overlay_width: 400,
                border_gap: 8,
                border_width: 4,
                position_right: true,
                cloud_position_default: None,
            },
            &ctx(),
        );
        // Content sits one border width inside the zero-width slot on every
        // side, so the border lands within the gaps.
        assert_eq!(placement.pos, (1512 - 4, 54 + 4));
        assert_eq!(placement.size, (400, 1018 - 8));
    }

    #[test]
    fn overlay_without_ssd_shrinks_by_decorations() {
        let placement = place_overlay_window(
            &OverlaySnapshot {
                box_geom: Rect { x: 100, y: 100, width: 400, height: 500 },
                min_width: 0,
                is_cloud: false,
                ssd: false,
                decorations_size: (2, 18),
            },
            &OverlayParams { overlay_width: 400, border_gap: 8, border_width: 0, position_right: false, cloud_position_default: None },
            &ctx(),
        );
        // Existing geometry is kept; the client is sized minus decorations.
        assert_eq!(placement.pos, (100, 100));
        assert_eq!(placement.size, (398, 482));
        assert_eq!(placement.box_geom_write, None);
    }

    #[test]
    fn maximized_transitions() {
        // Entering with no usable geometry falls back to 800x600.
        assert_eq!(
            maximized_transition(true, false, (0, 0), (0, 0), (0, 0), (0.0, 0.0)),
            Some(MaximizedTransition::Enter { width: 800, height: 600 })
        );
        // Entering keeps real geometry.
        assert_eq!(
            maximized_transition(true, false, (640, 480), (0, 0), (0, 0), (0.0, 0.0)),
            Some(MaximizedTransition::Enter { width: 640, height: 480 })
        );
        // Steady states do nothing.
        assert_eq!(maximized_transition(true, true, (640, 480), (0, 0), (640, 480), (0.0, 0.0)), None);
        assert_eq!(maximized_transition(false, false, (640, 480), (0, 0), (0, 0), (0.0, 0.0)), None);
        // Exit restores the saved geometry; invalid saved size is a no-op.
        assert_eq!(
            maximized_transition(false, true, (0, 0), (0, 0), (640, 480), (10.0, 20.0)),
            Some(MaximizedTransition::Exit { width: 640, height: 480, virtual_x: 10.0, virtual_y: 20.0 })
        );
        assert_eq!(maximized_transition(false, true, (0, 0), (0, 0), (0, 480), (10.0, 20.0)), None);
    }

    #[test]
    fn maximized_snaps_to_grid_cells() {
        let snap = NormalSnapshot {
            mode: TilingMode::Maximized,
            box_geom: Rect { x: 0, y: 0, width: 100, height: 50 },
            min_size: (0, 0),
            virtual_pos: (150.0, 120.0),
            active_resize: None,
            is_cloud: false,
            saved_maximized_size: (100, 50),
            saved_maximized_virtual: (150.0, 120.0),
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, border_width: 0, cloud_position_default: None, desktop_grid_scale: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
        let placement = place_normal_window(&snap, &p, &ctx());
        // Saved geometry spans grid columns 1-2 and row 1 → snapped to
        // (100,100) with size 200x100.
        assert_eq!(placement.virtual_write, Some((100.0, 100.0)));
        assert_eq!(placement.pos, (100, 100));
        assert_eq!(placement.size, (200, 100));
        assert_eq!(placement.hidden, Some(false));
        assert_eq!(placement.scale, 1.0);
    }

    #[test]
    fn maximized_insets_by_border_width() {
        let snap = NormalSnapshot {
            mode: TilingMode::Maximized,
            box_geom: Rect { x: 0, y: 0, width: 100, height: 50 },
            min_size: (0, 0),
            virtual_pos: (150.0, 120.0),
            active_resize: None,
            is_cloud: false,
            saved_maximized_size: (100, 50),
            saved_maximized_virtual: (150.0, 120.0),
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, border_width: 4, cloud_position_default: None, desktop_grid_scale: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
        let placement = place_normal_window(&snap, &p, &ctx());
        // Same cell span as without borders (100,100)+200x100, with the
        // content inset so the border stays inside the covered cells.
        assert_eq!(placement.virtual_write, Some((104.0, 104.0)));
        assert_eq!(placement.pos, (104, 104));
        assert_eq!(placement.size, (192, 92));
    }

    #[test]
    fn maximized_snaps_to_visible_cell_edges() {
        let snap = NormalSnapshot {
            mode: TilingMode::Maximized,
            box_geom: Rect { x: 0, y: 0, width: 100, height: 50 },
            min_size: (0, 0),
            virtual_pos: (150.0, 120.0),
            active_resize: None,
            is_cloud: false,
            saved_maximized_size: (100, 50),
            saved_maximized_virtual: (150.0, 120.0),
        };
        // period 110 (gap 10), inset 5, border 4: cells x 1-2 visibly span
        // [115, 315], row y 1 spans [115, 205]; content insets by the border.
        let p = NormalParams {
            gap_right: 10,
            gap_top: 6,
            border_width: 4,
            cloud_position_default: None,
            desktop_grid_scale: 100.0,
            desktop_gap_width: 10.0,
            desktop_cell_inset: 5.0,
        };
        let placement = place_normal_window(&snap, &p, &ctx());
        assert_eq!(placement.virtual_write, Some((119.0, 119.0)));
        assert_eq!(placement.pos, (119, 119));
        assert_eq!(placement.size, (192, 82));
    }

    #[test]
    fn popup_docks_top_right_of_usable_area() {
        let snap = NormalSnapshot {
            mode: TilingMode::Popup,
            box_geom: Rect { x: 0, y: 0, width: 0, height: 0 },
            min_size: (0, 0),
            virtual_pos: (0.0, 0.0),
            active_resize: None,
            is_cloud: false,
            saved_maximized_size: (0, 0),
            saved_maximized_virtual: (0.0, 0.0),
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, border_width: 0, cloud_position_default: None, desktop_grid_scale: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
        let placement = place_normal_window(&snap, &p, &ctx());
        // Defaults to 360x100, docked inside the usable area (below the bar).
        assert_eq!(placement.pos, (1920 - 360 - 10, 30 + 6));
        assert_eq!(placement.size, (360, 100));
        assert_eq!(placement.hidden, None);

        // With a border, the dock position insets so the border stays inside
        // the gaps; the popup keeps its size.
        let p = NormalParams { border_width: 4, ..p };
        let placement = place_normal_window(&snap, &p, &ctx());
        assert_eq!(placement.pos, (1920 - 360 - 10 - 4, 30 + 6 + 4));
        assert_eq!(placement.size, (360, 100));
    }

    #[test]
    fn pannable_window_follows_viewport() {
        let snap = NormalSnapshot {
            mode: TilingMode::Cascade,
            box_geom: Rect { x: 0, y: 0, width: 640, height: 480 },
            min_size: (0, 0),
            virtual_pos: (100.0, 200.0),
            active_resize: None,
            is_cloud: false,
            saved_maximized_size: (0, 0),
            saved_maximized_virtual: (0.0, 0.0),
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, border_width: 0, cloud_position_default: None, desktop_grid_scale: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
        let mut c = ctx();
        c.pan_x = 50.0;
        c.pan_y = 100.0;
        c.zoom = 2.0;
        let placement = place_normal_window(&snap, &p, &c);
        assert_eq!(placement.pos, (100, 200));
        assert_eq!(placement.scale, 2.0);
        assert_eq!(placement.size, (640, 480));
        assert_eq!(placement.hidden, Some(false));

        // Pan far enough away and the window is culled.
        c.pan_x = 5000.0;
        let placement = place_normal_window(&snap, &p, &c);
        assert_eq!(placement.hidden, Some(true));

        // Interactive resize dimensions override stored geometry.
        let resizing = NormalSnapshot { active_resize: Some((800, 600)), ..snap };
        c.pan_x = 50.0;
        let placement = place_normal_window(&resizing, &p, &c);
        assert_eq!(placement.size, (800, 600));
    }

    #[test]
    fn opacity_policy() {
        assert_eq!(window_opacity(true, true, OVERLAY_UNFOCUSED_OPACITY), 1.0);
        assert_eq!(window_opacity(false, false, OVERLAY_UNFOCUSED_OPACITY), 1.0);
        assert_eq!(window_opacity(false, true, OVERLAY_UNFOCUSED_OPACITY), 0.85);
        assert_eq!(window_opacity(false, true, NORMAL_UNFOCUSED_OPACITY), 0.90);
    }

    fn snap(app_id: &str) -> WindowSnapshot {
        WindowSnapshot {
            app_id: Some(app_id.to_string()),
            title: None,
            role: WindowRole::from_app_id(Some(app_id)),
            minimized: false,
            closing_or_init: false,
            mode: TilingMode::Floating,
            rule_ssd: None,
            being_moved: false,
            status_edge: StatusEdge::Unspecified,
            is_focused: false,
            box_geom: Rect { x: 0, y: 0, width: 640, height: 480 },
            min_size: (0, 0),
            virtual_pos: (100.0, 200.0),
            active_resize: None,
            ssd: true,
            decorations_size: (0, 16),
            was_maximized: false,
            saved_maximized_size: (0, 0),
            saved_maximized_virtual: (0.0, 0.0),
        }
    }

    fn arrange_params() -> ArrangeParams {
        ArrangeParams {
            bar_height: 30,
            status_hide_mode: false,
            hide_mode_preview: 5,
            status_blur: true,
            window_blur: true,
            opacity_enabled: true,
            decoration: DecorationSpec {
                border_width: 0,
                border_color: crate::api::Rgba([0.1, 0.2, 0.3, 0.4]),
                corner_radius: 0,
            },
            border_color_focused: crate::api::Rgba([0.9, 0.1, 0.1, 1.0]),
            overlay: OverlayParams {
                overlay_width: 400,
                border_gap: 8,
                border_width: 0,
                position_right: true,
                cloud_position_default: None,
            },
            normal: NormalParams {
                gap_right: 10,
                gap_top: 6,
                border_width: 0,
                cloud_position_default: None,
                desktop_grid_scale: 100.0,
                desktop_gap_width: 0.0,
                desktop_cell_inset: 0.0,
            },
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
        }
    }

    fn one_output() -> Vec<OutputSnapshot> {
        vec![OutputSnapshot {
            layout_box: Rect { x: 0, y: 0, width: 1920, height: 1080 },
            non_exclusive: Rect { x: 0, y: 0, width: 0, height: 0 },
        }]
    }

    #[test]
    fn arrange_background_covers_output_and_disables_fallback_rect() {
        let windows = vec![snap("cce-wallpaper"), snap("firefox")];
        let plan = arrange(&windows, &one_output(), &arrange_params());

        assert!(!plan.background_rect_enabled);
        let wp = &plan.windows[0];
        assert_eq!(wp.tiling_mode, Some(TilingMode::Status));
        assert_eq!(wp.tiled, Some(0));
        assert_eq!(wp.ssd, Some(false));
        assert_eq!(wp.scene_enabled, Some(true));
        assert_eq!(wp.hidden, Some(false));
        assert_eq!(wp.blur, Some(false));
        assert_eq!(wp.pos, Some((0, 0)));
        assert_eq!(wp.size, Some((1920, 1080)));

        // Without a wallpaper window, the fallback rect stays on.
        let plan = arrange(&windows[1..], &one_output(), &arrange_params());
        assert!(plan.background_rect_enabled);
    }

    #[test]
    fn arrange_normal_window_pans_with_border_blur_opacity() {
        let mut w = snap("firefox");
        w.is_focused = false;
        let plan = arrange(&[w], &one_output(), &arrange_params());

        let wp = &plan.windows[0];
        assert_eq!(wp.scene_enabled, Some(true));
        assert_eq!(wp.tiling_mode, Some(TilingMode::Floating));
        assert_eq!(wp.pos, Some((100, 200)));
        assert_eq!(wp.scale, Some(1.0));
        assert_eq!(wp.size, Some((640, 480)));
        assert_eq!(wp.hidden, Some(false));
        assert_eq!(wp.tiled, None);
        assert_eq!(wp.decoration, Some(arrange_params().decoration));
        assert_eq!(wp.blur, Some(true));
        assert_eq!(wp.opacity, Some(NORMAL_UNFOCUSED_OPACITY));
    }

    #[test]
    fn arrange_focused_window_gets_focused_border_color() {
        let mut focused = snap("firefox");
        focused.is_focused = true;
        let unfocused = snap("terminal");
        let p = arrange_params();
        let plan = arrange(&[focused, unfocused], &one_output(), &p);

        assert_eq!(plan.windows[0].decoration.unwrap().border_color, p.border_color_focused);
        assert_eq!(plan.windows[1].decoration.unwrap().border_color, p.decoration.border_color);
    }

    #[test]
    fn arrange_fullscreen_window_gets_no_border() {
        let mut w = snap("mpv");
        w.mode = TilingMode::Fullscreen;
        let mut p = arrange_params();
        p.decoration.border_width = 4;
        p.decoration.corner_radius = 12;
        let plan = arrange(&[w], &one_output(), &p);

        let dec = plan.windows[0].decoration.unwrap();
        assert_eq!(dec.border_width, 0);
        assert_eq!(dec.corner_radius, 0);
    }

    #[test]
    fn arrange_hidden_window_disabled_in_scene() {
        let mut w = snap("firefox");
        w.minimized = true;
        let plan = arrange(&[w], &one_output(), &arrange_params());

        let wp = &plan.windows[0];
        assert_eq!(wp.scene_enabled, Some(false));
        assert_eq!(wp.hidden, Some(true));
        assert_eq!(wp.pos, None);
        assert_eq!(wp.size, None);
    }

    #[test]
    fn arrange_first_overlay_gets_slot_second_demotes_to_normal() {
        let mut first = snap("scratchpad");
        first.mode = TilingMode::Overlay;
        first.box_geom = Rect { x: 0, y: 0, width: 0, height: 0 };
        let mut second = snap("other-overlay");
        second.mode = TilingMode::Overlay;

        let plan = arrange(&[first, second], &one_output(), &arrange_params());

        // Fresh overlay: right slot (no bars, so the full output is usable),
        // box_geom written back.
        let wp = &plan.windows[0];
        assert_eq!(wp.pos, Some((1512, 24)));
        assert_eq!(wp.size, Some((400, 1048)));
        assert_eq!(wp.box_geom, Some(Rect { x: 1512, y: 24, width: 400, height: 1048 }));
        assert_eq!(wp.tiled, Some(15));
        assert_eq!(wp.opacity, Some(OVERLAY_UNFOCUSED_OPACITY));

        // Second overlay arranges as a pannable normal window.
        let wp = &plan.windows[1];
        assert_eq!(wp.tiling_mode, Some(TilingMode::Overlay));
        assert_eq!(wp.pos, Some((100, 200)));
        assert_eq!(wp.size, Some((640, 480)));
        assert_eq!(wp.opacity, Some(NORMAL_UNFOCUSED_OPACITY));
    }

    #[test]
    fn arrange_rule_ssd_reaches_overlay_sizing() {
        let mut w = snap("scratchpad");
        w.mode = TilingMode::Overlay;
        w.ssd = true;
        w.rule_ssd = Some(false);
        w.decorations_size = (2, 18);

        let plan = arrange(&[w], &one_output(), &arrange_params());
        let wp = &plan.windows[0];
        // The rule override is planned and the client size shrinks by the
        // decorations, proving the override was visible to placement.
        assert_eq!(wp.ssd, Some(false));
        assert_eq!(wp.size, Some((638, 462)));
    }

    #[test]
    fn arrange_maximized_enter_saves_geometry() {
        let mut w = snap("firefox");
        w.mode = TilingMode::Maximized;
        w.box_geom = Rect { x: 0, y: 0, width: 150, height: 50 };
        w.virtual_pos = (150.0, 120.0);

        let plan = arrange(&[w], &one_output(), &arrange_params());
        let wp = &plan.windows[0];
        assert_eq!(wp.was_maximized, Some(true));
        assert_eq!(wp.saved_maximized, Some(((150, 50), (150.0, 120.0))));
        // Grid snap: spans columns 1-2, row 1 of the 100px grid.
        assert_eq!(wp.virtual_pos, Some((100.0, 100.0)));
        assert_eq!(wp.pos, Some((100, 100)));
        assert_eq!(wp.size, Some((200, 100)));
    }

    #[test]
    fn arrange_maximized_exit_restores_saved_geometry() {
        let mut w = snap("firefox");
        w.mode = TilingMode::Floating;
        w.was_maximized = true;
        w.saved_maximized_size = (500, 400);
        w.saved_maximized_virtual = (10.0, 20.0);

        let plan = arrange(&[w], &one_output(), &arrange_params());
        let wp = &plan.windows[0];
        assert_eq!(wp.was_maximized, Some(false));
        assert_eq!(wp.box_geom, Some(Rect { x: 0, y: 0, width: 500, height: 400 }));
        // The restored geometry flows into the pannable placement.
        assert_eq!(wp.virtual_pos, Some((10.0, 20.0)));
        assert_eq!(wp.pos, Some((10, 20)));
        assert_eq!(wp.size, Some((500, 400)));
    }

    #[test]
    fn arrange_status_bar_placed_unless_dragged() {
        let mut bar = snap("cce-status-left-viewport");
        bar.status_edge = StatusEdge::TopLeft;
        bar.box_geom = Rect { x: 0, y: 0, width: 200, height: 30 };

        let plan = arrange(&[bar.clone()], &one_output(), &arrange_params());
        let wp = &plan.windows[0];
        assert_eq!(wp.tiling_mode, Some(TilingMode::Status));
        assert_eq!(wp.blur, Some(true));
        assert_eq!(wp.pos, Some((12, 0)));
        assert_eq!(wp.size, Some((200, 30)));

        // A dragged bar keeps whatever geometry it has: no placement writes.
        bar.being_moved = true;
        let plan = arrange(&[bar], &one_output(), &arrange_params());
        let wp = &plan.windows[0];
        assert_eq!(wp.pos, None);
        assert_eq!(wp.size, None);
    }

    #[test]
    fn arrange_multi_output_is_last_wins() {
        let outputs = vec![
            OutputSnapshot {
                layout_box: Rect { x: 0, y: 0, width: 1920, height: 1080 },
                non_exclusive: Rect { x: 0, y: 0, width: 0, height: 0 },
            },
            OutputSnapshot {
                layout_box: Rect { x: 1920, y: 0, width: 1280, height: 720 },
                non_exclusive: Rect { x: 0, y: 0, width: 0, height: 0 },
            },
        ];
        let windows = vec![snap("cce-wallpaper"), snap("firefox")];
        let plan = arrange(&windows, &outputs, &arrange_params());

        // The background covers the second output — the last pass wins.
        assert_eq!(plan.windows[0].pos, Some((1920, 0)));
        assert_eq!(plan.windows[0].size, Some((1280, 720)));

        // Maximize state machine only fires once across passes: entering on
        // pass one must not re-enter (and re-save) on pass two.
        let mut w = snap("firefox");
        w.mode = TilingMode::Maximized;
        w.box_geom = Rect { x: 0, y: 0, width: 150, height: 50 };
        w.virtual_pos = (150.0, 120.0);
        let plan = arrange(&[w], &outputs, &arrange_params());
        // Saved from the original geometry, not the pass-one grid snap.
        assert_eq!(plan.windows[0].saved_maximized, Some(((150, 50), (150.0, 120.0))));
    }

    #[test]
    fn side_stacks_center_vertically() {
        let items = vec![
            item("cce-status-a", StatusEdge::Left, 200),
            item("cce-status-b", StatusEdge::Left, 100),
        ];
        let p = layout_status_bars(&items, &params());
        // Total stack: 200 + 12 + 100 = 312, centered in 1080 → starts at 384.
        assert_eq!(p[0], Some(StatusBarPlacement { x: 0, y: 384, width: 30, height: 200 }));
        assert_eq!(p[1], Some(StatusBarPlacement { x: 0, y: 596, width: 30, height: 100 }));
    }
}
