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
//! getting them would type spaces nobody asked for (until it comes up, the key is **trailing**). A
//! function key alone has no modifiers to let go of.
//!
//! **A press is a key-down of a key that was not already down (decided).** The hook's event has
//! no repeat flag, and the key state cannot stand in for one while the hotkey is ours: a key-down
//! the hook swallows never reaches it (measured: a held, swallowed key reads as up at each of its
//! repeats). So while a hold or its trail is on, a key-down of the key is a repeat if it comes
//! within [`repeat_gap_ms`] of the key's last one (the longest interval the keyboard settings
//! allow between repeats, with a margin), or if the key state reads it as down already (Windows
//! removed the hook mid-hold, so the repeats in between reached the OS, and the gap means
//! nothing). Anything later is a fresh press: the key came up where the hook could not see it (the
//! lock screen, the secure desktop after Ctrl+Alt+Del, an app running as administrator). That ends
//! the hold there, `Released` (then `Pressed`, if the press is the hotkey again), or the trail
//! silently, and the press is judged on its own: nothing the user types next is eaten, and their
//! next dictation starts on its press. A press back within the gap still reads as a repeat and is
//! swallowed, as before. Outside a hold the key's events reached the OS, and a key-down is judged
//! as a press, as it always was. A key-up the OS still needs is never swallowed: if the key state
//! reads the key as down at its key-up (repeats reached the OS while the hook was out, or the key
//! was down before the hook started), the key-up passes, the hold still ending there. Swallowing
//! it would leave the key state reading down for good, and every later lost key-up would read as
//! a repeat.
//!
//! **After a panic in the core's sink** the hook abandons the hold ([`HoldMachine::reset`]) and
//! sends `Cancelled`, but the key still down becomes trailing, a held modifier's too: its repeats
//! reach no app and start no hold, and its key-up is swallowed like its press was.
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
    /// `WM_KEYDOWN` or `WM_SYSKEYDOWN`, auto-repeats included.
    KeyDown {
        /// The virtual key.
        vk: u32,
        /// The chord modifiers down at the time ([`super::binding::modifier`] bits).
        modifiers: u8,
        /// The key state reads the key as down already: a repeat of a key the OS saw (one the
        /// hook passed on). A swallowed key reads as up at its repeats too. Read for the hotkey's
        /// key only ([`HoldMachine::key_vk`]); false for any other.
        reads_down: bool,
        /// The event's time on the tick counter, in milliseconds.
        at_ms: u32,
    },
    /// `WM_KEYUP` or `WM_SYSKEYUP`.
    KeyUp {
        /// The virtual key.
        vk: u32,
        /// The chord modifiers still down once this key is up ([`super::binding::modifier`] bits).
        modifiers: u8,
        /// The key state still reads the key as down: the OS saw it go down, and needs its key-up.
        /// Read for the hotkey's key only; false for any other.
        reads_down: bool,
    },
}

/// What the hotkey did, before the hook stamps it with a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    Pressed,
    Released,
    /// `Released`, then `Pressed`: the key came up where the hook could not see it, and this is
    /// its next press.
    ReleasedThenPressed,
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

/// The longest pause between two key-downs of a held key that is still a repeat, from the
/// keyboard settings: the repeat delay (`SPI_GETKEYBOARDDELAY`, 0 to 3, about 250 ms to a second)
/// or the slowest repeat rate (about 2.5 a second), whichever is longer, and FilterKeys' own delay
/// and rate when it is on and they are longer (up to 20 s each; both zero under BounceKeys, so
/// they never shorten it). Microsoft says the hardware may stray from these, so the gap is half
/// as long again.
pub(crate) fn repeat_gap_ms(delay_setting: u32, filter_keys: Option<(u32, u32)>) -> u32 {
    const SLOWEST_REPEAT_MS: u32 = 400;
    let delay_ms = (delay_setting.min(3) + 1) * 250;
    let (filter_delay_ms, filter_repeat_ms) = filter_keys.unwrap_or_default();
    let longest = delay_ms
        .max(SLOWEST_REPEAT_MS)
        .max(filter_delay_ms)
        .max(filter_repeat_ms);
    longest.saturating_mul(3) / 2
}

