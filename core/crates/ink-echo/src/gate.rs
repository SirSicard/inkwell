//! The echo-only word gate: drops words of the "you" transcript where AEC3's full output says
//! nobody on the near end was talking.
//!
//! The "you" transcript reads AEC3's **linear** output, because its **full** output (after the
//! residual echo suppressor) garbles and ducks the user in double talk: in S0.3's real double
//! talk the full output cost 9.9 WER points and the linear output 0.7. The linear output has a
//! cost of its own: with nobody talking it still carries enough residual echo to transcribe the
//! far end's words, which would be attributed to "you". The full output transcribed nothing there.
//! So the words come from the linear output, and the full output only votes on whether each one
//! could be the user's.
//!
//! # The rule
//!
//! A word (or any timed span of the linear transcript) is **dropped** when both hold:
//!
//! 1. **Echo was possible:** in at least half of its 10 ms frames the far end had played within
//!    the last 300 ms (the reference above −60 dBFS; the room's tail lasts that long).
//! 2. **The full output heard no speech:** every voice-activity window of the full output from
//!    250 ms before the word to 250 ms after it is below 0.5.
//!
//! Every other word is kept, including any the tracks do not cover: the gate never drops a word
//! without evidence. The 250 ms pad absorbs ASR timestamp error and the VAD's 32 ms windows.
//!
//! The voice activity comes from the caller (the pipeline runs its VAD over each
//! [`EchoFrame::full`]); this crate never runs a model. Energy alone cannot stand in for it: on
//! S0.3's recordings the suppressor lets short loud bursts of residual echo through, and ducks
//! some of the user's words by more than 10 dB. Silero on the full output of the same recordings
//! (measured once, outside these tests) was at 0.5 or above in 0.0 % of the echo-only take's
//! far-end windows, against 82 % of the double-talk mix's near-end speech.

use ink_core::TimedText;

use crate::FRAME;
use crate::canceller::{EchoFrame, level_db};

/// The word gate's thresholds. The defaults are the rule in the module docs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GateConfig {
    /// A voice-activity window at or above this probability is speech.
    pub speech_threshold: f32,
    /// How far either side of a word the full output must be silent, ms.
    pub pad_ms: u64,
    /// The reference counts as playing above this level, dBFS.
    pub far_floor_db: f32,
    /// How long echo can follow the far end, ms.
    pub tail_ms: u64,
    /// The share of a word's frames in which echo must have been possible.
    pub min_echo_share: f64,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            speech_threshold: 0.5,
            pad_ms: 250,
            far_floor_db: -60.0,
            tail_ms: 300,
            min_echo_share: 0.5,
        }
    }
}

/// What the gate decided about one word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Kept: the full output heard speech around it.
    NearSpeech,
    /// Kept: the far end was quiet, so it cannot be echo.
    NoEcho,
    /// Kept: the tracks do not cover it (no evidence either way).
    NotCovered,
    /// Dropped: echo was possible and the full output heard no speech around it.
    EchoOnly,
}

impl Verdict {
    /// Whether the word stays in the transcript.
    pub fn keep(self) -> bool {
        self != Verdict::EchoOnly
    }
}

/// The evidence the gate reads, collected as the canceller's frames go by: the far end's
/// activity per 10 ms frame, and the voice activity of the full output per VAD window. Both are
/// on the mic's timeline from its first sample.
///
/// **Worker.** It keeps one byte per 10 ms frame and one float per VAD window (under 2 MB for an
/// hour), never audio.
#[derive(Clone, Debug)]
pub struct EchoGate {
    config: GateConfig,
    /// Per frame: the reference was above the floor.
    far: Vec<bool>,
    /// Samples per voice-activity window.
    vad_window: usize,
    /// Per VAD window of the full output: the speech probability.
    speech: Vec<f32>,
}

impl EchoGate {
    /// A gate whose voice activity comes in windows of `vad_window` samples (512 for Silero).
    pub fn new(config: GateConfig, vad_window: usize) -> Self {
        Self {
            config,
            far: Vec::new(),
            vad_window: vad_window.max(1),
            speech: Vec::new(),
        }
    }

    /// Records one canceller frame. Frames must come in order, from the first.
    pub fn push_frame(&mut self, frame: &EchoFrame) {
        let n = frame.len.clamp(1, FRAME);
        let playing = level_db(&frame.reference[..n]) > self.config.far_floor_db;
        let idx = frame.index as usize;
        if self.far.len() <= idx {
            self.far.resize(idx + 1, false);
        }
        self.far[idx] = playing;
    }

    /// Records the full output's speech probability for its next VAD window.
    pub fn push_speech(&mut self, probability: f32) {
        self.speech.push(probability);
    }

