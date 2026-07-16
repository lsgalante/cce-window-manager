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
//     vocabulary (WindowId, WindowRole, DecorationSpec, Action, …). Traits
//     are a skeleton — defined but not yet driven by the compositor.
//   - `arrange`: the whole arrange pass as pure functions
//     (snapshot → plan → apply instructions).
//   - `bindings`: keybinding vocabulary — action names, chord grammar,
//     `BindingTable`, stock defaults. The compositor feeds it plain data
//     parsed from `input.kdl`; keysym name→code lookup stays mechanism-side.
//   - `tiling`: `TilingMode` and pure layout formulas.
//   - `snap`: magnetic grid snapping for interactive move/resize.
//   - `state`: persisted session state (serialization/matching only; the
//     save/load I/O stays in the compositor).
//   - `slotmap`: generational-index map (river-derived, 0BSD); `api::WindowId`
//     wraps its `Key`.

pub mod api;
pub mod arrange;
pub mod bindings;
pub mod slotmap;
pub mod snap;
pub mod state;
pub mod tiling;
