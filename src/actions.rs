// Action dispatch policy: `DefaultPolicy` decides user actions as pure
// snapshot → command mappings. The camera actions (zoom, pan, viewport
// jumps, overview) live here, moved verbatim from the compositor's
// execute_action arms; an action this policy doesn't claim returns an empty
// vec and the mechanism's remaining legacy arms handle it.

use crate::api::{Action, ActionCtx, Command, Policy, WindowId};
use crate::camera::{self, Camera};
use crate::pan;

pub struct DefaultPolicy;

impl Policy for DefaultPolicy {
    fn action(&mut self, ctx: &ActionCtx, action: Action) -> Vec<Command> {
        match action {
            Action::ZoomIn | Action::ZoomOut | Action::ZoomReset => zoom(ctx, action),
            Action::PanLeft | Action::PanRight | Action::PanUp | Action::PanDown => {
                pan_step(ctx, action)
            }
            Action::View1 | Action::View2 | Action::View3 | Action::View4 => view(ctx, action),
            Action::SetViewport1
            | Action::SetViewport2
            | Action::SetViewport3
            | Action::SetViewport4 => set_viewport(ctx, action),
            Action::Expose => expose(ctx),
            _ => Vec::new(),
        }
    }
}

/// The four fixed viewport anchors shared by View1-4 and SetViewport1-4.
fn anchor_point(action: Action) -> (f64, f64) {
    match action {
        Action::View1 | Action::SetViewport1 => (0.0, 0.0),
        Action::View2 | Action::SetViewport2 => (2000.0, 0.0),
        Action::View3 | Action::SetViewport3 => (0.0, 2000.0),
        _ => (2000.0, 2000.0),
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
        Command::SetCamera { camera: cam, overview: Some(camera::is_overview(cam.zoom)) },
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
        x = Some(pan::aligned_step(base, ctx.grid_period, dx));
    }
    if dy != 0.0 {
        let base = ctx.pan_target_y.unwrap_or(ctx.camera.pan_y);
        y = Some(pan::aligned_step(base, ctx.grid_period, dy));
    }
    vec![Command::PanTo { x, y }]
}

/// Jump the viewport to one of the four fixed anchors, keeping the zoom.
fn view(ctx: &ActionCtx, action: Action) -> Vec<Command> {
    let (tx, ty) = anchor_point(action);
    let cam = camera::center_on(tx, ty, ctx.viewport_w, ctx.viewport_h, ctx.camera.zoom);
    vec![Command::SetCamera { camera: cam, overview: None }, Command::Relayout]
}

/// Send the focused window to one of the four fixed anchors (centered on
/// it). Note the legacy quirk kept as-is: the window extent is output px,
/// halved without dividing by zoom.
fn set_viewport(ctx: &ActionCtx, action: Action) -> Vec<Command> {
    let (tx, ty) = anchor_point(action);
    let Some(id) = ctx.focused else { return Vec::new() };
    let Some(win) = window(ctx, id) else { return Vec::new() };
    vec![
        Command::MoveWindow { id, x: tx - win.w / 2.0, y: ty - win.h / 2.0 },
        Command::Relayout,
    ]
}

