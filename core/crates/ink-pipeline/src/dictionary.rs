//! Stage 7: the user's dictionary: whole-word, case-insensitive corrections, and the same words as
//! the engine's context so they can be decoded right in the first place. Ported from Inkwell 0.2's
//! `dictionary.rs`.
//!
//! **Fixed in the port:** 0.2 lowercased the text to search it and then cut the original at the
//! lowercase copy's byte offsets. Lowercasing can change a character's length in UTF-8 (`İ` has
//! two bytes, its lowercase three), so after such a character the wrong span was replaced, or the
//! cut fell inside a character and panicked. Matching here compares ASCII letters case-insensitively and every other
//! byte exactly, so a match always starts and ends on a character boundary of the original.
//! Dictation is English only (a locked decision), so ASCII case-folding is the case that matters.

use std::fmt;

use ink_core::{Store, StoreError};
use serde_json::{Value, json};

/// The settings key the dictionary is stored under.
pub const SETTING_KEY: &str = "dictionary";

/// The most dictionary words handed to the engine as context. A long bias list dilutes itself.
pub const MAX_CONTEXT_WORDS: usize = 100;

/// One correction: `find`, as a whole word in any case, becomes `replace`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DictEntry {
    /// What the engine writes (matched case-insensitively, as a whole word).
    pub find: String,
    /// What the user means, written exactly.
    pub replace: String,
}

/// The user's dictionary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dictionary {
    /// The corrections, applied in order.
    pub entries: Vec<DictEntry>,
}

