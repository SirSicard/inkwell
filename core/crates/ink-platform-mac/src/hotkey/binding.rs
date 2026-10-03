//! Hotkey tokens, parsed in pure Rust.
//!
//! Two shapes:
//! - **A modifier held on its own:** `"fn"`, `"right_option"`, `"right_command"`,
//!   `"right_control"`, `"right_shift"`. These arrive as `flagsChanged` events only, and the
//!   right-hand keys are told apart from the left by the device-dependent flag bits, because the
//!   public Command (or Option...) flag is set for either side. Left-hand modifiers alone are not
//!   offered: watching left Command would start a hold on every Cmd+C in every app.
//! - **A chord:** modifiers and one key, `"ctrl+shift+space"`, `"cmd+option+d"`, or a function
//!   key alone, `"f13"`. Any other key needs a modifier, or the tap would swallow it everywhere;
//!   Shift alone counts only with a function key (with any other it already types or selects),
//!   and a chord naming Fn takes no key the keyboard sets Fn on by itself (the function row, the
//!   arrows), where the tap could not tell Fn from no Fn.
//!
//! The user may choose any binding of those shapes. [`check_token`] is the one judge: it gives a
//! binding's canonical spelling, or a [`refusal`] in plain words, and a binding parses exactly
//! when it says yes. Keys are letters, digits, F1 to F20, and the keys named in [`NAMED_KEYS`]
//! (Space, Return, the arrows, punctuation...).
//!
//! Letter, digit and punctuation tokens name the key at that position on an ANSI keyboard
//! (`kVK_ANSI_*`). The
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
    /// Its 1.0 token, the canonical spelling.
    const fn token(self) -> &'static str {
        match self {
            Self::Fn => "fn",
            Self::RightOption => "right_option",
            Self::RightCommand => "right_command",
            Self::RightControl => "right_control",
            Self::RightShift => "right_shift",
        }
    }

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
    /// [`PlatformError::Unsupported`], with the [`refusal`] that says why.
    pub(crate) fn parse(token: &str) -> Result<Self, PlatformError> {
        parse_token(token).map_err(PlatformError::Unsupported)
    }

    /// The one spelling of this binding ([`check_token`]): a modifier's 1.0 token, or a chord's
    /// modifiers in the Mac's order (Fn, Control, Option, Shift, Command) and its key's name.
    pub(crate) fn canonical(self) -> String {
        match self {
            Self::Modifier(key) => key.token().to_owned(),
            Self::Chord(chord) => {
                let mut parts: Vec<&str> = CHORD_ORDER
                    .iter()
                    .filter(|(bit, _)| chord.modifiers & bit != 0)
                    .map(|(_, name)| *name)
                    .collect();
                parts.push(key_name(chord.keycode));
                parts.join("+")
            }
        }
    }

    /// The `CGEventMask` the tap needs: `flagsChanged` for a modifier, key down and up for a
    /// chord, and `flagsChanged` too for a chord with modifiers, whose hold ends when one comes
    /// up. Nothing else passes through the callback, which keeps the tap off the path of every
    /// other keystroke.
    pub(crate) fn event_mask(self) -> u64 {
        let keys = (1 << event_type::KEY_DOWN) | (1 << event_type::KEY_UP);
        match self {
            Self::Modifier(_) => 1 << event_type::FLAGS_CHANGED,
            Self::Chord(Chord { modifiers: 0, .. }) => keys,
            Self::Chord(_) => keys | (1 << event_type::FLAGS_CHANGED),
        }
    }
}

/// Whether the Mac can watch `token`: its canonical spelling ([`Binding::canonical`]), or the
/// [`refusal`] that says why not. The one rule set: a hotkey binds exactly when this says yes.
pub(crate) fn check_token(token: &str) -> Result<String, &'static str> {
    parse_token(token).map(Binding::canonical)
}

