//! What the Stats screen counts of a record, without its words: [`RecordDigest`].
//!
//! A digest is counts and times from a record's current transcript, never text, so a store may
//! keep one beside the record (the SQLite store does, so a stats query never re-reads every
//! transcript in the library) and hand it out freely. [`digest`] is the one definition every store
//! uses; [`Store::digests`](crate::Store::digests) and
//! [`Store::commitment_states`](crate::Store::commitment_states) are how the stats are read.
//!
//! Me versus them is stream identity (architecture rule 5): the mic channel is the user, the far
//! channel everyone else. Nothing here judges what was said. The one count that reads the text is
//! [`TranscriptDigest::mic_questions`], and it is literal: the user's lines that end in a question
//! mark.

use crate::audio::Channel;
use crate::store::{CommitmentId, RecordId, RecordKind, Segment};

/// The longest pause that still leaves the user's speech one monologue. A longer silence, or any
/// speech from the far end, ends it.
pub const MONOLOGUE_PAUSE_MS: u64 = 3_000;

/// One channel of a transcript, counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelDigest {
    /// Words, split on whitespace (as [`word_count`](crate::store::word_count) counts them).
    pub words: u64,
    /// How long the channel's lines cover, ms: the union of their spans, so overlapping lines
    /// count once. For a dictation (one line from 0 to the release of the key) it is how long the
    /// key was held.
    pub speech_ms: u64,
    /// Its lines (segments).
    pub lines: u64,
}

/// A transcript, counted: no text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TranscriptDigest {
    /// The user's side.
    pub mic: ChannelDigest,
    /// Everyone else.
    pub far: ChannelDigest,
    /// The longest stretch of the user speaking with no one else speaking and no pause longer than
    /// [`MONOLOGUE_PAUSE_MS`], ms: from the start of its first line to the end of its last.
    pub longest_monologue_ms: u64,
    /// The user's lines whose text ends in a question mark (`?` or the full-width `？`), trailing
    /// space aside. A plain count, labelled as such: no judgement of what a question is.
    pub mic_questions: u64,
}

/// A record and its transcript's digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordDigest {
    /// The record.
    pub record: RecordId,
    /// What produced it.
    pub kind: RecordKind,
    /// When it started, Unix ms.
    pub started_at_unix_ms: i64,
    /// When it ended, Unix ms; `None` while live.
    pub ended_at_unix_ms: Option<i64>,
    /// Whether an import wrote it (see [`Record::imported`](crate::Record::imported)).
    pub imported: bool,
    /// Its current transcript, counted.
    pub transcript: TranscriptDigest,
}

/// A commitment as the stats count it: its state, never its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitmentState {
    /// The commitment.
    pub commitment: CommitmentId,
    /// The record it was said in.
    pub record: RecordId,
    /// When that record started, Unix ms: when the promise was made.
    pub record_started_at_unix_ms: i64,
    /// When it is due, resolved, Unix ms.
    pub due_at_unix_ms: Option<i64>,
    /// Whether it is done.
    pub done: bool,
    /// Whether it was folded into another ("said twice"): the other one is the promise.
    pub merged: bool,
}

/// The version of [`digest`]'s rules. A store that keeps digests keeps this beside each one and
/// recounts any kept under another, so changing what a digest counts (the word split, the pause
/// that ends a monologue, the question rule) must raise it: the pinned test below fails until it
/// is raised.
pub const DIGEST_VERSION: u32 = 1;

/// Counts `segments` (one record's current transcript).
pub fn digest(segments: &[Segment]) -> TranscriptDigest {
    // The side's counts, its lines' spans as said, and their union.
    type Spans = Vec<(u64, u64)>;
    let side = |channel: Channel| -> (ChannelDigest, Spans, Spans) {
        let mut d = ChannelDigest::default();
        let mut spans = Vec::new();
        for s in segments.iter().filter(|s| s.channel == channel) {
            d.words += s.text.split_whitespace().count() as u64;
            d.lines += 1;
            spans.push((s.start_ms, s.end_ms.max(s.start_ms)));
        }
        let merged = union(spans.clone());
        d.speech_ms = merged.iter().map(|(a, b)| b - a).sum();
        (d, spans, merged)
    };
    let (mic, mic_spans, _) = side(Channel::Mic);
    let (far, _, far_spans) = side(Channel::Far);
    TranscriptDigest {
        mic,
        far,
        longest_monologue_ms: longest_monologue(mic_spans, &far_spans),
        mic_questions: segments
            .iter()
            .filter(|s| s.channel == Channel::Mic)
            .filter(|s| s.text.trim_end().ends_with(['?', '？']))
            .count() as u64,
    }
}

