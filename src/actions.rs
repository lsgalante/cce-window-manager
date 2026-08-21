// Action dispatch policy: `DefaultPolicy` decides user actions as pure
// snapshot → command mappings. The camera actions (zoom, pan, viewport
// jumps, overview) live here, moved verbatim from the compositor's
// execute_action arms; an action this policy doesn't claim returns an empty
// vec and the mechanism's remaining legacy arms handle it.

use crate::api::{Action, ActionCtx, Command, OverlaySide, Policy, WindowId};
use crate::camera::{self, Camera};
use crate::focus;
use crate::pan;
use crate::tiling::TilingMode;

pub struct DefaultPolicy;

impl Policy for DefaultPolicy {
    fn action(&mut self, ctx: &ActionCtx, action: Action, arg: Option<&str>) -> Vec<Command> {
        match action {
            Action::Toggle => toggle(ctx, arg),
            Action::ZoomIn | Action::ZoomOut | Action::ZoomReset => zoom(ctx, action),
            Action::PanLeft | Action::PanRight | Action::PanUp | Action::PanDown => {
                pan_step(ctx, action)
            }
            Action::Overview => toggle_overview(ctx),
            Action::Close => close(ctx),
            Action::Minimize => minimize(ctx),
            Action::FocusNext | Action::FocusPrev => focus_cycle(ctx, action),
            Action::FocusUp | Action::FocusDown | Action::FocusLeft | Action::FocusRight => {
                focus_directional(ctx, action)
            }
            Action::Fullscreen => fullscreen(ctx),
            Action::ModeNext => mode_next(ctx),
            Action::ModeNextShared => mode_next_shared(ctx),
            Action::OverlayLeft => {
                vec![Command::SetOverlayPosition(OverlaySide::Left), Command::Relayout]
            }
            Action::OverlayRight => {
                vec![Command::SetOverlayPosition(OverlaySide::Right), Command::Relayout]
            }
            Action::VolumeUp
            | Action::VolumeDown
            | Action::VolumeMute
            | Action::MicMute
            | Action::BrightnessUp
            | Action::BrightnessDown => {
                vec![Command::Spawn(arg.unwrap_or(media_command(action)).to_string())]
            }
            _ => Vec::new(),
        }
    }
}

fn window(ctx: &ActionCtx, id: WindowId) -> Option<&crate::api::ActionWindow> {
    ctx.windows.iter().find(|w| w.id == id)
}

/// Keyed zooms pivot about the viewport center.
fn zoom(ctx: &ActionCtx, action: Action) -> Vec<Command> {
    let dir = match action {
        Action::ZoomIn => 1.0,
        Action::ZoomOut => -1.0,
        _ => 0.0,
    };
    let new_zoom = camera::keyed_zoom(ctx.camera.zoom, dir);
    let cam = camera::zoom_about_anchor(
        ctx.camera,
        ctx.viewport_w / 2.0,
        ctx.viewport_h / 2.0,
        new_zoom,
    );
    vec![
        Command::SetCamera { camera: cam, overview: Some(camera::is_overview(cam.zoom)), animate: false },
        Command::Relayout,
    ]
}

/// Keyed pans move cell-by-cell and ease to an aligned viewport. Stepping
/// from the pending target (not the current offset) lets rapid presses queue
/// one cell apiece.
fn pan_step(ctx: &ActionCtx, action: Action) -> Vec<Command> {
    let (dx, dy) = match action {
        Action::PanLeft => (-1.0, 0.0),
        Action::PanRight => (1.0, 0.0),
        Action::PanUp => (0.0, -1.0),
        _ => (0.0, 1.0),
    };
    let mut x = None;
    let mut y = None;
    if dx != 0.0 {
        let base = ctx.pan_target_x.unwrap_or(ctx.camera.pan_x);
        x = Some(pan::aligned_step(base, ctx.grid_period_x, dx));
    }
    if dy != 0.0 {
        let base = ctx.pan_target_y.unwrap_or(ctx.camera.pan_y);
        y = Some(pan::aligned_step(base, ctx.grid_period_y, dy));
    }
    vec![Command::PanTo { x, y }]
}

