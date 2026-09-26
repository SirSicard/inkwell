//! Hotkey tokens, parsed in pure Rust.
//!
//! Two shapes:
//! - **A modifier held on its own:** `"fn"`, `"right_option"`, `"right_command"`,
//!   `"right_control"`, `"right_shift"`. These arrive as `flagsChanged` events only, and the
//!   right-hand keys are told apart from the left by the device-dependent flag bits, because the
//!   public Command (or Option...) flag is set for either side. Left-hand modifiers alone are not
//!   offered: watching left Command would start a hold on every Cmd+C in every app.
//! - **A chord:** modifiers and one key, `"ctrl+shift+space"`, `"cmd+option+d"`, or a function
//!   key alone, `"f13"`. A typing key needs at least one modifier, or the tap would swallow it.
//!
//! Letter and digit tokens name the key at that position on an ANSI keyboard (`kVK_ANSI_*`). The
//! keycode is fixed here, never looked up through the keyboard layout: that lookup goes through
//! Text Services, which asserts it is on the main thread and kills the process from any other.
//!
//! Inkwell 0.2 spelled three of the modifier tokens `right_cmd`, `right_opt` and `right_ctrl`;
//! they are accepted as aliases so an imported 0.2 setting still binds.
#![cfg(target_os = "macos")]

use ink_core::PlatformError;

/// Carbon virtual keycodes (`HIToolbox/Events.h`), as `CGEvent` carries them.
pub(crate) mod keycode {
    /// `kVK_Function`: the Fn (globe) key, reported in `flagsChanged` events.
    pub const FUNCTION: u16 = 0x3F;
    /// `kVK_RightCommand`.
    pub const RIGHT_COMMAND: u16 = 0x36;
    /// `kVK_RightOption`.
    pub const RIGHT_OPTION: u16 = 0x3D;
    /// `kVK_RightControl`.
    pub const RIGHT_CONTROL: u16 = 0x3E;
    /// `kVK_RightShift`.
    pub const RIGHT_SHIFT: u16 = 0x3C;
    /// `kVK_Space`.
    pub const SPACE: u16 = 0x31;
    /// `kVK_Return`.
    pub const RETURN: u16 = 0x24;
    /// `kVK_Tab`.
    pub const TAB: u16 = 0x30;
    /// `kVK_Escape`.
    pub const ESCAPE: u16 = 0x35;
    /// `kVK_ANSI_V`: the paste key, by position.
    pub const ANSI_V: u16 = 0x09;

    /// `kVK_ANSI_A` to `kVK_ANSI_Z`, in alphabetical order. The ANSI codes follow the physical
    /// layout, not the alphabet, hence the table.
    pub const ANSI_LETTERS: [u16; 26] = [
        0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D, 0x1F,
        0x23, 0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06,
    ];

    /// `kVK_ANSI_0` to `kVK_ANSI_9`.
    pub const ANSI_DIGITS: [u16; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];

    /// `kVK_F1` to `kVK_F20`.
    pub const FUNCTION_KEYS: [u16; 20] = [
        0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71,
        0x6A, 0x40, 0x4F, 0x50, 0x5A,
    ];
}

/// `CGEventFlags` bits (`CGEventTypes.h`) and the device-dependent bits under them
/// (`IOKit/hidsystem/IOLLEvent.h`).
pub(crate) mod flag {
    /// `kCGEventFlagMaskShift`.
    pub const SHIFT: u64 = 0x0002_0000;
    /// `kCGEventFlagMaskControl`.
    pub const CONTROL: u64 = 0x0004_0000;
    /// `kCGEventFlagMaskAlternate` (Option).
    pub const OPTION: u64 = 0x0008_0000;
    /// `kCGEventFlagMaskCommand`.
    pub const COMMAND: u64 = 0x0010_0000;
    /// `kCGEventFlagMaskSecondaryFn`: Fn is down.
    pub const SECONDARY_FN: u64 = 0x0080_0000;
    /// `NX_DEVICERSHIFTKEYMASK`.
    pub const DEVICE_RIGHT_SHIFT: u64 = 0x0000_0004;
    /// `NX_DEVICERCMDKEYMASK`.
    pub const DEVICE_RIGHT_COMMAND: u64 = 0x0000_0010;
    /// `NX_DEVICERALTKEYMASK`.
    pub const DEVICE_RIGHT_OPTION: u64 = 0x0000_0040;
    /// `NX_DEVICERCTLKEYMASK`.
    pub const DEVICE_RIGHT_CONTROL: u64 = 0x0000_2000;

