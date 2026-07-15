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