/// Toggle overview. Exit re-centers at zoom 1 — on the hovered window
/// (focusing it) when there is one, else on the virtual point under the
/// cursor — in the cursor's output. Enter fits the bounding box of all
/// eligible windows into the first enabled output.
fn toggle_overview(ctx: &ActionCtx) -> Vec<Command> {
    if ctx.overview {
        let out = ctx.cursor_viewport;
        let (ow, oh) = (out.width as f64, out.height as f64);
        if !ctx.has_cursor {
            // No seat: fall back to the origin at zoom 1.
            return vec![
                Command::StopPanAnimation,
                Command::SetCamera {
                    camera: Camera { pan_x: 0.0, pan_y: 0.0, zoom: 1.0 },
                    overview: Some(false),
                    animate: true,
                },
                Command::RefreshCamera,
            ];
        }
        if let Some(id) = ctx.hovered {
            if let Some(win) = window(ctx, id) {
                let cam =
                    camera::center_on(win.x + win.w / 2.0, win.y + win.h / 2.0, ow, oh, 1.0);
                return vec![
                    Command::Focus(id),
                    Command::StopPanAnimation,
                    Command::SetCamera { camera: cam, overview: Some(false), animate: true },
                    Command::RefreshCamera,
                ];
            }
        }
        let vx = ctx.camera.pan_x + (ctx.cursor_x - out.x as f64) / ctx.camera.zoom;
        let vy = ctx.camera.pan_y + (ctx.cursor_y - out.y as f64) / ctx.camera.zoom;
        let cam = camera::center_on(vx, vy, ow, oh, 1.0);
        vec![
            Command::StopPanAnimation,
            Command::SetCamera { camera: cam, overview: Some(false), animate: true },
            Command::RefreshCamera,
        ]
    } else {
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for w in ctx.windows.iter().filter(|w| w.overview_eligible) {
            let (min_x, min_y, max_x, max_y) =
                bounds.unwrap_or((f64::MAX, f64::MAX, f64::MIN, f64::MIN));
            bounds = Some((
                min_x.min(w.x),
                min_y.min(w.y),
                max_x.max(w.x + w.w),
                max_y.max(w.y + w.h),
            ));
        }
        let Some((min_x, min_y, max_x, max_y)) = bounds else { return Vec::new() };
        let cam = camera::fit_bounds(min_x, min_y, max_x, max_y, ctx.viewport_w, ctx.viewport_h);
        // Overview by fiat even when the fit lands at zoom 1 (a desktop
        // smaller than the screen): the next Overview must exit, not re-enter.
        vec![
            Command::SetCamera { camera: cam, overview: Some(true), animate: true },
            Command::RefreshCamera,
        ]
    }
}

/// The bare program name of a command line: first token, basename only.
pub fn program_name(cmd: &str) -> String {
    let first_token = cmd.trim().split_whitespace().next().unwrap_or("");
    match first_token.rfind('/') {
        Some(pos) => first_token[pos + 1..].to_string(),
        None => first_token.to_string(),
    }
}

/// Toggle a program: if a mapped window matches its name, close it (and
/// refocus if it was the focused one); otherwise spawn the command. Matching
/// is case-insensitive — app_id equal to or containing (either way) the
/// program name; a window with NO app_id falls back to a title-contains
/// match. First match in window order wins.
fn toggle(ctx: &ActionCtx, arg: Option<&str>) -> Vec<Command> {
    let Some(cmd) = arg else { return Vec::new() };
    let prog = program_name(cmd).to_lowercase();
    let matched = ctx.windows.iter().find(|w| {
        if !w.mapped {
            return false;
        }
        if let Some(aid) = &w.app_id {
            let aid = aid.to_lowercase();
            aid == prog || aid.contains(&prog) || prog.contains(&aid)
        } else if let Some(title) = &w.title {
            title.to_lowercase().contains(&prog)
        } else {
            false
        }
    });
    match matched {
        Some(w) => {
            let mut cmds = vec![Command::CloseWindow(w.id)];
            if ctx.focused == Some(w.id) {
                cmds.push(Command::FocusNextVisible);
            }
            cmds.push(Command::Relayout);
            cmds
        }
        None => vec![Command::Spawn(cmd.to_string())],
    }
}

/// Close the focused window, then refocus by the mechanism's next-visible
/// rule.
fn close(ctx: &ActionCtx) -> Vec<Command> {
    let Some(id) = ctx.focused else { return Vec::new() };
    vec![Command::CloseWindow(id), Command::FocusNextVisible, Command::Relayout]
}

/// Minimize the focused window, then refocus.
fn minimize(ctx: &ActionCtx) -> Vec<Command> {
    let Some(id) = ctx.focused else { return Vec::new() };
    vec![
        Command::SetMinimized { id, minimized: true },
        Command::FocusNextVisible,
        Command::Relayout,
    ]
}