    /// The modifiers a chord compares. Caps Lock, the numeric-pad bit and Fn are left out unless
    /// the chord names Fn: laptops set the Fn bit on arrows and function keys by themselves.
    pub const CHORD_MODIFIERS: u64 = SHIFT | CONTROL | OPTION | COMMAND;
}

/// `CGEventType` numbers, for the tap's event mask.
pub(crate) mod event_type {
    /// `kCGEventKeyDown`.
    pub const KEY_DOWN: u32 = 10;
    /// `kCGEventKeyUp`.
    pub const KEY_UP: u32 = 11;
    /// `kCGEventFlagsChanged`.
    pub const FLAGS_CHANGED: u32 = 12;
}

/// A modifier key held on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModifierKey {
    /// Fn, the globe key.
    Fn,
    /// Right Option.
    RightOption,
    /// Right Command.
    RightCommand,
    /// Right Control.
    RightControl,
    /// Right Shift.
    RightShift,
}

impl ModifierKey {
    /// The keycode its `flagsChanged` events carry.
    pub(crate) const fn keycode(self) -> u16 {
        match self {
            Self::Fn => keycode::FUNCTION,
            Self::RightOption => keycode::RIGHT_OPTION,
            Self::RightCommand => keycode::RIGHT_COMMAND,
            Self::RightControl => keycode::RIGHT_CONTROL,
            Self::RightShift => keycode::RIGHT_SHIFT,
        }
    }

    /// The flag bit that answers "is this key down after the change". Only the per-key bits can:
    /// with both Command keys held, releasing the right one leaves the Command flag set.
    pub(crate) const fn down_mask(self) -> u64 {
        match self {
            Self::Fn => flag::SECONDARY_FN,
            Self::RightOption => flag::DEVICE_RIGHT_OPTION,
            Self::RightCommand => flag::DEVICE_RIGHT_COMMAND,
            Self::RightControl => flag::DEVICE_RIGHT_CONTROL,
            Self::RightShift => flag::DEVICE_RIGHT_SHIFT,
        }
    }
}

/// Modifiers plus one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chord {
    /// The `CGEventFlags` bits that must be down, exactly.
    pub(crate) modifiers: u64,
    /// The key.
    pub(crate) keycode: u16,
}

impl Chord {
    /// Whether a key-down of `keycode` with `flags` is this chord.
    pub(crate) fn matches(self, keycode: u16, flags: u64) -> bool {
        let compared = if self.modifiers & flag::SECONDARY_FN != 0 {
            flag::CHORD_MODIFIERS | flag::SECONDARY_FN
        } else {
            flag::CHORD_MODIFIERS
        };
        keycode == self.keycode && flags & compared == self.modifiers
    }
}

/// A parsed hotkey token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Binding {
    /// Hold one modifier.
    Modifier(ModifierKey),
    /// Hold a chord.
    Chord(Chord),
}