/// Why a token cannot be watched, in words for the person choosing a key: the shell shows them
/// after "can't use that:", so each starts in lower case and has no full stop.
pub(crate) mod refusal {
    pub const EMPTY: &str = "no key was given";
    /// A letter, digit, arrow... alone: the tap would swallow it everywhere.
    pub const KEY_ALONE: &str =
        "that key on its own would stop working everywhere else; add Control, Option or Command";
    /// Shift and any key but a function key already does something everywhere: a capital, a
    /// symbol, selecting text, a back-tab. The tap would swallow it.
    pub const SHIFT_TYPES: &str = "Shift with that key already does something everywhere (a capital, a symbol, selecting text); add Control, Option or Command";
    /// The keyboard sets Fn on the function row and the arrows by itself, so Fn named with one of
    /// them cannot be told from the key alone.
    pub const FN_ALREADY: &str = "the keyboard sets Fn on that key by itself, so Fn with it can't be told from the key alone; leave Fn out";
    /// Watching left Command would start a hold on every Cmd+C in every app.
    pub const LEFT_MODIFIER_ALONE: &str = "a left-hand modifier on its own would start dictation with every shortcut that uses it; use a right-hand one, or add a key";
    pub const MODIFIERS_ONLY: &str = "modifiers together need a key with them; hold one right-hand modifier on its own, or add a key";
    /// Caps Lock reports a switch, not a hold.
    pub const CAPS_LOCK: &str = "Caps Lock switches on and off instead of being held";
    pub const UNKNOWN_KEY: &str = "Inkwell doesn't know that key";
    pub const UNKNOWN_MODIFIER: &str = "one of the modifiers isn't one Inkwell knows";
    pub const REPEATED_MODIFIER: &str = "a modifier is named twice";
    pub const EMPTY_PART: &str = "a part of the shortcut is empty";
}

fn parse_token(token: &str) -> Result<Binding, &'static str> {
    let token = token.trim().to_ascii_lowercase();
    if token.is_empty() {
        return Err(refusal::EMPTY);
    }
    if !token.contains('+') {
        if let Some(key) = modifier_key(&token) {
            return Ok(Binding::Modifier(key));
        }
        if let Some(code) = function_key(&token) {
            return Ok(Binding::Chord(Chord {
                modifiers: 0,
                keycode: code,
            }));
        }
        return Err(if is_modifier_name(&token) {
            refusal::LEFT_MODIFIER_ALONE
        } else if token == "caps_lock" || token == "capslock" {
            refusal::CAPS_LOCK
        } else if key_code(&token).is_some() {
            refusal::KEY_ALONE
        } else {
            refusal::UNKNOWN_KEY
        });
    }

    let parts: Vec<&str> = token.split('+').map(str::trim).collect();
    let Some((key, modifiers)) = parts.split_last() else {
        return Err(refusal::EMPTY);
    };
    if key.is_empty() || modifiers.iter().any(|m| m.is_empty()) {
        return Err(refusal::EMPTY_PART);
    }
    let mut bits = 0;
    for name in modifiers {
        let Some(bit) = chord_modifier(name) else {
            return Err(refusal::UNKNOWN_MODIFIER);
        };
        if bits & bit != 0 {
            return Err(refusal::REPEATED_MODIFIER);
        }
        bits |= bit;
    }
    if is_modifier_name(key) || modifier_key(key).is_some() {
        return Err(refusal::MODIFIERS_ONLY);
    }
    let Some(code) = key_code(key) else {
        return Err(refusal::UNKNOWN_KEY);
    };
    if bits == flag::SHIFT && function_key(key).is_none() {
        return Err(refusal::SHIFT_TYPES);
    }
    if bits & flag::SECONDARY_FN != 0 && sets_fn(code) {
        return Err(refusal::FN_ALREADY);
    }
    Ok(Binding::Chord(Chord {
        modifiers: bits,
        keycode: code,
    }))
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

/// A chord's modifiers in the order a canonical token spells them.
const CHORD_ORDER: [(u64, &str); 5] = [
    (flag::SECONDARY_FN, "fn"),
    (flag::CONTROL, "ctrl"),
    (flag::OPTION, "option"),
    (flag::SHIFT, "shift"),
    (flag::COMMAND, "cmd"),
];

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

/// A modifier named without a side, or by its left-hand key: never watched on its own, and never
/// a chord's key.
fn is_modifier_name(name: &str) -> bool {
    let side = name.strip_prefix("left_").unwrap_or(name);
    chord_modifier(side).is_some_and(|bit| bit != flag::SECONDARY_FN)
}

