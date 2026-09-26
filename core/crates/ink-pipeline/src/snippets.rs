//! Stage 8: snippets, short spoken triggers that expand to longer text. Ported from Inkwell 0.2's
//! `snippets.rs`.
//!
//! Changes in the port:
//! - **The time is an argument.** `{date}` and `{time}` come from [`SnippetVars`], built from the
//!   platform clock and the user's UTC offset, so expansion is pure and the tests are exact.
//! - **`{clipboard}` is left as typed when the clipboard is not known.** 0.2 replaced it with
//!   nothing when the clipboard could not be read, which pasted a silently shortened text. The
//!   core has no clipboard-reading seam yet, so the dictation chain passes `None` and the user sees
//!   the placeholder.
//! - **Matching is on the text as spoken, case-insensitively for ASCII letters.** 0.2 matched on a
//!   lowercased copy and cut the original at its offsets, which breaks after a character whose
//!   lowercase has a different UTF-8 length (see [`dictionary`](crate::dictionary)).
//! - **The longest trigger wins where two overlap** ("sig" and "signature"). 0.2 took whichever
//!   the matcher reported first, so "signature" was found as "sig", rejected as part of a longer
//!   word, and never expanded at all.

use aho_corasick::{AhoCorasick, MatchKind};

use crate::civil::CivilTime;
use crate::dictionary::is_whole_word;

/// One snippet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    /// A stable id.
    pub id: String,
    /// What the user says, matched as a whole word or phrase, case-insensitively.
    pub trigger: String,
    /// What it becomes. `{date}`, `{time}` and `{clipboard}` are replaced.
    pub expansion: String,
    /// A grouping for the settings screen.
    pub category: String,
    /// Disabled snippets never expand.
    pub enabled: bool,
}

/// The user's snippets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnippetStore {
    /// In the order the user sees them.
    pub snippets: Vec<Snippet>,
}

/// The values of the variables an expansion may contain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnippetVars {
    /// `{date}`: the local date, `YYYY-MM-DD`.
    pub date: String,
    /// `{time}`: the local time, `HH:MM`.
    pub time: String,
    /// `{clipboard}`: the clipboard's text, or `None` when it is not known (the placeholder then
    /// stays as typed).
    pub clipboard: Option<String>,
}

impl SnippetVars {
    /// The date and time at `unix_ms` on the user's clock (`utc_offset_minutes`, `120` for UTC+2),
    /// with no clipboard.
    pub fn at(unix_ms: i64, utc_offset_minutes: i32) -> Self {
        let now = CivilTime::at(unix_ms, utc_offset_minutes);
        Self {
            date: now.date(),
            time: now.time(),
            clipboard: None,
        }
    }

    fn interpolate(&self, expansion: &str) -> String {
        let text = expansion
            .replace("{date}", &self.date)
            .replace("{time}", &self.time);
        match &self.clipboard {
            Some(clip) => text.replace("{clipboard}", clip),
            None => text,
        }
    }
}

impl SnippetStore {
    /// `text` with every enabled trigger that stands as a whole word or phrase replaced by its
    /// expansion. Pure.
    pub fn expand(&self, text: &str, vars: &SnippetVars) -> String {
        let active: Vec<&Snippet> = self
            .snippets
            .iter()
            .filter(|s| s.enabled && !s.trigger.is_empty())
            .collect();
        if active.is_empty() {
            return text.to_owned();
        }
        let matcher = match AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .match_kind(MatchKind::LeftmostLongest)
            .build(active.iter().map(|s| s.trigger.as_str()))
        {
            Ok(m) => m,
            Err(e) => {
                // Only a pathological trigger set fails to build (the matcher's size limits). The
                // text goes out unexpanded, which the user can see; the log says why.
                log::warn!("snippets not expanded: the trigger matcher failed to build: {e}");
                return text.to_owned();
            }
        };
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for m in matcher.find_iter(text) {
            if is_whole_word(text, m.start(), m.end()) {
                out.push_str(&text[last..m.start()]);
                out.push_str(&vars.interpolate(&active[m.pattern().as_usize()].expansion));
                last = m.end();
            }
        }
        out.push_str(&text[last..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(trigger: &str, expansion: &str) -> SnippetStore {
        SnippetStore {
            snippets: vec![Snippet {
                id: "t".into(),
                trigger: trigger.into(),
                expansion: expansion.into(),
                category: String::new(),
                enabled: true,
            }],
        }
    }

    /// 2026-03-30 23:30:00 UTC.
    const LATE_UTC: i64 = 1_774_913_400_000;

    // Ported from 0.2's `snippets.rs` (2 tests). 0.2 sampled the machine's clock either side of
    // the call; with the time passed in, the local date and time are exact.

    #[test]
    fn date_variable_uses_local_date() {
        // At UTC+2 it is already the next day.
        let vars = SnippetVars::at(LATE_UTC, 120);
        assert_eq!(
            store("today", "{date}").expand("today", &vars),
            "2026-03-31"
        );
    }

    #[test]
    fn time_variable_uses_local_clock_not_utc() {
        // The hour is what differed between local time and UTC before 0.2's fix.
        let vars = SnippetVars::at(LATE_UTC, 120);
        assert_eq!(store("now", "{time}").expand("now", &vars), "01:30");
        let utc = SnippetVars::at(LATE_UTC, 0);
        assert_eq!(store("now", "{time}").expand("now", &utc), "23:30");
    }

    // New in 1.0.

    #[test]
    fn an_unknown_clipboard_leaves_the_placeholder_visible() {
        let s = store("paste", "[{clipboard}]");
        let mut vars = SnippetVars::at(LATE_UTC, 0);
        assert_eq!(s.expand("paste", &vars), "[{clipboard}]");
        vars.clipboard = Some("copied".into());
        assert_eq!(s.expand("paste", &vars), "[copied]");
    }

    #[test]
    fn the_longest_overlapping_trigger_wins() {
        let s = SnippetStore {
            snippets: vec![
                Snippet {
                    trigger: "sig".into(),
                    expansion: "S".into(),
                    ..store("", "").snippets.remove(0)
                },
                Snippet {
                    trigger: "signature".into(),
                    expansion: "LONG".into(),
                    ..store("", "").snippets.remove(0)
                },
            ],
        };
        let vars = SnippetVars::at(LATE_UTC, 0);
        assert_eq!(s.expand("my signature and sig", &vars), "my LONG and S");
    }

    #[test]
    fn a_character_that_grows_when_lowercased_does_not_break_matching() {
        let vars = SnippetVars::at(LATE_UTC, 0);
        assert_eq!(store("sig", "S").expand("İİİ sig", &vars), "İİİ S");
    }
}
