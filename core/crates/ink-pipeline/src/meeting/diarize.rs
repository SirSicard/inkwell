//! The far end's speakers (architecture rule 5).
//!
//! Only the far end is diarized: the mic is one person by construction, so diarizing it could only
//! add error. The diarizer hears the far end's speech regions end to end (silence never reaches an
//! engine), and its turns are mapped back onto the meeting.
//!
//! **Labels are kept only with at least two substantial clusters,** each holding at least
//! [`SUBSTANTIAL_SHARE`] (2 %) of the diarized speech. A cluster below that is a fragment of
//! someone already counted, and naming it asserts a person who was never there; with one
//! substantial cluster, "them" already says who spoke. An earlier implementation measured this on
//! a real call: two remote speakers came back as four clusters, the two spurious ones holding about
//! six seconds between them.
//!
//! With labels kept, each far region is cut where the speaker changes before it is transcribed, so
//! no engine call spans two speakers (the final-pass engine gives one segment per call, with no
//! word timings), and each segment gets the speaker holding most of it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use ink_core::{SpeakerId, SpeakerTurn};

/// The share of diarized speech a cluster needs to count as a person.
pub const SUBSTANTIAL_SHARE: f64 = 0.02;

/// Substantial clusters needed to keep labels at all.
pub const MIN_SPEAKERS: usize = 2;

/// The shortest piece a speaker change may cut off, in ms. A shorter one would be a word, and a
/// cut that close to another is more likely the diarizer's jitter than a turn.
pub const MIN_PIECE_MS: u64 = 1_000;

/// What rule 5 decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule5 {
    /// Clusters the diarizer returned.
    pub clusters: usize,
    /// Of those, the substantial ones.
    pub substantial: BTreeSet<SpeakerId>,
    /// The turns to label with, when labels are kept: the substantial clusters' turns, sorted by
    /// start and clipped so they do not overlap.
    pub turns: Option<Vec<SpeakerTurn>>,
}

/// Applies rule 5 to the diarizer's turns.
pub fn rule5(turns: &[SpeakerTurn]) -> Rule5 {
    let mut held: BTreeMap<&SpeakerId, u64> = BTreeMap::new();
    for t in turns {
        *held.entry(&t.speaker).or_default() += t.end_ms.saturating_sub(t.start_ms);
    }
    let total: u64 = held.values().sum();
    let substantial: BTreeSet<SpeakerId> = held
        .iter()
        // Durations in ms of one meeting: far inside f64's exact range.
        .filter(|&(_, &ms)| total > 0 && ms as f64 >= SUBSTANTIAL_SHARE * total as f64)
        .map(|(s, _)| (*s).clone())
        .collect();
    let kept = (substantial.len() >= MIN_SPEAKERS).then(|| {
        let mut kept: Vec<SpeakerTurn> = turns
            .iter()
            .filter(|t| substantial.contains(&t.speaker) && t.end_ms > t.start_ms)
            .cloned()
            .collect();
        kept.sort_by(|a, b| (a.start_ms, &a.speaker).cmp(&(b.start_ms, &b.speaker)));
        // Where two overlap, the earlier one ends as the later one starts: at any moment one
        // speaker holds the floor, which is what a cut and an attribution need.
        for i in 1..kept.len() {
            let next = kept[i].start_ms;
            let prev = &mut kept[i - 1];
            prev.end_ms = prev.end_ms.min(next).max(prev.start_ms);
        }
        kept.retain(|t| t.end_ms > t.start_ms);
        kept
    });
    Rule5 {
        clusters: held.len(),
        substantial,
        turns: kept,
    }
}

/// Where each piece of the concatenated far speech came from: `(compact_start, meeting_start,
/// len)`, all in samples, in order.
pub type Pieces = [(u64, u64, u64)];

/// Maps turns on the concatenated speech (ms) back onto the meeting (ms). A turn that spans the
/// seam between two regions is split at it; nothing lands in the silence between them.
pub fn to_meeting(turns: &[SpeakerTurn], pieces: &Pieces) -> Vec<SpeakerTurn> {
    let mut out = Vec::new();
    for turn in turns {
        let (a, b) = (turn.start_ms * 16, turn.end_ms.max(turn.start_ms) * 16);
        for &(compact, meeting, len) in pieces {
            let lo = a.max(compact);
            let hi = b.min(compact + len);
            if lo < hi {
                out.push(SpeakerTurn {
                    speaker: turn.speaker.clone(),
                    start_ms: (meeting + lo - compact) / 16,
                    end_ms: (meeting + hi - compact) / 16,
                });
            }
        }
    }
    out
}

