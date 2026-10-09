//! The keys the user may choose: `hotkey.check`, and the rule the key settings share with it.
//!
//! The user picks any dictation key and any edit key this computer can watch: a right-hand
//! modifier (or Fn on the Mac) held on its own, a function key, or modifiers and one key. The
//! platform's parser is the one judge (ink-platform-mac's `hotkey::check`, ink-platform-win's):
//! a key binds exactly when it says yes. `hotkey.check` asks it about a key the user recorded,
//! before the shell stores it, and answers `hotkey.checked` with the key's one spelling or why
//! not, in words the shell shows. `setting.set` and `consent.allow` store a key only on the same
//! rule, in that spelling, so two spellings of one chord are one key everywhere after.
//!
//! **The named tokens.** [`KEYS`], the right-hand modifiers either OS names, are stored on either
//! OS as before, even where this one cannot watch the key: the platform refuses it when dictation
//! binds it (`dictation.off` with `key_refused`, by name), so a key brought from the other OS is
//! said, never quietly swapped.

use serde_json::Value;

use crate::events::event;
use crate::voice::KEYS;

/// Whether this computer can watch `binding` as a dictation or edit key: its one spelling, or why
/// not, in lower case and without a full stop (the shell shows it after "can't use that:").
#[cfg(target_os = "macos")]
pub fn check(binding: &str) -> Result<String, &'static str> {
    ink_platform_mac::hotkey::check(binding)
}

/// Whether this computer can watch `binding` as a dictation or edit key: its one spelling, or why
/// not, in lower case and without a full stop (the shell shows it after "can't use that:").
#[cfg(windows)]
pub fn check(binding: &str) -> Result<String, &'static str> {
    ink_platform_win::hotkey::check(binding)
}

/// A build with no platform for dictation watches no keys.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn check(_binding: &str) -> Result<String, &'static str> {
    Err("this build watches no keys on this computer")
}

/// What `setting.set` and `consent.allow` store for a key: its one spelling when this computer
/// can watch it, a named token from [`KEYS`] as it is, else the refusal.
pub fn stored_value(value: &str) -> Result<String, &'static str> {
    check(value).or_else(|why| {
        if KEYS.contains(&value) {
            Ok(value.to_owned())
        } else {
            Err(why)
        }
    })
}

/// A stored key as dictation binds and compares it: its one spelling where this computer can
/// watch it, else as stored (the platform then refuses it by name).
pub fn spelling(key: &str) -> String {
    check(key).unwrap_or_else(|_| key.trim().to_owned())
}

/// Refuses a shortcut already assigned to another Inkwell action, before any write.
pub fn unique_setting(store: &dyn ink_core::Store, key: &str, value: &str) -> Result<(), String> {
    if value == "off" || !is_key_setting(key) {
        return Ok(());
    }
    let wanted = spelling(value);
    for (other, default, action) in [
        (
            crate::voice::KEY_SETTING,
            crate::voice::DEFAULT_KEY,
            "dictation",
        ),
        (crate::voice::EDIT_KEY_SETTING, "off", "editing a selection"),
        (
            crate::meeting_keys::KEY_SETTING,
            crate::meeting_keys::DEFAULT_KEY,
            "meetings",
        ),
    ] {
        if other == key {
            continue;
        }
        let chosen = store
            .setting(other)
            .map_err(|e| format!("couldn't check the {action} shortcut: {e}"))?
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| default.into());
        if chosen != "off" && spelling(&chosen) == wanted {
            return Err(format!(
                "that shortcut is used for {action}; choose another shortcut or change that one first"
            ));
        }
    }
    Ok(())
}

/// The settings whose assignments must remain distinct.
pub fn is_key_setting(key: &str) -> bool {
    [
        crate::voice::KEY_SETTING,
        crate::voice::EDIT_KEY_SETTING,
        crate::meeting_keys::KEY_SETTING,
    ]
    .contains(&key)
}

/// `hotkey.checked`: the binding as asked, and the check's answer.
pub fn checked(
    binding: &str,
    answer: Result<String, &'static str>,
    reference: Option<&str>,
) -> Value {
    let (canonical, reason) = match answer {
        Ok(canonical) => (Some(canonical), None),
        Err(why) => (None, Some(why)),
    };
    event(
        "hotkey.checked",
        &[
            ("binding", Some(binding.into())),
            ("ok", Some(canonical.is_some().into())),
            ("canonical", canonical.map(Into::into)),
            ("reason", reason.map(Into::into)),
            ("ref", reference.map(Into::into)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::Store;

    #[test]
    fn meeting_shortcut_conflicts_are_canonical_and_symmetric() {
        let store = ink_store::SqliteStore::open_in_memory().unwrap();
        store.set_setting("dictation.key", "Ctrl+Space").unwrap();
        assert!(unique_setting(&store, "meetings.key", "ctrl+space").is_err());
        store.set_setting("meetings.key", "Shift+Ctrl+R").unwrap();
        assert!(unique_setting(&store, "dictation.key", "ctrl+shift+r").is_err());
        assert!(unique_setting(&store, "dictation.edit_key", "shift+ctrl+r").is_err());
        assert!(unique_setting(&store, "meetings.key", "off").is_ok());
        assert!(unique_setting(&store, "dictation.key", "f13").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn meeting_shortcut_defaults_also_reserve_their_chord() {
        let store = ink_store::SqliteStore::open_in_memory().unwrap();
        assert!(unique_setting(&store, "dictation.key", "ctrl+shift+r").is_err());
        assert!(unique_setting(&store, "meetings.key", crate::voice::DEFAULT_KEY).is_err());
    }

    /// Every named token is storable on every OS, whether or not this one can watch it: as it is,
    /// or in this OS's spelling (the Mac's right_alt is right_option).
    #[test]
    fn the_named_tokens_are_always_storable() {
        for key in KEYS {
            let stored = stored_value(key);
            assert!(stored.is_ok(), "{key}: {stored:?}");
            if check(key).is_err() {
                assert_eq!(stored.as_deref(), Ok(*key), "{key}");
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_the_mac_a_chord_is_stored_and_compared_in_its_one_spelling() {
        assert_eq!(
            stored_value("Shift+Ctrl+Space").as_deref(),
            Ok("ctrl+shift+space")
        );
        assert_eq!(spelling(" shift+ctrl+space "), spelling("ctrl+shift+space"));
        assert!(stored_value("a").is_err());
        // Not watchable here, and not a named token: kept as stored for the platform to refuse.
        assert_eq!(spelling(" left_option "), "left_option");
        // Windows' right-hand Windows key: named, so stored, and refused by name at binding.
        assert_eq!(stored_value("right_win").as_deref(), Ok("right_win"));
        assert!(check("right_win").is_err());
    }

    #[test]
    fn the_answer_carries_either_the_spelling_or_the_reason() {
        let ok = checked("F13", Ok("f13".into()), Some("r"));
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["canonical"], "f13");
        assert!(ok.get("reason").is_none());
        let no = checked("a", Err("why"), None);
        assert_eq!(no["ok"], false);
        assert_eq!(no["reason"], "why");
        assert!(no.get("canonical").is_none());
        assert!(no.get("ref").is_none());
    }
}
