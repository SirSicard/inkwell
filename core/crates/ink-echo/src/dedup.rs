//! Duplicate-line suppression: a far-end line that reappears in the "you" transcript as echo is
//! removed.
//!
//! The acoustic stages (AEC along a found path, the echo-only word gate) catch almost all echo,
//! but not all of it: AEC3 takes seconds to converge, a path can change mid-meeting, and double
//! talk lets residue through. What slips past shows up as a "you" line that repeats, at the same
//! moment, what the far end said. This is the transcript-level backstop.
//!
//! # The rule
//!
//! Words are compared after normalising: lower case, with punctuation stripped from both ends of
//! each word (apostrophes inside a word stay). A "you" line is **echo**, and is removed, when all
//! three hold:
//!
//! 1. It has at least **3** words. Shorter lines ("yes", "okay thanks") are too common to call
//!    echo on words alone; the acoustic gate judges those.
//! 2. At least **80 %** of its words, in order, match words of the far-end lines around it: those
//!    that start no later than 500 ms after it ends and end no earlier than 1 s before it starts.
//!    (Echo arrives within 500 ms of the far end; the rest is ASR timestamp slop.) The match is
//!    the longest common subsequence of the two word sequences.
//! 3. It starts no more than 500 ms before the earliest far-end line that supplied a matched
//!    word. Echo cannot come before the far end said it: a user who says a line first, which the
//!    far end then repeats, keeps it.
//!
//! A user who repeats the far end's words later (reading back a number) keeps them: the time
//! window excludes it, or the line holds enough words of their own to fall under 80 %.

use ink_core::TimedText;

/// The rule's thresholds. The defaults are the rule in the module docs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DedupConfig {
    /// A "you" line needs at least this many words to be judged.
    pub min_words: usize,
    /// The share of its words that must match, in order.
    pub min_share: f64,
    /// How long before a far-end line an echo line may appear to start, ms (ASR slop).
    pub lead_ms: u64,
    /// How long after a far-end line ends an echo line may start, ms (echo delay plus slop).
    pub lag_ms: u64,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            min_words: 3,
            min_share: 0.8,
            lead_ms: 500,
            lag_ms: 1_000,
        }
    }
}

/// A "you" line found to be echo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Duplicate {
    /// Its index in the "you" lines.
    pub you: usize,
    /// Its words after normalising.
    pub words: usize,
    /// How many of them matched the far end, in order.
    pub matched: usize,
}

/// Lower case, punctuation stripped from both ends of each word, empty words dropped.
pub fn normalise(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Longest common subsequence of `a` and `b`, with, for each matched word of `b`, its index.
fn lcs(a: &[String], b: &[String]) -> (usize, Vec<usize>) {
    let (n, m) = (a.len(), b.len());
    let mut t = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in 1..=n {
        for j in 1..=m {
            t[at(i, j)] = if a[i - 1] == b[j - 1] {
                t[at(i - 1, j - 1)] + 1
            } else {
                t[at(i - 1, j)].max(t[at(i, j - 1)])
            };
        }
    }
    let mut matched = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            matched.push(j - 1);
            i -= 1;
            j -= 1;
        } else if t[at(i - 1, j)] >= t[at(i, j - 1)] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    (t[at(n, m)] as usize, matched)
}

/// The "you" lines that are echo of the far end's, by the rule in the module docs. Both inputs
/// are on one timeline (the record's), in any order.
pub fn echo_duplicates(
    you: &[TimedText],
    them: &[TimedText],
    config: &DedupConfig,
) -> Vec<Duplicate> {
    let far: Vec<(u64, u64, Vec<String>)> = them
        .iter()
        .map(|l| (l.start_ms, l.end_ms, normalise(&l.text)))
        .collect();
    let mut out = Vec::new();
    for (idx, line) in you.iter().enumerate() {
        let words = normalise(&line.text);
        if words.len() < config.min_words {
            continue;
        }
        // The far-end lines around it, in time order.
        let mut near: Vec<&(u64, u64, Vec<String>)> = far
            .iter()
            .filter(|(s, e, _)| {
                *s <= line.end_ms + config.lead_ms && e + config.lag_ms >= line.start_ms
            })
            .collect();
        near.sort_by_key(|(s, _, _)| *s);
        let mut pool: Vec<String> = Vec::new();
        let mut owner: Vec<u64> = Vec::new();
        for (s, _, w) in &near {
            pool.extend(w.iter().cloned());
            owner.extend(std::iter::repeat_n(*s, w.len()));
        }
        let (matched, at) = lcs(&words, &pool);
        let needed = (config.min_share * words.len() as f64).ceil() as usize;
        if matched < needed || matched == 0 {
            continue;
        }
        let earliest = at.iter().map(|&j| owner[j]).min().unwrap_or(u64::MAX);
        if line.start_ms + config.lead_ms < earliest {
            continue;
        }
        out.push(Duplicate {
            you: idx,
            words: words.len(),
            matched,
        });
    }
    out
}

