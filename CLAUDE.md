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

This directory is its **own git repository**, sitting side-by-side with the other
`cce-*` crates to form an uncommitted build workspace at the parent directory. Its
`origin` is the local *bare* repo `~/git/cce-window-manager.git`, a real pushable
remote: **committing is not publishing — `git push origin main` is**, after which
`gitsite.timer` mirrors it to `https://git.lucas.co/cce-window-manager.git` (kept as the
`published` remote; it is the old static mirror and never accepted a push).
Commit here, not at the workspace root. The crate must **build standalone** — no
`workspace = true` dependency inheritance; versions are declared in this
`Cargo.toml`.

## Commands

```sh
cargo build                      # standalone build (fast; no compositor deps)
cargo test                       # run all tests (~143 unit tests, all in-crate)
cargo test snap::                # tests in one module
cargo test -p cce-window-manager # same, from the workspace root
```

Because this crate is pure Rust, building/testing it never triggers the
compositor's native `build.rs` pipeline — prefer working here directly when the
change is policy-side.

Tests live in `#[cfg(test)]` modules at the bottom of the module they cover —
every module has one except `api.rs` and `state.rs`, which are plain-data
vocabulary. This crate is where the DE's testable logic is concentrated —
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
  (`Background`/`StatusBar`/`Grid`/`Hidden`/`Overlay`/`Normal`); an Overlay
  window mid-drag arranges as Normal, and `Grid` is the world-anchored grid
  client, placed at its patch's virtual origin with scale `zoom / patch.scale`.
- Placement is composed from per-section pure functions — `compute_usable_area`,
  `place_overlay_window`, `place_normal_window`, `layout_status_bars`,
  `tiled_transition` — each individually callable and tested.
- `tiled_transition()` is a state-machine step (Enter saves restore geometry,
  Exit restores it); the saved state itself lives on the mechanism side. It was
  `maximized_transition` until the mode it steps was renamed `Maximized` →
  `Tiled` (see `tiling.rs` below).

### The `Policy` / `Compositor` trait boundary (`api.rs` + `actions.rs`)

