// Keybinding vocabulary and resolution for the window-manager domain.
//
// This crate owns what a binding MEANS: the action names users write in
// `input.kdl` (`Action::from_name`), the chord grammar ("super+shift+h"),
// the resolved table, and the stock defaults. The mechanism side owns the
// physical half: reading the file, XKB keysym lookup, and key delivery.
//
// A `Chord` keeps its key as an XKB keysym NAME — resolving names to keysym
// codes needs xkbcommon, so the compositor does that and this crate only
// ever sees the resulting `u32`.
//
// Touchpad gestures share the domain: an entry whose "key" is a gesture name
// (`swipe3_left`, `pinch_out`) is a `GestureChord`, not a `Chord`, and the
// compositor matches it against libinput swipe/pinch events instead of key
// presses. `parse_gesture` is tried first so the two grammars never collide.

use super::api::Action;

/// Modifier bitmask values (river seat conventions — the same values the
/// compositor has always packed into its keybind masks).
pub mod mods {
    pub const SHIFT: u32 = 0x01;
    pub const CTRL: u32 = 0x04;
    pub const ALT: u32 = 0x08;
    pub const SUPER: u32 = 0x40;
}

fn mod_from_name(name: &str) -> Option<u32> {
    match name {
        "shift" => Some(mods::SHIFT),
        "ctrl" | "control" => Some(mods::CTRL),
        "alt" | "mod1" | "meta" => Some(mods::ALT),
        "super" | "mod4" | "logo" | "win" => Some(mods::SUPER),
        _ => None,
    }
}

/// A parsed key chord: modifier mask plus the key's XKB keysym name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub mods: u32,
    pub key: String,
}

/// Parse `"super+shift+h"` → mods SUPER|SHIFT, key `"h"`. The last segment
/// is the key (an XKB keysym name, e.g. `slash`, `equal`, `Left`); every
/// segment before it must be a known modifier. Strict on purpose: a typo'd
/// modifier returns `None` so the loader can warn, instead of silently
/// binding the wrong chord.
pub fn parse_chord(s: &str) -> Option<Chord> {
    let mut mods = 0u32;
    let mut segments = s.split('+').map(str::trim);
    let key = segments.next_back()?;
    if key.is_empty() {
        return None;
    }
    for seg in segments {
        mods |= mod_from_name(&seg.to_lowercase())?;
    }
    Some(Chord { mods, key: key.to_string() })
}

/// Which libinput gesture a `GestureChord` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureKind {
    Swipe,
    Pinch,
}

impl GestureKind {
    /// The name the compositor's gesture table keys on.
    pub fn as_str(self) -> &'static str {
        match self {
            GestureKind::Swipe => "swipe",
            GestureKind::Pinch => "pinch",
        }
    }
}

/// A parsed gesture chord: modifier mask plus the gesture. `fingers` is
/// `None` for the fingerless spelling (`"swipe_down"`), which the
/// compositor binds to both three- and four-finger gestures — the legacy
/// `window_manager { toggle_overview "swipe_down" }` meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GestureChord {
    pub mods: u32,
    pub kind: GestureKind,
    pub fingers: Option<u32>,
    /// `left` / `right` / `up` / `down` for a swipe, `in` / `out` for a pinch.
    pub direction: String,
}

/// Parse `"super+swipe3_left"` → mods SUPER, three-finger swipe left. The
/// last `+` segment is the gesture — `swipe` or `pinch`, an optional finger
/// count (2–4, what libinput reports), `_` or `-`, then the direction —
/// and every segment before it must be a known modifier. Case-insensitive.
/// Returns `None` for anything else, including a plain key chord, so
/// callers try this before `parse_chord`.
pub fn parse_gesture(s: &str) -> Option<GestureChord> {
    let mut mods = 0u32;
    let mut segments = s.split('+').map(str::trim);
    let gesture = segments.next_back()?.to_lowercase().replace('-', "_");
    for seg in segments {
        mods |= mod_from_name(&seg.to_lowercase())?;
    }
    let (kind, rest) = if let Some(rest) = gesture.strip_prefix("swipe") {
        (GestureKind::Swipe, rest)
    } else if let Some(rest) = gesture.strip_prefix("pinch") {
        (GestureKind::Pinch, rest)
    } else {
        return None;
    };
    let (count, direction) = rest.split_once('_')?;
    let fingers = if count.is_empty() {
        None
    } else {
        let n: u32 = count.parse().ok()?;
        if !(2..=4).contains(&n) {
            return None;
        }
        Some(n)
    };
    let valid = match kind {
        GestureKind::Swipe => matches!(direction, "left" | "right" | "up" | "down"),
        GestureKind::Pinch => matches!(direction, "in" | "out"),
    };
    if !valid {
        return None;
    }
    Some(GestureChord { mods, kind, fingers, direction: direction.to_string() })
}

/// One resolved binding: chord (mods + keysym code) → action, with the
/// command argument for `Spawn`/`Toggle` (required) and the media-key
/// actions (optional override of their stock command).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub mods: u32,
    pub keysym: u32,
    pub action: Action,
    pub command: Option<String>,
}

