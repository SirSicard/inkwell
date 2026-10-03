//! Hotkey tokens, parsed in pure Rust, and the Windows default.
//!
//! Two shapes, as on the Mac:
//! - **A modifier held on its own:** `"right_control"`, `"right_alt"`, `"right_shift"`,
//!   `"right_win"`. The low-level hook reports left and right modifiers as different virtual keys
//!   (`VK_RCONTROL`, `VK_RMENU`...), so the right-hand key is watched on its own. Left-hand
//!   modifiers alone are not offered: watching left Ctrl would start a hold on every Ctrl+C.
//! - **A chord:** modifiers and one key, `"ctrl+shift+space"`, `"alt+d"`, or a function key alone,
//!   `"f13"`. Any other key needs at least one modifier, or the hook would swallow it everywhere.
//!   The keys are named as the Mac names them ([`NAMED_KEYS`]: arrows, Delete, Home and End,
//!   Page Up and Down, the punctuation keys by their US position), plus `oem_102`, the extra key
//!   by left Shift on ISO keyboards.
//!
//! A token Windows cannot watch is refused with a reason in plain words ([`refusal`], the Mac's
//! list in Windows' terms), which the shell shows as it is.
//!
//! **Fn is not offered:** the keyboard handles it in firmware and Windows never sees it. The
//! Windows default is [`DEFAULT_BINDING`], right Ctrl: it is on full-size and most laptop
//! keyboards, and pressed on its own it does nothing in Windows or in apps (right Alt would open
//! menus on US layouts, and is AltGr on many others).
//!
//! Letters and digits are virtual-key codes (`VK_A`...), which Windows maps through the active
//! layout, as every Windows shortcut does.
//!
//! The Mac spellings of the right-hand modifiers are accepted where Windows has the key
//! (`right_ctrl`, `right_option` and `right_opt` for right Alt). `right_command` is refused: its
//! place on a Windows keyboard is the Windows key, which most keyboards lack on the right.
#![cfg(windows)]

use ink_core::PlatformError;

/// The dictation key on Windows until the user picks another.
pub const DEFAULT_BINDING: &str = "right_control";

/// The modifier keys a shell may offer on Windows, held on their own.
pub const KEYS: &[&str] = &["right_control", "right_alt", "right_shift", "right_win"];

/// Virtual-key codes (`WinUser.h`), as the low-level hook reports them.
pub(crate) mod vk {
    pub const RCONTROL: u32 = 0xA3;
    pub const RMENU: u32 = 0xA5;
    pub const RSHIFT: u32 = 0xA1;
    pub const RWIN: u32 = 0x5C;
    pub const LWIN: u32 = 0x5B;
    pub const CONTROL: u32 = 0x11;
    pub const SHIFT: u32 = 0x10;
    pub const MENU: u32 = 0x12;
    pub const SPACE: u32 = 0x20;
    pub const RETURN: u32 = 0x0D;
    pub const TAB: u32 = 0x09;
    pub const ESCAPE: u32 = 0x1B;
    /// `VK_V`: the paste key.
    pub const V: u32 = 0x56;
    /// The left-hand modifiers, as the hook reports them (a chord's modifier coming up).
    pub const LCONTROL: u32 = 0xA2;
    pub const LSHIFT: u32 = 0xA0;
    pub const LMENU: u32 = 0xA4;
    /// `VK_F1`; F1 to F24 are consecutive.
    pub const F1: u32 = 0x70;
}

/// Chord modifier bits, as the hook reads them at the key's press.
pub(crate) mod modifier {
    pub const CTRL: u8 = 1;
    pub const SHIFT: u8 = 2;
    pub const ALT: u8 = 4;
    pub const WIN: u8 = 8;
}

/// A right-hand modifier held on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RightModifier {
    Control,
    Alt,
    Shift,
    Win,
}