/// The focus-cycling ring: cyclable windows in stable id order.
fn focus_ring(ctx: &ActionCtx) -> Vec<&crate::api::ActionWindow> {
    let mut ring: Vec<_> = ctx.windows.iter().filter(|w| w.focus_cyclable).collect();
    ring.sort_by_key(|w| w.id.0.index);
    ring
}

/// FocusNext/FocusPrev walk the ring; with nothing focused they start at
/// its first/last entry.
fn focus_cycle(ctx: &ActionCtx, action: Action) -> Vec<Command> {
    let ring = focus_ring(ctx);
    let n = ring.len();
    if n == 0 {
        return Vec::new();
    }
    let forward = action == Action::FocusNext;
    let current = ctx.focused.and_then(|f| ring.iter().position(|w| w.id == f));
    let target = match current {
        Some(idx) => {
            if forward {
                (idx + 1) % n
            } else {
                (idx + n - 1) % n
            }
        }
        None => {
            if forward {
                0
            } else {
                n - 1
            }
        }
    };
    let id = ring[target].id;
    vec![Command::Focus(id), Command::Raise(id), Command::Relayout]
}

/// Directional focus over the ring's window centers on the virtual surface.
fn focus_directional(ctx: &ActionCtx, action: Action) -> Vec<Command> {
    let ring = focus_ring(ctx);
    let centers: Vec<(f64, f64)> = ring
        .iter()
        .map(|w| (w.x + w.w * w.scale / 2.0, w.y + w.h * w.scale / 2.0))
        .collect();
    let focused_idx = ctx.focused.and_then(|f| ring.iter().position(|w| w.id == f));
    let dir = focus::Direction::from_action(action)
        .expect("arm only matches directional focus actions");
    let Some(target) = focus::directional_focus(&centers, focused_idx, dir) else {
        return Vec::new();
    };
    let id = ring[target].id;
    vec![Command::Focus(id), Command::Raise(id), Command::Relayout]
}

/// Toggle fullscreen on the focused window. Leaving fullscreen unlocks the
/// window back to its viewport-resolved mode (Floating if that resolution is
/// itself Fullscreen).
fn fullscreen(ctx: &ActionCtx) -> Vec<Command> {
    let Some(id) = ctx.focused else { return Vec::new() };
    let Some(win) = window(ctx, id) else { return Vec::new() };
    let cmd = if win.mode == TilingMode::Fullscreen {
        let target = if win.resolved_mode == TilingMode::Fullscreen {
            TilingMode::Floating
        } else {
            win.resolved_mode
        };
        Command::SetWindowMode { id, mode: target, locked: false }
    } else {
        Command::SetWindowMode { id, mode: TilingMode::Fullscreen, locked: true }
    };
    vec![cmd, Command::Relayout]
}

/// The keyed mode cycle.
fn next_mode(current: TilingMode) -> TilingMode {
    let cycle = [TilingMode::Floating, TilingMode::Fullscreen];
    cycle
        .iter()
        .position(|m| *m == current)
        .map(|i| cycle[(i + 1) % cycle.len()])
        .unwrap_or(TilingMode::Floating)
}

/// Cycle the focused window's mode.
fn mode_next(ctx: &ActionCtx) -> Vec<Command> {
    let Some(id) = ctx.focused else { return Vec::new() };
    let Some(win) = window(ctx, id) else { return Vec::new() };
    vec![
        Command::SetWindowMode { id, mode: next_mode(win.mode), locked: true },
        Command::Relayout,
    ]
}

/// Cycle every visible window that shares the focused window's mode.
fn mode_next_shared(ctx: &ActionCtx) -> Vec<Command> {
    let Some(id) = ctx.focused else { return Vec::new() };
    let Some(win) = window(ctx, id) else { return Vec::new() };
    let current = win.mode;
    let next = next_mode(current);
    let mut cmds: Vec<Command> = ctx
        .windows
        .iter()
        .filter(|w| w.visible && w.mode == current)
        .map(|w| Command::SetWindowMode { id: w.id, mode: next, locked: true })
        .collect();
    cmds.push(Command::Relayout);
    cmds
}