/// The resolved binding table. Insertion order is priority order: `resolve`
/// returns the first match, so load primary sources before fallbacks and
/// use `add_default` for anything that must not shadow what's already there.
#[derive(Debug, Clone, Default)]
pub struct BindingTable {
    bindings: Vec<Binding>,
}

impl BindingTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains(&self, mods: u32, keysym: u32) -> bool {
        self.bindings.iter().any(|b| b.mods == mods && b.keysym == keysym)
    }

    /// Push unconditionally. Returns `true` if an earlier binding already
    /// claims the chord (the new one is shadowed) so the caller can warn.
    pub fn add(&mut self, binding: Binding) -> bool {
        let shadowed = self.contains(binding.mods, binding.keysym);
        self.bindings.push(binding);
        shadowed
    }

    /// Push only if the chord is still free. Returns whether it was added.
    pub fn add_default(&mut self, binding: Binding) -> bool {
        if self.contains(binding.mods, binding.keysym) {
            false
        } else {
            self.bindings.push(binding);
            true
        }
    }

    /// First match wins, mirroring the compositor's dispatch loop.
    pub fn resolve(&self, mods: u32, keysym: u32) -> Option<&Binding> {
        self.bindings.iter().find(|b| b.mods == mods && b.keysym == keysym)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Binding> {
        self.bindings.iter()
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub fn into_bindings(self) -> Vec<Binding> {
        self.bindings
    }
}

/// A stock binding: chord with the key still as a keysym name.
#[derive(Debug, Clone, Copy)]
pub struct DefaultBinding {
    pub mods: u32,
    pub key: &'static str,
    pub action: Action,
}

/// The built-in fallback set (previously hardcoded in the compositor's
/// config loader). Applied with `add_default` after every configured source,
/// so any of these chords can be rebound in `input.kdl`.
pub const DEFAULT_BINDINGS: &[DefaultBinding] = &[
    DefaultBinding { mods: mods::SUPER, key: "k", action: Action::FocusUp },
    DefaultBinding { mods: mods::SUPER, key: "j", action: Action::FocusDown },
    DefaultBinding { mods: mods::SUPER, key: "h", action: Action::FocusLeft },
    DefaultBinding { mods: mods::SUPER, key: "l", action: Action::FocusRight },
    DefaultBinding { mods: mods::SUPER, key: "Left", action: Action::OverlayLeft },
    DefaultBinding { mods: mods::SUPER, key: "Right", action: Action::OverlayRight },
    DefaultBinding { mods: 0, key: "Print", action: Action::Screenshot },
    DefaultBinding { mods: mods::SUPER | mods::CTRL, key: "Up", action: Action::PanUp },
    DefaultBinding { mods: mods::SUPER | mods::CTRL, key: "Down", action: Action::PanDown },
    DefaultBinding { mods: mods::SUPER | mods::CTRL, key: "Left", action: Action::PanLeft },
    DefaultBinding { mods: mods::SUPER | mods::CTRL, key: "Right", action: Action::PanRight },
    // Zoom chords ship UNBOUND by default: they live in the user's
    // input.kdl (cce-window-manager domain: zoom_in / zoom_out /
    // zoom_reset) rather than in this table.
    DefaultBinding { mods: mods::SUPER | mods::SHIFT, key: "r", action: Action::Reload },
    // Reverse companion to the (user-configured) super+tab window switcher.
    DefaultBinding { mods: mods::SUPER | mods::SHIFT, key: "Tab", action: Action::WindowSwitcherPrev },
    // Media keys. Each action spawns a stock wpctl/brightnessctl command
    // (see `actions::media_command`); an input.kdl binding can rebind the
    // chord and/or override the command with a `command="..."` property.
    DefaultBinding { mods: 0, key: "XF86AudioRaiseVolume", action: Action::VolumeUp },
    DefaultBinding { mods: 0, key: "XF86AudioLowerVolume", action: Action::VolumeDown },
    DefaultBinding { mods: 0, key: "XF86AudioMute", action: Action::VolumeMute },
    DefaultBinding { mods: 0, key: "XF86AudioMicMute", action: Action::MicMute },
    DefaultBinding { mods: 0, key: "XF86MonBrightnessUp", action: Action::BrightnessUp },
    DefaultBinding { mods: 0, key: "XF86MonBrightnessDown", action: Action::BrightnessDown },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chord_splits_mods_and_key() {
        assert_eq!(
            parse_chord("super+shift+h"),
            Some(Chord { mods: mods::SUPER | mods::SHIFT, key: "h".into() })
        );
        assert_eq!(parse_chord("escape"), Some(Chord { mods: 0, key: "escape".into() }));
        // Keysym names keep their case; modifiers are case-insensitive.
        assert_eq!(
            parse_chord("Super+Ctrl+Left"),
            Some(Chord { mods: mods::SUPER | mods::CTRL, key: "Left".into() })
        );
    }

    #[test]
    fn parse_chord_rejects_invalid() {
        assert_eq!(parse_chord(""), None);
        assert_eq!(parse_chord("super+"), None); // empty key
        assert_eq!(parse_chord("hyper+x"), None); // unknown modifier
    }

    #[test]
    fn parse_gesture_reads_kind_fingers_and_direction() {
        assert_eq!(
            parse_gesture("swipe3_left"),
            Some(GestureChord { mods: 0, kind: GestureKind::Swipe, fingers: Some(3), direction: "left".into() })
        );
        assert_eq!(
            parse_gesture("Super+Swipe4-Down"),
            Some(GestureChord { mods: mods::SUPER, kind: GestureKind::Swipe, fingers: Some(4), direction: "down".into() })
        );
        // The fingerless legacy spelling means "three or four fingers".
        assert_eq!(
            parse_gesture("swipe_down"),
            Some(GestureChord { mods: 0, kind: GestureKind::Swipe, fingers: None, direction: "down".into() })
        );
        assert_eq!(
            parse_gesture("pinch_out"),
            Some(GestureChord { mods: 0, kind: GestureKind::Pinch, fingers: None, direction: "out".into() })
        );
        assert_eq!(GestureKind::Swipe.as_str(), "swipe");
    }

    #[test]
    fn parse_gesture_rejects_keys_and_nonsense() {
        // Plain key chords are not gestures — they fall through to parse_chord.
        assert_eq!(parse_gesture("super+shift+h"), None);
        assert_eq!(parse_gesture("s"), None);
        assert_eq!(parse_gesture("swipe"), None); // no direction
        assert_eq!(parse_gesture("swipe3"), None);
        assert_eq!(parse_gesture("swipe3_in"), None); // pinch direction on a swipe
        assert_eq!(parse_gesture("pinch_left"), None);
        assert_eq!(parse_gesture("swipe5_left"), None); // libinput reports 2–4
        assert_eq!(parse_gesture("swipe0_left"), None);
        assert_eq!(parse_gesture("hyper+swipe3_left"), None); // unknown modifier
    }

    #[test]
    fn action_names_round_trip() {
        // Every variant's canonical name resolves back to the variant.
        for action in [
            Action::None, Action::Spawn, Action::Toggle, Action::Close,
            Action::FocusNext, Action::FocusPrev, Action::FocusUp,
            Action::FocusDown, Action::FocusLeft, Action::FocusRight,
            Action::WindowSwitcher, Action::WindowSwitcherPrev,
            Action::Move, Action::Resize,
            Action::MoveWindowLeft, Action::MoveWindowRight,
            Action::MoveWindowUp, Action::MoveWindowDown,
            Action::Exit, Action::Reload,
            Action::Fullscreen, Action::ModeNext,
            Action::ModeNextShared,
            Action::Overview, Action::OverviewEnter, Action::OverviewExit,
            Action::Minimize, Action::OverlayLeft,
            Action::OverlayRight, Action::ZoomIn, Action::ZoomOut,
            Action::ZoomReset, Action::PanLeft, Action::PanRight,
            Action::PanUp, Action::PanDown, Action::VolumeUp,
            Action::VolumeDown, Action::VolumeMute, Action::MicMute,
            Action::BrightnessUp, Action::BrightnessDown,
        ] {
            assert_eq!(Action::from_name(action.name()), Some(action), "{}", action.name());
        }
        // Legacy aliases from the old config.kdl vocabulary.
        assert_eq!(Action::from_name("close"), Some(Action::Close));
        assert_eq!(Action::from_name("fullscreen"), Some(Action::Fullscreen));
        assert_eq!(Action::from_name("toggle_overview"), Some(Action::Overview));
        assert_eq!(Action::from_name("expose"), Some(Action::Overview));
        assert_eq!(Action::from_name("no_such_action"), None);
    }

    #[test]
    fn table_priority_is_insertion_order() {
        let mut t = BindingTable::new();
        let close = Binding { mods: mods::SUPER, keysym: 0x71, action: Action::Close, command: None };
        let min = Binding { mods: mods::SUPER, keysym: 0x71, action: Action::Minimize, command: None };
        assert!(!t.add(close.clone())); // first claim: not shadowed
        assert!(t.add(min)); // same chord: shadowed
        assert_eq!(t.resolve(mods::SUPER, 0x71), Some(&close));
        assert_eq!(t.resolve(mods::SUPER, 0x72), None);
    }

    #[test]
    fn add_default_never_shadows() {
        let mut t = BindingTable::new();
        let user = Binding { mods: mods::SUPER, keysym: 0x71, action: Action::Close, command: None };
        t.add(user.clone());
        let stock = Binding { mods: mods::SUPER, keysym: 0x71, action: Action::Fullscreen, command: None };
        assert!(!t.add_default(stock));
        assert_eq!(t.len(), 1);
        assert_eq!(t.resolve(mods::SUPER, 0x71), Some(&user));
        let free = Binding { mods: mods::SUPER, keysym: 0x72, action: Action::Fullscreen, command: None };
        assert!(t.add_default(free));
        assert_eq!(t.len(), 2);
    }
}
