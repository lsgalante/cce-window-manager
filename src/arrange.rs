// Pure layout computation extracted from `WindowManager::arrange_views()`.
//
// Functions here take plain-data snapshots and return placement plans; the
// mechanism side (`window_manager.rs`) builds the snapshots from FFI state and
// applies the plans to the scene graph. `arrange()` is the whole frame in one
// call — the future `Policy::arrange` entry point — composed from the
// per-section functions below it.

use super::api::{DecorationSpec, Rect, WindowRole};
use super::tiling::TilingMode;

/// `CCE_ARRANGE_DEBUG=1` — status-bar placement tracing. These sites were
/// `info!`, so they wrote on every arrange regardless of log level, and a
/// status-bar commit runs an arrange every second. Mirrors the mechanism-side
/// switch of the same name in the compositor's `window_manager.rs`.
fn arrange_debug() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CCE_ARRANGE_DEBUG").is_some())
}

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
    /// The segment's length along the bar axis. While a segment is expanded
    /// (see [`Self::expanded`]) the mechanism supplies the FROZEN collapsed
    /// length, so the slot the segment occupies — and every neighbor —
    /// stays put while the surface itself grows.
    pub prev_len: i32,
    /// The segment's committed thickness (perpendicular to the bar axis)
    /// exceeds the bar height: the client has grown its surface into an
    /// in-surface menu. The layout keeps the segment's slot but stops
    /// enforcing its size ([`StatusBarPlacement::enforce_size`]).
    pub expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatusBarPlacement {
    pub x: i32,
    pub y: i32,
    /// Slot size. Only applied when [`Self::enforce_size`] is set.
    pub width: u32,
    pub height: u32,
    /// False for an expanded segment: position it, but leave its size to the
    /// client (the surface currently IS an open menu; configuring it back to
    /// the slot size would fight the client every commit).
    pub enforce_size: bool,
}

pub struct StatusBarLayoutParams {
    /// The output's layout box.
    pub output: Rect,
    pub bar_height: u32,
    pub hide_mode: bool,
    /// Pixels of bar left peeking when hide_mode pushes top bars offscreen.
    pub hide_mode_preview: i32,
    /// Gap between adjacent status segments (the bar's `module { spacing }`;
    /// [`DEFAULT_STATUS_MODULE_SPACING`] when unconfigured).
    pub spacing: i32,
    /// Local time as a fraction of the day (0 = midnight, 0.5 = noon).
    /// Some(_) sends the light_source segment traveling the screen
    /// perimeter — noon at top-center, counterclockwise (the sun's path:
    /// morning up the right edge, evening down the left), midnight at
    /// bottom-center. None keeps it in its configured edge group.
    pub day_fraction: Option<f64>,
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
    /// The world-anchored grid client (role `Grid`): placed at its patch's
    /// virtual origin with scale `zoom / patch.scale`.
    Grid,
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
        WindowRole::Grid => {
            if closing_or_init {
                WindowClass::Hidden
            } else {
                WindowClass::Grid
            }
        }
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

/// Opacity of an unfocused window's scene tree. Both are 1.0 — the same as
/// focused — since 2026-09-03. They used to be 0.85 (overlay) and 0.90
/// (normal), and on a translucent backplate that dimming did not read as
/// "dimmer": it thinned the plate's tint, so the blurred backdrop showed
/// through with more contrast and the window looked LESS frosted than the
/// focused one, which then appeared to gain blur on every focus click. The
/// blur radius never changed. Keep the knob (and `opacity_enabled`) so the
/// level can be re-tuned without re-plumbing.
pub const OVERLAY_UNFOCUSED_OPACITY: f32 = 1.0;
pub const NORMAL_UNFOCUSED_OPACITY: f32 = 1.0;

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
    /// Server-side border width. Placement no longer insets by it (the border
    /// overhangs the content box); it only sets a floor on the decoration
    /// strip height reserved above the overlay slot.
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
/// The fresh slot is content-aligned: the content box sits directly on the
/// configured gaps and the border, which draws outside it, overhangs them.
/// Stored geometry behaves the same way.
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
        sp_h = (ctx.usable.height - dec_h - 2 * g).max(1);

        sp_x = if p.position_right {
            ctx.usable.x + ctx.usable.width - sp_w - g
        } else {
            ctx.usable.x + g
        };
        sp_y = ctx.usable.y + dec_h + g;

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

/// State-machine step for entering/leaving Tiled mode. `Enter` tells the
/// mechanism to save the given restore size (plus the window's current virtual
/// position) and set `was_tiled`; `Exit` restores the saved geometry — the
/// client-unmaximize path. (A geometric demotion — dragging a tiled window
/// off-grid — clears `was_tiled` mechanism-side without an Exit, so the
/// window stays where it was dropped.) Leaving with an invalid saved size
/// does nothing (`was_tiled` stays set).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TiledTransition {
    Enter { width: i32, height: i32 },
    Exit { width: i32, height: i32, virtual_x: f64, virtual_y: f64 },
}

