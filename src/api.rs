// The Policy / Compositor trait boundary — snapshot-style.
//
// `Policy` is implemented policy-side (`actions::DefaultPolicy`): its methods
// take a plain-data snapshot the mechanism captured at dispatch time and
// return `Command`s — the same snapshot → plan convention as the arrange
// pass, with the mechanism owning all state. `Compositor` is implemented by
// the mechanism (`window_manager.rs`): it applies one `Command` at a time
// against the wlroots/scenefx world.
//
// Migration status: `Policy::action` is live — the compositor routes user
// actions through it and falls back to its legacy arms only for actions the
// policy doesn't claim. Further flows (window lifecycle, the animation tick)
// grow new snapshot-taking methods here as they migrate; don't add
// speculative signatures ahead of a real mechanism caller. Effects are
// declarative on purpose: new scenefx capabilities extend `EffectSpec`
// without changing either trait.

use crate::camera::Camera;
use crate::tiling::TilingMode;

/// Opaque handle to a window. Wraps the `SlotMap` key that the mechanism side
/// uses internally; policy code never sees a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub crate::slotmap::Key);

/// What a surface is for. Assigned once at map time, this replaces scattered
/// app_id string-matching (`"cce-wallpaper"`, `"cce-status*"`) in mechanism code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole {
    Normal,
    StatusBar,
    Background,
    Overlay,
    /// The desktop-grid layer: a client surface world-anchored to a patch of
    /// the virtual desktop (`WindowSnapshot::grid_patch`). The compositor
    /// pans/zooms it per frame exactly like window content — the client is
    /// never in the frame loop; it re-renders only when handed a new patch.
    /// Input-transparent, stacked above the wallpaper and below everything
    /// else.
    Grid,
}

impl WindowRole {
    /// The single place the special app_id conventions are interpreted.
    /// `Overlay` is never derived from an app_id — it comes from tiling mode.
    /// `Grid` also has a protocol declaration (`set_grid`); the app_id match
    /// makes the fallback swap and placement correct from map time.
    pub fn from_app_id(app_id: Option<&str>) -> Self {
        match app_id {
            Some("cce-wallpaper") => WindowRole::Background,
            Some("cce-grid") => WindowRole::Grid,
            Some(id) if id.starts_with("cce-status") => WindowRole::StatusBar,
            _ => WindowRole::Normal,
        }
    }
}

/// User-triggered window-management actions, bound to keys/pointers/gestures
/// by the compositor's config and dispatched into policy code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    Spawn,
    Toggle,
    Close,
    FocusNext,
    FocusPrev,
    FocusUp,
    FocusDown,
    FocusLeft,
    FocusRight,
    WindowSwitcher,
    WindowSwitcherPrev,
    Move,
    Resize,
    MoveWindowLeft,
    MoveWindowRight,
    MoveWindowUp,
    MoveWindowDown,
    Exit,
    Reload,
    Fullscreen,
    ModeNext,
    ModeNextShared,
    Overview,
    OverviewEnter,
    OverviewExit,
    Minimize,
    OverlayLeft,
    OverlayRight,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    Screenshot,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MicMute,
    BrightnessUp,
    BrightnessDown,
}

