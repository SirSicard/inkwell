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
        Ok(Self::Chord(Chord {
            modifiers: bits,
            vk: code,
        }))
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

    #[test]
    fn a_chord_matches_exactly_its_modifiers() {
        let c = chord("ctrl+shift+space");
        assert!(c.matches(vk::SPACE, modifier::CTRL | modifier::SHIFT));
        assert!(!c.matches(vk::SPACE, modifier::CTRL));
        assert!(!c.matches(vk::SPACE, modifier::CTRL | modifier::SHIFT | modifier::ALT));
        assert!(!c.matches(vk::TAB, modifier::CTRL | modifier::SHIFT));
    }
}