/// `spans` merged where they overlap or touch: disjoint, by start.
fn union(mut spans: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    spans.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Whether any of `far` (disjoint, by start) touches `[a, b]`.
fn far_touches(far: &[(u64, u64)], a: u64, b: u64) -> bool {
    // Disjoint and by start, so their ends are in order too.
    let first = far.partition_point(|f| f.1 < a);
    far.get(first).is_some_and(|f| f.0 <= b)
}

/// The longest monologue (see [`TranscriptDigest::longest_monologue_ms`]).
///
/// The user's lines less every moment the far end speaks are the pieces of speech that were the
/// user's alone; pieces join while the silence between them is at most [`MONOLOGUE_PAUSE_MS`] and
/// the far end said nothing in it.
fn longest_monologue(mic: Vec<(u64, u64)>, far: &[(u64, u64)]) -> u64 {
    let mut pieces = Vec::with_capacity(mic.len());
    for (s, e) in mic {
        let mut cursor = s;
        let first = far.partition_point(|f| f.1 <= s);
        for f in far[first..].iter().take_while(|f| f.0 < e) {
            if f.0 > cursor {
                pieces.push((cursor, f.0));
            }
            cursor = cursor.max(f.1);
        }
        if e > cursor {
            pieces.push((cursor, e));
        }
    }
    pieces.sort_unstable();
    let mut longest = 0;
    let mut run: Option<(u64, u64)> = None;
    for (a, b) in pieces {
        run = match run {
            // Overlapping lines of the user's: one stretch of speech.
            Some((ra, rb)) if a < rb => Some((ra, rb.max(b))),
            Some((ra, rb)) if a - rb <= MONOLOGUE_PAUSE_MS && !far_touches(far, rb, a) => {
                Some((ra, rb.max(b)))
            }
            Some((ra, rb)) => {
                longest = longest.max(rb - ra);
                Some((a, b))
            }
            None => Some((a, b)),
        };
    }
    if let Some((ra, rb)) = run {
        longest = longest.max(rb - ra);
    }
    longest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(channel: Channel, start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment {
            channel,
            start_ms,
            end_ms,
            text: text.into(),
            speaker: None,
        }
    }
    fn mic(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        seg(Channel::Mic, start_ms, end_ms, text)
    }
    fn far(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        seg(Channel::Far, start_ms, end_ms, text)
    }

    /// Pins what version 1 of the rules counts for one transcript. If this fails, the rules
    /// changed: raise [`DIGEST_VERSION`] (so kept digests are recounted), then update the numbers.
    #[test]
    fn digest_version_1_counts_this_transcript_so() {
        assert_eq!(DIGEST_VERSION, 1);
        let d = digest(&[
            mic(0, 4_000, "shall we start?"),
            mic(6_500, 9_000, "one two  three"),
            far(9_500, 12_000, "yes we can"),
            mic(16_000, 17_000, "ok？"),
        ]);
        assert_eq!(
            d,
            TranscriptDigest {
                mic: ChannelDigest {
                    words: 7,
                    speech_ms: 7_500,
                    lines: 3
                },
                far: ChannelDigest {
                    words: 3,
                    speech_ms: 2_500,
                    lines: 1
                },
                longest_monologue_ms: 9_000,
                mic_questions: 2,
            }
        );
    }

    #[test]
    fn an_empty_transcript_counts_nothing() {
        assert_eq!(digest(&[]), TranscriptDigest::default());
    }

    #[test]
    fn a_dictation_counts_its_words_and_how_long_the_key_was_held() {
        let d = digest(&[mic(0, 12_500, "  one two  three\nfour ")]);
        assert_eq!(
            d.mic,
            ChannelDigest {
                words: 4,
                speech_ms: 12_500,
                lines: 1
            }
        );
        assert_eq!(d.far, ChannelDigest::default());
        assert_eq!(d.longest_monologue_ms, 12_500);
        assert_eq!(d.mic_questions, 0);
    }

    /// Talk time is per channel, and lines that overlap on one side count once.
    #[test]
    fn talk_time_is_the_union_of_each_sides_lines() {
        let d = digest(&[
            mic(0, 4_000, "a b"),
            mic(3_000, 6_000, "c"),
            far(6_000, 7_000, "d e f"),
            far(10_000, 10_000, "g"),
            mic(20_000, 21_000, "h"),
        ]);
        assert_eq!(d.mic.speech_ms, 7_000);
        assert_eq!(d.mic.words, 4);
        assert_eq!(d.mic.lines, 3);
        assert_eq!(d.far.speech_ms, 1_000);
        assert_eq!(d.far.words, 4);
        assert_eq!(d.far.lines, 2);
    }

    #[test]
    fn questions_are_the_users_lines_that_end_in_a_question_mark() {
        let d = digest(&[
            mic(0, 1, "Can we ship Friday?"),
            mic(1, 2, "really?  \n"),
            mic(2, 3, "全角？"),
            mic(3, 4, "Why? Because."),
            mic(4, 5, "?!"),
            far(5, 6, "What about you?"),
        ]);
        assert_eq!(d.mic_questions, 3);
    }

    /// A monologue: the user's lines joined across pauses of up to three seconds, ended by a
    /// longer pause or by anything the far end says, even a word inside one of the user's lines.
    #[test]
    fn the_longest_monologue_ends_at_a_long_pause_or_the_far_end() {
        // Lines with pauses of 3 s and under: one monologue of 0..20 s.
        let joined = [
            mic(0, 5_000, "a"),
            mic(8_000, 12_000, "b"),
            mic(13_000, 20_000, "c"),
        ];
        assert_eq!(digest(&joined).longest_monologue_ms, 20_000);

        // A pause of 3.001 s splits it.
        let split = [mic(0, 5_000, "a"), mic(8_001, 12_000, "b")];
        assert_eq!(digest(&split).longest_monologue_ms, 5_000);

        // The far end speaking in the pause splits it, however short the pause.
        let answered = [
            mic(0, 5_000, "a"),
            far(5_200, 5_400, "mm"),
            mic(5_500, 9_000, "b"),
        ];
        assert_eq!(digest(&answered).longest_monologue_ms, 5_000);

        // The far end speaking over one long line cuts it: 0..50 s and 51..100 s.
        let interrupted = [mic(0, 100_000, "a"), far(50_000, 51_000, "b")];
        assert_eq!(digest(&interrupted).longest_monologue_ms, 50_000);
        let later = [mic(0, 100_000, "a"), far(40_000, 41_000, "b")];
        assert_eq!(digest(&later).longest_monologue_ms, 59_000);

        // A far line that ends exactly where the user resumes still separates the two.
        let touching = [
            mic(0, 4_000, "a"),
            far(4_000, 5_000, "b"),
            mic(5_000, 6_000, "c"),
        ];
        assert_eq!(digest(&touching).longest_monologue_ms, 4_000);

        // Only the far end: no monologue.
        assert_eq!(digest(&[far(0, 9_000, "x")]).longest_monologue_ms, 0);
    }

    #[test]
    fn segments_out_of_order_are_counted_the_same() {
        let ordered = [
            mic(0, 2_000, "a"),
            far(2_500, 3_000, "b"),
            mic(4_000, 9_000, "c d?"),
        ];
        let mut shuffled = ordered.to_vec();
        shuffled.reverse();
        assert_eq!(digest(&ordered), digest(&shuffled));
        assert_eq!(digest(&ordered).longest_monologue_ms, 5_000);
    }
}
