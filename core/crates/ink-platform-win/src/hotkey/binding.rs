//! Hotkey tokens, parsed in pure Rust, and the Windows default.
//!
//! Two shapes, as on the Mac:
//! - **A modifier held on its own:** `"right_control"`, `"right_alt"`, `"right_shift"`,
//!   `"right_win"`. The low-level hook reports left and right modifiers as different virtual keys
//!   (`VK_RCONTROL`, `VK_RMENU`...), so the right-hand key is watched on its own. Left-hand
//!   modifiers alone are not offered: watching left Ctrl would start a hold on every Ctrl+C.
//! - **A chord:** modifiers and one key, `"ctrl+shift+space"`, `"alt+d"`, or a function key alone,
//!   `"f13"`. A typing key needs at least one modifier, or the hook would swallow it.
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
    /// Parses a platform token. Anything Windows cannot bind is [`PlatformError::Unsupported`].
    pub(crate) fn parse(token: &str) -> Result<Self, PlatformError> {
        let token = token.trim().to_ascii_lowercase();
        if token.is_empty() {
            return Err(PlatformError::Unsupported("an empty hotkey"));
        }
        if token == "fn" {
            return Err(PlatformError::Unsupported(
                "the Fn key never reaches Windows; pick another key",
            ));
        }
        if !token.contains('+') {
            if let Some(key) = modifier_key(&token)? {
                return Ok(Self::Modifier(key));
            }
            if let Some(code) = function_key(&token) {
                return Ok(Self::Chord(Chord {
                    modifiers: 0,
                    vk: code,
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
        let mut bits = 0u8;
        for name in modifiers {
            let bit = chord_modifier(name).ok_or(PlatformError::Unsupported(
                "an unknown modifier in a hotkey chord",
            ))?;
            if bits & bit != 0 {
                return Err(PlatformError::Unsupported(
                    "a repeated modifier in a hotkey",
                ));
            }
            bits |= bit;
        }
        let code = key_code(key).ok_or(PlatformError::Unsupported(
            "an unknown key in a hotkey chord",
        ))?;
        // Shift and any key but a function key already does something everywhere; the hook
        // would swallow it (as on the Mac).
        if bits == modifier::SHIFT && function_key(key).is_none() {
            return Err(PlatformError::Unsupported(SHIFT_TYPES));
        }
        Ok(Self::Chord(Chord {
            modifiers: bits,
            vk: code,
        }))
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

/// Why Shift goes only with a function key.
pub(crate) const SHIFT_TYPES: &str = "Shift with that key already does something everywhere (a capital, a symbol, selecting text); add Ctrl, Alt or Win";

/// A chord's modifiers in the order a canonical token spells them.
const CHORD_ORDER: [(u8, &str); 4] = [
    (modifier::CTRL, "ctrl"),
    (modifier::ALT, "alt"),
    (modifier::SHIFT, "shift"),
    (modifier::WIN, "win"),
];

/// The canonical name of a key [`key_code`] gave.
fn key_name(code: u32) -> String {
    match code {
        vk::SPACE => "space".to_owned(),
        vk::RETURN => "return".to_owned(),
        vk::TAB => "tab".to_owned(),
        vk::ESCAPE => "escape".to_owned(),
        f if (vk::F1..vk::F1 + 24).contains(&f) => format!("f{}", f - vk::F1 + 1),
        // VK_A..VK_Z and VK_0..VK_9 are the ASCII codes of the upper-case letter and the digit.
        c => char::from_u32(c).map_or_else(String::new, |c| c.to_ascii_lowercase().to_string()),
    }
}

fn modifier_key(token: &str) -> Result<Option<RightModifier>, PlatformError> {
    Ok(Some(match token {
        "right_control" | "right_ctrl" => RightModifier::Control,
        "right_alt" | "right_option" | "right_opt" | "altgr" => RightModifier::Alt,
        "right_shift" => RightModifier::Shift,
        "right_win" | "right_super" => RightModifier::Win,
        "right_command" | "right_cmd" => {
            return Err(PlatformError::Unsupported(
                "there is no Command key on Windows; pick another key",
            ));
        }
        _ => return Ok(None),
    }))
}

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
    match name {
        "space" => return Some(vk::SPACE),
        "return" | "enter" => return Some(vk::RETURN),
        "tab" => return Some(vk::TAB),
        "escape" | "esc" => return Some(vk::ESCAPE),
        _ => {}
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
                Err(PlatformError::Unsupported(SHIFT_TYPES)),
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