`api.rs` defines the plain-data vocabulary (`WindowId`, `WindowRole`, `Action`,
`Rect`, `ActionCtx`, `Command`, `DecorationSpec`, `EffectSpec`,
`BackgroundSpec`, …) and two traits, **snapshot-style** (the arrange-pass
convention — the mechanism owns all state): `Policy::action(ctx, action) ->
Vec<Command>` decides against a mechanism-built `ActionCtx` snapshot, and
`Compositor::apply(cmd)` (implemented by the compositor's `WindowManager`)
executes one command at a time. `actions.rs` holds `DefaultPolicy`, the live
`Policy` impl: the camera actions (keyed zoom, cell-aligned pans, View jumps,
SetViewport sends, Overview both directions) are decided there; an **empty
command list means "not mine"** and the compositor falls through to its legacy
arms. New flows grow snapshot methods here only alongside a real mechanism
caller — no speculative signatures. Effects are declarative on purpose: new
scenefx capabilities extend `EffectSpec` without changing either trait.
`WindowRole::from_app_id()` is the single place the special app_id conventions
(`cce-wallpaper`, `cce-status*`) are interpreted.

### Grid snapping (`snap.rs`)

Magnetic snapping math for interactive move/resize, the hard grid snap for
`Tiled` windows (`tiled_span`), and `is_cell_aligned` — the geometric test
that decides whether a window IS tiled (every content edge on a visible cell
edge). Conventions that everything here assumes:

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
  `input.kdl`, plus legacy aliases (`close`, `fullscreen`, `expose`, `toggle_overview`).
  `overview` toggles; `overview_enter` / `overview_exit` are its one-way
  halves, for users who want a key per direction. Asking for the mode you
  are already in returns no commands — a deliberate no-op, since the
  mechanism has no legacy arm for either action to fall through to. Like
  the zoom chords, both ship unbound. `overview_exit` lands on the FOCUSED
  window, where the toggle lands on the hovered one: a key press carries no
  cursor position, so the pointer's resting place is not evidence of where
  the user meant to go.
- `move_window_left` / `_right` / `_up` / `_down` (`actions::move_window`) step
  the focused window one grid period. A **tiled** window (`move_tiled`) swaps
  with the tiled window already holding the destination: an aligned window
  stays aligned without snapping, and occupancy is a rect overlap against the
  destination rather than a cell-index comparison — equivalent, and it needs
  nothing added to `ActionCtx`. A swap exchanges origins, not boxes, so each
  window keeps its size. A **floating** window (`move_floating`) just moves by
  the same period and covers whatever is there — floating windows overlap
  freely, so there is nothing to swap with; it is also the keyboard's only
  way to bring a floating window back on screen after a restore parks it off
  the viewport. Declines (no commands, hence a no-op) when nothing is focused,
  the focused window is in any other mode (Fullscreen, the internal roles),
  the grid is degenerate, or — tiled only — MORE than one tiled window is in
  the destination, where "swap with it" names no particular window. Unbound
  by default; reachable as `ccectl move-window-left` etc., the relative
  counterpart to `ccectl move-window <square>`.
- `parse_chord("super+shift+h")` — strict chord grammar; the key stays an XKB
  keysym *name* (`Chord.key: String`) because name→code lookup needs xkbcommon.
- `BindingTable` — insertion order is priority order (`resolve` = first match,
  mirroring the compositor's dispatch loop). `add` warns-by-return on shadowing;
  `add_default` never shadows. The compositor loads input.kdl entries first,
  then legacy config.kdl bindings, then `DEFAULT_BINDINGS`.
- The compositor re-exports `bindings::Binding` as `crate::config::Keybind`.

### Supporting modules

- `background.rs` — per-frame desktop-grid geometry: `grid_frame(spec, cam,
  viewport, output)` → `GridFrame` (modulo tree shift for the infinite grid,
  backdrop extent, density-faded cell lattice with safety caps). The
  compositor's `output.rs` keeps the scene rects/pool and scenefx encodings;
  `Layout::background_spec()` builds the `api::GridSpec`.
- `cells.rs` — chess-style addressing for desktop-grid squares: the origin
  square is `A1`, letters run right and numbers run DOWN, both 1-based with
  no zero (`-A1` is left of the origin, `A-1` above it, columns past Z carry
  on Excel-style). Provides `square_label`/`parse_square`, `square_rect` /
  `block_rect`, `window_span` (which squares a window covers) and
  `remap_block` (re-tile a block across a grid-geometry change). Its grid
  math must agree with `snap.rs` exactly — same virtual-surface content
  coordinates, same period/inset — or a "tiled" window would not land on a
  named square.
- `spawn.rs` — where a window launched *at* a square should land, as opposed
  to reopening where it last was. `place_at_cell` keeps the invocation square
  as one of the block's corners and picks WHICH corner by growing away from
  what is already there (top-left preferred, then the others, scored by
  collisions first and off-screen area second); `nearest_free` then steps a
  block off anything still occupying it. It never searches for somewhere
  else to be, so the result stays predictable.
- `camera.rs` — viewport pan/zoom math (`Camera` = pan_x/pan_y/zoom):
  `zoom_about_anchor` (wheel zoom at cursor, keyed zoom at viewport center),
  `center_on`, `fit_bounds` (overview fit), `visible_fraction` +
  `FOCUS_VISIBLE_THRESHOLD` (focus-follow panning), `is_overview`. The
  mechanism owns the actual fields and animation; these are pure maps.
  `recalled_origin` decides where a remembered FLOATING window reopens: its
  remembered origin when at least `RESTORE_VISIBLE_MIN` (a quarter) of it
  would be in view, or when it is `on_tiled_desk` (overlaps the tiled
  windows' bounding box inflated by one viewport — the caller passes the
  box, `None` for no tiled windows), else centered in the current view — a
  floating window a screen away from the camera AND from every tiled window
  is lost, not remembered; one parked beside a column is placed. Tiled
  windows are the grid's and never go through it.
- `ramp.rs` — speed-ramp evaluation for duration-based camera transitions.
  `SpeedRamp::from_spec` parses the DE-wide ramp spec string cce-ui's Ramp
  widget writes and integrates that SPEED profile into a cumulative
  `progress(t)` curve normalized to end at exactly 1 (so any profile arrives
  on target; zero-speed segments read as dwell, an all-zero ramp yields
  `None` and callers fall back to their non-ramp animation). The parser and
  interpolation are deliberately MIRRORED from cce-ui rather than shared —
  this crate stays dependency-minimal — so the two must be kept in step.
- `focus.rs` — directional focus selection (`directional_focus` over window
  center points in virtual coordinates; no wraparound, off-axis distance is
  penalized). Consumed by the compositor's `FocusUp/Down/Left/Right` action
  arm; default chords are super+k/j/h/l via `bindings::DEFAULT_BINDINGS`.
- `pan.rs` — cell-aligned viewport panning: `aligned_step` gives the keyed
  PanLeft/… actions their animation targets (pan offsets that are multiples
  of the grid period).
- `tiling.rs` — `TilingMode` enum: a window is `Floating` or `Tiled` (all
  content edges on visible desktop-grid cell edges; tiled windows report the
  xdg maximized state), plus `Fullscreen` and the internal `Popup` /
  `Overlay` / `Status` / `Utility` roles. `Utility` is `Status` minus the
  docking: a tool window whose shape its own contents decide, floating and
  movable like any window but offered no resize affordance and given no saved
  geometry. It is never inferred from a sizing hint — only the client declares
  it, via `set_utility` on the cce window-management protocol. Serialized into
  saved state — serde aliases map the retired names (`Cascade`/`Grid` →
  `Floating`, `Maximized` → `Tiled`); keep aliases when renaming variants.
- `overview.rs` — overview-mode move rules: `displace` relocates windows a
  drag covers (past an overlap threshold) to the side the drag vacated,
  called by the mechanism on every motion event of an overview move.
- `query.rs` — window-query resolution: how a user-supplied query string
  (`ccectl focus-window` / `center-window`, window-stream subscriptions)
  picks a window. An all-numeric query is tried as an exact window id first,
  then matched case-insensitively against app_ids with exact beating
  substring. The mechanism supplies the candidates (mapped windows, in
  window order); this module owns only the matching rules.
- `state.rs` — `SavedState` / `SavedWindowState` serde types, plus
  `SavedGrid`: the grid the file's geometries were measured under, so a
  session under a different grid re-tiles Tiled entries onto their squares
  (`cells::remap_block`) instead of growing them from misaligned pixels.
  New fields need `#[serde(default)]` to keep old state files loadable.
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