/// Stock command line for a media-key action (PipeWire's wpctl for audio,
/// brightnessctl for the backlight). A binding's `command="..."` property
/// overrides this wholesale — that's where a custom step size or a different
/// mixer goes.
fn media_command(action: Action) -> &'static str {
    match action {
        Action::VolumeUp => "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+",
        Action::VolumeDown => "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-",
        Action::VolumeMute => "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle",
        Action::MicMute => "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle",
        Action::BrightnessUp => "brightnessctl set 5%+",
        Action::BrightnessDown => "brightnessctl set 5%-",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ActionWindow, Rect};
    use crate::slotmap::Key;

    fn wid(index: u32) -> WindowId {
        WindowId(Key { generation: 0, index })
    }

    /// A plain visible, cyclable, overview-eligible Floating window.
    fn win(index: u32, x: f64, y: f64, w: f64, h: f64) -> ActionWindow {
        ActionWindow {
            id: wid(index),
            app_id: None,
            title: None,
            mapped: true,
            x,
            y,
            w,
            h,
            scale: 1.0,
            mode: TilingMode::Floating,
            resolved_mode: TilingMode::Floating,
            visible: true,
            focus_cyclable: true,
            overview_eligible: true,
        }
    }

    fn ctx() -> ActionCtx {
        ActionCtx {
            camera: Camera { pan_x: 0.0, pan_y: 0.0, zoom: 1.0 },
            overview: false,
            pan_target_x: None,
            pan_target_y: None,
            viewport_w: 1920.0,
            viewport_h: 1080.0,
            cursor_viewport: Rect { x: 0, y: 0, width: 1920, height: 1080 },
            has_cursor: true,
            cursor_x: 960.0,
            cursor_y: 540.0,
            hovered: None,
            focused: None,
            grid_period_x: 512.0,
            grid_period_y: 512.0,
            windows: Vec::new(),
        }
    }

    fn dispatch(ctx: &ActionCtx, action: Action) -> Vec<Command> {
        DefaultPolicy.action(ctx, action, None)
    }

    #[test]
    fn toggle_spawns_or_closes_by_program_match() {
        let mut c = ctx();
        // No arg: not claimed. No match: spawn.
        assert!(dispatch(&c, Action::Toggle).is_empty());
        assert_eq!(
            DefaultPolicy.action(&c, Action::Toggle, Some("firefox --new-window")),
            vec![Command::Spawn("firefox --new-window".to_string())]
        );
        // App-id containment match (either direction, case-insensitive)
        // closes; the focused match also refocuses.
        let mut w = win(1, 0.0, 0.0, 100.0, 100.0);
        w.app_id = Some("org.mozilla.Firefox".to_string());
        c.windows.push(w);
        assert_eq!(
            DefaultPolicy.action(&c, Action::Toggle, Some("/usr/bin/firefox -P")),
            vec![Command::CloseWindow(wid(1)), Command::Relayout]
        );
        c.focused = Some(wid(1));
        assert_eq!(
            DefaultPolicy.action(&c, Action::Toggle, Some("firefox")),
            vec![Command::CloseWindow(wid(1)), Command::FocusNextVisible, Command::Relayout]
        );
        // A window without an app_id falls back to title-contains; an
        // unmapped window never matches.
        let mut t = win(2, 0.0, 0.0, 100.0, 100.0);
        t.title = Some("Alacritty scratchpad".to_string());
        c.windows.push(t);
        assert_eq!(
            DefaultPolicy.action(&c, Action::Toggle, Some("alacritty")),
            vec![Command::CloseWindow(wid(2)), Command::Relayout]
        );
        c.windows[1].mapped = false;
        assert_eq!(
            DefaultPolicy.action(&c, Action::Toggle, Some("alacritty")),
            vec![Command::Spawn("alacritty".to_string())]
        );
    }

    #[test]
    fn program_name_takes_first_token_basename() {
        assert_eq!(program_name("/usr/bin/firefox --new-window"), "firefox");
        assert_eq!(program_name("  alacritty -e htop "), "alacritty");
        assert_eq!(program_name(""), "");
    }

    #[test]
    fn unclaimed_actions_return_empty() {
        assert!(dispatch(&ctx(), Action::Spawn).is_empty());
        assert!(dispatch(&ctx(), Action::Reload).is_empty());
        assert!(dispatch(&ctx(), Action::Exit).is_empty());
    }

    #[test]
    fn close_and_minimize_need_focus_and_refocus() {
        assert!(dispatch(&ctx(), Action::Close).is_empty());
        let mut c = ctx();
        c.focused = Some(wid(4));
        assert_eq!(
            dispatch(&c, Action::Close),
            vec![Command::CloseWindow(wid(4)), Command::FocusNextVisible, Command::Relayout]
        );
        assert_eq!(
            dispatch(&c, Action::Minimize),
            vec![
                Command::SetMinimized { id: wid(4), minimized: true },
                Command::FocusNextVisible,
                Command::Relayout
            ]
        );
    }

    #[test]
    fn focus_cycle_walks_id_order_and_wraps() {
        let mut c = ctx();
        // Inserted out of id order; the ring sorts by id: 1, 5, 9.
        c.windows.push(win(9, 0.0, 0.0, 100.0, 100.0));
        c.windows.push(win(1, 200.0, 0.0, 100.0, 100.0));
        c.windows.push(win(5, 400.0, 0.0, 100.0, 100.0));
        c.focused = Some(wid(9));
        // Next from the last entry wraps to the first.
        assert_eq!(dispatch(&c, Action::FocusNext)[0], Command::Focus(wid(1)));
        // Prev from 9 goes to 5.
        assert_eq!(dispatch(&c, Action::FocusPrev)[0], Command::Focus(wid(5)));
        // No focus: Next starts at the ring's first entry.
        c.focused = None;
        assert_eq!(dispatch(&c, Action::FocusNext)[0], Command::Focus(wid(1)));
        // Non-cyclable windows are not in the ring.
        for w in c.windows.iter_mut() {
            w.focus_cyclable = false;
        }
        assert!(dispatch(&c, Action::FocusNext).is_empty());
    }

    #[test]
    fn focus_directional_picks_by_center() {
        let mut c = ctx();
        c.windows.push(win(1, 0.0, 0.0, 100.0, 100.0));
        c.windows.push(win(2, 500.0, 0.0, 100.0, 100.0));
        c.focused = Some(wid(1));
        let cmds = dispatch(&c, Action::FocusRight);
        assert_eq!(cmds[0], Command::Focus(wid(2)));
        assert_eq!(cmds[1], Command::Raise(wid(2)));
        // Nothing to the left of window 1.
        assert!(dispatch(&c, Action::FocusLeft).is_empty());
    }

    #[test]
    fn fullscreen_toggles_and_unlocks_to_resolved_mode() {
        let mut c = ctx();
        c.focused = Some(wid(1));
        c.windows.push(win(1, 0.0, 0.0, 100.0, 100.0));
        // Enter: lock to Fullscreen.
        assert_eq!(
            dispatch(&c, Action::Fullscreen)[0],
            Command::SetWindowMode { id: wid(1), mode: TilingMode::Fullscreen, locked: true }
        );
        // Exit: unlock back to the resolved mode.
        c.windows[0].mode = TilingMode::Fullscreen;
        c.windows[0].resolved_mode = TilingMode::Tiled;
        assert_eq!(
            dispatch(&c, Action::Fullscreen)[0],
            Command::SetWindowMode { id: wid(1), mode: TilingMode::Tiled, locked: false }
        );
        // Exit when the viewport itself resolves Fullscreen: fall to Floating.
        c.windows[0].resolved_mode = TilingMode::Fullscreen;
        assert_eq!(
            dispatch(&c, Action::Fullscreen)[0],
            Command::SetWindowMode { id: wid(1), mode: TilingMode::Floating, locked: false }
        );
    }

    #[test]
    fn mode_next_cycles_and_shared_hits_all_matching() {
        let mut c = ctx();
        c.focused = Some(wid(1));
        c.windows.push(win(1, 0.0, 0.0, 100.0, 100.0));
        c.windows.push(win(2, 200.0, 0.0, 100.0, 100.0));
        let mut hidden = win(3, 400.0, 0.0, 100.0, 100.0);
        hidden.visible = false;
        c.windows.push(hidden);
        // Floating -> Fullscreen on the focused window only.
        assert_eq!(
            dispatch(&c, Action::ModeNext),
            vec![
                Command::SetWindowMode { id: wid(1), mode: TilingMode::Fullscreen, locked: true },
                Command::Relayout
            ]
        );
        // Shared: every visible window in the focused window's mode cycles;
        // the invisible one is untouched.
        assert_eq!(
            dispatch(&c, Action::ModeNextShared),
            vec![
                Command::SetWindowMode { id: wid(1), mode: TilingMode::Fullscreen, locked: true },
                Command::SetWindowMode { id: wid(2), mode: TilingMode::Fullscreen, locked: true },
                Command::Relayout
            ]
        );
    }

    #[test]
    fn zoom_in_sets_overview_and_relayouts() {
        let cmds = dispatch(&ctx(), Action::ZoomIn);
        assert_eq!(cmds.len(), 2);
        let Command::SetCamera { camera, overview, .. } = cmds[0] else { panic!() };
        assert!((camera.zoom - 1.1).abs() < 1e-9);
        assert_eq!(overview, Some(true));
        assert_eq!(cmds[1], Command::Relayout);
        // Reset from zoomed goes back to normal.
        let mut c = ctx();
        c.camera.zoom = 2.0;
        let Command::SetCamera { camera, overview, .. } = dispatch(&c, Action::ZoomReset)[0] else { panic!() };
        assert_eq!(camera.zoom, 1.0);
        assert_eq!(overview, Some(false));
    }

    #[test]
    fn pan_steps_one_aligned_cell_from_pending_target() {
        let Command::PanTo { x, y } = dispatch(&ctx(), Action::PanRight)[0] else { panic!() };
        assert_eq!((x, y), (Some(512.0), None));
        // A pending target queues the next cell from there.
        let mut c = ctx();
        c.pan_target_x = Some(512.0);
        let Command::PanTo { x, .. } = dispatch(&c, Action::PanRight)[0] else { panic!() };
        assert_eq!(x, Some(1024.0));
    }

    #[test]
    fn overview_enter_fits_eligible_windows_only() {
        let mut c = ctx();
        c.windows.push(win(1, 0.0, 0.0, 400.0, 300.0));
        let mut ineligible = win(2, 5000.0, 0.0, 400.0, 300.0);
        ineligible.overview_eligible = false;
        c.windows.push(ineligible);
        let cmds = dispatch(&c, Action::Overview);
        let Command::SetCamera { camera, overview, .. } = cmds[0] else { panic!() };
        assert_eq!(overview, Some(true));
        // Only window 1 counts: 400x300 fits without zooming out.
        assert_eq!(camera.zoom, 1.0);
        assert_eq!(cmds[1], Command::RefreshCamera);
        // No eligible windows: not claimed, nothing happens.
        c.windows.clear();
        assert!(dispatch(&c, Action::Overview).is_empty());
    }

    #[test]
    fn overview_exit_prefers_the_hovered_window() {
        let mut c = ctx();
        c.overview = true;
        c.camera.zoom = 0.5;
        c.windows.push(win(3, 1000.0, 2000.0, 400.0, 300.0));
        c.hovered = Some(wid(3));
        let cmds = dispatch(&c, Action::Overview);
        assert_eq!(cmds[0], Command::Focus(wid(3)));
        assert_eq!(cmds[1], Command::StopPanAnimation);
        let Command::SetCamera { camera, overview, .. } = cmds[2] else { panic!() };
        assert_eq!(overview, Some(false));
        assert_eq!(camera.zoom, 1.0);
        // Centered on the window's center (1200, 2150).
        assert_eq!(camera.pan_x, 1200.0 - 960.0);
        assert_eq!(camera.pan_y, 2150.0 - 540.0);
        // Without a hovered window, exit centers the point under the cursor.
        c.hovered = None;
        c.camera.pan_x = 100.0;
        let Command::SetCamera { camera, .. } = dispatch(&c, Action::Overview)[1] else { panic!() };
        // Virtual point under (960, 540) at zoom 0.5: 100 + 960/0.5 = 2020.
        assert_eq!(camera.pan_x, 2020.0 - 960.0);
    }

    #[test]
    fn media_keys_spawn_stock_or_overridden_command() {
        let c = ctx();
        // Every media action is claimed and spawns its stock command.
        for action in [
            Action::VolumeUp, Action::VolumeDown, Action::VolumeMute,
            Action::MicMute, Action::BrightnessUp, Action::BrightnessDown,
        ] {
            assert_eq!(
                dispatch(&c, action),
                vec![Command::Spawn(media_command(action).to_string())],
                "{}", action.name()
            );
        }
        assert_eq!(
            dispatch(&c, Action::VolumeMute),
            vec![Command::Spawn("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle".to_string())]
        );
        // A binding's command= property replaces the stock command.
        assert_eq!(
            DefaultPolicy.action(&c, Action::BrightnessUp, Some("brightnessctl set 10%+")),
            vec![Command::Spawn("brightnessctl set 10%+".to_string())]
        );
    }
}
