//! Joining transcribed chunks back into one transcript.
//!
//! The engine splits long audio into fixed windows that overlap by half a
//! second so no word is cut in half. That overlap makes the recognizer emit the
//! boundary words twice, once at the tail of one chunk and once at the head of
//! the next. Joining with a plain space therefore stutters on long dictations.
//! These helpers drop the duplicated head words before joining.

/// Most trailing words we will treat as an overlap duplicate.
/// The engine overlaps chunks by 0.5s, which is ~3 words at fast speech; the
/// headroom covers the recognizer stretching a word across the boundary. Kept
/// small on purpose: a longer window starts eating genuine repetition.
const MAX_OVERLAP_WORDS: usize = 8;

/// Fewest normalized characters an overlap run must carry to count as a repeat.
/// Without it a bare "a" or "the" landing on both sides of a seam eats a real
/// word. It also rejects punctuation-only runs, which normalize to nothing.
const MIN_OVERLAP_CHARS: usize = 4;

/// Join transcribed chunks, dropping the words the overlap duplicated.
pub fn merge_chunks<S: AsRef<str>>(parts: &[S]) -> String {
    let mut merged = String::new();
    for part in parts {
        append_chunk(&mut merged, part.as_ref());
    }
    merged
}

/// Append one chunk to an accumulating transcript, skipping any leading words
/// that repeat the tail of what is already there.
pub fn append_chunk(merged: &mut String, next: &str) {
    let next = next.trim();
    if next.is_empty() {
        return;
    }

    if merged.is_empty() {
        merged.push_str(next);
        return;
    }

    let tail: Vec<(usize, &str)> = words(merged);
    let head: Vec<(usize, &str)> = words(next);
    let max_k = MAX_OVERLAP_WORDS.min(tail.len()).min(head.len());

    let mut skip = 0usize;
    for k in (1..=max_k).rev() {
        let key = normalize(&tail[tail.len() - k..]);
        if key != normalize(&head[..k]) {
            continue;
        }
        // Too little substance to be a real repeat; see MIN_OVERLAP_CHARS.
        // Shorter runs are subsets of this one, so they cannot qualify either,
        // but the loop is cheap and reads clearer than breaking out here.
        if key.chars().filter(|c| !c.is_whitespace()).count() < MIN_OVERLAP_CHARS {
            continue;
        }
        skip = k;
        break;
    }

    if skip == head.len() {
        // The whole chunk was overlap.
        return;
    }

    merged.push(' ');
    merged.push_str(&next[head[skip].0..]);
}

