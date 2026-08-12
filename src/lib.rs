// cce-window-manager — the window-management policy layer of the cce
// desktop, split out of the compositor crate (`cce-fx`).
//
// Everything here is pure Rust — no FFI, no raw wlroots pointers. The
// mechanism side (scene graph, seats, shells, sockets) lives in the `cce`
// compositor crate, which depends on this one and talks to policy code only
// through the types in `api`.
//
// Modules:
//   - `api`: the `Policy` / `Compositor` trait boundary and the plain-data
//     vocabulary (WindowId, WindowRole, Action, ActionCtx, Command, …).
//     Snapshot-style: policy methods take mechanism-built snapshots and
//     return command lists; the compositor applies them.
//   - `actions`: `DefaultPolicy` — the live `Policy` impl; camera actions
//     (zoom/pan/view/overview) are decided here.
//   - `arrange`: the whole arrange pass as pure functions
//     (snapshot → plan → apply instructions).
//   - `bindings`: keybinding vocabulary — action names, chord grammar,
//     `BindingTable`, stock defaults. The compositor feeds it plain data
//     parsed from `input.kdl`; keysym name→code lookup stays mechanism-side.
//   - `background`: per-frame desktop-grid geometry (`grid_frame`) —
//     modulo tree shift, density fade, cell lattice with safety caps.
//   - `camera`: viewport pan/zoom math — keyed/wheel zoom about an anchor,
//     centering, overview fit, focus-follow visibility.
//   - `focus`: directional focus selection (which window is "up/left/…"
//     of the focused one) over virtual-surface center points.
//   - `tiling`: `TilingMode` — `Tiled`/`Floating` plus the internal roles.
//   - `overview`: overview-mode move rules — displacing windows a drag
//     covers to the vacated side.
//   - `pan`: cell-aligned viewport panning (the PanLeft/… step targets).
//   - `query`: window-query resolution (ccectl focus-window etc.) — numeric
//     id first, then app_id with exact-beats-substring.
//   - `snap`: magnetic grid snapping for interactive move/resize.
//   - `state`: persisted session state (serialization/matching only; the
//     save/load I/O stays in the compositor).
//   - `slotmap`: generational-index map (river-derived, 0BSD); `api::WindowId`
//     wraps its `Key`.

pub mod actions;
pub mod api;
pub mod arrange;
pub mod background;
pub mod bindings;
pub mod camera;
pub mod focus;
pub mod overview;
pub mod pan;
pub mod query;
pub mod ramp;
pub mod slotmap;
pub mod snap;
pub mod state;
pub mod tiling;
