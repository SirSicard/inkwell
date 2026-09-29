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
}

/// Whether the hotkey is held, and the rules above. `Copy`, so the hook keeps it in a `Cell`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoldMachine {
    binding: Binding,
    held: bool,
}

impl HoldMachine {
    pub(crate) const fn new(binding: Binding) -> Self {
        Self {
            binding,
            held: false,
        }
    }

    pub(crate) const fn is_held(&self) -> bool {
        self.held
    }

    /// Forgets any hold (the core may have missed an edge: its sink panicked).
    pub(crate) fn reset(&mut self) {
        self.held = false;
    }

    /// Decides one event.
    pub(crate) fn on(&mut self, input: HookInput) -> Verdict {
        match (self.binding, input) {
            (Binding::Modifier(key), HookInput::KeyDown { vk, .. }) if vk == key.vk() => {
                self.transition(true)
            }
            (Binding::Modifier(key), HookInput::KeyUp { vk }) if vk == key.vk() => {
                self.transition(false)
            }
            (Binding::Chord(chord), HookInput::KeyDown { vk, modifiers }) if vk == chord.vk => {
                if self.held {
                    // Auto-repeat of the held chord.
                    Verdict {
                        swallow: true,
                        edge: None,
                    }
                } else if chord.matches(vk, modifiers) {
                    self.transition(true)
                } else {
                    // The key with other modifiers: the app's.
                    Verdict::default()
                }
            }
            (Binding::Chord(chord), HookInput::KeyUp { vk }) if vk == chord.vk => {
                self.transition(false)
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

    fn up(vk: u32) -> HookInput {
        HookInput::KeyUp { vk }
    }

    const SWALLOW: Verdict = Verdict {
        swallow: true,
        edge: None,
    };
    const PASS: Verdict = Verdict {
        swallow: false,
        edge: None,
    };
    const PRESSED: Verdict = Verdict {
        swallow: true,
        edge: Some(Edge::Pressed),
    };
    const RELEASED: Verdict = Verdict {
        swallow: true,
        edge: Some(Edge::Released),
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
            PRESSED
        );
        // Repeats swallow even after a modifier lets go.
        assert_eq!(m.on(down(vk::SPACE, modifier::CTRL)), SWALLOW);
        assert_eq!(m.on(up(vk::SPACE)), RELEASED);
    }

    #[test]
    fn a_function_key_alone_is_a_chord_without_modifiers() {
        let mut m = machine("f13");
        assert_eq!(m.on(down(0x7C, 0)), PRESSED);
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
