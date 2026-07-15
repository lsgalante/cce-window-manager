// Tiling formulas ported from cce-client

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TilingMode {
    Floating,
    Cascade,
    Grid,
    Fullscreen,
    Popup,
    Overlay,
    Status,
    Maximized,
}

impl TilingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            TilingMode::Floating => "Floating",
            TilingMode::Cascade => "Cascade",
            TilingMode::Grid => "Grid",
            TilingMode::Fullscreen => "Fullscreen",
            TilingMode::Popup => "Popup",
            TilingMode::Overlay => "Overlay",
            TilingMode::Status => "Status",
            TilingMode::Maximized => "Maximized",
        }
    }
}

/// Cascade depth factor: each depth step multiplies channels by this
pub const CASCADE_DEPTH_FACTOR: f64 = 0.80;

/// Full alpha, byte-replicated for River's color format
pub const CASCADE_ALPHA: u32 = 0xFFFFFFFFu32;

/// Tile a window in cascade mode.
pub fn tile_cascade(
    screen_w: i32,
    screen_h: i32,
    _gap: i32,
    gap_top: i32,
    gap_left: i32,
    gap_right: i32,
    gap_bottom: i32,
    bw: i32,
    dec_h: i32,
    cascade_offset: i32,
    bar_height: i32,
    n_cascade: i32,
    idx: i32,
) -> (i32, i32, i32, i32) {
    let max_offsets = 5;
    let eff_cascade = n_cascade.min(max_offsets);
    let width = screen_w - gap_left - gap_right - bw * 2 - cascade_offset * (eff_cascade - 1);
    let height = screen_h - bar_height - gap_top - gap_bottom - (dec_h + bw) - cascade_offset * (eff_cascade - 1);
    let width = if width < 1 { 1 } else { width };
    let height = if height < 1 { 1 } else { height };
    let pos_idx = idx.min(max_offsets - 1);
    let x = gap_left + bw + pos_idx * cascade_offset;
    let y = bar_height + gap_top + dec_h + pos_idx * cascade_offset;
    (x, y, width, height)
}

/// Tile a window in grid mode.
pub fn tile_grid(
    screen_w: i32,
    screen_h: i32,
    gap: i32,
    gap_top: i32,
    gap_left: i32,
    gap_right: i32,
    gap_bottom: i32,
    bw: i32,
    dec_h: i32,
    bar_height: i32,
    n_grid: i32,
    idx: i32,
) -> (i32, i32, i32, i32) {
    let cols = if n_grid == 1 { 1i32 } else { 2i32 };
    let row = idx / cols;
    let col = idx % cols;
    let rows = (n_grid + cols - 1) / cols;
    let width = (screen_w - gap_left - gap_right - (cols - 1) * gap) / cols - 2 * bw;
    let height = (screen_h - bar_height - gap_top - gap_bottom - (rows - 1) * gap) / rows - (dec_h + bw);
    let width = if width < 1 { 1 } else { width };
    let height = if height < 1 { 1 } else { height };
    let x = gap_left + bw + col * (width + 2 * bw + gap);
    let y = bar_height + gap_top + dec_h + row * (height + (dec_h + bw) + gap);
    (x, y, width, height)
}

/// Tile a window in fullscreen mode.
pub fn tile_fullscreen(
    screen_w: i32,
    screen_h: i32,
    _gap_top: i32,
    _gap_left: i32,
    _gap_right: i32,
    _gap_bottom: i32,
    _bw: i32,
    _bar_height: i32,
) -> (i32, i32, i32, i32) {
    (0, 0, screen_w, screen_h)
}

/// Interpolate a byte-replicated 32-bit channel (0xVVVVVVVV) by factor^depth.
pub fn interp_channel(fp_channel: u32, factor: f64, depth: i32) -> u32 {
    let base = (fp_channel & 0xFF) as u8;
    let f = factor.powi(depth);
    let val = ((base as f64) * f) as u8;
    val as u32 * 0x01010101
}