/// The gap when the settings cannot be read: the longest without FilterKeys.
pub(crate) const FALLBACK_REPEAT_GAP_MS: u32 = 1_500;

/// Whether the hotkey is held, and the rules above. `Copy`, so the hook keeps it in a `Cell`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoldMachine {
    binding: Binding,
    held: bool,
    /// The hotkey's key is still down after its hold ended without it (a modifier of the chord
    /// came up, or the sink panicked): its repeats and key-up are ours to swallow, without
    /// another edge, until it comes up or is pressed afresh.
    trailing: bool,
    /// When the hotkey's key last went down (or repeated), on the tick counter.
    last_down_ms: u32,
    /// See [`repeat_gap_ms`].
    repeat_gap_ms: u32,
    /// The OS saw this key go down before the hook was installed. Its repeats/up belong to
    /// the app, and no action may start until that initial hold has ended.
    initial_release: bool,
}

impl HoldMachine {
    pub(crate) const fn new(binding: Binding, repeat_gap_ms: u32) -> Self {
        Self {
            binding,
            held: false,
            trailing: false,
            last_down_ms: 0,
            repeat_gap_ms,
            initial_release: false,
        }
    }

    pub(crate) const fn is_held(&self) -> bool {
        self.held
    }

    /// A key pressed before this hook existed must be released before it can start a hold.
    pub(crate) fn wait_for_initial_release(&mut self, down: bool) {
        self.initial_release = down;
    }

    /// The hotkey's own key: the only one whose key state the hook reads for each event.
    pub(crate) const fn key_vk(&self) -> u32 {
        match self.binding {
            Binding::Modifier(key) => key.vk(),
            Binding::Chord(chord) => chord.vk,
        }
    }

    /// The keyboard settings changed: see [`repeat_gap_ms`].
    pub(crate) fn set_repeat_gap(&mut self, repeat_gap_ms: u32) {
        self.repeat_gap_ms = repeat_gap_ms;
    }

    /// Abandons the hold (the core may have missed an edge: its sink panicked). A key still down
    /// stays ours until it comes up: its repeats start nothing and reach no app.
    pub(crate) fn reset(&mut self) {
        self.trailing |= self.held;
        self.held = false;
    }

