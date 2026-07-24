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
        // An empty key means the run was punctuation only: never a match.
        if !key.is_empty() && key == normalize(&head[..k]) {
            skip = k;
            break;
        }
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

/// Comparison key for a run of words: lowercase, outer punctuation dropped.
/// Empty when the run is punctuation only; the caller rejects that as a match.
fn normalize(run: &[(usize, &str)]) -> String {
    let mut key = String::new();
    for (_, word) in run {
        let w = word.trim_matches(|c: char| c.is_ascii_punctuation());
        if w.is_empty() {
            continue;
        }
        if !key.is_empty() {
            key.push(' ');
        }
        key.push_str(&w.to_lowercase());
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
        assert_eq!(
            merge_chunks(&["a b c", "c d e", "e f g"]),
            "a b c d e f g"
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
