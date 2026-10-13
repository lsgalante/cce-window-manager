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
    /// argv joined with spaces: what restore MATCHES on (program, app), and
    /// what a state file from before `argv` relaunches. Not a shell command:
    /// an argument holding a space, `$`, a quote or `;` reads differently
    /// once joined.
    pub cmdline: String,
    #[serde(default)]
    pub focused: bool,
    /// The exact argv the window's process ran, which is what the session
    /// restore relaunches (each argument quoted for the `sh -c` it runs
    /// through). `None` in files saved before 2026-10-02, when the restore
    /// ran `cmdline` itself through the shell: a file named `$(…).pdf` open
    /// in a viewer then ran its command at the next login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
    /// A fullscreen window's desk spot — the virtual origin of the output
    /// rect it covers — which `virtual_x/y` cannot hold: those are where
    /// the window goes when it LEAVES fullscreen. Restored, the window's
    /// next fullscreen enter lands here rather than wherever the camera
    /// is. `None` for a window that was not fullscreen, and in older files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fullscreen_at: Option<(f64, f64)>,
    /// Where the window sat in the stacking order when the session was
    /// saved: its position in the compositor's render list, bottom first.
    /// Only the order matters, never the value: the restore slots each
    /// window among the restored ones already up ([`restored_stack_slot`]).
    /// Session entries only; `None` in `last_window_states` (a window the
    /// user reopens later comes up on top) and in files from before
    /// 2026-10-13.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<u32>,
}

/// Where in a stacking order (bottom first) a window restored with saved
/// stack position `rank` goes, given the saved position of each window
/// already in it — `None` for one with no say (not restored from the
/// session, or raised by the user since, which outranks the file).
///
/// Restored windows map in whatever order their apps start, so each is
/// slotted relative to the restored windows already up: directly ABOVE the
/// one whose saved position is the greatest below its own, or, when it is
/// the lowest so far, directly BELOW the lowest above it. Anchoring on the
/// nearest ranked neighbour rather than on the top or bottom leaves every
/// unranked window where it is — a window the user raised mid-login stays
/// over the late arrivals. With no ranked window up it goes on top, as any
/// new window does. Equal positions (entries saved at different times)
/// stack in arrival order.
pub fn restored_stack_slot(order: &[Option<u32>], rank: u32) -> usize {
    let below = order
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.filter(|&r| r <= rank).map(|r| (r, i)))
        .max();
    if let Some((_, i)) = below {
        return i + 1;
    }
    let above = order
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.filter(|&r| r > rank).map(|r| (r, i)))
        .min();
    match above {
        Some((_, i)) => i,
        None => order.len(),
    }
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
        assert!(s.windows[0].argv.is_none(), "a file from before argv loads without one");
        assert!(s.windows[0].fullscreen_at.is_none());
        assert!(s.windows[0].stack.is_none(), "a file from before stack loads without one");
        assert_eq!(s.windows.len(), 1);

        let with_grid = SavedState {
            grid: Some(SavedGrid { cell_w: 512.0, cell_h: 512.0, gap_width: 16.0, cell_inset: 4.0 }),
            ..s
        };
        let json = serde_json::to_string(&with_grid).unwrap();
        let back: SavedState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.grid, with_grid.grid);
        assert!(!json.contains("\"stack\""), "an unset stack is not written");

        let mut stacked = back.windows[0].clone();
        stacked.stack = Some(7);
        let json = serde_json::to_string(&stacked).unwrap();
        let back: SavedWindowState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.stack, Some(7));
    }

    /// Mapping in any order rebuilds the saved order.
    #[test]
    fn restored_windows_rebuild_the_saved_order_whatever_order_they_map_in() {
        for arrival in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1], [1, 3, 0, 2]] {
            let mut order: Vec<Option<u32>> = Vec::new();
            for rank in arrival {
                let at = restored_stack_slot(&order, rank);
                order.insert(at, Some(rank));
            }
            assert_eq!(order, vec![Some(0), Some(1), Some(2), Some(3)], "arrival {arrival:?}");
        }
    }

    #[test]
    fn with_nothing_ranked_up_a_restored_window_goes_on_top() {
        assert_eq!(restored_stack_slot(&[], 4), 0);
        assert_eq!(restored_stack_slot(&[None, None], 4), 2);
    }

    /// A window the user raised mid-login (its rank dropped) stays over the
    /// restored windows that arrive after it, whichever side of it their
    /// saved neighbours are.
    #[test]
    fn a_window_raised_mid_login_stays_on_top_of_late_arrivals() {
        // Saved bottom-up A0 B1 C2 D3. A and C mapped, then the user raised A
        // (unranked now, on top): [C, A].
        let order = [Some(2), None];
        // B (1): lowest ranked so far, so directly below C.
        assert_eq!(restored_stack_slot(&order, 1), 0);
        // D (3): directly above C, still below A.
        assert_eq!(restored_stack_slot(&order, 3), 1);
    }

    #[test]
    fn equal_ranks_stack_in_arrival_order() {
        assert_eq!(restored_stack_slot(&[Some(1), Some(2)], 1), 1);
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