    /// The verdict on one span of the linear transcript, in ms from the mic's first sample.
    pub fn verdict(&self, start_ms: u64, end_ms: u64) -> Verdict {
        let end_ms = end_ms.max(start_ms + 1);
        // Frames the word covers.
        let f0 = (start_ms / 10) as usize;
        let f1 = end_ms.div_ceil(10) as usize;
        if f1 > self.far.len() {
            return Verdict::NotCovered;
        }
        let tail = (self.config.tail_ms / 10) as usize;
        let possible = (f0..f1)
            .filter(|&f| self.far[f.saturating_sub(tail)..=f].iter().any(|p| *p))
            .count();
        if (possible as f64) < self.config.min_echo_share * (f1 - f0) as f64 {
            return Verdict::NoEcho;
        }
        // VAD windows from `pad` before to `pad` after.
        let rate_per_ms = crate::RATE / 1000.0;
        let lo_ms = start_ms.saturating_sub(self.config.pad_ms);
        let hi_ms = end_ms + self.config.pad_ms;
        let w0 = (lo_ms as f64 * rate_per_ms) as usize / self.vad_window;
        let w1 = ((hi_ms as f64 * rate_per_ms) as usize).div_ceil(self.vad_window);
        // A pad reaching past the recorded windows is fine at the stream's end, but the word
        // itself must be covered.
        let word_end_w = ((end_ms as f64 * rate_per_ms) as usize).div_ceil(self.vad_window);
        if word_end_w > self.speech.len() {
            return Verdict::NotCovered;
        }
        let heard = self.speech[w0..w1.min(self.speech.len())]
            .iter()
            .any(|p| *p >= self.config.speech_threshold);
        if heard {
            Verdict::NearSpeech
        } else {
            Verdict::EchoOnly
        }
    }

    /// The verdicts on a transcript's words (or segments), in order.
    pub fn verdicts(&self, words: &[TimedText]) -> Vec<Verdict> {
        words
            .iter()
            .map(|w| self.verdict(w.start_ms, w.end_ms))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VAD: usize = 512;

    fn frame(index: u64, far_db: f32) -> EchoFrame {
        let amp = if far_db <= -199.0 {
            0.0
        } else {
            10f32.powf(far_db / 20.0)
        };
        EchoFrame {
            index,
            len: FRAME,
            mic: [0.0; FRAME],
            reference: [amp; FRAME],
            linear: [0.0; FRAME],
            full: [0.0; FRAME],
        }
    }

    /// 10 s: the far end plays 1–6 s; the full output's VAD hears speech 3–4 s.
    fn gate() -> EchoGate {
        let mut g = EchoGate::new(GateConfig::default(), VAD);
        for i in 0..1_000u64 {
            let playing = (100..600).contains(&i);
            g.push_frame(&frame(i, if playing { -30.0 } else { -200.0 }));
        }
        let windows = 10 * 16_000 / VAD;
        for w in 0..windows {
            let t = (w * VAD) as f64 / 16_000.0;
            g.push_speech(if (3.0..4.0).contains(&t) { 0.9 } else { 0.02 });
        }
        g
    }

    #[test]
    fn a_word_in_echo_with_a_silent_full_output_is_dropped() {
        assert_eq!(gate().verdict(1_500, 1_800), Verdict::EchoOnly);
        assert_eq!(gate().verdict(5_000, 5_400), Verdict::EchoOnly);
    }

    #[test]
    fn a_word_where_the_full_output_hears_speech_is_kept() {
        assert_eq!(gate().verdict(3_200, 3_500), Verdict::NearSpeech);
        // Within the pad of the speech, a word is kept too: timestamps are approximate.
        assert_eq!(gate().verdict(4_100, 4_200), Verdict::NearSpeech);
        // Past the pad it is not.
        assert_eq!(gate().verdict(4_400, 4_600), Verdict::EchoOnly);
    }

    #[test]
    fn a_word_with_the_far_end_quiet_is_never_echo() {
        assert_eq!(gate().verdict(7_000, 7_300), Verdict::NoEcho);
        // The room's tail: 200 ms after the far end stops, echo is still possible.
        assert_eq!(gate().verdict(6_000, 6_200), Verdict::EchoOnly);
        // 400 ms after, it is not.
        assert_eq!(gate().verdict(6_400, 6_700), Verdict::NoEcho);
    }

    #[test]
    fn a_word_the_tracks_do_not_cover_is_kept() {
        assert_eq!(gate().verdict(9_900, 10_300), Verdict::NotCovered);
        assert_eq!(
            EchoGate::new(GateConfig::default(), VAD).verdict(0, 300),
            Verdict::NotCovered
        );
    }

    #[test]
    fn verdicts_follow_the_words_in_order() {
        let w = |s, e| TimedText {
            start_ms: s,
            end_ms: e,
            text: String::from("w"),
        };
        let v = gate().verdicts(&[w(1_500, 1_800), w(3_200, 3_500), w(7_000, 7_300)]);
        assert_eq!(v, [Verdict::EchoOnly, Verdict::NearSpeech, Verdict::NoEcho]);
        assert_eq!(
            v.iter().map(|v| v.keep()).collect::<Vec<_>>(),
            [false, true, true]
        );
    }
}
