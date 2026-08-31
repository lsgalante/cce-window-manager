// Persisted session state, saved to $XDG_STATE_HOME/cce/state.json on shutdown
// and restored on startup. Pure data — serialization and matching logic only;
// the save/load I/O lives in `window_manager.rs`.

use super::tiling::TilingMode;

/// The desktop-grid geometry the geometries in this file were measured
/// under. Saved so a LATER session under a different grid can re-tile a
/// Tiled entry onto the same block of squares (`cells::remap_block`) instead
/// of re-deriving its span from stale pixels — a box saved under one grid
/// lands misaligned on another, touches extra cells, and the tiled snap
/// then grows the window by a cell. Absent in files written before this
/// field existed; the loader then has nothing to remap from and applies the
/// geometry as-is.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SavedGrid {
    pub cell_w: f64,
    pub cell_h: f64,
    pub gap_width: f64,
    pub cell_inset: f64,
}

impl SavedGrid {
    pub fn from_params(p: &crate::snap::SnapParams) -> Self {
        Self { cell_w: p.cell_w, cell_h: p.cell_h, gap_width: p.gap_width, cell_inset: p.cell_inset }
    }

    /// The saved grid as snap params, borrowing everything non-geometric
    /// (threshold) from `current` — remapping needs geometry only.
    pub fn to_params(&self, current: &crate::snap::SnapParams) -> crate::snap::SnapParams {
        crate::snap::SnapParams {
            cell_w: self.cell_w,
            cell_h: self.cell_h,
            gap_width: self.gap_width,
            cell_inset: self.cell_inset,
            threshold: current.threshold,
        }
    }

    pub fn matches(&self, p: &crate::snap::SnapParams) -> bool {
        self.cell_w == p.cell_w
            && self.cell_h == p.cell_h
            && self.gap_width == p.gap_width
            && self.cell_inset == p.cell_inset
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SavedWindowState {
    pub app_id: String,
    pub title: String,
    pub tiling_mode: TilingMode,
    pub minimized: bool,
    pub virtual_x: f64,
    pub virtual_y: f64,
    pub scale: f64,
    pub width: u32,
    pub height: u32,
    pub cmdline: String,
    #[serde(default)]
    pub focused: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SavedState {
    pub desk_pan_x: f64,
    pub desk_pan_y: f64,
    pub desk_zoom: f64,
    pub windows: Vec<SavedWindowState>,
    #[serde(default)]
    pub last_window_states: Vec<SavedWindowState>,
    /// See [`SavedGrid`]. `None` in pre-field files.
    #[serde(default)]
    pub grid: Option<SavedGrid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state file from before the grid field must still load — and one
    /// with it must round-trip.
    #[test]
    fn legacy_state_without_grid_still_loads() {
        let legacy = r#"{
            "desk_pan_x": 0.0, "desk_pan_y": 0.0, "desk_zoom": 1.0,
            "windows": [{
                "app_id": "cce-terminal", "title": "t", "tiling_mode": "Tiled",
                "minimized": false, "virtual_x": 4.0, "virtual_y": 4.0,
                "scale": 1.0, "width": 1560, "height": 504, "cmdline": "cce-terminal"
            }]
        }"#;
        let s: SavedState = serde_json::from_str(legacy).unwrap();
        assert!(s.grid.is_none());
        assert_eq!(s.windows.len(), 1);

        let with_grid = SavedState {
            grid: Some(SavedGrid { cell_w: 512.0, cell_h: 512.0, gap_width: 16.0, cell_inset: 4.0 }),
            ..s
        };
        let json = serde_json::to_string(&with_grid).unwrap();
        let back: SavedState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.grid, with_grid.grid);
    }

    /// The saved-grid remap keeps a Tiled entry on ITS SQUARES across a gap
    /// change: 3 columns at gap 16 stays 3 columns at gap 8, where a raw
    /// pixel restore would misalign and span 4.
    #[test]
    fn saved_grid_remap_keeps_the_block() {
        let old = SavedGrid { cell_w: 512.0, cell_h: 512.0, gap_width: 16.0, cell_inset: 4.0 };
        let new = crate::snap::SnapParams {
            cell_w: 512.0, cell_h: 512.0, gap_width: 8.0, cell_inset: 4.0, threshold: 24.0,
        };
        // Column -5, 3 cells wide, 1 tall, under the old grid.
        let (x, y, w, h) = (-5.0 * 528.0 + 4.0, 4.0, 2.0 * 528.0 + 512.0 - 8.0, 504.0);
        let (nx, ny, nw, nh) =
            crate::cells::remap_block(x, y, w, h, &old.to_params(&new), &new);
        assert_eq!((nx, ny), (-5.0 * 520.0 + 4.0, 4.0));
        assert_eq!((nw, nh), (2.0 * 520.0 + 512.0 - 8.0, 504.0));
    }
}
