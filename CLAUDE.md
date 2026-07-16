# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this crate is

`cce-window-manager` is the **pure-Rust window-management policy layer** of the cce
Wayland desktop, extracted from the `cce` compositor crate (`cce-fx`). It contains
no FFI, no wlroots pointers, and only two dependencies (`log`, `serde`). The
compositor is its sole consumer: it depends on this crate by path and re-exports it
as `crate::policy` / `crate::tiling` / `crate::slotmap`.

The governing split is **policy vs. mechanism**:

- **Policy (this crate)** decides placement, focus, decoration, and background —
  as pure functions over plain-data snapshots.
- **Mechanism (the `cce` compositor)** owns the scene graph, seats, shells, and
  sockets. It builds snapshots from FFI state, calls into this crate, and applies
  the returned plans.

Nothing here does I/O. Even `state.rs` (persisted session state for
`~/.local/state/cce/state.json`) is serialization/matching only — the save/load
I/O lives in the compositor's `window_manager.rs`.

## Version control

This directory is its **own git repository**
(codeberg.org/lsgalante/cce-window-manager), cloned side-by-side with the other
`cce-*` crates to form an uncommitted build workspace at the parent directory.
Commit here, not at the workspace root. The crate must **build standalone** — no
`workspace = true` dependency inheritance; versions are declared in this
`Cargo.toml`.

## Commands

```sh
cargo build                      # standalone build (fast; no compositor deps)
cargo test                       # run all tests (38 unit tests, all in-crate)
cargo test snap::                # tests in one module
cargo test -p cce-window-manager # same, from the workspace root
```

Because this crate is pure Rust, building/testing it never triggers the
compositor's native `build.rs` pipeline — prefer working here directly when the
change is policy-side.

Tests live in `#[cfg(test)]` modules inside `arrange.rs`, `snap.rs`, and
`slotmap.rs`. This crate is where the DE's testable logic is concentrated —
placement/snapping changes should come with unit tests (the existing test
modules show the style: small numeric scenarios with worked-out expectations in
comments).

## Architecture

### The arrange pass: snapshot → plan → apply (`arrange.rs`)

The core of the crate. The compositor builds `WindowSnapshot`s / `OutputSnapshot`s
/ `ArrangeParams` once per frame (seat-dependent answers like `being_moved` and
`active_resize` are captured into the snapshot so the pure pass never queries
mid-computation), then `arrange()` returns an `ArrangePlan` of per-window
`WindowPlan` write instructions. Every `WindowPlan` field is an `Option`: `None`
means "leave untouched", so the mechanism apply loop is a flat sequence of
`if let Some` writes.

Key conventions inside the pass:

- The output loop is **last-wins**: every output pass re-plans every window, so
  with multiple outputs the final plan reflects the last one (mirroring the
  mechanism loop it replaced).
- `classify_window()` maps each window to a `WindowClass`
  (`Background`/`StatusBar`/`Hidden`/`Overlay`/`Normal`); an Overlay window
  mid-drag arranges as Normal.
- Placement is composed from per-section pure functions — `compute_usable_area`,
  `place_overlay_window`, `place_normal_window`, `layout_status_bars`,
  `maximized_transition` — each individually callable and tested.
- `maximized_transition()` is a state-machine step (Enter saves restore geometry,
  Exit restores it); the saved state itself lives on the mechanism side.

### The `Policy` / `Compositor` trait boundary (`api.rs`)

`api.rs` defines the plain-data vocabulary (`WindowId`, `WindowRole`, `Action`,
`Rect`, `DecorationSpec`, `EffectSpec`, `BackgroundSpec`, …) and two traits:
`Policy` (events from mechanism → policy) and `Compositor` (commands from policy
→ mechanism). **Skeleton status: nothing implements these traits yet** — the
migration plan is to route the compositor's window lifecycle, input actions, and
animation tick through them. Effects are declarative on purpose: new scenefx
capabilities extend `EffectSpec` without changing either trait.
`WindowRole::from_app_id()` is the single place the special app_id conventions
(`cce-wallpaper`, `cce-status*`) are interpreted.

### Grid snapping (`snap.rs`)

Magnetic snapping math for interactive move/resize, plus the hard grid snap for
`Maximized` windows. Conventions that everything here assumes:

- Coordinates are **virtual-surface content coordinates**.
- Snapping is **border-inclusive**: the border's *outer* edge lands on the snap
  target (content is inset by `border_width`).
- Targets are the **visible cell edges**, not raw grid lines: the desktop grid
  has period `cell_size + gap_width` and each cell fades inward by `cell_inset`,
  so left/top edges snap to `k*period + inset` and right/bottom edges to
  `k*period + cell_size - inset`.
- `resize_axis()` is the **single source** of interactive-resize sizing — both
  the compositor's seat op and the arrange snapshot derive sizes from it, so a
  snapped result can't be overridden by an unsnapped recomputation. Don't add a
  second place that computes resize sizes.

### Keybindings (`bindings.rs`)

The crate owns what a binding *means*; the compositor owns the physical half
(reading `~/.config/cce/input.kdl`, XKB keysym lookup, key delivery).

- `Action::name()` / `Action::from_name()` (in `api.rs`) — the canonical
  snake_case action names users write in the `cce-window-manager` domain of
  `input.kdl`, plus legacy aliases (`close`, `fullscreen`, `toggle_overview`).
- `parse_chord("super+shift+h")` — strict chord grammar; the key stays an XKB
  keysym *name* (`Chord.key: String`) because name→code lookup needs xkbcommon.
- `BindingTable` — insertion order is priority order (`resolve` = first match,
  mirroring the compositor's dispatch loop). `add` warns-by-return on shadowing;
  `add_default` never shadows. The compositor loads input.kdl entries first,
  then legacy config.kdl bindings, then `DEFAULT_BINDINGS`.
- The compositor re-exports `bindings::Binding` as `crate::config::Keybind`.

### Supporting modules

- `tiling.rs` — `TilingMode` enum (serialized into saved state — renaming
  variants breaks `state.json` compatibility) and the cascade/grid/fullscreen
  tiling formulas.
- `state.rs` — `SavedState` / `SavedWindowState` serde types. New fields need
  `#[serde(default)]` to keep old state files loadable.
- `slotmap.rs` — generational-index map (river-derived, 0BSD-licensed — keep the
  SPDX header). `api::WindowId` wraps its `Key`.

## Hard constraints

- **No FFI, no I/O, no compositor types.** If a change needs scene-graph access,
  a socket, or a file, that half belongs in the `cce` compositor; this crate gets
  the pure computation and plain-data types.
- Dependencies are intentionally minimal (`log`, `serde`). Adding one is a design
  decision, not a convenience.
- When policy code needs a new fact about a window/output, add it to the
  snapshot structs and have the mechanism fill it in — never query back into the
  compositor from inside the pure pass.
