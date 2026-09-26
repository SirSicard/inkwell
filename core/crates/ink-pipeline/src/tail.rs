//! The adaptive tail: how long a take keeps listening after the key comes up.
//!
//! Inkwell 0.2 waited a fixed ~450 ms after every release: it slept 350 ms before collecting the
//! 300 ms tail, then another 100 ms before pasting. `ink-audio`'s [`TakeRecorder`] already made the
//! tail arrive **in audio, not in time** (the take completes in the push that delivers the 300th
//! millisecond), and the chain drops the pre-paste sleep. This module makes the tail as short as
//! the speech allows:
//!
//! - The last word is still decaying when the key comes up, so the tail keeps listening; but once
//!   the audio has been quiet for [`TailConfig::quiet_run`] (80 ms) at or past the release, the
//!   decay is over and the take ends there. A speaker who stopped before letting go gets no tail
//!   at all: the take ends as soon as the audio reaches the release.
//! - "Quiet" is relative to the take's own speech: a 20 ms frame whose peak sits more than
//!   [`TailConfig::quiet_below_db`] (20 dB) under the take's robust peak, and never above the gain
//!   stage's noise floor for a silent take. Noise that never falls that far below the speech (a
//!   fan close to the mic) keeps the full tail, which is 0.2's behaviour, never less.
//! - The tail never exceeds 300 ms ([`TAIL_SAMPLES`]): the recorder completes the take itself.
//! - If the audio stops arriving (a device that vanished), the take ends at a deadline,
//!   [`TailConfig::grace`] after the full tail was due, with what arrived.
//!
//! Positions are absolute sample indices on the recorder's 16 kHz timeline.
//!
//! [`TakeRecorder`]: ink_audio::TakeRecorder
//! [`TAIL_SAMPLES`]: ink_audio::take::TAIL_SAMPLES

use std::time::Duration;

use ink_audio::gain::{LEVEL_FRAME, NOISE_FLOOR, TRANSIENT_FRAMES, from_dbfs};

/// The fewest level frames (200 ms) a take needs before its tail may end early: too little audio
/// gives no level to judge quiet against.
pub const MIN_LEVEL_FRAMES: usize = 10;

/// How the tail adapts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TailConfig {
    /// A frame is quiet when its peak is this many dB under the take's robust peak.
    pub quiet_below_db: f32,
    /// Quiet this long, reaching the release, ends the take.
    pub quiet_run: Duration,
    /// How long past the full tail to wait for audio that stopped arriving.
    pub grace: Duration,
}

impl Default for TailConfig {
    fn default() -> Self {
        Self {
            quiet_below_db: 20.0,
            quiet_run: Duration::from_millis(80),
            grace: Duration::from_millis(250),
        }
    }
}

impl TailConfig {
    fn quiet_frames(&self) -> usize {
        let frame_ms = LEVEL_FRAME as u128 * 1_000 / 16_000;
        (self.quiet_run.as_millis().div_ceil(frame_ms) as usize).max(1)
    }
}

/// Frame peaks of the take in progress, and the decision.
///
/// **Worker.** Allocates one `f32` per 20 ms of take.
#[derive(Debug, Default)]
pub struct TailTracker {
    /// Where the first frame starts.
    start: u64,
    /// Peaks of the complete frames, in order.
    peaks: Vec<f32>,
    /// The frame being filled: its peak and its length so far.
    partial_peak: f32,
    partial_len: usize,
    /// Frozen at the release: below this, a frame is quiet.
    threshold: Option<f32>,
}

impl TailTracker {
    /// Starts tracking a take whose next pushed sample sits at `position`.
    pub fn begin(&mut self, position: u64) {
        self.start = position;
        self.peaks.clear();
        self.partial_peak = 0.0;
        self.partial_len = 0;
        self.threshold = None;
    }

    /// Takes the next audio of the take, in order.
    pub fn observe(&mut self, samples: &[f32]) {
        let mut rest = samples;
        while !rest.is_empty() {
            let n = (LEVEL_FRAME - self.partial_len).min(rest.len());
            // `f32::max` ignores NaN, so a NaN sample cannot poison the level.
            self.partial_peak = rest[..n]
                .iter()
                .fold(self.partial_peak, |m, s| m.max(s.abs()));
            self.partial_len += n;
            rest = &rest[n..];
            if self.partial_len == LEVEL_FRAME {
                self.peaks.push(self.partial_peak);
                self.partial_peak = 0.0;
                self.partial_len = 0;
            }
        }
    }