impl Action {
    /// Canonical snake_case name — what users write in the
    /// `cce-window-manager` domain of `input.kdl`.
    pub fn name(&self) -> &'static str {
        match self {
            Action::None => "none",
            Action::Spawn => "spawn",
            Action::Toggle => "toggle",
            Action::Close => "close_window",
            Action::FocusNext => "focus_next",
            Action::FocusPrev => "focus_prev",
            Action::FocusUp => "focus_up",
            Action::FocusDown => "focus_down",
            Action::FocusLeft => "focus_left",
            Action::FocusRight => "focus_right",
            Action::WindowSwitcher => "window_switcher",
            Action::WindowSwitcherPrev => "window_switcher_prev",
            Action::Move => "move",
            Action::Resize => "resize",
            Action::MoveWindowLeft => "move_window_left",
            Action::MoveWindowRight => "move_window_right",
            Action::MoveWindowUp => "move_window_up",
            Action::MoveWindowDown => "move_window_down",
            Action::Exit => "exit",
            Action::Reload => "reload",
            Action::Fullscreen => "toggle_fullscreen",
            Action::ModeNext => "mode_next",
            Action::ModeNextShared => "mode_next_shared",
            Action::Overview => "overview",
            Action::OverviewEnter => "overview_enter",
            Action::OverviewExit => "overview_exit",
            Action::Minimize => "minimize",
            Action::OverlayLeft => "overlay_left",
            Action::OverlayRight => "overlay_right",
            Action::ZoomIn => "zoom_in",
            Action::ZoomOut => "zoom_out",
            Action::ZoomReset => "zoom_reset",
            Action::PanLeft => "pan_left",
            Action::PanRight => "pan_right",
            Action::PanUp => "pan_up",
            Action::PanDown => "pan_down",
            Action::Screenshot => "screenshot",
            Action::VolumeUp => "volume_up",
            Action::VolumeDown => "volume_down",
            Action::VolumeMute => "volume_mute",
            Action::MicMute => "mic_mute",
            Action::BrightnessUp => "brightness_up",
            Action::BrightnessDown => "brightness_down",
        }
    }

    /// Inverse of `name()`, plus aliases from the old `config.kdl`
    /// vocabulary (`close`, `fullscreen`, `expose`, `toggle_overview`).
    pub fn from_name(name: &str) -> Option<Action> {
        Some(match name.trim() {
            "none" => Action::None,
            "spawn" => Action::Spawn,
            "toggle" => Action::Toggle,
            "close_window" | "close" => Action::Close,
            "focus_next" => Action::FocusNext,
            "focus_prev" => Action::FocusPrev,
            "focus_up" => Action::FocusUp,
            "focus_down" => Action::FocusDown,
            "focus_left" => Action::FocusLeft,
            "focus_right" => Action::FocusRight,
            "window_switcher" => Action::WindowSwitcher,
            "window_switcher_prev" => Action::WindowSwitcherPrev,
            "move" => Action::Move,
            "resize" => Action::Resize,
            "move_window_left" => Action::MoveWindowLeft,
            "move_window_right" => Action::MoveWindowRight,
            "move_window_up" => Action::MoveWindowUp,
            "move_window_down" => Action::MoveWindowDown,
            "exit" => Action::Exit,
            "reload" => Action::Reload,
            "toggle_fullscreen" | "fullscreen" => Action::Fullscreen,
            "mode_next" => Action::ModeNext,
            "mode_next_shared" => Action::ModeNextShared,
            "overview" | "expose" | "toggle_overview" => Action::Overview,
            "overview_enter" => Action::OverviewEnter,
            "overview_exit" => Action::OverviewExit,
            "minimize" => Action::Minimize,
            "overlay_left" => Action::OverlayLeft,
            "overlay_right" => Action::OverlayRight,
            "zoom_in" => Action::ZoomIn,
            "zoom_out" => Action::ZoomOut,
            "zoom_reset" => Action::ZoomReset,
            "pan_left" => Action::PanLeft,
            "pan_right" => Action::PanRight,
            "pan_up" => Action::PanUp,
            "pan_down" => Action::PanDown,
            "screenshot" => Action::Screenshot,
            "volume_up" => Action::VolumeUp,
            "volume_down" => Action::VolumeDown,
            "volume_mute" | "mute" => Action::VolumeMute,
            "mic_mute" => Action::MicMute,
            "brightness_up" => Action::BrightnessUp,
            "brightness_down" => Action::BrightnessDown,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The world-anchored patch a grid client's buffer covers: virtual origin and
/// size, plus the buffer resolution. The compositor issues patches
/// (grid_patch events) and latches one when the client's rendered buffer
/// arrives; arrange places the surface at `(x, y)` with display scale
/// `zoom / scale`, so the buffer pans and zooms in lockstep with windows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridPatch {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Buffer px per virtual unit.
    pub scale: f64,
}

/// Premultiplied-alpha RGBA, 0.0–1.0 per channel (scenefx convention).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba(pub [f32; 4]);

/// Server-side decoration for one window: borders now, titlebars later.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecorationSpec {
    pub border_width: i32,
    pub border_color: Rgba,
    pub corner_radius: i32,
}

/// Declarative per-window scenefx effects.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectSpec {
    pub opacity: f32,
    pub blur: bool,
    pub shadow: Option<ShadowSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSpec {
    pub color: Rgba,
    pub blur_sigma: f32,
}

/// What the background layer shows. The desktop is normally Grid; Solid is
/// the degenerate single-color background.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundSpec {
    Solid(Rgba),
    Grid(GridSpec),
}

/// The desktop grid, as configured (unzoomed virtual units). The per-frame
/// geometry — pan/zoom offsets, density fade, cell counts — is derived by
/// `background::grid_frame`.
#[derive(Debug, Clone, PartialEq)]
pub struct GridSpec {
    /// Backdrop color behind and between the cells (premultiplied).
    pub gap_color: Rgba,
    pub cell_color: Rgba,
    /// Cell width (x axis); the column period is `cell_w + gap_width`.
    pub cell_w: f64,
    /// Cell height (y axis); the row period is `cell_h + gap_width`.
    pub cell_h: f64,
    pub gap_width: f64,
    pub cell_corner_radius: i32,
    /// Cells fade inward by this many virtual px.
    pub cell_fade_inset: i32,
    pub fade_mode: GridFadeMode,
}

/// Which side of the viewport the overlay column docks on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlaySide {
    Left,
    Right,
}

/// Shape of a cell's inward edge fade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridFadeMode {
    Linear,
    Smoothstep,
    Quadratic,
    Cosine,
    Gaussian,
}

impl GridFadeMode {
    /// Config-string names; anything unrecognized is Linear.
    pub fn from_name(name: &str) -> Self {
        match name {
            "smoothstep" => GridFadeMode::Smoothstep,
            "quadratic" => GridFadeMode::Quadratic,
            "cosine" => GridFadeMode::Cosine,
            "gaussian" => GridFadeMode::Gaussian,
            _ => GridFadeMode::Linear,
        }
    }
}

