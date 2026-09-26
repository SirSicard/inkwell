//! Keeping transcripts out of logs (I5): [`redact`] for log lines, [`Spoken`] for text that has to
//! travel in an event.
//!
//! Logs outlive the delete button: a user who deletes a dictation from the library expects it to
//! be gone, and a log file nobody opens would still hold it. What diagnosing a pipeline needs is a
//! length: it tells whether a stage dropped the text, doubled it, or returned nothing.
//!
//! **Changed from 0.2:** 0.2's `redact` wrote the text itself in debug builds. Here it never does,
//! in any build: tests run debug builds, and a debug build is also what a contributor runs on
//! their own dictations.

use std::fmt;

/// Text for a log line: its length in characters, never the text.
pub fn redact(text: &str) -> String {
    format!("{} chars", text.chars().count())
}

/// Text the user said, on its way to the shell in an event.
///
/// It prints as its length in `{:?}` and has no `Display`, so logging an event, or anything that
/// contains one, cannot write the words (I5). Read the words with [`as_str`](Self::as_str).
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Spoken(String);

impl Spoken {
    /// Wraps `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The words.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The words, owned.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for Spoken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Spoken({})", redact(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from 0.2's `lib.rs` (3 tests); the release-only condition is gone.

    #[test]
    fn release_logs_a_length_and_never_the_words() {
        let secret = "my bank password is hunter2";
        let out = redact(secret);
        assert!(!out.contains("hunter2"), "{out}");
        assert!(!out.contains("password"), "{out}");
        assert_eq!(out, "27 chars");
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert_eq!(redact("héllo wörld"), "11 chars");
    }

    #[test]
    fn handles_empty() {
        assert_eq!(redact(""), "0 chars");
    }

    // New in 1.0.

    #[test]
    fn spoken_debug_prints_a_length_only() {
        let s = Spoken::new("my bank password is hunter2");
        let printed = format!("{s:?} {:?}", Some(&s));
        assert!(!printed.contains("hunter2"), "{printed}");
        assert_eq!(printed, "Spoken(27 chars) Some(Spoken(27 chars))");
        assert_eq!(s.as_str(), "my bank password is hunter2");
    }
}