/// `you` without the lines that are echo of `them`; returns the kept lines and how many went.
pub fn remove_echo_duplicates(
    you: Vec<TimedText>,
    them: &[TimedText],
    config: &DedupConfig,
) -> (Vec<TimedText>, usize) {
    let dups = echo_duplicates(&you, them, config);
    let removed = dups.len();
    let mut drop = vec![false; you.len()];
    for d in &dups {
        drop[d.you] = true;
    }
    let kept = you
        .into_iter()
        .zip(drop)
        .filter(|(_, d)| !*d)
        .map(|(l, _)| l)
        .collect();
    (kept, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start_ms: u64, end_ms: u64, text: &str) -> TimedText {
        TimedText {
            start_ms,
            end_ms,
            text: text.to_owned(),
        }
    }

    fn echo(you: &[TimedText], them: &[TimedText]) -> Vec<usize> {
        echo_duplicates(you, them, &DedupConfig::default())
            .iter()
            .map(|d| d.you)
            .collect()
    }

    #[test]
    fn words_are_compared_without_case_or_edge_punctuation() {
        assert_eq!(
            normalise("  The DEADLINE, isn't it... Friday?! — 42 "),
            ["the", "deadline", "isn't", "it", "friday", "42"]
        );
    }

    #[test]
    fn a_far_line_echoed_at_the_same_time_is_removed() {
        let them = [line(
            10_000,
            12_500,
            "The quarterly numbers go out on Friday.",
        )];
        let you = [line(
            10_080,
            12_600,
            "the quarterly numbers go out on friday",
        )];
        assert_eq!(echo(&you, &them), [0]);
    }

    #[test]
    fn a_partly_heard_echo_still_counts() {
        // The ASR caught four of the far end's six words, one of them wrong: 3 of 4 is under
        // 80 %, all 4 of 4 is not.
        let them = [line(5_000, 7_000, "please send the draft by noon")];
        assert_eq!(echo(&[line(5_300, 7_100, "send the draft by")], &them), [0]);
        assert!(echo(&[line(5_300, 7_100, "send a draft by")], &them).is_empty());
    }

    #[test]
    fn an_echo_spanning_two_far_lines_is_removed() {
        let them = [
            line(1_000, 2_000, "shall we start"),
            line(2_200, 4_000, "yes the agenda is short today"),
        ];
        let you = [line(
            1_100,
            4_100,
            "shall we start yes the agenda is short today",
        )];
        assert_eq!(echo(&you, &them), [0]);
    }

    #[test]
    fn the_users_own_words_at_the_same_time_are_kept() {
        let them = [line(3_000, 6_000, "we could move the launch to march")];
        assert!(echo(&[line(3_500, 5_500, "I think that works for us")], &them).is_empty());
        // Mostly their own words around a few echoed ones: under 80 %.
        let mixed = [line(
            3_000,
            6_500,
            "honestly I would move the launch to april instead",
        )];
        assert!(echo(&mixed, &them).is_empty());
    }

    #[test]
    fn short_lines_are_left_to_the_acoustic_gate() {
        let them = [line(8_000, 8_600, "yeah okay")];
        assert!(echo(&[line(8_050, 8_650, "yeah okay")], &them).is_empty());
    }

    #[test]
    fn a_later_read_back_is_kept() {
        let them = [line(1_000, 3_000, "the code is four seven one nine")];
        // Two seconds after the far end finished.
        assert!(echo(&[line(5_000, 6_500, "four seven one nine")], &them).is_empty());
    }

    #[test]
    fn a_line_the_user_said_first_is_kept_when_the_far_end_repeats_it() {
        let them = [line(4_200, 6_000, "ship it on thursday")];
        let you = [line(2_000, 4_000, "ship it on thursday")];
        assert!(echo(&you, &them).is_empty());
    }

    #[test]
    fn removal_keeps_the_other_lines_in_order() {
        let them = [line(
            10_000,
            12_500,
            "the quarterly numbers go out on friday",
        )];
        let you = vec![
            line(1_000, 2_000, "good morning everyone here"),
            line(10_080, 12_600, "the quarterly numbers go out on friday"),
            line(13_000, 14_000, "sounds good to me"),
        ];
        let (kept, removed) = remove_echo_duplicates(you, &them, &DedupConfig::default());
        assert_eq!(removed, 1);
        assert_eq!(
            kept.iter().map(|l| l.start_ms).collect::<Vec<_>>(),
            [1_000, 13_000]
        );
    }
}