impl Binding {
    /// Parses a platform token. Anything this platform cannot bind is
    /// [`PlatformError::Unsupported`].
    pub(crate) fn parse(token: &str) -> Result<Self, PlatformError> {
        let token = token.trim().to_ascii_lowercase();
        if token.is_empty() {
            return Err(PlatformError::Unsupported("an empty hotkey"));
        }
        if !token.contains('+') {
            if let Some(key) = modifier_key(&token) {
                return Ok(Self::Modifier(key));
            }
            if let Some(code) = function_key(&token) {
                return Ok(Self::Chord(Chord {
                    modifiers: 0,
                    keycode: code,
                }));
            }
            if key_code(&token).is_some() {
                return Err(PlatformError::Unsupported(
                    "a hotkey on a typing key needs a modifier",
                ));
            }
            return Err(PlatformError::Unsupported("an unknown hotkey token"));
        }

        let parts: Vec<&str> = token.split('+').map(str::trim).collect();
        let Some((key, modifiers)) = parts.split_last() else {
            return Err(PlatformError::Unsupported("an empty hotkey"));
        };
        if key.is_empty() || modifiers.iter().any(|m| m.is_empty()) {
            return Err(PlatformError::Unsupported("a hotkey with an empty part"));
        }
        let mut bits = 0;
        for name in modifiers {
            let Some(bit) = chord_modifier(name) else {
                return Err(PlatformError::Unsupported(
                    "an unknown modifier in a hotkey chord",
                ));
            };
            if bits & bit != 0 {
                return Err(PlatformError::Unsupported(
                    "a repeated modifier in a hotkey",
                ));
            }
            bits |= bit;
        }
        let Some(code) = key_code(key) else {
            return Err(PlatformError::Unsupported(
                "an unknown key in a hotkey chord",
            ));
        };
        Ok(Self::Chord(Chord {
            modifiers: bits,
            keycode: code,
        }))
    }

    /// The `CGEventMask` the tap needs: `flagsChanged` for a modifier, key down and up for a
    /// chord. Nothing else passes through the callback, which keeps the tap off the path of
    /// every other keystroke.
    pub(crate) fn event_mask(self) -> u64 {
        match self {
            Self::Modifier(_) => 1 << event_type::FLAGS_CHANGED,
            Self::Chord(_) => (1 << event_type::KEY_DOWN) | (1 << event_type::KEY_UP),
        }
    }
}

fn modifier_key(token: &str) -> Option<ModifierKey> {
    Some(match token {
        "fn" => ModifierKey::Fn,
        "right_option" | "right_opt" | "right_alt" => ModifierKey::RightOption,
        "right_command" | "right_cmd" => ModifierKey::RightCommand,
        "right_control" | "right_ctrl" => ModifierKey::RightControl,
        "right_shift" => ModifierKey::RightShift,
        _ => return None,
    })
}

fn chord_modifier(name: &str) -> Option<u64> {
    Some(match name {
        "ctrl" | "control" => flag::CONTROL,
        "shift" => flag::SHIFT,
        "option" | "opt" | "alt" => flag::OPTION,
        "cmd" | "command" => flag::COMMAND,
        "fn" => flag::SECONDARY_FN,
        _ => return None,
    })
}

fn function_key(name: &str) -> Option<u16> {
    let n: usize = name.strip_prefix('f')?.parse().ok()?;
    keycode::FUNCTION_KEYS.get(n.checked_sub(1)?).copied()
}

