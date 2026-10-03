//! The hook's decisions, in pure Rust: which key events to swallow, and when a hold starts and
//! ends. The hold-versus-toggle state machine lives in the core; this only turns raw key events
//! into `Pressed` and `Released`.
//!
//! **Swallowing (decided, as on the Mac).** The hotkey's own events never reach the focused app:
//! a chord's key-down, its auto-repeats and its key-up are swallowed, and so are a held
//! modifier's. On Windows the price of swallowing a modifier is that the OS never sees it down, so
//! with right Ctrl as the dictation key, right Ctrl+C does not copy (left Ctrl still does). The
//! gain is that the key does nothing else while it dictates: right Alt would otherwise open an
//! app's menu on release. A release is swallowed only when its press was, so an app that saw a
//! press (before the hook existed) also sees the release. Everything else passes through.
//!
//! **A chord ends when any part of it is let go of (decided, as on the Mac).** Hold to talk means
//! the hold lasts while the whole chord is down: one of its modifiers coming up ends it as surely
//! as its key does. The modifier's own key-up passes through (the app saw it go down), and the
//! key, still down, keeps its repeats and its key-up swallowed: they were the hotkey's, and an app
//! getting them would type spaces nobody asked for. A function key alone has no modifiers to let
//! go of.
//!
//! **The stray modifier tap.** A swallowed chord key leaves its modifiers looking pressed and
//! released on their own: Windows then opens the Start menu (Win), activates a menu bar (Alt) or
//! switches the keyboard layout (Ctrl+Shift, Alt+Shift). So a chord press with modifiers asks for a
//! **mask key**: the hook thread injects one unassigned key while they are still down, which
//! Windows counts as "another key was pressed", as a typed chord would have been.
#![cfg(windows)]

use super::binding::Binding;

/// One key event from the hook, reduced to what the decision needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HookInput {
    /// `WM_KEYDOWN` or `WM_SYSKEYDOWN` (auto-repeats included: the hook cannot tell them apart).
    KeyDown {
        /// The virtual key.
        vk: u32,
        /// The chord modifiers down at the time ([`super::binding::modifier`] bits).
        modifiers: u8,
    },
    /// `WM_KEYUP` or `WM_SYSKEYUP`.
    KeyUp {
        /// The virtual key.
        vk: u32,
        /// The chord modifiers still down once this key is up ([`super::binding::modifier`] bits).
        modifiers: u8,
    },
}

/// What the hotkey did, before the hook stamps it with a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    Pressed,
    Released,
}

/// The decision for one event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Verdict {
    /// Drop the event instead of passing it on.
    pub(crate) swallow: bool,
    /// Report this to the core.
    pub(crate) edge: Option<Edge>,
    /// Inject the mask key now: a chord with modifiers was swallowed.
    pub(crate) mask: bool,
}

/// Whether the hotkey is held, and the rules above. `Copy`, so the hook keeps it in a `Cell`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoldMachine {
    binding: Binding,
    held: bool,
    /// A chord's key is still down after a modifier ended the hold: its repeats and key-up are
    /// ours to swallow, without another edge.
    trailing: bool,
}

impl HoldMachine {
    pub(crate) const fn new(binding: Binding) -> Self {
        Self {
            binding,
            held: false,
            trailing: false,
        }
    }

    pub(crate) const fn is_held(&self) -> bool {
        self.held
    }

    /// Forgets any hold (the core may have missed an edge: its sink panicked).
    pub(crate) fn reset(&mut self) {
        self.held = false;
        self.trailing = false;
    }

    /// Decides one event.
    pub(crate) fn on(&mut self, input: HookInput) -> Verdict {
        match (self.binding, input) {
            (Binding::Modifier(key), HookInput::KeyDown { vk, .. }) if vk == key.vk() => {
                self.transition(true)
            }
            (Binding::Modifier(key), HookInput::KeyUp { vk, .. }) if vk == key.vk() => {
                self.transition(false)
            }
            (Binding::Chord(chord), HookInput::KeyDown { vk, modifiers }) if vk == chord.vk => {
                if self.held || self.trailing {
                    // Auto-repeat of the held chord, or of its key after a modifier ended the
                    // hold (the hook cannot tell a repeat from a press: the key has not come up).
                    Verdict {
                        swallow: true,
                        edge: None,
                        mask: false,
                    }
                } else if chord.matches(vk, modifiers) {
                    Verdict {
                        mask: chord.modifiers != 0,
                        ..self.transition(true)
                    }
                } else {
                    // The key with other modifiers: the app's.
                    Verdict::default()
                }
            }
            (Binding::Chord(chord), HookInput::KeyUp { vk, .. }) if vk == chord.vk => {
                if self.trailing {
                    self.trailing = false;
                    Verdict {
                        swallow: true,
                        edge: None,
                        mask: false,
                    }
                } else {
                    self.transition(false)
                }
            }
            (Binding::Chord(chord), HookInput::KeyUp { modifiers, .. })
                if self.held && modifiers & chord.modifiers != chord.modifiers =>
            {
                // One of the chord's modifiers came up: the hold ends here. The key-up itself is
                // the app's (it saw the modifier go down); the chord key's up is still to come.
                self.held = false;
                self.trailing = true;
                Verdict {
                    swallow: false,
                    edge: Some(Edge::Released),
                    mask: false,
                }
            }
            _ => Verdict::default(),
        }
    }