/// Word slices paired with their byte offset into `s`.
fn words(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;

    for (i, ch) in s.char_indices() {
        if ch.is_whitespace() {
            if let Some(st) = start.take() {
                out.push((st, &s[st..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }

    if let Some(st) = start {
        out.push((st, &s[st..]));
    }

    out
}

/// Comparison key for a run of words: lowercase, all punctuation dropped.
/// Punctuation goes entirely rather than just at the edges because the
/// recognizer does not place the same comma or apostrophe in the same spot on
/// both sides of a seam. Empty when the run is punctuation only.
fn normalize(run: &[(usize, &str)]) -> String {
    let mut key = String::new();
    for (_, word) in run {
        let w: String = word
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect();
        if w.is_empty() {
            continue;
        }
        if !key.is_empty() {
            key.push(' ');
        }
        key.push_str(&w);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_empty() {
        let parts: [&str; 0] = [];
        assert_eq!(merge_chunks(&parts), "");
    }

    #[test]
    fn single_chunk_is_trimmed_passthrough() {
        assert_eq!(merge_chunks(&["  hello world  "]), "hello world");
    }

    #[test]
    fn no_overlap_joins_with_space() {
        assert_eq!(
            merge_chunks(&["the quick brown", "fox jumps over"]),
            "the quick brown fox jumps over"
        );
    }

    #[test]
    fn drops_repeated_boundary_words() {
        assert_eq!(
            merge_chunks(&["I went to the store", "to the store and bought milk"]),
            "I went to the store and bought milk"
        );
    }

    #[test]
    fn drops_single_repeated_word() {
        assert_eq!(merge_chunks(&["hello there", "there friend"]), "hello there friend");
    }

    #[test]
    fn overlap_match_ignores_case_and_punctuation() {
        assert_eq!(
            merge_chunks(&["that is all, folks.", "Folks we are done"]),
            "that is all, folks. we are done"
        );
    }

    #[test]
    fn prefers_the_longest_overlap() {
        // "the mat" (2) must win over "mat" alone (1).
        assert_eq!(
            merge_chunks(&["the cat sat on the mat", "the mat is red"]),
            "the cat sat on the mat is red"
        );
    }

    #[test]
    fn prefers_a_three_word_overlap_over_its_shorter_suffixes() {
        // "beta gamma delta" (3) must win over "gamma delta" (2) and "delta" (1).
        assert_eq!(
            merge_chunks(&["alpha beta gamma delta", "beta gamma delta epsilon"]),
            "alpha beta gamma delta epsilon"
        );
    }

    #[test]
    fn fully_duplicated_chunk_adds_nothing() {
        assert_eq!(merge_chunks(&["one two three", "two three"]), "one two three");
    }

    #[test]
    fn empty_chunks_are_skipped() {
        assert_eq!(merge_chunks(&["alpha", "", "   ", "beta"]), "alpha beta");
        assert_eq!(merge_chunks(&["", "alpha"]), "alpha");
    }

    #[test]
    fn repetition_beyond_the_window_is_left_alone() {
        // The repeated word sits 9 words back, past MAX_OVERLAP_WORDS, so it is
        // real speech and must survive.
        let a = "yes one two three four five six seven eight";
        let b = "yes nine ten";
        assert_eq!(merge_chunks(&[a, b]), format!("{} {}", a, b));
    }

    #[test]
    fn three_chunks_merge_pairwise() {
        // Real words, not single letters: a one-character overlap is below
        // MIN_OVERLAP_CHARS and is deliberately left alone.
        assert_eq!(
            merge_chunks(&["alpha beta gamma", "gamma delta epsilon", "epsilon zeta eta"]),
            "alpha beta gamma delta epsilon zeta eta"
        );
    }

    #[test]
    fn short_common_word_is_not_treated_as_overlap() {
        // "the" straddling the seam is a coincidence, not a repeat.
        assert_eq!(
            merge_chunks(&["we looked at the", "the report was late"]),
            "we looked at the the report was late"
        );
    }

    #[test]
    fn overlap_match_ignores_inner_punctuation() {
        assert_eq!(
            merge_chunks(&["that is the plan, agreed", "Agreed? then let's go"]),
            "that is the plan, agreed then let's go"
        );
    }

    #[test]
    fn drops_a_multi_word_seam_repeat() {
        assert_eq!(
            merge_chunks(&["we should ship it on monday", "on monday morning at nine"]),
            "we should ship it on monday morning at nine"
        );
    }

    #[test]
    fn full_chunk_repeat_collapses() {
        assert_eq!(
            merge_chunks(&["identical phrase here", "identical phrase here"]),
            "identical phrase here"
        );
    }

    #[test]
    fn joins_chunks_with_no_overlap() {
        assert_eq!(
            merge_chunks(&["the quick brown fox", "jumps over"]),
            "the quick brown fox jumps over"
        );
    }

    #[test]
    fn punctuation_only_tokens_do_not_match() {
        assert_eq!(merge_chunks(&["done -", "- next"]), "done - - next");
    }

    #[test]
    fn preserves_inner_spacing_of_the_kept_remainder() {
        // The join itself normalizes to one space; spacing inside what is kept
        // is left as the recognizer produced it.
        assert_eq!(
            merge_chunks(&["hello world", "world  spaced   out"]),
            "hello world spaced   out"
        );
    }
}
