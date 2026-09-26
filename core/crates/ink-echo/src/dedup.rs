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
//! four hold:
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
//! 4. **The acoustic evidence agrees:** the full output's voice activity heard no near-end speech
//!    over the line, with every window there recorded ([`NearSpeech`], normally the
//!    [`EchoGate`](crate::EchoGate)). Words alone cannot tell echo from a user reading a number
//!    straight back inside the echo window; the full output can, because the suppressor removes
//!    echo and keeps a voice. With no evidence (a gap in the VAD, no track at all), the line is
//!    kept.
//!
//! A user who repeats the far end's words later keeps them too: the time window excludes it, or
//! the line holds enough words of their own to fall under 80 %.
//!
//! Removed lines come back by index and span, with the far-end lines they matched, never their
//! text, so the caller can store them and undo the removal.

use ink_core::TimedText;

/// Whether the near end spoke over a stretch of the record: the acoustic half of the rule.
///
/// [`EchoGate`](crate::EchoGate) implements it from the full output's voice activity.
pub trait NearSpeech {
    /// Over `start_ms..end_ms` on the mic's timeline: `Some(true)` when the near end was heard,
    /// `Some(false)` when every window was recorded and none heard it, `None` when there is no
    /// evidence either way.
    fn near_speech(&self, start_ms: u64, end_ms: u64) -> Option<bool>;
}

impl NearSpeech for crate::EchoGate {
    fn near_speech(&self, start_ms: u64, end_ms: u64) -> Option<bool> {
        crate::EchoGate::near_speech(self, start_ms, end_ms)
    }
}

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

/// A "you" line removed as echo. It carries no text: the caller holds the line by its index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duplicate {
    /// Its index in the "you" lines.
    pub you: usize,
    /// Its start, ms.
    pub start_ms: u64,
    /// Its end, ms.
    pub end_ms: u64,
    /// The indices, in the far-end lines, of those whose words it matched, in time order.
    pub far: Vec<usize>,
    /// Its words after normalising.
    pub words: usize,
    /// How many of them matched the far end, in order.
    pub matched: usize,
}

/// What duplicate-line suppression did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DedupReport {
    /// The lines removed as echo, in the order of the "you" lines.
    pub removed: Vec<Duplicate>,
    /// Lines whose words matched but which were kept: the full output heard the near end over
    /// them (a read-back, the user repeating).
    pub kept_near_speech: usize,
    /// Lines whose words matched but which were kept for want of acoustic evidence.
    pub kept_no_evidence: usize,
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

/// A longest common subsequence with at least one word in it.
struct Lcs {
    /// Words matched.
    len: usize,
    /// The matched words' indices in the second sequence, ascending.
    in_b: Vec<usize>,
    /// The first of them.
    first_b: usize,
}

/// The longest common subsequence of `a` and `b`, or `None` when they share no word.
fn lcs(a: &[String], b: &[String]) -> Option<Lcs> {
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
    let mut in_b = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            in_b.push(j - 1);
            i -= 1;
            j -= 1;
        } else if t[at(i - 1, j)] >= t[at(i, j - 1)] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    in_b.reverse();
    // No word in common is the `None` case.
    let &first_b = in_b.first()?;
    Some(Lcs {
        len: in_b.len(),
        in_b,
        first_b,
    })
}

/// The far-end lines a "you" line's words match by rules 1–3, as indices into `them` in time
/// order, with the match's size; `None` when the words do not make it a candidate.
fn text_match(
    line: &TimedText,
    them: &[TimedText],
    far_words: &[Vec<String>],
    config: &DedupConfig,
) -> Option<(Vec<usize>, usize, usize)> {
    let words = normalise(&line.text);
    if words.len() < config.min_words {
        return None;
    }
    // The far-end lines around it, in time order.
    let mut near: Vec<usize> = (0..them.len())
        .filter(|&f| {
            them[f].start_ms <= line.end_ms + config.lead_ms
                && them[f].end_ms + config.lag_ms >= line.start_ms
        })
        .collect();
    near.sort_by_key(|&f| them[f].start_ms);
    let mut pool: Vec<String> = Vec::new();
    let mut owner: Vec<usize> = Vec::new();
    for &f in &near {
        pool.extend(far_words[f].iter().cloned());
        owner.extend(std::iter::repeat_n(f, far_words[f].len()));
    }
    // No word in common: not a duplicate whatever the thresholds say.
    let m = lcs(&words, &pool)?;
    let needed = (config.min_share * words.len() as f64).ceil() as usize;
    if m.len < needed {
        return None;
    }
    // The pool runs in start order, so the first matched word's line started first among those
    // matched.
    let earliest = them[owner[m.first_b]].start_ms;
    if line.start_ms + config.lead_ms < earliest {
        return None;
    }
    let mut far: Vec<usize> = m.in_b.iter().map(|&j| owner[j]).collect();
    far.dedup();
    Some((far, words.len(), m.len))
}

/// The "you" lines that are echo of the far end's, by the rule in the module docs, with
/// `evidence` for rule 4. Both line lists are on one timeline (the record's), in any order.
pub fn echo_duplicates(
    you: &[TimedText],
    them: &[TimedText],
    evidence: &impl NearSpeech,
    config: &DedupConfig,
) -> DedupReport {
    let far_words: Vec<Vec<String>> = them.iter().map(|l| normalise(&l.text)).collect();
    let mut report = DedupReport::default();
    for (idx, line) in you.iter().enumerate() {
        let Some((far, words, matched)) = text_match(line, them, &far_words, config) else {
            continue;
        };
        match evidence.near_speech(line.start_ms, line.end_ms) {
            Some(false) => report.removed.push(Duplicate {
                you: idx,
                start_ms: line.start_ms,
                end_ms: line.end_ms,
                far,
                words,
                matched,
            }),
            Some(true) => report.kept_near_speech += 1,
            None => report.kept_no_evidence += 1,
        }
    }
    report
}