impl RightModifier {
    /// Its token, the canonical spelling.
    const fn token(self) -> &'static str {
        match self {
            Self::Control => "right_control",
            Self::Alt => "right_alt",
            Self::Shift => "right_shift",
            Self::Win => "right_win",
        }
    }

    /// The virtual key its events carry.
    pub(crate) const fn vk(self) -> u32 {
        match self {
            Self::Control => vk::RCONTROL,
            Self::Alt => vk::RMENU,
            Self::Shift => vk::RSHIFT,
            Self::Win => vk::RWIN,
        }
    }
}

/// Modifiers plus one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chord {
    /// The [`modifier`] bits that must be down, exactly.
    pub(crate) modifiers: u8,
    /// The key.
    pub(crate) vk: u32,
}

impl Chord {
    /// Whether a press of `vk` with `modifiers` down is this chord.
    pub(crate) fn matches(self, vk: u32, modifiers: u8) -> bool {
        vk == self.vk && modifiers == self.modifiers
    }
}

/// A parsed hotkey token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Binding {
    /// Hold one modifier.
    Modifier(RightModifier),
    /// Hold a chord.
    Chord(Chord),
}

impl Binding {
    /// Parses a platform token. Anything Windows cannot bind is [`PlatformError::Unsupported`],
    /// with a [`refusal`] reason.
    pub(crate) fn parse(token: &str) -> Result<Self, PlatformError> {
        parse_token(token).map_err(PlatformError::Unsupported)
    }

    /// The one spelling of this binding (`hotkey::check`): a modifier's token, or a chord's
    /// modifiers in Windows' order (Ctrl, Alt, Shift, Win) and its key's name, so two spellings of
    /// one chord, or a Mac alias, compare as one key.
    pub(crate) fn canonical(self) -> String {
        match self {
            Self::Modifier(key) => key.token().to_owned(),
            Self::Chord(chord) => {
                let mut parts: Vec<String> = CHORD_ORDER
                    .iter()
                    .filter(|(bit, _)| chord.modifiers & bit != 0)
                    .map(|(_, name)| (*name).to_owned())
                    .collect();
                parts.push(key_name(chord.vk));
                parts.join("+")
            }
        }
    }
}

