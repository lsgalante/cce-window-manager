// Window modes.
//
// A window is either `Floating` (positioned freely on the virtual surface) or
// `Tiled` (every content edge lies on a visible desktop-grid cell edge). Tiled
// windows report the xdg maximized state to their client. The remaining
// variants are internal roles (`Popup`, `Overlay`, `Status`, `Utility`) or the
// orthogonal `Fullscreen` toggle.
//
// `Utility` is `Status` minus the docking: a tool window whose shape is decided
// by its contents (stacked sliders, fixed rows, nothing worth dragging). The
// client owns the size, the compositor offers no resize affordance and saves no
// geometry for it, but it floats and moves like any ordinary window. It is
// never inferred from a sizing hint — a window is `Utility` only because the
// client said so, via `set_utility` on the cce window-management protocol.
//
// Serde aliases keep old `state.json` files loading: the retired `Cascade` /
// `Grid` layout modes collapse to `Floating`, and `Maximized` (the old name
// for grid-locked windows) maps to `Tiled`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TilingMode {
    #[serde(alias = "Cascade", alias = "Grid")]
    Floating,
    #[serde(alias = "Maximized")]
    Tiled,
    Fullscreen,
    Popup,
    Overlay,
    Status,
    Utility,
}

impl TilingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            TilingMode::Floating => "Floating",
            TilingMode::Tiled => "Tiled",
            TilingMode::Fullscreen => "Fullscreen",
            TilingMode::Popup => "Popup",
            TilingMode::Overlay => "Overlay",
            TilingMode::Status => "Status",
            TilingMode::Utility => "Utility",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_state_names_still_deserialize() {
        // Old state.json files carry the retired mode names.
        for (json, expected) in [
            ("\"Cascade\"", TilingMode::Floating),
            ("\"Grid\"", TilingMode::Floating),
            ("\"Maximized\"", TilingMode::Tiled),
            ("\"Floating\"", TilingMode::Floating),
            ("\"Tiled\"", TilingMode::Tiled),
        ] {
            let mode: TilingMode = serde_json::from_str(json).unwrap();
            assert_eq!(mode, expected, "{json}");
        }
    }
}