/// `you` without the lines that are echo of `them`: the kept lines, and the report naming those
/// removed (by index into the `you` given here) so the caller can store them.
pub fn remove_echo_duplicates(
    you: Vec<TimedText>,
    them: &[TimedText],
    evidence: &impl NearSpeech,
    config: &DedupConfig,
) -> (Vec<TimedText>, DedupReport) {
    let report = echo_duplicates(&you, them, evidence, config);
    let mut drop = vec![false; you.len()];
    for d in &report.removed {
        drop[d.you] = true;
    }
    let kept = you
        .into_iter()
        .zip(drop)
        .filter(|(_, d)| !*d)
        .map(|(l, _)| l)
        .collect();
    (kept, report)
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

    /// Evidence that gives the same answer over every span.
    struct Everywhere(Option<bool>);

    impl NearSpeech for Everywhere {
        fn near_speech(&self, _: u64, _: u64) -> Option<bool> {
            self.0
        }
    }

    /// Evidence from a VAD that heard the near end between two times, and nothing else.
    struct SpeechBetween(u64, u64);

    impl NearSpeech for SpeechBetween {
        fn near_speech(&self, start_ms: u64, end_ms: u64) -> Option<bool> {
            Some(start_ms < self.1 && end_ms > self.0)
        }
    }

    const SILENT: Everywhere = Everywhere(Some(false));

    fn echo(you: &[TimedText], them: &[TimedText]) -> Vec<usize> {
        echo_duplicates(you, them, &SILENT, &DedupConfig::default())
            .removed
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
    fn a_true_echo_repeat_is_removed_and_reported_by_index_and_span() {
        let them = [
            line(1_000, 2_000, "good morning"),
            line(10_000, 12_500, "the quarterly numbers go out on friday"),
        ];
        let you = [line(
            10_080,
            12_600,
            "the quarterly numbers go out on friday",
        )];
        let report = echo_duplicates(&you, &them, &SILENT, &DedupConfig::default());
        assert_eq!(
            report.removed,
            [Duplicate {
                you: 0,
                start_ms: 10_080,
                end_ms: 12_600,
                far: vec![1],
                words: 7,
                matched: 7,
            }]
        );
        assert_eq!((report.kept_near_speech, report.kept_no_evidence), (0, 0));
    }

    #[test]
    fn an_immediate_read_back_with_the_near_end_speaking_is_kept() {
        // The far end reads a code; the user reads it straight back, inside the echo window.
        // The words match in full, but the full output heard the user.
        let them = [line(1_000, 3_000, "the code is four seven one nine")];
        let you = [line(3_200, 4_400, "four seven one nine")];
        let report = echo_duplicates(
            &you,
            &them,
            &SpeechBetween(3_200, 4_400),
            &DedupConfig::default(),
        );
        assert!(report.removed.is_empty(), "{report:?}");
        assert_eq!(report.kept_near_speech, 1);
    }

    #[test]
    fn a_line_with_no_evidence_is_kept() {
        let them = [line(
            10_000,
            12_500,
            "the quarterly numbers go out on friday",
        )];
        let you = [line(
            10_080,
            12_600,
            "the quarterly numbers go out on friday",
        )];
        let report = echo_duplicates(&you, &them, &Everywhere(None), &DedupConfig::default());
        assert!(report.removed.is_empty());
        assert_eq!(report.kept_no_evidence, 1);
    }

    #[test]
    fn the_echo_gate_is_the_evidence() {
        use crate::{EchoGate, GateConfig};
        // A full output whose VAD heard nothing for 5 s, then speech from 5 s.
        let mut gate = EchoGate::new(GateConfig::default(), 512);
        for w in 0..(10 * 16_000 / 512) as u64 {
            let t = w as f64 * 512.0 / 16_000.0;
            gate.push_speech(w, if t >= 5.0 { 0.9 } else { 0.0 })
                .expect("a probability");
        }
        let them = [
            line(1_000, 3_000, "the code is four seven one nine"),
            line(5_000, 7_000, "shall we move on to the budget"),
        ];
        let you = [
            line(1_050, 3_050, "the code is four seven one nine"),
            line(5_050, 7_050, "shall we move on to the budget"),
        ];
        let report = echo_duplicates(&you, &them, &gate, &DedupConfig::default());
        assert_eq!(
            report.removed.iter().map(|d| d.you).collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(report.kept_near_speech, 1);
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
        let report = echo_duplicates(&you, &them, &SILENT, &DedupConfig::default());
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].far, [0, 1]);
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
    fn a_zero_share_threshold_still_needs_a_matched_word() {
        let config = DedupConfig {
            min_share: 0.0,
            ..DedupConfig::default()
        };
        let them = [line(1_000, 3_000, "completely different words here")];
        let you = [line(1_000, 3_000, "nothing in common at all")];
        assert!(
            echo_duplicates(&you, &them, &SILENT, &config)
                .removed
                .is_empty()
        );
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
        let (kept, report) = remove_echo_duplicates(you, &them, &SILENT, &DedupConfig::default());
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].you, 1);
        assert_eq!(
            kept.iter().map(|l| l.start_ms).collect::<Vec<_>>(),
            [1_000, 13_000]
        );
    }
}