/// Why a stored setting could not be read or written. Names the setting, never its contents.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettingsError {
    /// The store failed.
    Store(StoreError),
    /// The stored value is not in the expected shape. It is reported, never replaced by an empty
    /// default: 0.2 silently loaded an empty dictionary from a damaged file, and the next save
    /// then erased the user's words for good.
    Malformed {
        /// The settings key.
        key: &'static str,
        /// What is wrong with it.
        what: &'static str,
    },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => write!(f, "settings store: {e}"),
            Self::Malformed { key, what } => write!(f, "setting {key} is malformed: {what}"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl From<StoreError> for SettingsError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl Dictionary {
    /// Applies every correction, in order. Pure.
    pub fn apply(&self, text: &str) -> String {
        self.entries
            .iter()
            .filter(|e| !e.find.is_empty())
            .fold(text.to_owned(), |acc, e| {
                replace_words(&acc, &e.find, &e.replace)
            })
    }

    /// The corrections' targets as the engine's context, one per line, or `None` when there is
    /// nothing to favour.
    ///
    /// The `replace` side is what the user means ("Inkwell"); given to the engine as context, the
    /// word can win while decoding instead of being repaired afterwards. The corrections still run
    /// afterwards, as a backstop and for engines that take no context. Blank and letterless
    /// targets are skipped, duplicates (ignoring case) kept once, and at most
    /// [`MAX_CONTEXT_WORDS`] are given.
    pub fn hotwords(&self) -> Option<String> {
        let mut seen = std::collections::HashSet::new();
        let words: Vec<&str> = self
            .entries
            .iter()
            .map(|e| e.replace.trim())
            .filter(|w| !w.is_empty() && w.chars().any(char::is_alphabetic))
            .filter(|w| seen.insert(w.to_lowercase()))
            .take(MAX_CONTEXT_WORDS)
            .collect();
        (!words.is_empty()).then(|| words.join("\n"))
    }

    /// The stored form: 0.2's `dictionary.json` shape, `{"entries":[{"find":…,"replace":…}]}`, so
    /// the importer can read an old file with [`from_json`](Self::from_json).
    pub fn to_json(&self) -> String {
        let entries: Vec<Value> = self
            .entries
            .iter()
            .map(|e| json!({ "find": e.find, "replace": e.replace }))
            .collect();
        json!({ "entries": entries }).to_string()
    }

    /// Parses the stored form. Anything but that shape is [`SettingsError::Malformed`].
    pub fn from_json(text: &str) -> Result<Self, SettingsError> {
        let malformed = |what| SettingsError::Malformed {
            key: SETTING_KEY,
            what,
        };
        let value: Value = serde_json::from_str(text).map_err(|_| malformed("not JSON"))?;
        let entries = value
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("no `entries` array"))?;
        let entries = entries
            .iter()
            .map(|e| {
                let field = |name| e.get(name).and_then(Value::as_str).map(str::to_owned);
                match (field("find"), field("replace")) {
                    (Some(find), Some(replace)) => Ok(DictEntry { find, replace }),
                    _ => Err(malformed("an entry without `find` and `replace` strings")),
                }
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { entries })
    }

    /// **Worker.** Saves the dictionary in the store's settings.
    pub fn save(&self, store: &dyn Store) -> Result<(), SettingsError> {
        Ok(store.set_setting(SETTING_KEY, &self.to_json())?)
    }

    /// **Worker.** Loads the dictionary from the store's settings: empty when none was ever saved,
    /// an error when the stored value is damaged.
    pub fn load(store: &dyn Store) -> Result<Self, SettingsError> {
        match store.setting(SETTING_KEY)? {
            None => Ok(Self::default()),
            Some(text) => Self::from_json(&text),
        }
    }
}

/// `text` with every whole-word, ASCII-case-insensitive occurrence of `find` replaced, scanning
/// left to right without overlaps (as `str::match_indices` does).
fn replace_words(text: &str, find: &str, replace: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for range in ascii_matches(text, find) {
        if is_whole_word(text, range.start, range.end) {
            out.push_str(&text[last..range.start]);
            out.push_str(replace);
            last = range.end;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// Non-overlapping matches of `needle` in `haystack`, left to right, ASCII letters compared
/// without case and every other byte exactly.
///
/// Every range lies on character boundaries: a non-ASCII byte only matches itself, and `needle`
/// is whole characters, so a match cannot begin or end inside a character.
pub(crate) fn ascii_matches<'a>(
    haystack: &'a str,
    needle: &'a str,
) -> impl Iterator<Item = std::ops::Range<usize>> + 'a {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    let mut i = 0;
    std::iter::from_fn(move || {
        if n.is_empty() {
            return None;
        }
        while i + n.len() <= h.len() {
            if h[i..i + n.len()].eq_ignore_ascii_case(n) {
                let found = i..i + n.len();
                i += n.len();
                return Some(found);
            }
            i += 1;
        }
        None
    })
}

/// Whether `text[start..end]` is a whole word: no letter or digit touches it on either side.
pub(crate) fn is_whole_word(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(entries: &[(&str, &str)]) -> Dictionary {
        Dictionary {
            entries: entries
                .iter()
                .map(|&(find, replace)| DictEntry {
                    find: find.into(),
                    replace: replace.into(),
                })
                .collect(),
        }
    }

    // 0.2's `dictionary.rs` had no inline tests; these cover what the port changed.

    #[test]
    fn a_character_that_grows_when_lowercased_does_not_break_matching() {
        // 'İ' lowercases to three bytes from two; 0.2 cut the text at the wrong offset here.
        let d = dict(&[("ink", "Ink")]);
        assert_eq!(d.apply("İstanbul ink"), "İstanbul Ink");
    }

    #[test]
    fn a_non_ascii_letter_is_part_of_the_word() {
        // 0.2 checked only ASCII neighbours, so "caf" matched inside "café".
        let d = dict(&[("caf", "CAF")]);
        assert_eq!(d.apply("café caf"), "café CAF");
    }

    #[test]
    fn hotwords_are_the_targets_deduplicated_and_capped() {
        let d = dict(&[
            ("inkwel", "Inkwell"),
            ("ink well", "inkwell"),
            ("x", "  "),
            ("one", "123"),
        ]);
        assert_eq!(d.hotwords().as_deref(), Some("Inkwell"));
        assert_eq!(Dictionary::default().hotwords(), None);
        let many = Dictionary {
            entries: (0..150)
                .map(|i| DictEntry {
                    find: format!("w{i}"),
                    replace: format!("Word{i}"),
                })
                .collect(),
        };
        assert_eq!(
            many.hotwords().map(|h| h.lines().count()),
            Some(MAX_CONTEXT_WORDS)
        );
    }

    #[test]
    fn a_damaged_setting_is_an_error_not_an_empty_dictionary() {
        for bad in [
            "",
            "{",
            "[]",
            r#"{"entries": 3}"#,
            r#"{"entries":[{"find":"a"}]}"#,
        ] {
            assert!(
                matches!(
                    Dictionary::from_json(bad),
                    Err(SettingsError::Malformed {
                        key: SETTING_KEY,
                        ..
                    })
                ),
                "{bad:?}"
            );
        }
        // 0.2's own file shape reads back.
        let old = r#"{"entries":[{"find":"inkwel","replace":"Inkwell"}]}"#;
        assert_eq!(
            Dictionary::from_json(old),
            Ok(dict(&[("inkwel", "Inkwell")]))
        );
    }
}