    /// Decides one event.
    pub(crate) fn on(&mut self, input: HookInput) -> Verdict {
        if self.initial_release {
            match input {
                HookInput::KeyDown { vk, .. } if vk == self.key_vk() => return Verdict::default(),
                HookInput::KeyUp { vk, .. } if vk == self.key_vk() => {
                    self.initial_release = false;
                    return Verdict::default();
                }
                _ => {}
            }
        }
        match (self.binding, input) {
            (
                Binding::Modifier(key),
                HookInput::KeyDown {
                    vk,
                    reads_down,
                    at_ms,
                    ..
                },
            ) if vk == key.vk() => self.key_down(reads_down, at_ms, true, false),
            (
                Binding::Chord(chord),
                HookInput::KeyDown {
                    vk,
                    modifiers,
                    reads_down,
                    at_ms,
                },
            ) if vk == chord.vk => {
                // The key with other modifiers is the app's.
                let starts = chord.matches(vk, modifiers);
                self.key_down(reads_down, at_ms, starts, chord.modifiers != 0)
            }
            (Binding::Modifier(key), HookInput::KeyUp { vk, reads_down, .. }) if vk == key.vk() => {
                self.key_up(reads_down)
            }
            (Binding::Chord(chord), HookInput::KeyUp { vk, reads_down, .. }) if vk == chord.vk => {
                self.key_up(reads_down)
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

    /// A key-down of the hotkey's key. `starts`: it is the hotkey (a chord's exact modifiers are
    /// down); `mask`: a hold it starts asks for the mask key.
    fn key_down(&mut self, reads_down: bool, at_ms: u32, starts: bool, mask: bool) -> Verdict {
        let since_ms = at_ms.wrapping_sub(self.last_down_ms);
        // A time a little behind the last key-down (injected input need not be in order) is a
        // repeat, not a wrapped gap of 49 days.
        let behind_ms = self.last_down_ms.wrapping_sub(at_ms);
        self.last_down_ms = at_ms;
        let soon = since_ms <= self.repeat_gap_ms || behind_ms <= self.repeat_gap_ms;
        if (self.held || self.trailing) && (reads_down || soon) {
            // A repeat of the held key, or of the trailing one: still ours, no edge.
            return Verdict {
                swallow: true,
                edge: None,
                mask: false,
            };
        }
        // A press. A hold or trail still on means the key came up unseen: that ends here, and
        // this press is judged on its own.
        let lost = self.held;
        self.held = starts;
        self.trailing = false;
        Verdict {
            swallow: starts,
            edge: match (lost, starts) {
                (false, true) => Some(Edge::Pressed),
                (true, true) => Some(Edge::ReleasedThenPressed),
                (true, false) => Some(Edge::Released),
                (false, false) => None,
            },
            mask: starts && mask,
        }
    }

    /// A key-up of the hotkey's key: the end of its hold, or of its trail, or the app's (its press
    /// was). A key the OS still reads as down gets its key-up, hold or not: it saw the key go down
    /// (repeats that reached it while Windows had removed the hook, or a key already down when the
    /// hook started), and swallowing the up would leave the key state reading down for good.
    fn key_up(&mut self, reads_down: bool) -> Verdict {
        let edge = self.held.then_some(Edge::Released);
        let swallow = (self.held || self.trailing) && !reads_down;
        self.held = false;
        self.trailing = false;
        Verdict {
            swallow,
            edge,
            mask: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::super::binding::{modifier, vk};
    use super::*;

    /// The gap at Windows' default repeat delay (setting 1, about 500 ms).
    const GAP: u32 = 750;

    thread_local! {
        /// The test's tick counter: each test runs on its own thread.
        static NOW_MS: Cell<u32> = const { Cell::new(0) };
    }

    /// The tick counter, `ms` on.
    fn after(ms: u32) -> u32 {
        NOW_MS.with(|now| {
            now.set(now.get().wrapping_add(ms));
            now.get()
        })
    }

    fn machine(token: &str) -> HoldMachine {
        HoldMachine::new(Binding::parse(token).expect("valid token"), GAP)
    }

    /// A press, well after anything before it.
    fn down(vk: u32, modifiers: u8) -> HookInput {
        down_after(10_000, vk, modifiers)
    }

    /// A key-down `ms` after the last event, of a key that reads as up: a press, or a repeat of a
    /// key the hook swallows.
    fn down_after(ms: u32, vk: u32, modifiers: u8) -> HookInput {
        HookInput::KeyDown {
            vk,
            modifiers,
            reads_down: false,
            at_ms: after(ms),
        }
    }

    /// An auto-repeat as the hook sees one of a key it swallows: soon after the last, reading as up.
    fn repeat(vk: u32, modifiers: u8) -> HookInput {
        down_after(33, vk, modifiers)
    }

    /// A key coming up with nothing left down.
    fn up(vk: u32) -> HookInput {
        up_with(vk, 0)
    }

    /// A key coming up with `modifiers` still down.
    fn up_with(vk: u32, modifiers: u8) -> HookInput {
        HookInput::KeyUp {
            vk,
            modifiers,
            reads_down: false,
        }
    }

    /// A key coming up that the OS saw go down (the key state still reads it as down).
    fn up_seen(vk: u32) -> HookInput {
        HookInput::KeyUp {
            vk,
            modifiers: 0,
            reads_down: true,
        }
    }

    /// A key-down of a key the OS saw go down, `ms` after the last event.
    fn down_seen_after(ms: u32, vk: u32, modifiers: u8) -> HookInput {
        HookInput::KeyDown {
            vk,
            modifiers,
            reads_down: true,
            at_ms: after(ms),
        }
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
            m.on(repeat(vk::RCONTROL, modifier::CTRL)),
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
    fn a_new_hook_never_starts_from_a_key_still_held_after_shortcut_capture() {
        let mut m = machine("ctrl+shift+space");
        m.wait_for_initial_release(true);
        assert_eq!(
            m.on(repeat(vk::SPACE, modifier::CTRL | modifier::SHIFT)),
            PASS
        );
        assert!(!m.is_held());
        assert_eq!(
            m.on(up(vk::SPACE)),
            PASS,
            "the app saw the original down and needs its up"
        );
        assert_eq!(
            m.on(down(vk::SPACE, modifier::CTRL | modifier::SHIFT)),
            Verdict {
                mask: true,
                ..PRESSED
            }
        );
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
        assert_eq!(m.on(repeat(vk::SPACE, modifier::CTRL)), SWALLOW);
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
        assert_eq!(m.on(repeat(vk::SPACE, modifier::CTRL)), SWALLOW);
        assert_eq!(
            m.on(repeat(vk::SPACE, both)),
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

    /// A modifier let go of first, then the sink panicked: the key still down stays ours.
    #[test]
    fn reset_keeps_a_trailing_key_swallowed() {
        let mut m = machine("ctrl+space");
        m.on(down(vk::SPACE, modifier::CTRL));
        m.on(up(vk::LCONTROL));
        m.reset();
        assert_eq!(m.on(repeat(vk::SPACE, 0)), SWALLOW, "no space typed");
        assert_eq!(m.on(up(vk::SPACE)), SWALLOW, "its press was ours");
        assert_eq!(m.on(down(vk::SPACE, 0)), PASS, "a later space types");
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
                m.on(repeat(key, mods)),
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

    /// The sink panicked mid-hold (risk b): the hold is forgotten, but the chord key, still down,
    /// keeps its repeats and key-up swallowed and starts no hold until it is pressed again.
    #[test]
    fn after_a_reset_a_held_chord_stays_swallowed_until_its_key_comes_up() {
        let mut m = machine("ctrl+space");
        assert_eq!(m.on(down(vk::SPACE, modifier::CTRL)), PRESSED_MASKED);
        m.reset();
        assert!(!m.is_held());
        assert_eq!(
            m.on(repeat(vk::SPACE, modifier::CTRL)),
            SWALLOW,
            "a repeat is no new hold, and no space for the app"
        );
        assert_eq!(m.on(up(vk::SPACE)), SWALLOW, "no edge: the core was told");
        assert_eq!(m.on(down(vk::SPACE, modifier::CTRL)), PRESSED_MASKED);
        assert_eq!(m.on(up(vk::SPACE)), RELEASED);
    }

    /// The same for a modifier held on its own: its repeats used to start a new hold at once.
    #[test]
    fn after_a_reset_a_held_modifier_stays_swallowed_until_it_comes_up() {
        let mut m = machine("right_shift");
        assert_eq!(m.on(down(vk::RSHIFT, modifier::SHIFT)), PRESSED);
        m.reset();
        assert!(!m.is_held());
        assert_eq!(m.on(repeat(vk::RSHIFT, modifier::SHIFT)), SWALLOW);
        assert!(!m.is_held(), "no new hold");
        assert_eq!(m.on(up(vk::RSHIFT)), SWALLOW, "its press was ours");
        assert_eq!(m.on(down(vk::RSHIFT, modifier::SHIFT)), PRESSED);
    }

    #[test]
    fn reset_with_nothing_held_changes_nothing() {
        let mut m = machine("right_shift");
        m.reset();
        assert_eq!(m.on(up(vk::RSHIFT)), PASS);
        assert_eq!(m.on(down(vk::RSHIFT, modifier::SHIFT)), PRESSED);
    }

    /// Risk a: Shift let go of first, then the key-up of Space never reached the hook (the screen
    /// was locked). The next space the user types is theirs.
    #[test]
    fn a_lost_key_up_after_a_modifier_first_release_eats_no_later_press() {
        let mut m = machine("ctrl+shift+space");
        let both = modifier::CTRL | modifier::SHIFT;
        m.on(down(vk::SPACE, both));
        m.on(up_with(vk::LSHIFT, modifier::CTRL));
        assert_eq!(m.on(repeat(vk::SPACE, modifier::CTRL)), SWALLOW);
        // Space comes up on the lock screen, unseen; Ctrl comes up afterwards.
        assert_eq!(m.on(up(vk::LCONTROL)), PASS);
        assert_eq!(m.on(down(vk::SPACE, 0)), PASS, "the user's space types");
        assert_eq!(m.on(repeat(vk::SPACE, 0)), PASS, "and so do its repeats");
        assert_eq!(m.on(up(vk::SPACE)), PASS);
    }

    /// The same, but the next press is the hotkey: a new hold, at once.
    #[test]
    fn a_lost_key_up_after_a_modifier_first_release_lets_the_next_hold_start() {
        let mut m = machine("ctrl+shift+space");
        let both = modifier::CTRL | modifier::SHIFT;
        m.on(down(vk::SPACE, both));
        m.on(up_with(vk::LSHIFT, modifier::CTRL));
        assert_eq!(m.on(down(vk::SPACE, both)), PRESSED_MASKED);
        assert_eq!(m.on(repeat(vk::SPACE, both)), SWALLOW);
        assert_eq!(m.on(up(vk::SPACE)), RELEASED);
    }

    /// A held chord whose key-up was lost: the next press of its key ends the old hold and, being
    /// the hotkey again, starts the new one, so the user's next dictation works.
    #[test]
    fn a_lost_key_up_of_a_held_chord_ends_the_hold_at_the_next_press() {
        let mut m = machine("ctrl+space");
        m.on(down(vk::SPACE, modifier::CTRL));
        // Space and Ctrl come up unseen.
        assert_eq!(
            m.on(down(vk::SPACE, modifier::CTRL)),
            Verdict {
                edge: Some(Edge::ReleasedThenPressed),
                ..PRESSED_MASKED
            }
        );
        assert!(m.is_held());
        assert_eq!(m.on(repeat(vk::SPACE, modifier::CTRL)), SWALLOW);
        assert_eq!(m.on(up(vk::SPACE)), RELEASED);
    }

    /// The same, but the next press is a plain space: the old hold ends, and the space is the
    /// app's.
    #[test]
    fn a_lost_key_up_of_a_held_chord_does_not_eat_a_plain_press() {
        let mut m = machine("ctrl+space");
        m.on(down(vk::SPACE, modifier::CTRL));
        assert_eq!(
            m.on(down(vk::SPACE, 0)),
            Verdict {
                swallow: false,
                edge: Some(Edge::Released),
                mask: false,
            }
        );
        assert!(!m.is_held());
        assert_eq!(m.on(up(vk::SPACE)), PASS);
    }

    /// A modifier held on its own whose key-up was lost: its next press is a new hold.
    #[test]
    fn a_lost_key_up_of_a_held_modifier_ends_the_hold_at_the_next_press() {
        let mut m = machine("right_control");
        m.on(down(vk::RCONTROL, modifier::CTRL));
        assert_eq!(
            m.on(down(vk::RCONTROL, modifier::CTRL)),
            Verdict {
                edge: Some(Edge::ReleasedThenPressed),
                ..PRESSED
            }
        );
        assert_eq!(m.on(up(vk::RCONTROL)), RELEASED);
    }

    /// A held key that is swallowed reads as up at every repeat; its repeats are still repeats,
    /// the first one after the longest repeat delay included.
    #[test]
    fn a_swallowed_keys_repeats_are_told_apart_by_their_timing() {
        let mut m = HoldMachine::new(
            Binding::parse("right_control").unwrap(),
            repeat_gap_ms(3, None),
        );
        assert_eq!(m.on(down(vk::RCONTROL, modifier::CTRL)), PRESSED);
        assert_eq!(
            m.on(down_after(1_000, vk::RCONTROL, modifier::CTRL)),
            SWALLOW,
            "the first repeat, a second on"
        );
        for _ in 0..60 {
            assert_eq!(m.on(repeat(vk::RCONTROL, modifier::CTRL)), SWALLOW);
        }
        assert!(m.is_held());
        assert_eq!(m.on(up(vk::RCONTROL)), RELEASED);
    }

    /// The gap's edge: a key-down within it is a repeat, one just past it a new press (the key-up
    /// was lost in between).
    #[test]
    fn a_key_down_past_the_gap_is_a_press() {
        let mut m = machine("ctrl+space");
        m.on(down(vk::SPACE, modifier::CTRL));
        assert_eq!(m.on(down_after(GAP, vk::SPACE, modifier::CTRL)), SWALLOW);
        assert_eq!(
            m.on(down_after(GAP + 1, vk::SPACE, modifier::CTRL)),
            Verdict {
                edge: Some(Edge::ReleasedThenPressed),
                ..PRESSED_MASKED
            }
        );
    }

    /// Windows removed the hook mid-hold and the heartbeat put it back: the repeats in between
    /// reached the OS, so the key reads as down, and a key-down however late is a repeat.
    #[test]
    fn a_key_down_that_reads_down_is_a_repeat_however_late() {
        let mut m = machine("right_alt");
        assert_eq!(m.on(down(vk::RMENU, modifier::ALT)), PRESSED);
        let late = HookInput::KeyDown {
            vk: vk::RMENU,
            modifiers: modifier::ALT,
            reads_down: true,
            at_ms: after(60_000),
        };
        assert_eq!(m.on(late), SWALLOW);
        assert!(m.is_held());
        assert_eq!(m.on(up(vk::RMENU)), RELEASED);
    }

    /// Outside a hold a key-down is judged as a press, as before: the key state is the OS's word
    /// on a key the hook passed on, and only then.
    #[test]
    fn outside_a_hold_a_key_down_is_judged_as_a_press() {
        let mut m = machine("right_control");
        let reads_down = HookInput::KeyDown {
            vk: vk::RCONTROL,
            modifiers: modifier::CTRL,
            reads_down: true,
            at_ms: after(10),
        };
        assert_eq!(m.on(reads_down), PRESSED);
    }

    /// The trail of a chord after a panic uses the same timing.
    #[test]
    fn a_trailing_keys_late_press_ends_the_trail() {
        let mut m = machine("ctrl+space");
        m.on(down(vk::SPACE, modifier::CTRL));
        m.reset();
        assert_eq!(m.on(repeat(vk::SPACE, modifier::CTRL)), SWALLOW);
        assert_eq!(
            m.on(down_after(GAP + 1, vk::SPACE, modifier::CTRL)),
            PRESSED_MASKED,
            "the key-up was lost: a new hold, and no Released (the core had its Cancelled)"
        );
    }

    #[test]
    fn the_gap_follows_the_keyboard_settings() {
        assert_eq!(repeat_gap_ms(1, None), 750, "Windows' default delay");
        assert_eq!(
            repeat_gap_ms(0, None),
            600,
            "the slowest repeat rate is longer"
        );
        assert_eq!(repeat_gap_ms(3, None), FALLBACK_REPEAT_GAP_MS);
        assert_eq!(
            repeat_gap_ms(9, None),
            FALLBACK_REPEAT_GAP_MS,
            "out of range"
        );
        assert_eq!(repeat_gap_ms(1, Some((2_000, 500))), 3_000, "FilterKeys");
        assert_eq!(repeat_gap_ms(1, Some((300, 20_000))), 30_000);
        assert_eq!(
            repeat_gap_ms(1, Some((0, 0))),
            750,
            "BounceKeys zeroes them: they never shorten the gap"
        );
    }

    /// The tick counter wraps every 49.7 days; a repeat across the wrap is still a repeat.
    #[test]
    fn a_repeat_across_the_tick_wrap_is_a_repeat() {
        NOW_MS.with(|now| now.set(u32::MAX - 20));
        let mut m = machine("right_control");
        assert_eq!(m.on(down_after(0, vk::RCONTROL, modifier::CTRL)), PRESSED);
        assert_eq!(m.on(repeat(vk::RCONTROL, modifier::CTRL)), SWALLOW);
        assert_eq!(m.on(repeat(vk::RCONTROL, modifier::CTRL)), SWALLOW);
        assert_eq!(m.on(up(vk::RCONTROL)), RELEASED);
    }

    /// Windows removed the hook mid-hold and the heartbeat put it back: the repeats in between
    /// reached the OS, so it holds the key down until it sees a key-up. That key-up passes (the
    /// hold still ends), or the key state would read down for good and hide every later lost
    /// key-up. The next hold's lost key-up is still caught.
    #[test]
    fn a_hold_the_os_saw_go_down_passes_its_key_up() {
        let mut m = machine("right_control");
        assert_eq!(m.on(down(vk::RCONTROL, modifier::CTRL)), PRESSED);
        assert_eq!(
            m.on(down_seen_after(5_000, vk::RCONTROL, modifier::CTRL)),
            SWALLOW,
            "a repeat after the reinstall"
        );
        assert_eq!(
            m.on(up_seen(vk::RCONTROL)),
            Verdict {
                swallow: false,
                edge: Some(Edge::Released),
                mask: false,
            },
            "the OS gets its key-up"
        );
        // The next hold, its key-up lost on the lock screen: the key state reads up again.
        assert_eq!(m.on(down(vk::RCONTROL, modifier::CTRL)), PRESSED);
        assert_eq!(
            m.on(down(vk::RCONTROL, modifier::CTRL)),
            Verdict {
                edge: Some(Edge::ReleasedThenPressed),
                ..PRESSED
            },
            "the lost key-up is still detected"
        );
    }

    /// The same for a trailing chord key the OS saw go down, and for a hold that started on a
    /// key the OS already had down (the hook started while it was held).
    #[test]
    fn a_trailing_or_late_started_key_the_os_saw_passes_its_key_up() {
        let mut m = machine("ctrl+space");
        m.on(down(vk::SPACE, modifier::CTRL));
        m.reset();
        assert_eq!(
            m.on(down_seen_after(5_000, vk::SPACE, modifier::CTRL)),
            SWALLOW
        );
        assert_eq!(m.on(up_seen(vk::SPACE)), PASS);
        let mut r = machine("right_alt");
        assert_eq!(m.on(up(vk::SPACE)), PASS, "nothing left over");
        assert_eq!(r.on(down_seen_after(10, vk::RMENU, modifier::ALT)), PRESSED);
        assert_eq!(
            r.on(up_seen(vk::RMENU)),
            Verdict {
                swallow: false,
                edge: Some(Edge::Released),
                mask: false,
            }
        );
    }

    /// Injected input can carry a time a little behind the last key-down: a repeat, not a wrapped
    /// gap of 49 days. A time further behind than the gap is the counter having wrapped since:
    /// a press.
    #[test]
    fn a_key_down_slightly_behind_the_last_is_a_repeat() {
        let mut m = machine("ctrl+space");
        m.on(down(vk::SPACE, modifier::CTRL));
        let behind = |ms: u32| HookInput::KeyDown {
            vk: vk::SPACE,
            modifiers: modifier::CTRL,
            reads_down: false,
            at_ms: NOW_MS.with(Cell::get).wrapping_sub(ms),
        };
        assert_eq!(m.on(behind(5)), SWALLOW);
        assert_eq!(m.on(behind(GAP)), SWALLOW, "the gap's edge, backwards");
        assert_eq!(
            m.on(behind(2 * GAP + 1)),
            Verdict {
                edge: Some(Edge::ReleasedThenPressed),
                ..PRESSED_MASKED
            }
        );
    }
}
