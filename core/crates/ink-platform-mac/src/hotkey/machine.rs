//! The tap's decisions, in pure Rust: which events to swallow, when a hold starts and ends, and
//! what to do when the OS disables the tap.
//!
//! The hold-versus-toggle state machine lives in the core; this one only turns raw key events
//! into `Pressed`, `Released` and `Cancelled`.
//!
//! **Swallowing (decided):** the tap is an active filter, not listen-only, so the hotkey's own
//! events never reach the focused app. A chord's key-down, its auto-repeats and its key-up are
//! swallowed (`ctrl+shift+space` would otherwise type a space or fire an app's shortcut). A
//! modifier's `flagsChanged` events are swallowed too, so an app never sees the hotkey modifier
//! go down; key events typed meanwhile still carry the modifier flag, so Fn+arrow and similar
//! keep working. A release is swallowed only when its press was, so an app that saw a press
//! (while the tap was disabled) also sees the release. Everything else passes through untouched.
#![cfg(target_os = "macos")]

use super::binding::Binding;

/// One event from the tap, reduced to what the decision needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TapInput {
    /// `kCGEventFlagsChanged`.
    FlagsChanged {
        /// The key whose change this is.
        keycode: u16,
        /// The flags after the change.
        flags: u64,
    },
    /// `kCGEventKeyDown`.
    KeyDown {
        /// The key.
        keycode: u16,
        /// The modifier flags.
        flags: u64,
        /// Whether this is an auto-repeat of a key already down.
        autorepeat: bool,
    },
    /// `kCGEventKeyUp`.
    KeyUp {
        /// The key.
        keycode: u16,
    },
    /// `kCGEventTapDisabledByTimeout` or `kCGEventTapDisabledByUserInput`: the OS switched the
    /// tap off, and events may have been missed.
    Disabled,
}

/// What the hotkey did, before the tap stamps it with a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    /// The hold started.
    Pressed,
    /// The hold ended.
    Released,
    /// The hold was lost: events may have been missed while the tap was disabled.
    Cancelled,
}

/// The decision for one event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Verdict {
    /// Drop the event instead of passing it on.
    pub(crate) swallow: bool,
    /// Report this to the core.
    pub(crate) edge: Option<Edge>,
    /// Switch the tap back on. A hotkey that silently dies after the OS disables its tap is a
    /// stuck recording in disguise.
    pub(crate) reenable: bool,
}

/// Whether the hotkey is held, and the rules above. `Copy`, so the tap callback can keep it in a
/// `Cell` and never borrow or lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoldMachine {
    binding: Binding,
    held: bool,
}

impl HoldMachine {
    /// Idle, for `binding`.
    pub(crate) const fn new(binding: Binding) -> Self {
        Self {
            binding,
            held: false,
        }
    }

    /// Whether a hold is in progress.
    pub(crate) const fn is_held(&self) -> bool {
        self.held
    }