/// Everything `Policy::action` may consult, captured by the mechanism at
/// dispatch time. Seat- and scene-dependent answers (cursor output, hovered
/// window, focus) are resolved into the snapshot up front — the arrange-pass
/// convention; policy never queries back mid-decision.
#[derive(Debug, Clone)]
pub struct ActionCtx {
    pub camera: Camera,
    /// The mechanism is in overview mode. Set by fiat on Overview enter, so
    /// this is NOT always `camera::is_overview(zoom)` — an overview fit can
    /// land at zoom 1.
    pub overview: bool,
    /// Pending pan-animation targets, if the camera is mid-ease.
    pub pan_target_x: Option<f64>,
    pub pan_target_y: Option<f64>,
    /// First enabled output's extent — the legacy "viewport" for keyed zooms
    /// and View jumps (the multi-output quirk, preserved by construction).
    pub viewport_w: f64,
    pub viewport_h: f64,
    /// Output box under the cursor, falling back to the first enabled
    /// output: the viewport Overview enters/exits in.
    pub cursor_viewport: Rect,
    /// False when there is no seat; the cursor fields then hold zeros.
    pub has_cursor: bool,
    pub cursor_x: f64,
    pub cursor_y: f64,
    /// Non-status, non-background window under the cursor.
    pub hovered: Option<WindowId>,
    pub focused: Option<WindowId>,
    /// Desktop grid periods (cell size + gap width, per axis) for
    /// cell-aligned panning: keyed horizontal pans step `grid_period_x`,
    /// vertical pans `grid_period_y`.
    pub grid_period_x: f64,
    pub grid_period_y: f64,
    pub windows: Vec<ActionWindow>,
}

/// A window as `Policy::action` sees it.
#[derive(Debug, Clone)]
pub struct ActionWindow {
    pub id: WindowId,
    pub app_id: Option<String>,
    pub title: Option<String>,
    /// state == Mapped (narrower than `visible`, which also spans teardown).
    pub mapped: bool,
    /// Virtual-space position. `w`/`h` are the mechanism's working extent in
    /// output px (box_geom, defaulted to 800x600 while unmapped).
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Per-window output scale; the virtual footprint is `w * scale`.
    pub scale: f64,
    /// The window's own tiling mode (as set, before viewport resolution).
    pub mode: TilingMode,
    /// The mode the window resolves to through the viewport-mode rules
    /// (mechanism's `get_mode_for_window`) — what leaving Fullscreen
    /// falls back to.
    pub resolved_mode: TilingMode,
    /// Mapped and not in Closing/Init teardown/startup.
    pub visible: bool,
    /// In the focus-cycling set: currently rendered, not minimized, not a
    /// status bar.
    pub focus_cyclable: bool,
    /// Participates in the overview fit: mapped, not minimized, not
    /// status/background, not popup/overlay.
    pub overview_eligible: bool,
}

/// One mechanism write, returned by policy decisions and applied in order —
/// the command-stream counterpart of the arrange plan.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Spawn a command line (the mechanism forks `sh -c`).
    Spawn(String),
    /// Write the camera. `overview: None` leaves the mode untouched.
    /// `animate: true` eases pan and zoom toward the target (the overview
    /// enter/exit transition); `false` snaps and cancels any easing in
    /// flight. The mode flip itself always applies immediately.
    SetCamera { camera: Camera, overview: Option<bool>, animate: bool },
    /// Set pan-animation targets (a `None` axis is left alone) and start
    /// easing toward them.
    PanTo { x: Option<f64>, y: Option<f64> },
    StopPanAnimation,
    Focus(WindowId),
    /// Focus whichever visible window the mechanism's next-visible rule
    /// picks — the refocus step after closing/minimizing the focused window.
    FocusNextVisible,
    /// Raise a window to the top of the stacking order.
    Raise(WindowId),
    /// Ask the window to close (and let teardown proceed).
    CloseWindow(WindowId),
    SetMinimized { id: WindowId, minimized: bool },
    /// Set a window's tiling mode; `locked` pins it against viewport-mode
    /// resolution.
    SetWindowMode { id: WindowId, mode: TilingMode, locked: bool },
    /// Dock the overlay column on the given side.
    SetOverlayPosition(OverlaySide),
    /// Reposition a window in virtual space.
    MoveWindow { id: WindowId, x: f64, y: f64 },
    /// Full re-arrange (the mechanism's `dirty_windowing`).
    Relayout,
    /// Camera-only refresh: the fast viewport path when the WM is idle.
    RefreshCamera,
}

/// Decisions, policy-side. Implemented by `actions::DefaultPolicy`.
pub trait Policy {
    /// Decide a user action against the snapshot; `arg` is the binding's
    /// command string (Spawn/Toggle carry one; media-key actions may carry
    /// an override of their stock command). An empty vec means "not
    /// mine" — the mechanism falls through to its remaining legacy arms.
    fn action(&mut self, ctx: &ActionCtx, action: Action, arg: Option<&str>) -> Vec<Command>;
}

/// Execution, mechanism-side. Implemented by the compositor's WindowManager.
pub trait Compositor {
    fn apply(&mut self, cmd: &Command);
}