/// Cut points (ms) inside `span` where the floor passes to another speaker, at least
/// [`MIN_PIECE_MS`] from the span's ends and from each other. `turns` as [`Rule5::turns`] gives
/// them. A turn shorter than [`MIN_PIECE_MS`] never causes a cut (a flicker, or a one-word
/// interjection): it is attributed with the piece around it.
pub fn speaker_changes(span: Range<u64>, turns: &[SpeakerTurn]) -> Vec<u64> {
    let inside: Vec<&SpeakerTurn> = turns
        .iter()
        .filter(|t| t.start_ms < span.end && t.end_ms > span.start)
        .filter(|t| t.end_ms - t.start_ms >= MIN_PIECE_MS)
        .collect();
    let mut cuts = Vec::new();
    let mut last = span.start;
    for pair in inside.windows(2) {
        let (prev, next) = (pair[0], pair[1]);
        if prev.speaker == next.speaker {
            continue;
        }
        // In the gap between them (clipped turns never overlap).
        let cut = (prev.end_ms + next.start_ms) / 2;
        if cut >= last + MIN_PIECE_MS && cut + MIN_PIECE_MS <= span.end {
            cuts.push(cut);
            last = cut;
        }
    }
    cuts
}

/// The speaker holding most of `span`, or `None` when no turn overlaps it. A tie goes to the
/// speaker whose label sorts first, so the answer never depends on order.
pub fn speaker_of(span: Range<u64>, turns: &[SpeakerTurn]) -> Option<SpeakerId> {
    let mut held: BTreeMap<&SpeakerId, u64> = BTreeMap::new();
    for t in turns {
        let overlap = t
            .end_ms
            .min(span.end)
            .saturating_sub(t.start_ms.max(span.start));
        if overlap > 0 {
            *held.entry(&t.speaker).or_default() += overlap;
        }
    }
    let best = held.values().copied().max()?;
    held.into_iter()
        .find(|&(_, ms)| ms == best)
        .map(|(s, _)| s.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(speaker: &str, start_ms: u64, end_ms: u64) -> SpeakerTurn {
        SpeakerTurn {
            speaker: SpeakerId(speaker.into()),
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn two_substantial_clusters_keep_labels_and_fragments_are_dropped() {
        // 50 s and 40 s, plus a 1 s fragment (1.1 % of 91 s).
        let r = rule5(&[
            turn("a", 0, 50_000),
            turn("b", 50_000, 90_000),
            turn("c", 90_000, 91_000),
        ]);
        assert_eq!(r.clusters, 3);
        assert_eq!(r.substantial.len(), 2);
        let kept = r.turns.expect("labels kept");
        assert!(kept.iter().all(|t| t.speaker.0 != "c"));
    }

    #[test]
    fn one_substantial_cluster_keeps_no_labels() {
        // The only other cluster holds 1.5 %.
        let r = rule5(&[turn("a", 0, 98_500), turn("b", 98_500, 100_000)]);
        assert_eq!(r.substantial.len(), 1);
        assert_eq!(r.turns, None, "stream identity is enough");
        assert_eq!(rule5(&[]).turns, None);
    }

    #[test]
    fn exactly_two_percent_counts() {
        let r = rule5(&[turn("a", 0, 98_000), turn("b", 98_000, 100_000)]);
        assert_eq!(r.substantial.len(), 2);
        assert!(r.turns.is_some());
    }

    #[test]
    fn overlapping_turns_are_clipped_so_one_holds_the_floor() {
        let r = rule5(&[turn("a", 0, 10_000), turn("b", 8_000, 20_000)]);
        let kept = r.turns.unwrap();
        assert_eq!((kept[0].start_ms, kept[0].end_ms), (0, 8_000));
        assert_eq!((kept[1].start_ms, kept[1].end_ms), (8_000, 20_000));
    }

    #[test]
    fn turns_map_back_across_the_silence_between_regions() {
        // Two regions: meeting 10..12 s and 30..33 s, concatenated as 0..2 s and 2..5 s.
        let pieces = [(0, 160_000, 32_000), (32_000, 480_000, 48_000)];
        let mapped = to_meeting(&[turn("a", 1_000, 3_000)], &pieces);
        assert_eq!(
            mapped,
            vec![turn("a", 11_000, 12_000), turn("a", 30_000, 31_000)]
        );
    }

    #[test]
    fn a_region_is_cut_where_the_speaker_changes_and_not_for_jitter() {
        let turns = [
            turn("a", 0, 4_000),
            turn("b", 4_500, 9_000),
            // A 300 ms flicker to "a" and back: no piece that short.
            turn("a", 9_000, 9_300),
            turn("b", 9_300, 12_000),
            turn("a", 12_000, 15_000),
        ];
        assert_eq!(speaker_changes(0..15_000, &turns), vec![4_250, 12_000]);
        // Too close to the span's end to cut there.
        assert_eq!(speaker_changes(0..12_500, &turns), vec![4_250]);
        assert_eq!(speaker_of(0..4_250, &turns), Some(SpeakerId("a".into())));
        assert_eq!(
            speaker_of(4_250..9_150, &turns),
            Some(SpeakerId("b".into()))
        );
        assert_eq!(speaker_of(20_000..21_000, &turns), None);
    }
}
