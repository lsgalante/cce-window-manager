// Persisted session state, saved to $XDG_STATE_HOME/cce/state.json on shutdown
// and restored on startup. Pure data — serialization and matching logic only;
// the save/load I/O lives in `window_manager.rs`.

use super::tiling::TilingMode;

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
    pub global_layout: TilingMode,
    pub windows: Vec<SavedWindowState>,
    #[serde(default)]
    pub last_window_states: Vec<SavedWindowState>,
}