fn key_code(name: &str) -> Option<u16> {
    if let Some(code) = function_key(name) {
        return Some(code);
    }
    match name {
        "space" => return Some(keycode::SPACE),
        "return" | "enter" => return Some(keycode::RETURN),
        "tab" => return Some(keycode::TAB),
        "escape" | "esc" => return Some(keycode::ESCAPE),
        _ => {}
    }
    let mut chars = name.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    match c {
        'a'..='z' => keycode::ANSI_LETTERS
            .get(usize::from(c as u8 - b'a'))
            .copied(),
        '0'..='9' => keycode::ANSI_DIGITS
            .get(usize::from(c as u8 - b'0'))
            .copied(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(token: &str) -> Chord {
        match Binding::parse(token) {
            Ok(Binding::Chord(c)) => c,
            other => panic!("{token:?} parsed as {other:?}"),
        }
    }

    #[test]
    fn modifier_tokens_parse() {
        let cases = [
            ("fn", ModifierKey::Fn),
            ("right_option", ModifierKey::RightOption),
            ("right_command", ModifierKey::RightCommand),
            ("right_control", ModifierKey::RightControl),
            ("right_shift", ModifierKey::RightShift),
        ];
        for (token, key) in cases {
            assert_eq!(Binding::parse(token), Ok(Binding::Modifier(key)), "{token}");
        }
    }

    #[test]
    fn inkwell_0_2_tokens_still_parse() {
        assert_eq!(
            Binding::parse("right_cmd"),
            Ok(Binding::Modifier(ModifierKey::RightCommand))
        );
        assert_eq!(
            Binding::parse("right_opt"),
            Ok(Binding::Modifier(ModifierKey::RightOption))
        );
        assert_eq!(
            Binding::parse("right_ctrl"),
            Ok(Binding::Modifier(ModifierKey::RightControl))
        );
    }

    #[test]
    fn chord_tokens_parse() {
        assert_eq!(
            chord("ctrl+shift+space"),
            Chord {
                modifiers: flag::CONTROL | flag::SHIFT,
                keycode: keycode::SPACE
            }
        );
        assert_eq!(
            chord("cmd+option+d"),
            Chord {
                modifiers: flag::COMMAND | flag::OPTION,
                keycode: 0x02
            }
        );
        assert_eq!(
            chord("fn+f5"),
            Chord {
                modifiers: flag::SECONDARY_FN,
                keycode: 0x60
            }
        );
        assert_eq!(
            chord("ctrl+1"),
            Chord {
                modifiers: flag::CONTROL,
                keycode: 0x12
            }
        );
    }

    #[test]
    fn a_function_key_binds_alone() {
        assert_eq!(
            chord("f13"),
            Chord {
                modifiers: 0,
                keycode: 0x69
            }
        );
        assert_eq!(chord("f20").keycode, 0x5A);
    }

    #[test]
    fn tokens_ignore_case_and_surrounding_space() {
        assert_eq!(chord(" Ctrl + Shift + SPACE "), chord("ctrl+shift+space"));
        assert_eq!(
            Binding::parse("  FN "),
            Ok(Binding::Modifier(ModifierKey::Fn))
        );
    }

    #[test]
    fn unknown_tokens_are_unsupported() {
        let refused = [
            "",
            "   ",
            "hyper",
            "left_option",
            "option",
            "cmd",
            "right_fn",
            "a",
            "space",
            "f0",
            "f21",
            "ctrl+",
            "+a",
            "ctrl++a",
            "ctrl+ctrl+a",
            "ctrl+shift",
            "ctrl+hyper+a",
            "fn+right_option",
            "cmd+right_option",
            "ctrl+é",
            "ctrl+ab",
        ];
        for token in refused {
            assert!(
                matches!(Binding::parse(token), Err(PlatformError::Unsupported(_))),
                "{token:?} should be unsupported, got {:?}",
                Binding::parse(token)
            );
        }
    }

    /// Values from `HIToolbox/Events.h` and `IOLLEvent.h`. A wrong number here fails silently at
    /// runtime (the tap just never matches), which is why they are pinned, not trusted.
    #[test]
    fn keycodes_and_masks_match_the_headers() {
        assert_eq!(ModifierKey::Fn.keycode(), 63);
        assert_eq!(ModifierKey::RightCommand.keycode(), 54);
        assert_eq!(ModifierKey::RightOption.keycode(), 61);
        assert_eq!(ModifierKey::RightControl.keycode(), 62);
        assert_eq!(ModifierKey::RightShift.keycode(), 60);
        assert_eq!(ModifierKey::Fn.down_mask(), 0x0080_0000);
        assert_eq!(ModifierKey::RightCommand.down_mask(), 0x0000_0010);
        assert_eq!(ModifierKey::RightOption.down_mask(), 0x0000_0040);
        assert_eq!(ModifierKey::RightControl.down_mask(), 0x0000_2000);
        assert_eq!(ModifierKey::RightShift.down_mask(), 0x0000_0004);
        assert_eq!(keycode::ANSI_V, 0x09);
        assert_eq!(keycode::SPACE, 49);
        // Spot checks across the ANSI tables: A, Q, Z, 0, 5, 9, F1, F13.
        assert_eq!(keycode::ANSI_LETTERS[0], 0x00);
        assert_eq!(keycode::ANSI_LETTERS[16], 0x0C);
        assert_eq!(keycode::ANSI_LETTERS[25], 0x06);
        assert_eq!(keycode::ANSI_DIGITS[0], 0x1D);
        assert_eq!(keycode::ANSI_DIGITS[5], 0x17);
        assert_eq!(keycode::ANSI_DIGITS[9], 0x19);
        assert_eq!(keycode::FUNCTION_KEYS[0], 0x7A);
        assert_eq!(keycode::FUNCTION_KEYS[12], 0x69);
    }

    /// The flag bits above are spelled out so the parser is plain Rust; this pins them to the
    /// framework crate's constants so the two can never drift.
    #[test]
    fn flag_bits_match_core_graphics() {
        use objc2_core_graphics::CGEventFlags as F;
        assert_eq!(flag::SHIFT, F::MaskShift.bits());
        assert_eq!(flag::CONTROL, F::MaskControl.bits());
        assert_eq!(flag::OPTION, F::MaskAlternate.bits());
        assert_eq!(flag::COMMAND, F::MaskCommand.bits());
        assert_eq!(flag::SECONDARY_FN, F::MaskSecondaryFn.bits());
    }

    #[test]
    fn event_types_match_core_graphics() {
        use objc2_core_graphics::CGEventType as T;
        assert_eq!(event_type::KEY_DOWN, T::KeyDown.0);
        assert_eq!(event_type::KEY_UP, T::KeyUp.0);
        assert_eq!(event_type::FLAGS_CHANGED, T::FlagsChanged.0);
    }

    #[test]
    fn letters_are_the_ansi_positions_not_the_alphabet() {
        // V is the paste key; the Unicode fallback types on A.
        assert_eq!(chord("cmd+v").keycode, keycode::ANSI_V);
        assert_eq!(chord("cmd+a").keycode, 0x00);
        assert_eq!(chord("cmd+s").keycode, 0x01);
    }

    #[test]
    fn a_chord_matches_its_exact_modifiers() {
        let c = chord("ctrl+shift+space");
        let both = flag::CONTROL | flag::SHIFT;
        assert!(c.matches(keycode::SPACE, both));
        assert!(!c.matches(keycode::SPACE, flag::CONTROL), "missing shift");
        assert!(
            !c.matches(keycode::SPACE, both | flag::COMMAND),
            "extra command"
        );
        assert!(!c.matches(keycode::RETURN, both), "wrong key");
        // Device bits, Caps Lock, the keypad bit and Fn do not matter unless named.
        use objc2_core_graphics::CGEventFlags as F;
        let noise = F::MaskAlphaShift.bits()
            | F::MaskNumericPad.bits()
            | flag::SECONDARY_FN
            | flag::DEVICE_RIGHT_SHIFT
            | 0x1;
        assert!(c.matches(keycode::SPACE, both | noise));
    }

    #[test]
    fn a_chord_that_names_fn_requires_it() {
        let c = chord("fn+f5");
        assert!(c.matches(0x60, flag::SECONDARY_FN));
        assert!(!c.matches(0x60, 0));
        let bare = chord("f5");
        assert!(
            bare.matches(0x60, flag::SECONDARY_FN),
            "laptops set Fn on F-keys"
        );
        assert!(bare.matches(0x60, 0));
    }

    #[test]
    fn the_tap_only_sees_the_events_its_binding_needs() {
        assert_eq!(Binding::parse("fn").map(Binding::event_mask), Ok(1 << 12));
        assert_eq!(
            Binding::parse("ctrl+shift+space").map(Binding::event_mask),
            Ok((1 << 10) | (1 << 11))
        );
    }
}
