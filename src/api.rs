// The Policy / Compositor trait boundary.
//
// `Policy` is implemented by the window-management side: it receives events
// and decides placement, focus, decoration, and background — using only the
// plain-data types in this file, never FFI. `Compositor` is implemented by
// the mechanism side (`window_manager.rs` and friends): it executes those
// decisions against the wlroots/scenefx scene graph.
//
// Skeleton status: nothing implements these traits yet. The migration plan is
// to split `arrange_views()` into a policy half (compute placements) and a
// mechanism half (apply to scene), then route window lifecycle, input actions,
// and the animation tick through `Policy`. Effects are declarative on purpose:
// new scenefx capabilities extend `EffectSpec` without changing either trait.

use super::state::SavedState;

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

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
    pub role: WindowRole,
    pub cmdline: String,
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

#[derive(Debug, Clone, Copy)]
pub struct OutputInfo {
    pub width: i32,
    pub height: i32,
    pub scale: f32,
}

#[derive(Debug, Clone, Copy)]
pub enum PointerEvent {
    Press { window: Option<WindowId>, x: f64, y: f64, button: u32 },
    Release { window: Option<WindowId>, x: f64, y: f64, button: u32 },
    Motion { x: f64, y: f64 },
}

/// Commands from policy to mechanism. Implemented by the compositor side;
/// every method maps onto existing `WindowManager` / scene operations.
pub trait Compositor {
    fn place(&mut self, window: WindowId, rect: Rect);
    fn focus(&mut self, window: Option<WindowId>);
    fn raise(&mut self, window: WindowId);
    fn close(&mut self, window: WindowId);
    /// Pans/zooms windows and the background in the same frame.
    fn set_viewport(&mut self, pan_x: f64, pan_y: f64, zoom: f64);
    fn set_decoration(&mut self, window: WindowId, spec: DecorationSpec);
    fn set_effects(&mut self, window: WindowId, spec: EffectSpec);
    fn set_background(&mut self, spec: BackgroundSpec);
    fn spawn(&mut self, cmdline: &str);
}

/// Events from mechanism to policy. Implemented by the window-management side.
pub trait Policy {
    fn window_mapped(&mut self, c: &mut dyn Compositor, window: WindowId, info: &WindowInfo);
    fn window_unmapped(&mut self, c: &mut dyn Compositor, window: WindowId);
    fn window_meta_changed(&mut self, c: &mut dyn Compositor, window: WindowId, info: &WindowInfo);
    fn action(&mut self, c: &mut dyn Compositor, action: &Action, arg: Option<&str>);
    fn pointer(&mut self, c: &mut dyn Compositor, event: PointerEvent);
    fn output_changed(&mut self, c: &mut dyn Compositor, outputs: &[OutputInfo]);
    /// Animation driver: easing for pan/zoom targets, effect transitions.
    fn tick(&mut self, c: &mut dyn Compositor, dt: f64);
    fn save_state(&self) -> SavedState;
    fn restore_state(&mut self, c: &mut dyn Compositor, state: SavedState);
}