/// Keys named in words, past letters, digits and function keys: (canonical name, keycode). Codes from `HIToolbox/Events.h`; the punctuation is by ANSI position,
/// as the letters are.
pub(crate) const NAMED_KEYS: &[(&str, u16)] = &[
    ("space", keycode::SPACE),
    ("return", keycode::RETURN),
    ("tab", keycode::TAB),
    ("escape", keycode::ESCAPE),
    ("delete", 0x33),
    ("forward_delete", 0x75),
    ("left", 0x7B),
    ("right", 0x7C),
    ("down", 0x7D),
    ("up", 0x7E),
    ("home", 0x73),
    ("end", 0x77),
    ("page_up", 0x74),
    ("page_down", 0x79),
    ("minus", 0x1B),
    ("equal", 0x18),
    ("left_bracket", 0x21),
    ("right_bracket", 0x1E),
    ("backslash", 0x2A),
    ("semicolon", 0x29),
    ("quote", 0x27),
    ("comma", 0x2B),
    ("period", 0x2F),
    ("slash", 0x2C),
    ("grave", 0x32),
    // `kVK_ISO_Section`: the key left of 1 on ISO keyboards (§ on many).
    ("section", 0x0A),
];

/// Other spellings of [`NAMED_KEYS`]: the character a punctuation key types unshifted, and the
/// names other apps use.
const KEY_ALIASES: &[(&str, &str)] = &[
    ("enter", "return"),
    ("esc", "escape"),
    ("backspace", "delete"),
    ("pageup", "page_up"),
    ("pagedown", "page_down"),
    ("-", "minus"),
    ("=", "equal"),
    ("[", "left_bracket"),
    ("]", "right_bracket"),
    ("\\", "backslash"),
    (";", "semicolon"),
    ("'", "quote"),
    (",", "comma"),
    (".", "period"),
    ("/", "slash"),
    ("`", "grave"),
    ("§", "section"),
];

const LETTER_NAMES: [&str; 26] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];
const DIGIT_NAMES: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
const FUNCTION_NAMES: [&str; 20] = [
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "f13", "f14", "f15",
    "f16", "f17", "f18", "f19", "f20",
];

fn function_key(name: &str) -> Option<u16> {
    let n: usize = name.strip_prefix('f')?.parse().ok()?;
    keycode::FUNCTION_KEYS.get(n.checked_sub(1)?).copied()
}

fn key_code(name: &str) -> Option<u16> {
    if let Some(code) = function_key(name) {
        return Some(code);
    }
    let name = KEY_ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name, |(_, canonical)| *canonical);
    if let Some((_, code)) = NAMED_KEYS.iter().find(|(n, _)| *n == name) {
        return Some(*code);
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

/// The canonical name of a key [`key_code`] gave. Every code it gives is in one of the tables.
fn key_name(code: u16) -> &'static str {
    let find = |codes: &[u16], names: &[&'static str]| {
        codes
            .iter()
            .position(|c| *c == code)
            .and_then(|i| names.get(i).copied())
    };
    find(&keycode::ANSI_LETTERS, &LETTER_NAMES)
        .or_else(|| find(&keycode::ANSI_DIGITS, &DIGIT_NAMES))
        .or_else(|| find(&keycode::FUNCTION_KEYS, &FUNCTION_NAMES))
        .or_else(|| {
            NAMED_KEYS
                .iter()
                .find(|(_, c)| *c == code)
                .map(|(name, _)| *name)
        })
        // Unreachable for a parsed chord; a name that parses to nothing rather than a panic.
        .unwrap_or("unknown")
}