pub fn tiled_transition(
    is_tiled_mode: bool,
    was_tiled: bool,
    box_size: (i32, i32),
    min_size: (i32, i32),
    saved_size: (i32, i32),
    saved_virtual: (f64, f64),
) -> Option<TiledTransition> {
    if is_tiled_mode && !was_tiled {
        let mut w = box_size.0;
        let mut h = box_size.1;
        if w <= 0 {
            w = if min_size.0 > 32 { min_size.0 } else { 800 };
        }
        if h <= 0 {
            h = if min_size.1 > 32 { min_size.1 } else { 600 };
        }
        Some(TiledTransition::Enter { width: w, height: h })
    } else if !is_tiled_mode && was_tiled {
        if saved_size.0 > 0 && saved_size.1 > 0 {
            Some(TiledTransition::Exit {
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
}

pub struct NormalParams {
    pub gap_right: i32,
    pub gap_top: i32,
    pub cloud_position_default: Option<[i32; 2]>,
    /// Desktop grid cell width/height (each axis's period is cell + gap).
    pub desktop_cell_w: f64,
    pub desktop_cell_h: f64,
    /// Desktop grid gap between cells.
    pub desktop_gap_width: f64,
    /// Visual inset of a cell's edge (the fade inset); Tiled windows
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
    /// Tiled grid-snap moves the window's virtual position.
    pub virtual_write: Option<(f64, f64)>,
}

/// Place a non-overlay window according to its tiling mode: `Popup` docks to
/// the usable area's top-right (or the cloud default position), `Fullscreen`
/// covers the physical output, `Tiled` snaps to cover every desktop-grid
/// cell its current geometry touches, and everything else pans on the virtual
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

            // The dock slot is content-aligned: the content box sits on the
            // configured gaps and the border overhangs them. Explicit cloud
            // positions are taken as content positions verbatim.
            let (fx, fy) = if snap.is_cloud && p.cloud_position_default.is_some() {
                let pos = p.cloud_position_default.unwrap();
                (ctx.usable.x + pos[0], ctx.usable.y + pos[1])
            } else {
                (ctx.usable.x + ctx.usable.width - fw - p.gap_right, ctx.usable.y + p.gap_top)
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
        TilingMode::Tiled => {
            // Cover the VISIBLE edges of every desktop-grid cell the current
            // geometry touches — the same cell-edge geometry as interactive
            // grid snapping (snap::tiled_span). A tiled window's geometry
            // is already cell-aligned (that's what makes it tiled), so this
            // is normally the identity; it aligns a client-requested maximize
            // and absorbs drift.
            let x1 = snap.virtual_pos.0;
            let y1 = snap.virtual_pos.1;
            let x2 = x1 + snap.box_geom.width.max(1) as f64;
            let y2 = y1 + snap.box_geom.height.max(1) as f64;

            let (low_x, high_x) = crate::snap::tiled_span(
                x1, x2, p.desktop_cell_w, p.desktop_gap_width, p.desktop_cell_inset,
            );
            let (low_y, high_y) = crate::snap::tiled_span(
                y1, y2, p.desktop_cell_h, p.desktop_gap_width, p.desktop_cell_inset,
            );

            // The content fills the covered cells edge to edge; the border
            // draws outside it and overhangs into the grid gap.
            let content_x = low_x;
            let content_y = low_y;
            let fw = (high_x - low_x).max(1.0);
            let fh = (high_y - low_y).max(1.0);

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
            // A fresh floating spawn (no established box, no resize in
            // flight) gets the xdg "you choose" size 0x0 instead of a guess:
            // the client maps at its natural size and the acked-commit path
            // adopts that as the box. Guessing from the min-size hint locked
            // self-sizing clients (every cce-ui app) to their minimum — they
            // obey any nonzero configure, so the guess became the box forever.
            if matches!(snap.mode, TilingMode::Floating | TilingMode::Utility)
                && snap.active_resize.is_none()
                && snap.box_geom.width <= 0
                && snap.box_geom.height <= 0
            {
                let (final_x, final_y) = ctx.virtual_to_screen(snap.virtual_pos.0, snap.virtual_pos.1);
                return NormalPlacement {
                    pos: (final_x, final_y),
                    scale: ctx.zoom,
                    size: (0, 0),
                    tiled_all_edges: false,
                    // Unmapped until its first buffer; sizeless culling would
                    // be meaningless, so leave the flag untouched.
                    hidden: None,
                    virtual_write: None,
                };
            }

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

            // A Utility window's size is the client's alone: the plan restates
            // the "you choose" 0x0 on EVERY pass, so the compositor never
            // dictates a size to it — not from a restore, not after an output
            // or scale change (the client re-commits at the new scale and the
            // commit path adopts that as the box). The cull still uses the
            // real box; only the configure size is withheld.
            let size = if snap.mode == TilingMode::Utility {
                (0, 0)
            } else {
                (fw as u32, fh as u32)
            };

            NormalPlacement {
                pos: (final_x, final_y),
                scale: ctx.zoom,
                size,
                tiled_all_edges: false,
                hidden: Some(ctx.is_offscreen(final_x, final_y, fw as f64 * ctx.zoom, fh as f64 * ctx.zoom)),
                virtual_write: None,
            }
        }
    }
}

/// Default gap between adjacent status segments — the fallback for
/// [`StatusBarLayoutParams::spacing`] / [`ArrangeParams::status_module_spacing`].
pub const DEFAULT_STATUS_MODULE_SPACING: i32 = 12;
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

fn is_light_source(app_id: &str) -> bool {
    app_id.ends_with("light_source")
}

/// A point on the output-rect perimeter for the traveling light_source
/// segment, plus the edge it lies on (0 top, 1 left, 2 bottom, 3 right).
/// `day` is the local day fraction (0 = midnight); the path runs
/// counterclockwise from top-center at noon.
fn sun_perimeter_point(output: Rect, day: f64) -> (f64, f64, u8) {
    let bw = output.width as f64;
    let bh = output.height as f64;
    // Perimeter fraction from top-center: noon → 0, 18:00 → ¼ (mid left
    // edge), midnight → ½ (bottom center), 06:00 → ¾ (mid right edge).
    let f = (day + 0.5).fract();
    let mut s = f * 2.0 * (bw + bh);
    if s < bw / 2.0 {
        return (bw / 2.0 - s, 0.0, 0);
    }
    s -= bw / 2.0;
    if s < bh {
        return (0.0, s, 1);
    }
    s -= bh;
    if s < bw {
        return (s, bh, 2);
    }
    s -= bw;
    if s < bh {
        return (bw, bh - s, 3);
    }
    s -= bh;
    (bw - s, 0.0, 0)
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
    let spacing = p.spacing;
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
    let mut sun_track: Vec<usize> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        // The light_source segment ignores edge groups: it travels the
        // screen perimeter with the time of day (see sun_perimeter_point).
        if p.day_fraction.is_some() && is_light_source(&item.app_id) {
            sun_track.push(idx);
            continue;
        }
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

    if arrange_debug() {
        log::debug!("[ArrangeStatus] top_left_len={}, top_center_len={}, top_right_len={}, left_side_len={}", top_left.len(), top_center.len(), top_right.len(), left_side.len());
    }

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
        if arrange_debug() {
            log::debug!("[TopLeftLayout] app_id={} x={}, w={}", items[i].app_id, cur_left_x, w);
        }
        placements[i] = Some(StatusBarPlacement { x: cur_left_x, y: status_y_top, width: w, height: bar_h, enforce_size: !items[i].expanded });
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
        placements[i] = Some(StatusBarPlacement { x: cur_center_x, y: status_y_top, width: w, height: bar_h, enforce_size: !items[i].expanded });
        cur_center_x += w as i32 + spacing;
    }

    // 1c. Right Group (TopRight / ne)
    let mut cur_right_x = wlr_box.x + wlr_box.width - margin;
    for &i in top_right.iter().rev() {
        let w = bar_len(items[i].prev_len);
        let x = cur_right_x - w as i32;
        placements[i] = Some(StatusBarPlacement { x, y: status_y_top, width: w, height: bar_h, enforce_size: !items[i].expanded });
        cur_right_x = x - spacing;
    }

    for &i in &full_top {
        placements[i] = Some(StatusBarPlacement { x: wlr_box.x, y: status_y_top, width: wlr_box.width as u32, height: bar_h, enforce_size: !items[i].expanded });
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
        placements[i] = Some(StatusBarPlacement { x: cur_left_x, y: status_y_bottom, width: w, height: bar_h, enforce_size: !items[i].expanded });
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
        placements[i] = Some(StatusBarPlacement { x: cur_center_x, y: status_y_bottom, width: w, height: bar_h, enforce_size: !items[i].expanded });
        cur_center_x += w as i32 + spacing;
    }

    // 2c. Right Group (BottomRight / se)
    let mut cur_right_x = wlr_box.x + wlr_box.width - margin;
    for &i in bottom_right.iter().rev() {
        let w = bar_len(items[i].prev_len);
        let x = cur_right_x - w as i32;
        placements[i] = Some(StatusBarPlacement { x, y: status_y_bottom, width: w, height: bar_h, enforce_size: !items[i].expanded });
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
        placements[i] = Some(StatusBarPlacement { x: wlr_box.x, y: cur_left_y, width: bar_h, height: actual_h, enforce_size: !items[i].expanded });
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
        placements[i] = Some(StatusBarPlacement { x: wlr_box.x + wlr_box.width - bar_h as i32, y: cur_right_y, width: bar_h, height: actual_h, enforce_size: !items[i].expanded });
        cur_right_y += actual_h as i32 + spacing;
    }

    // 4. The traveling light_source segment: center the (always-horizontal)
    // segment box on the day-fraction perimeter point, clamped inside the
    // output so the corners are turned smoothly. In hide mode it slips off
    // its current edge like every other segment, leaving the same preview.
    if let Some(day) = p.day_fraction {
        for &i in &sun_track {
            let w = bar_len(items[i].prev_len) ;
            let (px, py, edge) = sun_perimeter_point(wlr_box, day);
            let mut x = (wlr_box.x as f64 + px - w as f64 / 2.0).round() as i32;
            let mut y = (wlr_box.y as f64 + py - bar_h as f64 / 2.0).round() as i32;
            x = x.clamp(wlr_box.x, wlr_box.x + wlr_box.width - w as i32);
            y = y.clamp(wlr_box.y, wlr_box.y + wlr_box.height - bar_h as i32);
            if p.hide_mode {
                match edge {
                    0 => y = wlr_box.y - (bar_h as i32 - p.hide_mode_preview),
                    1 => x = wlr_box.x - (w as i32 - p.hide_mode_preview),
                    2 => y = wlr_box.y + wlr_box.height - p.hide_mode_preview,
                    _ => x = wlr_box.x + wlr_box.width - p.hide_mode_preview,
                }
            }
            placements[i] = Some(StatusBarPlacement { x, y, width: w, height: bar_h, enforce_size: !items[i].expanded });
        }
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
    /// Status segments only: the along-bar length last committed while the
    /// segment was at bar thickness, tracked by the mechanism. Keeps the
    /// segment's slot stable while it is EXPANDED (surface grown into an
    /// in-surface menu); 0 when never collapsed-committed (fall back to the
    /// live box).
    pub status_collapsed_len: i32,
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
    pub was_tiled: bool,
    pub saved_floating_size: (i32, i32),
    pub saved_floating_virtual: (f64, f64),
    /// Grid-role windows only: the latched world-anchored patch the current
    /// buffer covers. `None` until the first rendered patch arrives (the
    /// window stays out of the scene until then).
    pub grid_patch: Option<crate::api::GridPatch>,
}

/// Frame-wide inputs: config knobs plus the desktop viewport.
pub struct ArrangeParams {
    pub bar_height: i32,
    pub status_hide_mode: bool,
    pub hide_mode_preview: i32,
    /// Gap between adjacent status segments (see `StatusBarLayoutParams::spacing`).
    pub status_module_spacing: i32,
    /// Local day fraction for the traveling light_source segment (see
    /// `StatusBarLayoutParams::day_fraction`).
    pub day_fraction: Option<f64>,
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
    pub was_tiled: Option<bool>,
    /// Tiled-enter save: (restore size, restore virtual position).
    pub saved_floating: Option<((i32, i32), (f64, f64))>,
}

pub struct ArrangePlan {
    /// Index-aligned with the input snapshots.
    pub windows: Vec<WindowPlan>,
    /// Whether each output's fallback background rect should be shown
    /// (false while a wallpaper window exists).
    pub background_rect_enabled: bool,
    /// Whether the compositor-drawn cell lattice should be shown: false
    /// while a live grid client (role Grid, mapped, with a latched patch)
    /// covers the desktop. The gap-colored backdrop stays either way.
    pub grid_cells_enabled: bool,
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
/// virtual position, SSD overrides, the tiled save/restore machine) is
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
    let has_grid_client = state.iter().any(|w| {
        w.role == WindowRole::Grid && !w.closing_or_init && !w.minimized && w.grid_patch.is_some()
    });

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
                WindowClass::Grid => {
                    match w.grid_patch {
                        Some(patch) if patch.scale > 0.0 => {
                            wp.scene_enabled = Some(true);
                            wp.hidden = Some(false);
                            wp.tiled = Some(0);
                            wp.ssd = Some(false);
                            w.ssd = false;
                            let (sx, sy) = ctx.virtual_to_screen(patch.x, patch.y);
                            wp.pos = Some((sx, sy));
                            // The buffer holds patch.scale px per virtual
                            // unit; displaying it at zoom/patch.scale puts
                            // it in per-frame lockstep with window content.
                            wp.scale = Some(ctx.zoom / patch.scale);
                            // The content box IS the buffer: dest sizing and
                            // culling read box_geom, which nothing else
                            // maintains for a window the compositor never
                            // configures.
                            wp.box_geom = Some(Rect {
                                x: sx,
                                y: sy,
                                width: (patch.w * patch.scale).round() as i32,
                                height: (patch.h * patch.scale).round() as i32,
                            });
                            wp.blur = Some(false);
                            wp.opacity = Some(1.0);
                        }
                        _ => {
                            // No rendered patch yet: keep it out of the
                            // scene — no flash of an unanchored buffer. The
                            // xdg "you choose" size unblocks the map state
                            // machine (a window with NO planned dimensions
                            // can never leave Ready — same trick as Utility)
                            // but is planned ONLY in this pre-patch phase:
                            // once a patch is latched the client owns its
                            // size, and re-sending 0x0 per arrange bounced
                            // the surface between the settings size and the
                            // patch size — a swapchain-thrash that ate GBs.
                            wp.size = Some((0, 0));
                            wp.scene_enabled = Some(false);
                            wp.hidden = Some(true);
                        }
                    }
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

        // Manage entering/exiting Tiled state for normal windows.
        for &i in &normal_windows {
            let w = &state[i];
            let transition = tiled_transition(
                w.mode == TilingMode::Tiled,
                w.was_tiled,
                (w.box_geom.width, w.box_geom.height),
                w.min_size,
                w.saved_floating_size,
                w.saved_floating_virtual,
            );
            match transition {
                Some(TiledTransition::Enter { width, height }) => {
                    let w = &mut state[i];
                    w.saved_floating_size = (width, height);
                    w.saved_floating_virtual = w.virtual_pos;
                    w.was_tiled = true;
                    let wp = &mut plan[i];
                    wp.saved_floating = Some(((width, height), w.saved_floating_virtual));
                    wp.was_tiled = Some(true);
                    log::info!("[Tiled] Saved window {:?} geometry: {}x{} at ({}, {})",
                        w.title.as_deref().unwrap_or(""),
                        width, height,
                        w.saved_floating_virtual.0, w.saved_floating_virtual.1
                    );
                }
                Some(TiledTransition::Exit { width, height, virtual_x, virtual_y }) => {
                    let w = &mut state[i];
                    w.box_geom.width = width;
                    w.box_geom.height = height;
                    w.virtual_pos = (virtual_x, virtual_y);
                    w.was_tiled = false;
                    let wp = &mut plan[i];
                    wp.box_geom = Some(w.box_geom);
                    wp.virtual_pos = Some((virtual_x, virtual_y));
                    wp.was_tiled = Some(false);
                    wp.size = Some((width as u32, height as u32));
                    log::info!("[Tiled] Restored window {:?} geometry: {}x{} at ({}, {})",
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
                    if arrange_debug() {
                        log::debug!("[ArrangeStatus] app_id={} status_edge={:?}", app_id, w.status_edge);
                    }
                    // Thickness = the axis perpendicular to the segment's
                    // edge; a segment thicker than the bar has grown an
                    // in-surface menu (expanded) and keeps its FROZEN
                    // collapsed slot length instead of the live box.
                    let (len, thickness) = match w.status_edge {
                        StatusEdge::Left | StatusEdge::Right => (w.box_geom.height, w.box_geom.width),
                        _ => (w.box_geom.width, w.box_geom.height),
                    };
                    let expanded = thickness > p.bar_height && w.status_collapsed_len > 0;
                    status_items.push(StatusBarItem {
                        app_id: app_id.clone(),
                        edge: w.status_edge,
                        prev_len: if expanded { w.status_collapsed_len } else { len },
                        expanded,
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
                spacing: p.status_module_spacing,
                day_fraction: p.day_fraction,
            },
        );

        for (&i, placement) in status_idxs.iter().zip(placements.iter()) {
            if let Some(pl) = placement {
                plan[i].pos = Some((pl.x, pl.y));
                // An expanded segment keeps client-owned sizing: scheduling
                // the slot size would fight the open menu every commit.
                plan[i].size = pl.enforce_size.then_some((pl.width, pl.height));
            }
        }
    }

    ArrangePlan {
        windows: plan,
        background_rect_enabled: !has_wallpaper,
        grid_cells_enabled: !has_grid_client,
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
            spacing: DEFAULT_STATUS_MODULE_SPACING,
            day_fraction: None,
        }
    }

    fn item(app_id: &str, edge: StatusEdge, prev_len: i32) -> StatusBarItem {
        StatusBarItem { app_id: app_id.to_string(), edge, prev_len, expanded: false }
    }

    #[test]
    fn light_source_travels_the_perimeter() {
        // 1920x1080 output, bar 30, segment len 36. Noon → top center,
        // 18:00 → mid left edge, midnight → bottom center, 06:00 → mid
        // right edge; counterclockwise in between.
        let items = vec![item("cce-status-left-light_source", StatusEdge::TopLeft, 36)];
        let mut p = params();

        p.day_fraction = Some(0.5); // noon
        let pl = layout_status_bars(&items, &p)[0].unwrap();
        assert_eq!((pl.x, pl.y), (1920 / 2 - 18, 0));

        p.day_fraction = Some(0.75); // 18:00 — mid left edge
        let pl = layout_status_bars(&items, &p)[0].unwrap();
        assert_eq!((pl.x, pl.y), (0, 1080 / 2 - 15));

        p.day_fraction = Some(0.0); // midnight — bottom center
        let pl = layout_status_bars(&items, &p)[0].unwrap();
        assert_eq!((pl.x, pl.y), (1920 / 2 - 18, 1080 - 30));

        p.day_fraction = Some(0.25); // 06:00 — mid right edge
        let pl = layout_status_bars(&items, &p)[0].unwrap();
        assert_eq!((pl.x, pl.y), (1920 - 36, 1080 / 2 - 15));

        // Shortly after noon the segment is still on the top edge, left of
        // center (counterclockwise = leftward along the top).
        p.day_fraction = Some(0.51);
        let pl = layout_status_bars(&items, &p)[0].unwrap();
        assert_eq!(pl.y, 0);
        assert!(pl.x < 1920 / 2 - 18);

        // Without a day fraction it stays in its configured edge group.
        p.day_fraction = None;
        let pl = layout_status_bars(&items, &p)[0].unwrap();
        assert_eq!((pl.x, pl.y), (12, 0));
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
        assert_eq!(p[0], Some(StatusBarPlacement { x: 12, y: 0, width: 100, height: 30, enforce_size: true }));
        assert_eq!(p[1], Some(StatusBarPlacement { x: 124, y: 0, width: 100, height: 30, enforce_size: true }));
        // Right group is placed from the right edge inward.
        assert_eq!(p[2], Some(StatusBarPlacement { x: 1908 - 200, y: 0, width: 200, height: 30, enforce_size: true }));
    }

    #[test]
    fn expanded_segment_keeps_slot_and_client_size() {
        // battery expanded (in-surface menu open): its slot still consumes the
        // frozen collapsed length (100), so the neighbor (memory, left of it)
        // does not shift — and its placement stops enforcing size.
        let mut battery = item("cce-status-right-battery", StatusEdge::TopRight, 100);
        battery.expanded = true;
        let items = vec![
            item("cce-status-right-clock", StatusEdge::TopRight, 200),
            battery,
            item("cce-status-right-memory", StatusEdge::TopRight, 150),
        ];
        let p = layout_status_bars(&items, &params());
        // Right-to-left: clock at the edge, battery next, memory after —
        // identical x positions to the collapsed layout.
        assert_eq!(p[0].unwrap().x, 1908 - 200);
        let bat = p[1].unwrap();
        assert_eq!(bat.x, 1908 - 200 - 12 - 100);
        assert!(!bat.enforce_size, "expanded segment is position-only");
        let mem = p[2].unwrap();
        assert_eq!(mem.x, 1908 - 200 - 12 - 100 - 12 - 150);
        assert!(mem.enforce_size);
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
            classify_window(WindowRole::Normal, true, false, TilingMode::Floating, false),
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
            classify_window(WindowRole::Normal, false, false, TilingMode::Tiled, false),
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
    fn fresh_overlay_slot_ignores_border_width() {
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
        // Placement no longer insets by the border: the content box lands on
        // the gaps exactly as it does with no border, and the border overhangs
        // outward. (bw 4 < OVERLAY_DEC_H 16, so the decoration strip is
        // unaffected too.)
        assert_eq!(placement.pos, (1512, 54));
        assert_eq!(placement.size, (400, 1018));
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
    fn tiled_transitions() {
        // Entering with no usable geometry falls back to 800x600.
        assert_eq!(
            tiled_transition(true, false, (0, 0), (0, 0), (0, 0), (0.0, 0.0)),
            Some(TiledTransition::Enter { width: 800, height: 600 })
        );
        // Entering keeps real geometry.
        assert_eq!(
            tiled_transition(true, false, (640, 480), (0, 0), (0, 0), (0.0, 0.0)),
            Some(TiledTransition::Enter { width: 640, height: 480 })
        );
        // Steady states do nothing.
        assert_eq!(tiled_transition(true, true, (640, 480), (0, 0), (640, 480), (0.0, 0.0)), None);
        assert_eq!(tiled_transition(false, false, (640, 480), (0, 0), (0, 0), (0.0, 0.0)), None);
        // Exit restores the saved geometry; invalid saved size is a no-op.
        assert_eq!(
            tiled_transition(false, true, (0, 0), (0, 0), (640, 480), (10.0, 20.0)),
            Some(TiledTransition::Exit { width: 640, height: 480, virtual_x: 10.0, virtual_y: 20.0 })
        );
        assert_eq!(tiled_transition(false, true, (0, 0), (0, 0), (0, 480), (10.0, 20.0)), None);
    }

    #[test]
    fn tiled_snaps_to_grid_cells() {
        let snap = NormalSnapshot {
            mode: TilingMode::Tiled,
            box_geom: Rect { x: 0, y: 0, width: 100, height: 50 },
            min_size: (0, 0),
            virtual_pos: (150.0, 120.0),
            active_resize: None,
            is_cloud: false,
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, cloud_position_default: None, desktop_cell_w: 100.0, desktop_cell_h: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
        let placement = place_normal_window(&snap, &p, &ctx());
        // Current geometry spans grid columns 1-2 and row 1 → snapped to
        // (100,100) with size 200x100.
        assert_eq!(placement.virtual_write, Some((100.0, 100.0)));
        assert_eq!(placement.pos, (100, 100));
        assert_eq!(placement.size, (200, 100));
        assert_eq!(placement.hidden, Some(false));
        assert_eq!(placement.scale, 1.0);
    }

    #[test]
    fn tiled_ignores_border_width() {
        let snap = NormalSnapshot {
            mode: TilingMode::Tiled,
            box_geom: Rect { x: 0, y: 0, width: 100, height: 50 },
            min_size: (0, 0),
            virtual_pos: (150.0, 120.0),
            active_resize: None,
            is_cloud: false,
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, cloud_position_default: None, desktop_cell_w: 100.0, desktop_cell_h: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
        let placement = place_normal_window(&snap, &p, &ctx());
        // The content fills the covered cells (100,100)+200x100 exactly; the
        // border draws outside that and overhangs into the grid gap.
        assert_eq!(placement.virtual_write, Some((100.0, 100.0)));
        assert_eq!(placement.pos, (100, 100));
        assert_eq!(placement.size, (200, 100));
    }

    #[test]
    fn tiled_snaps_to_visible_cell_edges() {
        let snap = NormalSnapshot {
            mode: TilingMode::Tiled,
            box_geom: Rect { x: 0, y: 0, width: 100, height: 50 },
            min_size: (0, 0),
            virtual_pos: (150.0, 120.0),
            active_resize: None,
            is_cloud: false,
        };
        // period 110 (gap 10), inset 5: cells x 1-2 visibly span [115, 315],
        // row y 1 spans [115, 205]; the content fills them edge to edge.
        let p = NormalParams {
            gap_right: 10,
            gap_top: 6,
            cloud_position_default: None,
            desktop_cell_w: 100.0,
            desktop_cell_h: 100.0,
            desktop_gap_width: 10.0,
            desktop_cell_inset: 5.0,
        };
        let placement = place_normal_window(&snap, &p, &ctx());
        assert_eq!(placement.virtual_write, Some((115.0, 115.0)));
        assert_eq!(placement.pos, (115, 115));
        assert_eq!(placement.size, (200, 90));
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
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, cloud_position_default: None, desktop_cell_w: 100.0, desktop_cell_h: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
        let placement = place_normal_window(&snap, &p, &ctx());
        // Defaults to 360x100, docked inside the usable area (below the bar).
        assert_eq!(placement.pos, (1920 - 360 - 10, 30 + 6));
        assert_eq!(placement.size, (360, 100));
        assert_eq!(placement.hidden, None);
    }

    #[test]
    fn fresh_floating_window_gets_client_chosen_size() {
        // No established box, no resize in flight: the placement is the xdg
        // "you choose" 0x0, NOT the min-size hint — a self-sizing client
        // obeys any nonzero configure, so a min-size guess would become the
        // box forever.
        let snap = NormalSnapshot {
            mode: TilingMode::Floating,
            box_geom: Rect { x: 0, y: 0, width: 0, height: 0 },
            min_size: (320, 240),
            virtual_pos: (100.0, 200.0),
            active_resize: None,
            is_cloud: false,
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, cloud_position_default: None, desktop_cell_w: 100.0, desktop_cell_h: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
        let placement = place_normal_window(&snap, &p, &ctx());
        assert_eq!(placement.size, (0, 0));
        assert_eq!(placement.hidden, None);

        // Once the box is established (the acked commit adopted the client's
        // geometry), the placement keeps it.
        let established = NormalSnapshot {
            box_geom: Rect { x: 0, y: 0, width: 900, height: 700 },
            ..snap
        };
        let placement = place_normal_window(&established, &p, &ctx());
        assert_eq!(placement.size, (900, 700));

        // Non-floating modes keep the min-size fallback: their sizes are
        // dictated by tiling, not chosen by the client.
        let other = NormalSnapshot { mode: TilingMode::Overlay, ..established };
        let other = NormalSnapshot { box_geom: Rect { x: 0, y: 0, width: 0, height: 0 }, ..other };
        let placement = place_normal_window(&other, &p, &ctx());
        assert_eq!(placement.size, (320, 240));
    }

    #[test]
    fn utility_window_size_is_always_client_chosen() {
        // A Utility window restates the "you choose" 0x0 on EVERY pass — even
        // with an established box — so the compositor can never dictate a
        // size to it (a restored size, an output change). The established box
        // still drives the offscreen cull.
        let p = NormalParams { gap_right: 10, gap_top: 6, cloud_position_default: None, desktop_cell_w: 100.0, desktop_cell_h: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
        let fresh = NormalSnapshot {
            mode: TilingMode::Utility,
            box_geom: Rect { x: 0, y: 0, width: 0, height: 0 },
            min_size: (320, 240),
            virtual_pos: (100.0, 200.0),
            active_resize: None,
            is_cloud: false,
        };
        let placement = place_normal_window(&fresh, &p, &ctx());
        assert_eq!(placement.size, (0, 0));
        assert_eq!(placement.hidden, None);

        let established = NormalSnapshot {
            box_geom: Rect { x: 0, y: 0, width: 520, height: 896 },
            ..fresh
        };
        let placement = place_normal_window(&established, &p, &ctx());
        assert_eq!(placement.size, (0, 0));
        // The cull is computed (from the real box), unlike the unmapped case.
        assert!(placement.hidden.is_some());
    }

    #[test]
    fn pannable_window_follows_viewport() {
        let snap = NormalSnapshot {
            mode: TilingMode::Overlay,
            box_geom: Rect { x: 0, y: 0, width: 640, height: 480 },
            min_size: (0, 0),
            virtual_pos: (100.0, 200.0),
            active_resize: None,
            is_cloud: false,
        };
        let p = NormalParams { gap_right: 10, gap_top: 6, cloud_position_default: None, desktop_cell_w: 100.0, desktop_cell_h: 100.0, desktop_gap_width: 0.0, desktop_cell_inset: 0.0 };
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
        // Focused, or dimming disabled: always fully opaque.
        assert_eq!(window_opacity(true, true, 0.5), 1.0);
        assert_eq!(window_opacity(false, false, 0.5), 1.0);
        // Unfocused with dimming enabled: the given level passes through.
        assert_eq!(window_opacity(false, true, 0.5), 0.5);
        // The DE levels: an unfocused window looks exactly like a focused one.
        assert_eq!(window_opacity(false, true, OVERLAY_UNFOCUSED_OPACITY), 1.0);
        assert_eq!(window_opacity(false, true, NORMAL_UNFOCUSED_OPACITY), 1.0);
    }

    fn snap(app_id: &str) -> WindowSnapshot {
        WindowSnapshot {
            app_id: Some(app_id.to_string()),
            title: None,
            role: WindowRole::from_app_id(Some(app_id)),
            minimized: false,
            closing_or_init: false,
            mode: TilingMode::Floating,
            status_collapsed_len: 0,
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
            was_tiled: false,
            saved_floating_size: (0, 0),
            saved_floating_virtual: (0.0, 0.0),
            grid_patch: None,
        }
    }

    fn arrange_params() -> ArrangeParams {
        ArrangeParams {
            bar_height: 30,
            status_hide_mode: false,
            hide_mode_preview: 5,
            status_module_spacing: DEFAULT_STATUS_MODULE_SPACING,
            day_fraction: None,
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
                cloud_position_default: None,
                desktop_cell_w: 100.0,
            desktop_cell_h: 100.0,
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
    fn arrange_tiled_enter_saves_geometry() {
        let mut w = snap("firefox");
        w.mode = TilingMode::Tiled;
        w.box_geom = Rect { x: 0, y: 0, width: 150, height: 50 };
        w.virtual_pos = (150.0, 120.0);

        let plan = arrange(&[w], &one_output(), &arrange_params());
        let wp = &plan.windows[0];
        assert_eq!(wp.was_tiled, Some(true));
        assert_eq!(wp.saved_floating, Some(((150, 50), (150.0, 120.0))));
        // Grid snap: spans columns 1-2, row 1 of the 100px grid.
        assert_eq!(wp.virtual_pos, Some((100.0, 100.0)));
        assert_eq!(wp.pos, Some((100, 100)));
        assert_eq!(wp.size, Some((200, 100)));
    }

    #[test]
    fn arrange_tiled_exit_restores_saved_geometry() {
        let mut w = snap("firefox");
        w.mode = TilingMode::Floating;
        w.was_tiled = true;
        w.saved_floating_size = (500, 400);
        w.saved_floating_virtual = (10.0, 20.0);

        let plan = arrange(&[w], &one_output(), &arrange_params());
        let wp = &plan.windows[0];
        assert_eq!(wp.was_tiled, Some(false));
        assert_eq!(wp.box_geom, Some(Rect { x: 0, y: 0, width: 500, height: 400 }));
        // The restored geometry flows into the pannable placement.
        assert_eq!(wp.virtual_pos, Some((10.0, 20.0)));
        assert_eq!(wp.pos, Some((10, 20)));
        assert_eq!(wp.size, Some((500, 400)));
    }

    #[test]
    fn arrange_grid_client_world_anchored() {
        let mut g = snap("cce-grid");
        assert_eq!(g.role, WindowRole::Grid);
        // No patch yet: hidden, and the compositor cells stay on.
        let plan = arrange(&[g.clone()], &one_output(), &arrange_params());
        assert_eq!(plan.windows[0].scene_enabled, Some(false));
        assert!(plan.grid_cells_enabled);

        // With a latched patch: placed at the patch's virtual origin, scaled
        // by zoom/patch.scale, and the compositor cells yield.
        g.grid_patch = Some(crate::api::GridPatch {
            x: -1000.0,
            y: 500.0,
            w: 4000.0,
            h: 3000.0,
            scale: 0.5,
        });
        let mut p = arrange_params();
        p.pan_x = -1500.0;
        p.pan_y = 0.0;
        p.zoom = 1.0;
        let plan = arrange(&[g.clone()], &one_output(), &p);
        let wp = &plan.windows[0];
        assert_eq!(wp.scene_enabled, Some(true));
        // Screen pos = (virtual - pan) * zoom: (-1000 - -1500, 500 - 0).
        assert_eq!(wp.pos, Some((500, 500)));
        // Buffer at 0.5 px per unit shown at zoom 1 → display scale 2.
        assert_eq!(wp.scale, Some(2.0));
        assert_eq!(wp.tiled, Some(0));
        assert!(!plan.grid_cells_enabled);

        // A minimized or closing grid client gives the cells back.
        g.minimized = true;
        let plan = arrange(&[g], &one_output(), &p);
        assert!(plan.grid_cells_enabled);
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

        // Tiled state machine only fires once across passes: entering on
        // pass one must not re-enter (and re-save) on pass two.
        let mut w = snap("firefox");
        w.mode = TilingMode::Tiled;
        w.box_geom = Rect { x: 0, y: 0, width: 150, height: 50 };
        w.virtual_pos = (150.0, 120.0);
        let plan = arrange(&[w], &outputs, &arrange_params());
        // Saved from the original geometry, not the pass-one grid snap.
        assert_eq!(plan.windows[0].saved_floating, Some(((150, 50), (150.0, 120.0))));
    }

    #[test]
    fn side_stacks_center_vertically() {
        let items = vec![
            item("cce-status-a", StatusEdge::Left, 200),
            item("cce-status-b", StatusEdge::Left, 100),
        ];
        let p = layout_status_bars(&items, &params());
        // Total stack: 200 + 12 + 100 = 312, centered in 1080 → starts at 384.
        assert_eq!(p[0], Some(StatusBarPlacement { x: 0, y: 384, width: 30, height: 200, enforce_size: true }));
        assert_eq!(p[1], Some(StatusBarPlacement { x: 0, y: 596, width: 30, height: 100, enforce_size: true }));
    }
}