/// Why a token is refused, in plain words the shell shows after "Can't use X: " (the Mac's list,
/// in Windows' terms).
pub(crate) mod refusal {
    pub const EMPTY: &str = "no key was given";
    /// A letter, digit, arrow... alone: the hook would swallow it everywhere.
    pub const KEY_ALONE: &str =
        "that key on its own would stop working everywhere else; add Ctrl, Alt or Win";
    /// Shift and any key but a function key already does something everywhere: a capital, a
    /// symbol, selecting text, a back-tab. The hook would swallow it.
    pub const SHIFT_TYPES: &str = "Shift with that key already does something everywhere (a capital, a symbol, selecting text); add Ctrl, Alt or Win";
    /// Watching left Ctrl would start a hold on every Ctrl+C in every app.
    pub const LEFT_MODIFIER_ALONE: &str = "a left-hand modifier on its own would start dictation with every shortcut that uses it; use a right-hand one, or add a key";
    pub const MODIFIERS_ONLY: &str = "modifiers together need a key with them; hold one right-hand modifier on its own, or add a key";
    /// Caps Lock reports a switch, not a hold.
    pub const CAPS_LOCK: &str = "Caps Lock switches on and off instead of being held";
    /// The keyboard handles Fn in its firmware.
    pub const FN: &str =
        "the keyboard handles the Fn key itself and never tells Windows; pick another key";
    /// Its place on a Windows keyboard is the Windows key, which most keyboards lack on the right.
    pub const NO_COMMAND: &str = "there is no Command key on Windows; use Win, or pick another key";
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
        if let Some(key) = modifier_key(&token)? {
            return Ok(Binding::Modifier(key));
        }
        if let Some(code) = function_key(&token) {
            return Ok(Binding::Chord(Chord {
                modifiers: 0,
                vk: code,
            }));
        }
        return Err(if token == "fn" {
            refusal::FN
        } else if is_command(&token) {
            refusal::NO_COMMAND
        } else if is_modifier_name(&token) {
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
    let mut bits = 0u8;
    for name in modifiers {
        if *name == "fn" {
            return Err(refusal::FN);
        }
        if is_command(name) {
            return Err(refusal::NO_COMMAND);
        }
        let Some(bit) = chord_modifier(name) else {
            return Err(refusal::UNKNOWN_MODIFIER);
        };
        if bits & bit != 0 {
            return Err(refusal::REPEATED_MODIFIER);
        }
        bits |= bit;
    }
    if is_modifier_name(key) || matches!(modifier_key(key), Ok(Some(_))) || is_command(key) {
        return Err(refusal::MODIFIERS_ONLY);
    }
    let Some(code) = key_code(key) else {
        return Err(refusal::UNKNOWN_KEY);
    };
    // Shift and any key but a function key already does something everywhere; the hook would
    // swallow it (as on the Mac).
    if bits == modifier::SHIFT && function_key(key).is_none() {
        return Err(refusal::SHIFT_TYPES);
    }
    Ok(Binding::Chord(Chord {
        modifiers: bits,
        vk: code,
    }))
}

/// A chord's modifiers in the order a canonical token spells them.
const CHORD_ORDER: [(u8, &str); 4] = [
    (modifier::CTRL, "ctrl"),
    (modifier::ALT, "alt"),
    (modifier::SHIFT, "shift"),
    (modifier::WIN, "win"),
];

/// The canonical name of a key [`key_code`] gave.
fn key_name(code: u32) -> String {
    if let Some((name, _)) = NAMED_KEYS.iter().find(|(_, c)| *c == code) {
        return (*name).to_owned();
    }
    match code {
        f if (vk::F1..vk::F1 + 24).contains(&f) => format!("f{}", f - vk::F1 + 1),
        // VK_A..VK_Z and VK_0..VK_9 are the ASCII codes of the upper-case letter and the digit.
        c => char::from_u32(c).map_or_else(String::new, |c| c.to_ascii_lowercase().to_string()),
    }
}

fn modifier_key(token: &str) -> Result<Option<RightModifier>, &'static str> {
    Ok(Some(match token {
        "right_control" | "right_ctrl" => RightModifier::Control,
        "right_alt" | "right_option" | "right_opt" | "altgr" => RightModifier::Alt,
        "right_shift" => RightModifier::Shift,
        "right_win" | "right_super" => RightModifier::Win,
        "right_command" | "right_cmd" => return Err(refusal::NO_COMMAND),
        _ => return Ok(None),
    }))
}

/// The Mac's Command, by any of its names and sides.
fn is_command(name: &str) -> bool {
    matches!(
        name.strip_prefix("left_").unwrap_or(name),
        "cmd" | "command"
    )
}

/// A modifier named without a side, or by its left-hand key: never watched on its own, and never
/// a chord's key.
fn is_modifier_name(name: &str) -> bool {
    chord_modifier(name.strip_prefix("left_").unwrap_or(name)).is_some()
}

/// Keys named in words, past letters, digits and function keys: (canonical name, virtual key).
/// The names are the Mac's (`delete` is the key left of the backspace-arrow, Backspace on a PC;
/// `forward_delete` is Delete), so a token reads the same on both; the punctuation keys are named
/// by what they type on a US layout, as Windows' `VK_OEM_*` codes are, wherever the user's layout
/// puts other characters on them.
pub(crate) const NAMED_KEYS: &[(&str, u32)] = &[
    ("space", vk::SPACE),
    ("return", vk::RETURN),
    ("tab", vk::TAB),
    ("escape", vk::ESCAPE),
    ("delete", 0x08),
    ("forward_delete", 0x2E),
    ("left", 0x25),
    ("right", 0x27),
    ("down", 0x28),
    ("up", 0x26),
    ("home", 0x24),
    ("end", 0x23),
    ("page_up", 0x21),
    ("page_down", 0x22),
    ("minus", 0xBD),
    ("equal", 0xBB),
    ("left_bracket", 0xDB),
    ("right_bracket", 0xDD),
    ("backslash", 0xDC),
    ("semicolon", 0xBA),
    ("quote", 0xDE),
    ("comma", 0xBC),
    ("period", 0xBE),
    ("slash", 0xBF),
    ("grave", 0xC0),
    // VK_OEM_102: the extra key by left Shift on ISO keyboards (< > on many layouts).
    ("oem_102", 0xE2),
];

/// Other spellings of [`NAMED_KEYS`]: the character a punctuation key types unshifted on a US
/// layout, and the names Windows and other apps use.
const KEY_ALIASES: &[(&str, &str)] = &[
    ("enter", "return"),
    ("esc", "escape"),
    ("backspace", "delete"),
    ("del", "forward_delete"),
    ("pageup", "page_up"),
    ("pagedown", "page_down"),
    ("pgup", "page_up"),
    ("pgdn", "page_down"),
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
];

fn chord_modifier(name: &str) -> Option<u8> {
    Some(match name {
        "ctrl" | "control" => modifier::CTRL,
        "shift" => modifier::SHIFT,
        "alt" | "option" | "opt" => modifier::ALT,
        "win" | "super" | "meta" => modifier::WIN,
        _ => return None,
    })
}

fn function_key(name: &str) -> Option<u32> {
    let n: u32 = name.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&n).then(|| vk::F1 + n - 1)
}

