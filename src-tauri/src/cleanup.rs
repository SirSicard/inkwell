//! Disfluency removal: the fillers and stutters that speech carries and writing
//! does not.
//!
//! This is the most-cited complaint in the whole dictation category ("manual
//! transcript cleanup"), and it was visible in the first nine real transcripts
//! this app produced: "Hello. Um Can you actually hear me?" went to the
//! clipboard verbatim. Nothing in the pipeline touched it, because `style`
//! handles casing and punctuation only and polish is off by default and needs an
//! API key.
//!
//! Deliberately conservative. Every word here is one that carries no meaning in
//! writing. "like", "you know", "I mean", "actually", "basically" and "right"
//! are NOT included even though competitors strip some of them: each has real
//! uses ("a tool like this", "you know the answer"), and a dictation tool that
//! silently deletes meaningful words is worse than one that leaves an "um" in.
//! The rule is that removing a word here must never change what a sentence says.

/// Interjections with no written meaning. Matched whole-word, case-insensitive.
const FILLERS: &[&str] = &[
    "um", "umm", "ummm", "uh", "uhh", "uhhh", "erm", "hmm", "hmmm", "mmm", "mhm",
    "uhm", "eh",
];

/// Is this token a filler, ignoring case and any trailing punctuation?
fn is_filler(token: &str) -> bool {
    let bare = token
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    !bare.is_empty() && FILLERS.contains(&bare.as_str())
}

/// Words whose immediate repetition is always a stutter rather than grammar.
///
/// A curated list, not "any repeated word", because English doubles words
/// legitimately: "he had had enough", "I know that that is true". Collapsing
/// those would change the sentence, which is the one thing this module promises
/// not to do. "had" and "that" are therefore deliberately absent.
const STUTTER_SAFE: &[&str] = &[
    "i", "the", "a", "an", "we", "you", "they", "to", "of", "and", "but", "so",
    "in", "on", "at", "is", "are", "was", "were", "my", "your", "this", "it",
];

/// The comparable form of a token, for detecting a repeat.
fn normalized(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Remove fillers and immediate stutters.
///
/// Two passes over the tokens, in this order:
///
/// 1. Fillers are dropped entirely.
/// 2. A word immediately repeated ("the the cat") collapses to one.
///
/// Order matters: "the um the cat" only collapses once the filler between the
/// repeats is gone, which is exactly the disfluency people produce while
/// thinking. Doing it the other way round leaves "the the cat".
pub fn remove_disfluencies(text: &str) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }

    let kept: Vec<&str> = text.split_whitespace().filter(|t| !is_filler(t)).collect();

    let mut out: Vec<&str> = Vec::with_capacity(kept.len());
    for token in kept {
        let repeat = out
            .last()
            .map(|prev| {
                let a = normalized(prev);
                let b = normalized(token);
                a == b && STUTTER_SAFE.contains(&a.as_str())
            })
            .unwrap_or(false);

        if repeat {
            // Keep whichever copy carries the punctuation, so "the the." keeps
            // its full stop.
            if token.len() > out.last().map(|t| t.len()).unwrap_or(0) {
                out.pop();
                out.push(token);
            }
            continue;
        }
        out.push(token);
    }

    let joined = out.join(" ");

    // A filler at the head of a sentence leaves the next word lowercase where
    // the recogniser had capitalised the filler instead. Restore the capital.
    restore_leading_capital(text, &joined)
}

fn restore_leading_capital(original: &str, cleaned: &str) -> String {
    let original_started_upper = original
        .trim_start()
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false);

    if !original_started_upper {
        return cleaned.to_string();
    }

    let mut chars = cleaned.chars();
    match chars.next() {
        Some(first) if first.is_lowercase() => {
            first.to_uppercase().collect::<String>() + chars.as_str()
        }
        _ => cleaned.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::remove_disfluencies;

    #[test]
    fn removes_the_filler_from_a_real_transcript() {
        // Verbatim from the first nine transcripts this app produced.
        assert_eq!(
            remove_disfluencies("Hello. Um Can you actually hear me?"),
            "Hello. Can you actually hear me?"
        );
    }

    #[test]
    fn removes_fillers_case_insensitively_and_with_punctuation() {
        assert_eq!(remove_disfluencies("So, um, we should go"), "So, we should go");
        assert_eq!(remove_disfluencies("Uh well yes"), "Well yes");
    }

    #[test]
    fn collapses_an_immediate_stutter() {
        assert_eq!(remove_disfluencies("I I think so"), "I think so");
        assert_eq!(remove_disfluencies("the the cat"), "the cat");
    }

    #[test]
    fn collapses_a_repeat_that_a_filler_was_hiding() {
        // The reason fillers are removed before repeats are collapsed.
        assert_eq!(remove_disfluencies("the um the cat"), "the cat");
    }

    #[test]
    fn keeps_the_copy_carrying_punctuation() {
        assert_eq!(remove_disfluencies("that was the the."), "that was the.");
    }

    #[test]
    fn emphatic_repetition_survives() {
        // "stop" is not on the curated list, so this stays as spoken. Emphatic
        // repetition is meaningful and a stutter list should not eat it.
        assert_eq!(remove_disfluencies("stop stop"), "stop stop");
    }

    #[test]
    fn leaves_meaningful_words_alone() {
        // Every one of these is stripped by at least one competitor. All of them
        // change the sentence when removed.
        let meaningful = [
            "a tool like this",
            "you know the answer",
            "I mean it sincerely",
            "actually correct",
            "right hand side",
        ];
        for s in meaningful {
            assert_eq!(remove_disfluencies(s), s, "must not alter {s:?}");
        }
    }

    #[test]
    fn does_not_collapse_repeats_that_are_valid_english() {
        // The reason repeats are collapsed from a curated list rather than
        // wholesale: both of these are grammatical, and losing a word changes
        // what the sentence says.
        assert_eq!(remove_disfluencies("he had had enough"), "he had had enough");
        assert_eq!(remove_disfluencies("I know that that is true"), "I know that that is true");
    }

    #[test]
    fn restores_the_capital_when_a_leading_filler_goes() {
        assert_eq!(remove_disfluencies("Um so we start here"), "So we start here");
    }

    #[test]
    fn leaves_clean_text_untouched() {
        let clean = "The quick brown fox jumps over the lazy dog.";
        assert_eq!(remove_disfluencies(clean), clean);
    }

    #[test]
    fn handles_empty_and_whitespace() {
        assert_eq!(remove_disfluencies(""), "");
        assert_eq!(remove_disfluencies("   "), "   ");
    }

    #[test]
    fn a_transcript_of_only_fillers_becomes_empty() {
        assert_eq!(remove_disfluencies("um uh hmm"), "");
    }
}