/// Toggle overview. Exit re-centers at zoom 1 — on the hovered window
/// (focusing it) when there is one, else on the virtual point under the
/// cursor — in the cursor's output. Enter fits the bounding box of all
/// eligible windows into the first enabled output.
fn expose(ctx: &ActionCtx) -> Vec<Command> {
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
                    Command::SetCamera { camera: cam, overview: Some(false) },
                    Command::RefreshCamera,
                ];
            }
        }
        let vx = ctx.camera.pan_x + (ctx.cursor_x - out.x as f64) / ctx.camera.zoom;
        let vy = ctx.camera.pan_y + (ctx.cursor_y - out.y as f64) / ctx.camera.zoom;
        let cam = camera::center_on(vx, vy, ow, oh, 1.0);
        vec![
            Command::StopPanAnimation,
            Command::SetCamera { camera: cam, overview: Some(false) },
            Command::RefreshCamera,
        ]
    } else {
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for w in ctx.windows.iter().filter(|w| w.expose_eligible) {
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
        // smaller than the screen): the next Expose must exit, not re-enter.
        vec![
            Command::SetCamera { camera: cam, overview: Some(true) },
            Command::RefreshCamera,
        ]
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
            grid_period: 512.0,
            windows: Vec::new(),
        }
    }

    fn dispatch(ctx: &ActionCtx, action: Action) -> Vec<Command> {
        DefaultPolicy.action(ctx, action)
    }

    #[test]
    fn unclaimed_actions_return_empty() {
        assert!(dispatch(&ctx(), Action::Close).is_empty());
        assert!(dispatch(&ctx(), Action::Spawn).is_empty());
    }

    #[test]
    fn zoom_in_sets_overview_and_relayouts() {
        let cmds = dispatch(&ctx(), Action::ZoomIn);
        assert_eq!(cmds.len(), 2);
        let Command::SetCamera { camera, overview } = cmds[0] else { panic!() };
        assert!((camera.zoom - 1.1).abs() < 1e-9);
        assert_eq!(overview, Some(true));
        assert_eq!(cmds[1], Command::Relayout);
        // Reset from zoomed goes back to normal.
        let mut c = ctx();
        c.camera.zoom = 2.0;
        let Command::SetCamera { camera, overview } = dispatch(&c, Action::ZoomReset)[0] else { panic!() };
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
    fn set_viewport_needs_focus_and_centers_it() {
        assert!(dispatch(&ctx(), Action::SetViewport2).is_empty());
        let mut c = ctx();
        c.focused = Some(wid(7));
        c.windows.push(ActionWindow { id: wid(7), x: 0.0, y: 0.0, w: 400.0, h: 300.0, expose_eligible: true });
        let cmds = dispatch(&c, Action::SetViewport2);
        assert_eq!(cmds[0], Command::MoveWindow { id: wid(7), x: 1800.0, y: -150.0 });
        assert_eq!(cmds[1], Command::Relayout);
    }

    #[test]
    fn expose_enter_fits_eligible_windows_only() {
        let mut c = ctx();
        c.windows.push(ActionWindow { id: wid(1), x: 0.0, y: 0.0, w: 400.0, h: 300.0, expose_eligible: true });
        c.windows.push(ActionWindow { id: wid(2), x: 5000.0, y: 0.0, w: 400.0, h: 300.0, expose_eligible: false });
        let cmds = dispatch(&c, Action::Expose);
        let Command::SetCamera { camera, overview } = cmds[0] else { panic!() };
        assert_eq!(overview, Some(true));
        // Only window 1 counts: 400x300 fits without zooming out.
        assert_eq!(camera.zoom, 1.0);
        assert_eq!(cmds[1], Command::RefreshCamera);
        // No eligible windows: not claimed, nothing happens.
        c.windows.clear();
        assert!(dispatch(&c, Action::Expose).is_empty());
    }

    #[test]
    fn expose_exit_prefers_the_hovered_window() {
        let mut c = ctx();
        c.overview = true;
        c.camera.zoom = 0.5;
        c.windows.push(ActionWindow { id: wid(3), x: 1000.0, y: 2000.0, w: 400.0, h: 300.0, expose_eligible: true });
        c.hovered = Some(wid(3));
        let cmds = dispatch(&c, Action::Expose);
        assert_eq!(cmds[0], Command::Focus(wid(3)));
        assert_eq!(cmds[1], Command::StopPanAnimation);
        let Command::SetCamera { camera, overview } = cmds[2] else { panic!() };
        assert_eq!(overview, Some(false));
        assert_eq!(camera.zoom, 1.0);
        // Centered on the window's center (1200, 2150).
        assert_eq!(camera.pan_x, 1200.0 - 960.0);
        assert_eq!(camera.pan_y, 2150.0 - 540.0);
        // Without a hovered window, exit centers the point under the cursor.
        c.hovered = None;
        c.camera.pan_x = 100.0;
        let Command::SetCamera { camera, .. } = dispatch(&c, Action::Expose)[1] else { panic!() };
        // Virtual point under (960, 540) at zoom 0.5: 100 + 960/0.5 = 2020.
        assert_eq!(camera.pan_x, 2020.0 - 960.0);
    }
}
