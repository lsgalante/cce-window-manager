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
}

impl WindowRole {
    /// The single place the special app_id conventions are interpreted.
    /// `Overlay` is never derived from an app_id — it comes from tiling mode.
    pub fn from_app_id(app_id: Option<&str>) -> Self {
        match app_id {
            Some("cce-wallpaper") => WindowRole::Background,
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
    Exit,
    Reload,
    Fullscreen,
    LayoutNext,
    ModeNext,
    ModeNextShared,
    View1,
    View2,
    View3,
    View4,
    SetViewport1,
    SetViewport2,
    SetViewport3,
    SetViewport4,
    Expose,
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
            Action::Exit => "exit",
            Action::Reload => "reload",
            Action::Fullscreen => "toggle_fullscreen",
            Action::LayoutNext => "layout_next",
            Action::ModeNext => "mode_next",
            Action::ModeNextShared => "mode_next_shared",
            Action::View1 => "view_1",
            Action::View2 => "view_2",
            Action::View3 => "view_3",
            Action::View4 => "view_4",
            Action::SetViewport1 => "set_viewport_1",
            Action::SetViewport2 => "set_viewport_2",
            Action::SetViewport3 => "set_viewport_3",
            Action::SetViewport4 => "set_viewport_4",
            Action::Expose => "expose",
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
        }
    }

    /// Inverse of `name()`, plus aliases from the old `config.kdl`
    /// vocabulary (`close`, `fullscreen`, `toggle_overview`).
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
            "exit" => Action::Exit,
            "reload" => Action::Reload,
            "toggle_fullscreen" | "fullscreen" => Action::Fullscreen,
            "layout_next" => Action::LayoutNext,
            "mode_next" => Action::ModeNext,
            "mode_next_shared" => Action::ModeNextShared,
            "view_1" => Action::View1,
            "view_2" => Action::View2,
            "view_3" => Action::View3,
            "view_4" => Action::View4,
            "set_viewport_1" => Action::SetViewport1,
            "set_viewport_2" => Action::SetViewport2,
            "set_viewport_3" => Action::SetViewport3,
            "set_viewport_4" => Action::SetViewport4,
            "expose" | "toggle_overview" => Action::Expose,
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

/// What the background layer shows. Absorbs cce-wallpaper (Solid) and the
/// grid_tree drawing in `output.rs` (Grid).
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundSpec {
    Solid(Rgba),
    Grid(GridSpec),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GridSpec {
    pub background: Rgba,
    pub line_color: Rgba,
    pub cell_size: i32,
    pub line_width: i32,
}

/// Everything `Policy::action` may consult, captured by the mechanism at
/// dispatch time. Seat- and scene-dependent answers (cursor output, hovered
/// window, focus) are resolved into the snapshot up front — the arrange-pass
/// convention; policy never queries back mid-decision.
#[derive(Debug, Clone)]
pub struct ActionCtx {
    pub camera: Camera,
    /// The mechanism is in overview mode. Set by fiat on Expose enter, so
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
    /// output: the viewport Expose enters/exits in.
    pub cursor_viewport: Rect,
    /// False when there is no seat; the cursor fields then hold zeros.
    pub has_cursor: bool,
    pub cursor_x: f64,
    pub cursor_y: f64,
    /// Non-status, non-background window under the cursor.
    pub hovered: Option<WindowId>,
    pub focused: Option<WindowId>,
    /// Desktop grid period (cell size + gap width) for cell-aligned panning.
    pub grid_period: f64,
    pub windows: Vec<ActionWindow>,
}

/// A window as `Policy::action` sees it.
#[derive(Debug, Clone, Copy)]
pub struct ActionWindow {
    pub id: WindowId,
    /// Virtual-space position. `w`/`h` are the mechanism's working extent in
    /// output px (box_geom, defaulted to 800x600 while unmapped).
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Participates in the overview fit: mapped, not minimized, not
    /// status/background, not popup/overlay.
    pub expose_eligible: bool,
}

/// One mechanism write, returned by policy decisions and applied in order —
/// the command-stream counterpart of the arrange plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Write the camera. `overview: None` leaves the mode untouched.
    SetCamera { camera: Camera, overview: Option<bool> },
    /// Set pan-animation targets (a `None` axis is left alone) and start
    /// easing toward them.
    PanTo { x: Option<f64>, y: Option<f64> },
    StopPanAnimation,
    Focus(WindowId),
    /// Reposition a window in virtual space.
    MoveWindow { id: WindowId, x: f64, y: f64 },
    /// Full re-arrange (the mechanism's `dirty_windowing`).
    Relayout,
    /// Camera-only refresh: the fast viewport path when the WM is idle.
    RefreshCamera,
}

/// Decisions, policy-side. Implemented by `actions::DefaultPolicy`.
pub trait Policy {
    /// Decide a user action against the snapshot. An empty vec means "not
    /// mine" — the mechanism falls through to its remaining legacy arms.
    fn action(&mut self, ctx: &ActionCtx, action: Action) -> Vec<Command>;
}

/// Execution, mechanism-side. Implemented by the compositor's WindowManager.
pub trait Compositor {
    fn apply(&mut self, cmd: &Command);
}