/// Whether the keyboard sets the Fn flag on this key by itself: the function row, the arrows,
/// and Home, End, Page Up, Page Down and Forward Delete (the keys Fn+arrow and Fn+Delete make on
/// a laptop). [`Chord::matches`] ignores Fn unless a chord names it for the same reason.
fn sets_fn(code: u16) -> bool {
    const NAVIGATION: [u16; 9] = [0x7B, 0x7C, 0x7D, 0x7E, 0x73, 0x77, 0x74, 0x79, 0x75];
    keycode::FUNCTION_KEYS.contains(&code) || NAVIGATION.contains(&code)
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
            chord("fn+d"),
            Chord {
                modifiers: flag::SECONDARY_FN,
                keycode: 0x02
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
        let c = chord("fn+d");
        assert!(c.matches(0x02, flag::SECONDARY_FN));
        assert!(!c.matches(0x02, 0));
        let bare = chord("f5");
        assert!(
            bare.matches(0x60, flag::SECONDARY_FN),
            "laptops set Fn on F-keys"
        );
        assert!(bare.matches(0x60, 0));
    }

    /// What `hotkey.check` stores and shows: one spelling per binding, modifiers in the Mac's own
    /// order (Fn, Control, Option, Shift, Command), 0.2's aliases under their 1.0 names.
    #[test]
    fn every_binding_has_one_canonical_spelling() {
        let cases = [
            (" Shift + Ctrl + SPACE ", "ctrl+shift+space"),
            ("cmd+option+d", "option+cmd+d"),
            (
                "command+alt+shift+control+fn+k",
                "fn+ctrl+option+shift+cmd+k",
            ),
            ("right_opt", "right_option"),
            ("right_cmd", "right_command"),
            ("right_alt", "right_option"),
            ("FN", "fn"),
            ("F13", "f13"),
            ("fn+d", "fn+d"),
            ("ctrl+§", "ctrl+section"),
            ("ctrl+enter", "ctrl+return"),
            ("ctrl+esc", "ctrl+escape"),
            ("ctrl+backspace", "ctrl+delete"),
            ("option+-", "option+minus"),
            ("ctrl+/", "ctrl+slash"),
            ("cmd+shift+pageup", "shift+cmd+page_up"),
        ];
        for (token, canonical) in cases {
            assert_eq!(check_token(token), Ok(canonical.to_owned()), "{token:?}");
            assert_eq!(
                check_token(canonical),
                Ok(canonical.to_owned()),
                "{canonical}"
            );
            assert_eq!(
                Binding::parse(token),
                Binding::parse(canonical),
                "{token:?}"
            );
        }
    }

    /// Every key name reads back as itself: the canonical spelling never names a key the parser
    /// does not know.
    #[test]
    fn every_key_name_round_trips() {
        let mut names: Vec<String> = ('a'..='z').chain('0'..='9').map(String::from).collect();
        names.extend((1..=20).map(|n| format!("f{n}")));
        names.extend(NAMED_KEYS.iter().map(|(name, _)| (*name).to_owned()));
        for name in names {
            let token = format!("ctrl+{name}");
            assert_eq!(check_token(&token), Ok(token.clone()), "{token}");
        }
    }

    /// The keys past letters, digits and function keys, pinned to `HIToolbox/Events.h`.
    #[test]
    fn named_keys_match_the_headers() {
        let expected = [
            ("delete", 0x33),
            ("forward_delete", 0x75),
            ("left", 0x7B),
            ("right", 0x7C),
            ("down", 0x7D),
            ("up", 0x7E),
            ("home", 0x73),
            ("end", 0x77),
            ("page_up", 0x74),
            ("page_down", 0x79),
            ("minus", 0x1B),
            ("equal", 0x18),
            ("left_bracket", 0x21),
            ("right_bracket", 0x1E),
            ("backslash", 0x2A),
            ("semicolon", 0x29),
            ("quote", 0x27),
            ("comma", 0x2B),
            ("period", 0x2F),
            ("slash", 0x2C),
            ("grave", 0x32),
            ("section", 0x0A),
        ];
        for (name, code) in expected {
            assert_eq!(chord(&format!("ctrl+{name}")).keycode, code, "{name}");
        }
    }

    /// Each refusal says why, in words for the person choosing the key.
    #[test]
    fn refusals_say_why() {
        let cases = [
            ("", refusal::EMPTY),
            ("  ", refusal::EMPTY),
            ("a", refusal::KEY_ALONE),
            ("space", refusal::KEY_ALONE),
            ("left", refusal::KEY_ALONE),
            ("escape", refusal::KEY_ALONE),
            ("shift+a", refusal::SHIFT_TYPES),
            ("shift+7", refusal::SHIFT_TYPES),
            ("shift+space", refusal::SHIFT_TYPES),
            ("shift+slash", refusal::SHIFT_TYPES),
            ("shift+left", refusal::SHIFT_TYPES),
            ("shift+return", refusal::SHIFT_TYPES),
            ("shift+tab", refusal::SHIFT_TYPES),
            ("shift+delete", refusal::SHIFT_TYPES),
            ("shift+page_down", refusal::SHIFT_TYPES),
            ("fn+f5", refusal::FN_ALREADY),
            ("fn+left", refusal::FN_ALREADY),
            ("fn+ctrl+home", refusal::FN_ALREADY),
            ("fn+forward_delete", refusal::FN_ALREADY),
            ("left_option", refusal::LEFT_MODIFIER_ALONE),
            ("option", refusal::LEFT_MODIFIER_ALONE),
            ("cmd", refusal::LEFT_MODIFIER_ALONE),
            ("left_command", refusal::LEFT_MODIFIER_ALONE),
            ("left_shift", refusal::LEFT_MODIFIER_ALONE),
            ("ctrl", refusal::LEFT_MODIFIER_ALONE),
            ("ctrl+shift", refusal::MODIFIERS_ONLY),
            ("fn+right_option", refusal::MODIFIERS_ONLY),
            ("cmd+right_option", refusal::MODIFIERS_ONLY),
            ("ctrl+left_shift", refusal::MODIFIERS_ONLY),
            ("caps_lock", refusal::CAPS_LOCK),
            ("hyper", refusal::UNKNOWN_KEY),
            ("right_fn", refusal::UNKNOWN_KEY),
            ("f0", refusal::UNKNOWN_KEY),
            ("f21", refusal::UNKNOWN_KEY),
            ("ctrl+é", refusal::UNKNOWN_KEY),
            ("ctrl+ab", refusal::UNKNOWN_KEY),
            ("ctrl+hyper+a", refusal::UNKNOWN_MODIFIER),
            ("ctrl+ctrl+a", refusal::REPEATED_MODIFIER),
            ("ctrl+control+a", refusal::REPEATED_MODIFIER),
            ("ctrl+", refusal::EMPTY_PART),
            ("+a", refusal::EMPTY_PART),
            ("ctrl++a", refusal::EMPTY_PART),
        ];
        for (token, why) in cases {
            assert_eq!(check_token(token), Err(why), "{token:?}");
            assert_eq!(
                Binding::parse(token),
                Err(PlatformError::Unsupported(why)),
                "{token:?}"
            );
        }
    }

    /// Shift alone goes only with a function key: with any other key it already does something
    /// everywhere (a capital, a symbol, selecting, a back-tab). With another modifier it is fine.
    #[test]
    fn shift_alone_goes_only_with_a_function_key() {
        for token in [
            "shift+f5",
            "shift+f13",
            "ctrl+shift+left",
            "shift+cmd+return",
        ] {
            assert!(check_token(token).is_ok(), "{token}");
        }
    }

    /// The keyboard sets Fn on the function row, the arrows and the keys Fn+arrow makes by itself,
    /// so the tap cannot tell Fn+F5 from F5: a chord naming Fn takes another key. Without Fn named
    /// those keys are fine, and Fn with a letter is too.
    #[test]
    fn fn_goes_only_with_keys_that_do_not_set_it_themselves() {
        for token in ["f5", "ctrl+left", "fn+d", "fn+ctrl+space", "fn+return"] {
            assert!(check_token(token).is_ok(), "{token}");
        }
    }

    #[test]
    fn the_tap_only_sees_the_events_its_binding_needs() {
        assert_eq!(Binding::parse("fn").map(Binding::event_mask), Ok(1 << 12));
        assert_eq!(
            Binding::parse("ctrl+shift+space").map(Binding::event_mask),
            Ok((1 << 10) | (1 << 11) | (1 << 12)),
            "a chord's modifiers coming up end its hold"
        );
        assert_eq!(
            Binding::parse("f13").map(Binding::event_mask),
            Ok((1 << 10) | (1 << 11)),
            "a function key alone has no modifier to watch"
        );
    }
}