fn key_code(name: &str) -> Option<u32> {
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
    // VK_A..VK_Z and VK_0..VK_9 are the ASCII codes of the upper-case letter and the digit.
    match c {
        'a'..='z' => Some(u32::from(c.to_ascii_uppercase())),
        '0'..='9' => Some(u32::from(c)),
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

    fn unsupported(token: &str) -> bool {
        matches!(Binding::parse(token), Err(PlatformError::Unsupported(_)))
    }

    #[test]
    fn the_default_and_every_offered_key_parse() {
        assert_eq!(
            Binding::parse(DEFAULT_BINDING).unwrap(),
            Binding::Modifier(RightModifier::Control)
        );
        for key in KEYS {
            assert!(
                matches!(Binding::parse(key), Ok(Binding::Modifier(_))),
                "{key}"
            );
        }
    }

    #[test]
    fn modifier_tokens_and_their_mac_spellings() {
        let m = |t| Binding::parse(t).unwrap();
        assert_eq!(m("right_ctrl"), Binding::Modifier(RightModifier::Control));
        assert_eq!(m(" Right_Alt "), Binding::Modifier(RightModifier::Alt));
        assert_eq!(m("right_option"), Binding::Modifier(RightModifier::Alt));
        assert_eq!(m("right_shift"), Binding::Modifier(RightModifier::Shift));
        assert_eq!(m("right_win"), Binding::Modifier(RightModifier::Win));
        assert_eq!(RightModifier::Control.vk(), vk::RCONTROL);
        assert_eq!(RightModifier::Alt.vk(), vk::RMENU);
    }

    fn refused(token: &str) -> &'static str {
        match Binding::parse(token) {
            Err(PlatformError::Unsupported(why)) => why,
            other => panic!("{token:?} parsed as {other:?}"),
        }
    }

    /// Every refusal says why in plain words, as the Mac's do, in Windows' terms.
    #[test]
    fn refusals_say_why_in_plain_words() {
        let cases = [
            ("", refusal::EMPTY),
            ("   ", refusal::EMPTY),
            ("a", refusal::KEY_ALONE),
            ("space", refusal::KEY_ALONE),
            ("left", refusal::KEY_ALONE),
            ("page_down", refusal::KEY_ALONE),
            ("slash", refusal::KEY_ALONE),
            ("left_alt", refusal::LEFT_MODIFIER_ALONE),
            ("left_control", refusal::LEFT_MODIFIER_ALONE),
            ("left_ctrl", refusal::LEFT_MODIFIER_ALONE),
            ("left_shift", refusal::LEFT_MODIFIER_ALONE),
            ("left_win", refusal::LEFT_MODIFIER_ALONE),
            ("alt", refusal::LEFT_MODIFIER_ALONE),
            ("ctrl", refusal::LEFT_MODIFIER_ALONE),
            ("win", refusal::LEFT_MODIFIER_ALONE),
            ("ctrl+shift", refusal::MODIFIERS_ONLY),
            ("ctrl+alt+win", refusal::MODIFIERS_ONLY),
            ("ctrl+right_shift", refusal::MODIFIERS_ONLY),
            ("alt+left_ctrl", refusal::MODIFIERS_ONLY),
            ("caps_lock", refusal::CAPS_LOCK),
            ("capslock", refusal::CAPS_LOCK),
            ("fn", refusal::FN),
            ("fn+f5", refusal::FN),
            ("right_command", refusal::NO_COMMAND),
            ("cmd", refusal::NO_COMMAND),
            ("left_command", refusal::NO_COMMAND),
            ("cmd+space", refusal::NO_COMMAND),
            ("ctrl+nosuchkey", refusal::UNKNOWN_KEY),
            ("nosuchkey", refusal::UNKNOWN_KEY),
            ("f25", refusal::UNKNOWN_KEY),
            ("hyper+a", refusal::UNKNOWN_MODIFIER),
            ("ctrl+ctrl+a", refusal::REPEATED_MODIFIER),
            ("ctrl+control+a", refusal::REPEATED_MODIFIER),
            ("ctrl+", refusal::EMPTY_PART),
            ("+a", refusal::EMPTY_PART),
            ("shift+a", refusal::SHIFT_TYPES),
            ("shift+left", refusal::SHIFT_TYPES),
        ];
        for (token, why) in cases {
            assert_eq!(refused(token), why, "{token:?}");
        }
        // The core's Windows test reads "Fn" in the refusal of the Mac's default.
        assert!(refusal::FN.contains("Fn"));
    }

    /// The Mac's key names parse on Windows to their virtual keys, and each reads back as one
    /// canonical name; the aliases (a punctuation key's character, Windows' own names) too.
    #[test]
    fn the_named_keys_parse_and_read_back() {
        for (name, code) in NAMED_KEYS {
            let token = format!("ctrl+{name}");
            assert_eq!(chord(&token).vk, *code, "{name}");
            assert_eq!(Binding::parse(&token).unwrap().canonical(), token);
        }
        let cases = [
            ("ctrl+left", 0x25),
            ("ctrl+up", 0x26),
            ("alt+page_up", 0x21),
            ("alt+pageup", 0x21),
            ("ctrl+pgdn", 0x22),
            ("ctrl+home", 0x24),
            ("ctrl+end", 0x23),
            ("ctrl+backspace", 0x08),
            ("ctrl+del", 0x2E),
            ("ctrl+-", 0xBD),
            ("ctrl+=", 0xBB),
            ("ctrl+[", 0xDB),
            ("ctrl+]", 0xDD),
            ("ctrl+\\", 0xDC),
            ("ctrl+;", 0xBA),
            ("ctrl+'", 0xDE),
            ("ctrl+,", 0xBC),
            ("ctrl+.", 0xBE),
            ("ctrl+/", 0xBF),
            ("ctrl+`", 0xC0),
            ("ctrl+oem_102", 0xE2),
        ];
        for (token, code) in cases {
            assert_eq!(chord(token).vk, code, "{token}");
        }
        assert_eq!(
            Binding::parse("ctrl+backspace").unwrap().canonical(),
            "ctrl+delete"
        );
        assert_eq!(
            Binding::parse("ctrl+del").unwrap().canonical(),
            "ctrl+forward_delete"
        );
        assert_eq!(Binding::parse("alt+,").unwrap().canonical(), "alt+comma");
        assert_eq!(Binding::parse("shift+f5").unwrap().canonical(), "shift+f5");
    }

    #[test]
    fn keys_windows_cannot_bind_are_refused() {
        assert!(unsupported("fn"));
        assert!(unsupported("right_command"));
        assert!(unsupported("left_control"));
        assert!(unsupported(""));
        assert!(unsupported("space"), "a typing key alone");
        assert!(unsupported("a"));
        assert!(unsupported("ctrl+"));
        assert!(unsupported("+a"));
        assert!(unsupported("ctrl+ctrl+a"));
        assert!(unsupported("hyper+a"));
        assert!(unsupported("ctrl+nosuchkey"));
        assert!(unsupported("f25"));
        assert!(unsupported("f0"));
    }

    #[test]
    fn chords_carry_their_modifiers_and_key() {
        assert_eq!(
            chord("ctrl+shift+space"),
            Chord {
                modifiers: modifier::CTRL | modifier::SHIFT,
                vk: vk::SPACE
            }
        );
        assert_eq!(chord("alt+d").vk, 0x44);
        assert_eq!(chord("win+7").vk, 0x37);
        assert_eq!(
            chord("super+shift+space").modifiers,
            modifier::WIN | modifier::SHIFT
        );
        assert_eq!(
            chord("f13"),
            Chord {
                modifiers: 0,
                vk: 0x7C
            }
        );
        assert_eq!(chord("f24").vk, 0x87);
        assert_eq!(chord("ctrl+f1").vk, vk::F1);
    }

    /// One spelling per binding: modifiers in Windows' order, aliases (and the Mac's names for
    /// the same keys) under the Windows token. Every canonical spelling parses to its binding.
    #[test]
    fn every_binding_has_one_canonical_spelling() {
        let cases = [
            (" Shift + Ctrl + SPACE ", "ctrl+shift+space"),
            ("win+alt+d", "alt+win+d"),
            ("super+shift+control+option+k", "ctrl+alt+shift+win+k"),
            ("ctrl+enter", "ctrl+return"),
            ("alt+esc", "alt+escape"),
            ("F13", "f13"),
            ("ctrl+F24", "ctrl+f24"),
            ("ctrl+7", "ctrl+7"),
            ("right_ctrl", "right_control"),
            ("right_option", "right_alt"),
            ("altgr", "right_alt"),
            ("right_super", "right_win"),
        ];
        for (token, canonical) in cases {
            let parsed = Binding::parse(token).unwrap();
            assert_eq!(parsed.canonical(), canonical, "{token:?}");
            assert_eq!(Binding::parse(canonical), Ok(parsed), "{canonical}");
        }
    }

    #[test]
    fn shift_alone_goes_only_with_a_function_key() {
        for token in [
            "shift+a",
            "shift+space",
            "shift+tab",
            "shift+return",
            "shift+7",
        ] {
            assert_eq!(
                Binding::parse(token),
                Err(PlatformError::Unsupported(refusal::SHIFT_TYPES)),
                "{token}"
            );
        }
        assert!(Binding::parse("shift+f5").is_ok());
        assert!(Binding::parse("ctrl+shift+a").is_ok());
    }

    #[test]
    fn a_chord_matches_exactly_its_modifiers() {
        let c = chord("ctrl+shift+space");
        assert!(c.matches(vk::SPACE, modifier::CTRL | modifier::SHIFT));
        assert!(!c.matches(vk::SPACE, modifier::CTRL));
        assert!(!c.matches(vk::SPACE, modifier::CTRL | modifier::SHIFT | modifier::ALT));
        assert!(!c.matches(vk::TAB, modifier::CTRL | modifier::SHIFT));
    }
}