    /// Decides one event.
    pub(crate) fn on(&mut self, input: TapInput) -> Verdict {
        match (self.binding, input) {
            (_, TapInput::Disabled) => {
                let edge = self.held.then_some(Edge::Cancelled);
                self.held = false;
                Verdict {
                    swallow: false,
                    edge,
                    reenable: true,
                }
            }
            (Binding::Modifier(key), TapInput::FlagsChanged { keycode, flags })
                if keycode == key.keycode() =>
            {
                self.transition(flags & key.down_mask() != 0)
            }
            (
                Binding::Chord(chord),
                TapInput::KeyDown {
                    keycode,
                    flags,
                    autorepeat,
                },
            ) if keycode == chord.keycode => {
                if self.held {
                    // Auto-repeat of the held chord, or a down whose up was missed.
                    Verdict {
                        swallow: true,
                        ..Verdict::default()
                    }
                } else if autorepeat || !chord.matches(keycode, flags) {
                    // The key on its own, with other modifiers, or already down before we looked.
                    Verdict::default()
                } else {
                    self.transition(true)
                }
            }
            (Binding::Chord(chord), TapInput::KeyUp { keycode }) if keycode == chord.keycode => {
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
            // A release whose press we never saw (or already cancelled): the app's, not ours.
            (false, false) => return Verdict::default(),
        };
        self.held = down;
        Verdict {
            swallow: true,
            edge,
            reenable: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::binding::{flag, keycode};
    use super::*;

    fn machine(token: &str) -> HoldMachine {
        HoldMachine::new(Binding::parse(token).expect("valid token"))
    }

    fn fn_flags(down: bool) -> TapInput {
        TapInput::FlagsChanged {
            keycode: keycode::FUNCTION,
            flags: if down { flag::SECONDARY_FN } else { 0 },
        }
    }

    const SWALLOW: Verdict = Verdict {
        swallow: true,
        edge: None,
        reenable: false,
    };
    const PASS: Verdict = Verdict {
        swallow: false,
        edge: None,
        reenable: false,
    };

    fn pressed() -> Verdict {
        Verdict {
            swallow: true,
            edge: Some(Edge::Pressed),
            reenable: false,
        }
    }

    fn released() -> Verdict {
        Verdict {
            swallow: true,
            edge: Some(Edge::Released),
            reenable: false,
        }
    }

    #[test]
    fn fn_hold_presses_then_releases() {
        let mut m = machine("fn");
        assert_eq!(m.on(fn_flags(true)), pressed());
        assert!(m.is_held());
        assert_eq!(m.on(fn_flags(false)), released());
        assert!(!m.is_held());
    }

    #[test]
    fn a_repeated_down_does_not_press_twice() {
        let mut m = machine("fn");
        m.on(fn_flags(true));
        assert_eq!(m.on(fn_flags(true)), SWALLOW);
        assert!(m.is_held());
    }

    #[test]
    fn other_modifiers_pass_through_untouched() {
        let mut m = machine("right_option");
        let left_option_down = TapInput::FlagsChanged {
            keycode: 0x3A,
            flags: flag::OPTION | 0x20,
        };
        assert_eq!(m.on(left_option_down), PASS);
        assert!(!m.is_held());
        // Shift going down while the hotkey is held changes nothing either.
        m.on(TapInput::FlagsChanged {
            keycode: keycode::RIGHT_OPTION,
            flags: flag::OPTION | flag::DEVICE_RIGHT_OPTION,
        });
        let shift = TapInput::FlagsChanged {
            keycode: 0x38,
            flags: flag::OPTION | flag::DEVICE_RIGHT_OPTION | flag::SHIFT | 0x2,
        };
        assert_eq!(m.on(shift), PASS);
        assert!(m.is_held());
    }

    /// With both Command keys down, releasing the right one leaves the public Command flag set;
    /// only the device bit says the right key is up.
    #[test]
    fn right_command_release_reads_the_device_bit() {
        let mut m = machine("right_command");
        let both_down = flag::COMMAND | flag::DEVICE_RIGHT_COMMAND | 0x8;
        assert_eq!(
            m.on(TapInput::FlagsChanged {
                keycode: keycode::RIGHT_COMMAND,
                flags: both_down,
            }),
            pressed()
        );
        let left_still_down = flag::COMMAND | 0x8;
        assert_eq!(
            m.on(TapInput::FlagsChanged {
                keycode: keycode::RIGHT_COMMAND,
                flags: left_still_down,
            }),
            released()
        );
    }

    #[test]
    fn a_release_whose_press_was_missed_passes_through() {
        let mut m = machine("fn");
        assert_eq!(m.on(fn_flags(false)), PASS);
        let mut c = machine("ctrl+shift+space");
        assert_eq!(
            c.on(TapInput::KeyUp {
                keycode: keycode::SPACE
            }),
            PASS
        );
    }

    #[test]
    fn a_modifier_binding_ignores_key_events() {
        let mut m = machine("fn");
        m.on(fn_flags(true));
        let arrow = TapInput::KeyDown {
            keycode: 0x7E,
            flags: flag::SECONDARY_FN,
            autorepeat: false,
        };
        assert_eq!(m.on(arrow), PASS, "Fn+arrow keeps working");
        assert!(m.is_held());
    }

    #[test]
    fn a_chord_presses_swallows_repeats_and_releases() {
        let mut m = machine("ctrl+shift+space");
        let mods = flag::CONTROL | flag::SHIFT;
        let down = |autorepeat| TapInput::KeyDown {
            keycode: keycode::SPACE,
            flags: mods,
            autorepeat,
        };
        assert_eq!(m.on(down(false)), pressed());
        assert_eq!(m.on(down(true)), SWALLOW, "auto-repeat");
        // The user lets go of the modifiers first: still held until the key comes up.
        assert_eq!(
            m.on(TapInput::FlagsChanged {
                keycode: 0x3B,
                flags: 0
            }),
            PASS
        );
        assert!(m.is_held());
        assert_eq!(
            m.on(TapInput::KeyUp {
                keycode: keycode::SPACE
            }),
            released()
        );
    }

    #[test]
    fn a_chord_with_other_modifiers_passes_through() {
        let mut m = machine("ctrl+shift+space");
        let plain_space = TapInput::KeyDown {
            keycode: keycode::SPACE,
            flags: 0,
            autorepeat: false,
        };
        assert_eq!(m.on(plain_space), PASS, "typing a space is not the hotkey");
        let with_cmd = TapInput::KeyDown {
            keycode: keycode::SPACE,
            flags: flag::CONTROL | flag::SHIFT | flag::COMMAND,
            autorepeat: false,
        };
        assert_eq!(m.on(with_cmd), PASS);
        assert!(!m.is_held());
    }

    #[test]
    fn an_autorepeat_without_a_press_does_not_start_a_hold() {
        // The key was already down when the tap started (or while it was disabled).
        let mut m = machine("ctrl+shift+space");
        let repeat = TapInput::KeyDown {
            keycode: keycode::SPACE,
            flags: flag::CONTROL | flag::SHIFT,
            autorepeat: true,
        };
        assert_eq!(m.on(repeat), PASS);
        assert!(!m.is_held());
    }

    #[test]
    fn disabled_while_held_cancels_and_reenables() {
        let mut m = machine("fn");
        m.on(fn_flags(true));
        assert_eq!(
            m.on(TapInput::Disabled),
            Verdict {
                swallow: false,
                edge: Some(Edge::Cancelled),
                reenable: true,
            }
        );
        assert!(!m.is_held());
    }

    #[test]
    fn disabled_while_idle_only_reenables() {
        let mut m = machine("ctrl+shift+space");
        assert_eq!(
            m.on(TapInput::Disabled),
            Verdict {
                swallow: false,
                edge: None,
                reenable: true,
            }
        );
    }

    /// After a cancel, the release of the lost hold must not reach the core as a `Released`
    /// after its `Cancelled`. It passes through to the app instead, where a lone modifier release
    /// is harmless.
    #[test]
    fn the_release_after_a_cancel_is_not_reported() {
        let mut m = machine("fn");
        m.on(fn_flags(true));
        m.on(TapInput::Disabled);
        assert_eq!(m.on(fn_flags(false)), PASS);
        // And the next hold works normally.
        assert_eq!(m.on(fn_flags(true)), pressed());
    }
}