    fn transition(&mut self, down: bool) -> Verdict {
        let edge = match (self.held, down) {
            (false, true) => Some(Edge::Pressed),
            (true, false) => Some(Edge::Released),
            // A repeated down: still held, still ours to swallow.
            (true, true) => None,
            // A release whose press we never saw: the app's.
            (false, false) => return Verdict::default(),
        };
        self.held = down;
        Verdict {
            swallow: true,
            edge,
            mask: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::binding::{modifier, vk};
    use super::*;

    fn machine(token: &str) -> HoldMachine {
        HoldMachine::new(Binding::parse(token).expect("valid token"))
    }

    fn down(vk: u32, modifiers: u8) -> HookInput {
        HookInput::KeyDown { vk, modifiers }
    }

    /// A key coming up with nothing left down.
    fn up(vk: u32) -> HookInput {
        HookInput::KeyUp { vk, modifiers: 0 }
    }

    /// A key coming up with `modifiers` still down.
    fn up_with(vk: u32, modifiers: u8) -> HookInput {
        HookInput::KeyUp { vk, modifiers }
    }

    const SWALLOW: Verdict = Verdict {
        swallow: true,
        edge: None,
        mask: false,
    };
    const PASS: Verdict = Verdict {
        swallow: false,
        edge: None,
        mask: false,
    };
    const PRESSED: Verdict = Verdict {
        swallow: true,
        edge: Some(Edge::Pressed),
        mask: false,
    };
    /// A chord with modifiers: pressed, and the mask key asked for.
    const PRESSED_MASKED: Verdict = Verdict {
        mask: true,
        ..PRESSED
    };
    const RELEASED: Verdict = Verdict {
        swallow: true,
        edge: Some(Edge::Released),
        mask: false,
    };

    #[test]
    fn a_modifier_hold_presses_repeats_and_releases() {
        let mut m = machine("right_control");
        assert_eq!(m.on(down(vk::RCONTROL, modifier::CTRL)), PRESSED);
        assert!(m.is_held());
        assert_eq!(
            m.on(down(vk::RCONTROL, modifier::CTRL)),
            SWALLOW,
            "auto-repeat"
        );
        assert_eq!(m.on(down(0x43, modifier::CTRL)), PASS, "other keys pass");
        assert_eq!(m.on(up(vk::RCONTROL)), RELEASED);
        assert!(!m.is_held());
    }

    #[test]
    fn the_left_key_of_the_same_modifier_is_not_the_hotkey() {
        let mut m = machine("right_control");
        assert_eq!(m.on(down(0xA2, modifier::CTRL)), PASS);
        assert_eq!(m.on(up(0xA2)), PASS);
        assert!(!m.is_held());
    }

    #[test]
    fn a_release_without_its_press_passes() {
        let mut m = machine("right_alt");
        assert_eq!(m.on(up(vk::RMENU)), PASS);
        let mut c = machine("ctrl+shift+space");
        assert_eq!(c.on(up(vk::SPACE)), PASS);
    }

    #[test]
    fn a_chord_needs_its_exact_modifiers() {
        let mut m = machine("ctrl+shift+space");
        assert_eq!(m.on(down(vk::SPACE, 0)), PASS, "a plain space types");
        assert_eq!(m.on(up(vk::SPACE)), PASS);
        assert_eq!(m.on(down(vk::SPACE, modifier::CTRL)), PASS);
        assert_eq!(
            m.on(down(
                vk::SPACE,
                modifier::CTRL | modifier::SHIFT | modifier::ALT
            )),
            PASS
        );
        assert_eq!(
            m.on(down(vk::SPACE, modifier::CTRL | modifier::SHIFT)),
            PRESSED_MASKED,
            "Ctrl+Shift released alone would switch the layout"
        );
        // Repeats swallow even after a modifier lets go.
        assert_eq!(m.on(down(vk::SPACE, modifier::CTRL)), SWALLOW);
        assert_eq!(m.on(up(vk::SPACE)), RELEASED);
    }

    /// Letting go of any part of the chord ends the hold: Shift first, here. The app saw Shift go
    /// down, so it sees it come up; the key is still down, and its repeats and its key-up stay
    /// swallowed, or the app would get spaces it never asked for.
    #[test]
    fn letting_go_of_a_modifier_first_ends_the_hold() {
        let mut m = machine("ctrl+shift+space");
        let both = modifier::CTRL | modifier::SHIFT;
        assert_eq!(m.on(down(vk::SPACE, both)), PRESSED_MASKED);
        // Left Shift up: Ctrl is still down, Shift is not.
        let released_passing = Verdict {
            swallow: false,
            edge: Some(Edge::Released),
            mask: false,
        };
        assert_eq!(m.on(up_with(vk::LSHIFT, modifier::CTRL)), released_passing);
        assert!(!m.is_held());
        // The key, still down, repeats with what is left of the modifiers: swallowed, no edge.
        assert_eq!(m.on(down(vk::SPACE, modifier::CTRL)), SWALLOW);
        assert_eq!(
            m.on(down(vk::SPACE, both)),
            SWALLOW,
            "not a new press while it is down"
        );
        assert_eq!(
            m.on(up(vk::SPACE)),
            SWALLOW,
            "its key-up is ours, and reports nothing more"
        );
        // Ctrl up afterwards is the app's.
        assert_eq!(m.on(up(vk::LCONTROL)), PASS);
        // The next press is a new hold.
        assert_eq!(m.on(down(vk::SPACE, both)), PRESSED_MASKED);
        assert_eq!(m.on(up_with(vk::SPACE, both)), RELEASED);
    }

    /// The key let go of first ends the hold as before; the modifiers then are the app's.
    #[test]
    fn letting_go_of_the_key_first_ends_the_hold() {
        let mut m = machine("ctrl+shift+space");
        let both = modifier::CTRL | modifier::SHIFT;
        m.on(down(vk::SPACE, both));
        assert_eq!(m.on(up_with(vk::SPACE, both)), RELEASED);
        assert_eq!(m.on(up_with(vk::LSHIFT, modifier::CTRL)), PASS);
        assert_eq!(m.on(up(vk::LCONTROL)), PASS);
        assert!(!m.is_held());
    }

    /// Another modifier going up or down, or the other Ctrl key coming up while one is still held,
    /// is not a part of the chord let go of.
    #[test]
    fn key_changes_that_keep_the_chord_down_change_nothing() {
        let mut m = machine("ctrl+space");
        m.on(down(vk::SPACE, modifier::CTRL));
        assert_eq!(m.on(down(vk::LSHIFT, modifier::CTRL)), PASS);
        assert_eq!(
            m.on(up_with(vk::LSHIFT, modifier::CTRL)),
            PASS,
            "Shift is not part of the chord"
        );
        assert_eq!(
            m.on(up_with(vk::RCONTROL, modifier::CTRL)),
            PASS,
            "left Ctrl is still down"
        );
        assert!(m.is_held());
        assert_eq!(m.on(up_with(vk::SPACE, modifier::CTRL)), RELEASED);
    }

    /// A function key alone has no modifiers: other keys coming up never end its hold.
    #[test]
    fn a_function_key_hold_ends_only_at_its_key_up() {
        let mut m = machine("f13");
        assert_eq!(m.on(down(0x7C, 0)), PRESSED);
        assert_eq!(m.on(up(vk::LSHIFT)), PASS);
        assert!(m.is_held());
        assert_eq!(m.on(up(0x7C)), RELEASED);
    }

    /// A modifier held on its own is unchanged: only its own key-up ends it.
    #[test]
    fn a_modifier_hold_ignores_other_key_ups() {
        let mut m = machine("right_alt");
        assert_eq!(m.on(down(vk::RMENU, modifier::ALT)), PRESSED);
        assert_eq!(m.on(up(vk::LSHIFT)), PASS);
        assert!(m.is_held());
        assert_eq!(m.on(up(vk::RMENU)), RELEASED);
    }

    #[test]
    fn reset_forgets_a_trailing_key() {
        let mut m = machine("ctrl+space");
        m.on(down(vk::SPACE, modifier::CTRL));
        m.on(up(vk::LCONTROL));
        m.reset();
        assert_eq!(m.on(up(vk::SPACE)), PASS, "the app gets the key-up now");
    }

    #[test]
    fn a_win_or_alt_chord_asks_for_the_mask_key_once() {
        for (token, mods, key) in [
            ("win+7", modifier::WIN, 0x37),
            ("alt+d", modifier::ALT, 0x44),
        ] {
            let mut m = machine(token);
            assert_eq!(m.on(down(key, mods)), PRESSED_MASKED, "{token}");
            assert_eq!(
                m.on(down(key, mods)),
                SWALLOW,
                "{token}: repeats ask for nothing"
            );
            assert_eq!(m.on(up(key)), RELEASED, "{token}");
        }
    }

    #[test]
    fn a_function_key_alone_is_a_chord_without_modifiers() {
        let mut m = machine("f13");
        assert_eq!(m.on(down(0x7C, 0)), PRESSED, "no modifier, no mask");
        assert_eq!(m.on(up(0x7C)), RELEASED);
        assert_eq!(m.on(down(0x7C, modifier::SHIFT)), PASS);
    }

    #[test]
    fn reset_forgets_the_hold() {
        let mut m = machine("right_shift");
        m.on(down(vk::RSHIFT, modifier::SHIFT));
        m.reset();
        assert!(!m.is_held());
        assert_eq!(m.on(up(vk::RSHIFT)), PASS, "the app gets the release now");
    }
}