    /// The key came up: freezes the quiet threshold from the take so far.
    pub fn release(&mut self, cfg: &TailConfig) {
        let mut sorted = self.peaks.clone();
        sorted.sort_unstable_by(|a, b| b.total_cmp(a));
        let robust = sorted
            .get(TRANSIENT_FRAMES.min(sorted.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0.0);
        self.threshold = Some((robust * from_dbfs(-cfg.quiet_below_db)).max(NOISE_FLOOR));
    }

    /// Whether the take can end now: the audio reaches `release`, and its last
    /// [`quiet_run`](TailConfig::quiet_run) is quiet. Always `false` before
    /// [`release`](Self::release), and for a take shorter than [`MIN_LEVEL_FRAMES`].
    pub fn speech_ended(&self, release: u64, cfg: &TailConfig) -> bool {
        let Some(threshold) = self.threshold else {
            return false;
        };
        let frames = self.peaks.len();
        let heard_to = self.start + (frames * LEVEL_FRAME) as u64;
        let run = cfg.quiet_frames();
        frames >= MIN_LEVEL_FRAMES.max(run)
            && heard_to >= release
            && self.peaks[frames - run..].iter().all(|&p| p < threshold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(level: f32, n: usize) -> Vec<f32> {
        vec![level; n * LEVEL_FRAME]
    }

    #[test]
    fn quiet_reaching_the_release_ends_the_tail() {
        let cfg = TailConfig::default();
        let mut t = TailTracker::default();
        t.begin(0);
        t.observe(&frames(0.1, 20)); // speech
        t.observe(&frames(0.0005, 4)); // 80 ms of quiet, 26 dB down
        t.release(&cfg);
        let end = (24 * LEVEL_FRAME) as u64;
        assert!(t.speech_ended(end, &cfg), "quiet up to the release");
        assert!(
            !t.speech_ended(end + 1, &cfg),
            "not heard up to the release yet"
        );
    }

    #[test]
    fn a_sounding_last_word_keeps_the_tail_open() {
        let cfg = TailConfig::default();
        let mut t = TailTracker::default();
        t.begin(0);
        t.observe(&frames(0.1, 20));
        t.release(&cfg);
        let release = (20 * LEVEL_FRAME) as u64;
        assert!(!t.speech_ended(release, &cfg));
        // Decay: 15 dB down is not quiet yet; 30 dB down for 80 ms is.
        t.observe(&frames(0.018, 3));
        assert!(!t.speech_ended(release, &cfg));
        t.observe(&frames(0.003, 4));
        assert!(t.speech_ended(release, &cfg));
    }

    #[test]
    fn noise_that_never_drops_keeps_the_full_tail() {
        let cfg = TailConfig::default();
        let mut t = TailTracker::default();
        t.begin(0);
        t.observe(&frames(0.1, 20));
        t.observe(&frames(0.02, 30)); // a fan 14 dB under the speech, all the way
        t.release(&cfg);
        assert!(!t.speech_ended(0, &cfg));
    }

    #[test]
    fn nothing_ends_before_the_release_or_on_too_little_audio() {
        let cfg = TailConfig::default();
        let mut t = TailTracker::default();
        t.begin(0);
        t.observe(&frames(0.0, 30));
        assert!(!t.speech_ended(0, &cfg), "no release yet");
        let mut short = TailTracker::default();
        short.begin(0);
        short.observe(&frames(0.0, MIN_LEVEL_FRAMES - 1));
        short.release(&cfg);
        assert!(!short.speech_ended(0, &cfg), "too short to judge");
        // A partial frame is not judged either.
        let mut partial = TailTracker::default();
        partial.begin(0);
        partial.observe(&vec![0.0; MIN_LEVEL_FRAMES * LEVEL_FRAME - 1]);
        partial.release(&cfg);
        assert!(!partial.speech_ended(0, &cfg));
    }

    #[test]
    fn the_quiet_run_rounds_up_to_whole_frames() {
        let cfg = TailConfig {
            quiet_run: Duration::from_millis(50),
            ..TailConfig::default()
        };
        assert_eq!(cfg.quiet_frames(), 3);
        assert_eq!(TailConfig::default().quiet_frames(), 4);
    }
}
