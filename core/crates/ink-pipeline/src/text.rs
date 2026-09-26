//! Stages 5–8 composed: from the engine's words to the text the user gets (before polish).
//!
//! Stage 5 (voice commands) decides whether this runs at all; the chain checks it first. Then,
//! in this order, each for a reason:
//!
//! 1. **Filler removal** (when the mode has it), before styling: removing a leading "Um" leaves
//!    the next word lowercase, and style is the stage that fixes casing.
//! 2. **Style** (stage 6).
//! 3. **Dictionary** (stage 7), after style, so a correction's own casing ("Inkwell") is final.
//! 4. **Snippets** (stage 8), last, so an expansion is inserted exactly as the user wrote it.

use crate::cleanup::remove_disfluencies;
use crate::dictionary::Dictionary;
use crate::modes::Mode;
use crate::snippets::{SnippetStore, SnippetVars};

/// Writes `raw` as `mode` says. Pure.
pub fn write(
    raw: &str,
    mode: &Mode,
    dictionary: &Dictionary,
    snippets: &SnippetStore,
    vars: &SnippetVars,
) -> String {
    let cleaned = if mode.remove_fillers {
        remove_disfluencies(raw)
    } else {
        raw.to_owned()
    };
    let styled = mode.style.format(&cleaned);
    let corrected = dictionary.apply(&styled);
    snippets.expand(&corrected, vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::DictEntry;
    use crate::snippets::Snippet;
    use crate::style::Style;

    fn vars() -> SnippetVars {
        SnippetVars::at(1_774_872_000_000, 0)
    }

    #[test]
    fn fillers_go_before_styling_so_the_capital_is_restored() {
        let mode = Mode::builtin_default();
        let out = write(
            "um so we start here",
            &mode,
            &Dictionary::default(),
            &SnippetStore::default(),
            &vars(),
        );
        assert_eq!(out, "So we start here.");
    }

    #[test]
    fn a_mode_without_filler_removal_keeps_them() {
        let mode = Mode {
            remove_fillers: false,
            style: Style::Relaxed,
            ..Mode::builtin_default()
        };
        let out = write(
            "Um, okay.",
            &mode,
            &Dictionary::default(),
            &SnippetStore::default(),
            &vars(),
        );
        assert_eq!(out, "um, okay");
    }

    #[test]
    fn the_dictionary_casing_and_the_expansion_are_final() {
        let mode = Mode {
            style: Style::Relaxed,
            ..Mode::builtin_default()
        };
        let dictionary = Dictionary {
            entries: vec![DictEntry {
                find: "inkwel".into(),
                replace: "Inkwell".into(),
            }],
        };
        let snippets = SnippetStore {
            snippets: vec![Snippet {
                id: "a".into(),
                trigger: "my address".into(),
                expansion: "1 Example Street.".into(),
                category: String::new(),
                enabled: true,
            }],
        };
        let out = write(
            "Inkwel is at my address.",
            &mode,
            &dictionary,
            &snippets,
            &vars(),
        );
        assert_eq!(out, "Inkwell is at 1 Example Street.");
    }
}
