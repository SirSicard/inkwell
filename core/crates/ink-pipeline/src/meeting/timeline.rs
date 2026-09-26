//! Where a live stream's samples sit in the meeting.
//!
//! A side's canonical audio is counted in samples as it arrives (the AGC's input index). Its place
//! in the meeting comes from host time: each block's first sample carries the platform clock's
//! stamp, and the meeting started at a stamp on the same clock. Between stamps the samples advance
//! at 16 kHz, so only where a block's stamp disagrees with that count by more than
//! [`TOLERANCE`] (audio lost upstream, or the device clock drifting against the host's) does the
//! timeline take a new anchor. Anchors are few, and those older than the oldest time still to be
//! placed are dropped: a live meeting keeps seconds of them, not the session's.

use std::collections::VecDeque;

use ink_core::CANONICAL_RATE;

/// A block whose stamp is off the count by more than this re-anchors the timeline: 1 ms.
pub const TOLERANCE: i64 = CANONICAL_RATE as i64 / 1_000;

/// Samples of a host-time difference at 16 kHz, rounded toward negative infinity.
pub fn ns_to_samples(ns: i128) -> i64 {
    let samples = (ns * i128::from(CANONICAL_RATE)).div_euclid(1_000_000_000);
    i64::try_from(samples).unwrap_or(if samples < 0 { i64::MIN } else { i64::MAX })
}

/// Stream sample index to meeting sample position, piecewise.
#[derive(Debug, Default)]
pub struct Timeline {
    /// (stream index, meeting position): from that index on, positions advance one per sample.
    anchors: VecDeque<(u64, i64)>,
}

impl Timeline {
    /// Places a block: its first sample, stream index `index`, sits at meeting position `at`.
    pub fn observe(&mut self, index: u64, at: i64) {
        let expected = self.position(index);
        if self.anchors.is_empty() || (at - expected).abs() > TOLERANCE {
            self.anchors.push_back((index, at));
        }
    }

    /// Where stream index `index` sits in the meeting, in samples (negative before the start).
    /// Before the first anchor it extrapolates back from it; with none, it is the index itself.
    pub fn position(&self, index: u64) -> i64 {
        let anchor = self
            .anchors
            .iter()
            .rev()
            .find(|(i, _)| *i <= index)
            .or_else(|| self.anchors.front());
        match anchor {
            Some(&(i, at)) => at.saturating_add(index as i64 - i as i64),
            None => i64::try_from(index).unwrap_or(i64::MAX),
        }
    }

    /// Forgets anchors no index from `index` on can need.
    pub fn forget_before(&mut self, index: u64) {
        while self.anchors.len() > 1 && self.anchors[1].0 <= index {
            self.anchors.pop_front();
        }
    }

    /// Anchors held.
    pub fn anchors(&self) -> usize {
        self.anchors.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_steady_stream_keeps_one_anchor() {
        let mut t = Timeline::default();
        for k in 0..100u64 {
            // Jitter under a millisecond does not re-anchor.
            t.observe(k * 160, (k * 160) as i64 + 1_000 + (k % 3) as i64 * 5);
        }
        assert_eq!(t.anchors(), 1);
        assert_eq!(t.position(16_000), 17_000);
    }

    #[test]
    fn a_gap_re_anchors_and_times_on_both_sides_stay_right() {
        let mut t = Timeline::default();
        t.observe(0, 0);
        t.observe(160, 160);
        // 50 ms of audio lost upstream: the next block's stamp is 800 samples later than counted.
        t.observe(320, 1_120);
        assert_eq!(t.position(200), 200);
        assert_eq!(t.position(400), 1_200);
        t.forget_before(400);
        assert_eq!(t.anchors(), 1);
        assert_eq!(t.position(400), 1_200);
    }

    #[test]
    fn host_time_differences_round_down() {
        assert_eq!(ns_to_samples(1_000_000_000), 16_000);
        assert_eq!(ns_to_samples(62_500), 1);
        assert_eq!(ns_to_samples(62_499), 0);
        assert_eq!(ns_to_samples(-1), -1);
    }
}
