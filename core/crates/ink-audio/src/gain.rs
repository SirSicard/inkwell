//! The per-utterance gain stage ahead of every engine (architecture rule 11).
//!
//! A built-in microphone array read through the raw HAL skips the voice processing that normally
//! lifts it, so ordinary speech can arrive tens of dB below full scale, where a recogniser returns
//! empty text; the −75 dBFS RMS fixture is the case this crate is held to
//! (`minus_75_dbfs_rms_speech_reaches_the_target`). This stage lifts a whole utterance (a
//! dictation take, an imported file's window) with one gain, so its dynamics survive. The meeting
//! path uses the slow [`Agc`](crate::Agc) toward the same [`TARGET_PEAK`].
//!
//! # Two calls, one per situation
//!
//! - **[`normalise_speech`], whenever a VAD is installed.** The gain is learned from the speech in
//!   the take and nothing else (the WebRTC AGC2 pattern: level estimation gated by a VAD). A VAD
//!   cannot hear −75 dBFS speech either, so the flow is: a provisional gain from the whole take's
//!   robust peak ([`provisional_gain`]); the VAD over the take lifted by it; the final gain from
//!   the robust peak of the original's speech frames only ([`speech_levels`]). A take in which the
//!   VAD finds no speech gets no gain and is reported as [`GainOutcome::NoSpeech`]: the caller
//!   discards it. Rumble, a fan or knocks never set the level, whatever their shape.
//! - **[`normalise_without_vad`], only while no VAD is available** (the model missing, or still
//!   downloading), and only while the app says that voice detection is unavailable. It guesses on
//!   the speech band's contrast ([`speech_band`]) and errs toward lifting: losing a quiet
//!   dictation is worse than transcribing noise. It can lift rumble and other non-speech.
//!
//! # The level it keys on
//!
//! The gain is keyed to a **robust peak**: the loudest 20 ms frame peak after skipping the
//! [`TRANSIENT_FRAMES`] loudest. Not the absolute peak, which on a push-to-talk app is routinely
//! the hotkey's own click: one full-scale transient once made an earlier implementation decide a
//! take was loud enough and leave −60 dBFS speech untouched. And not a percentile of all frames,
//! which ties the level to how much silence surrounds the speech: a long take with a short
//! utterance in it read as room tone and got its speech amplified into a square wave. Skipping a
//! fixed handful of frames ignores clicks whatever the speech-to-silence ratio, and lands inside
//! the speech for any utterance longer than a fraction of a second.
//!
//! # What it leaves alone
//!
//! - **Silence:** a robust peak under [`NOISE_FLOOR`]. There is nothing to rescue, and the VAD
//!   after this stage should still see silence.
//! - **Healthy audio:** a robust peak at or above the target. Never attenuated.
//! - **Anything shorter than [`MIN_FRAMES`]:** too short to judge.
//! - **Stationary audio:** room tone, hum, rumble. Judged on the speech band, not on the level:
//!   see the next section.
//!
//! The gain is capped at [`MAX_GAIN`]. Samples are clamped to ±1.0 afterwards, so the rare
//! transient above the robust peak is shaved rather than wrapped: a shaved click is harmless where
//! an unamplified dictation is not.
//!
//! # Why the VAD, not a level heuristic
//!
//! The cap had to rise from 60× to [`MAX_GAIN`] for a −75 dBFS talker to reach the target, and at
//! that gain anything a quiet room holds becomes loud. Three heuristics that judged "a room" from
//! level statistics each met a counterexample in review (steeply filtered rumble, rumble swinging
//! slowly in level, fast breathy speech). A VAD's job is exactly that judgement, so the level is
//! learned only where it says speech.

use std::ops::Range;

use ink_core::EngineError;

use crate::speech_band;
use crate::vad::{self, Segment, SpeechProbability, VAD_WINDOW, VadConfig};

/// The length of a level frame: 20 ms at 16 kHz. Every level in this crate is measured on these.
pub const LEVEL_FRAME: usize = 320;

/// The robust peak everything is lifted to (about −9 dBFS). Engines see speech at this level.
pub const TARGET_PEAK: f32 = 0.35;

/// A robust peak below this (−80 dBFS) is silence, not quiet speech, and is left alone. The
/// −75 dBFS RMS speech fixture's robust peak sits 9–14 dB above its RMS, well above it.
pub const NOISE_FLOOR: f32 = 1.0e-4;

/// The largest gain applied (60 dB), enough to bring the −75 dBFS RMS speech fixture to the target
/// (`minus_75_dbfs_rms_speech_reaches_the_target`).
pub const MAX_GAIN: f32 = 1000.0;

/// How many of the loudest frames the robust peak skips as possible transients. A keyboard click
/// spans one or two 20 ms frames; real speech spans dozens.
pub const TRANSIENT_FRAMES: usize = 8;

/// The percentile of frame peaks that stands for the take's quiet frames (its noise).
pub const QUIET_PERCENTILE: usize = 10;

/// How far the speech band's envelope must stand above its quiet frames (as a ratio:
/// [`MIN_DYNAMICS_DB`]) for the no-VAD fallback to lift audio at all (see [`speech_band`]).
pub const MIN_DYNAMICS: f32 = 1.584_893_2;

/// [`MIN_DYNAMICS`] in dB: 4 dB.
pub const MIN_DYNAMICS_DB: f32 = 4.0;

/// The fewest level frames (200 ms) a buffer needs before it is judged at all.
pub const MIN_FRAMES: usize = 10;

/// Level statistics of a buffer, in linear full-scale units (1.0 = 0 dBFS).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    /// The robust peak: the loudest frame peak after skipping [`TRANSIENT_FRAMES`].
    pub robust_peak: f32,
    /// The [`QUIET_PERCENTILE`]th-percentile frame peak (for reference: the stationary guard
    /// measures the speech band's envelope instead).
    pub quiet: f32,
    /// Level frames measured (the last may be partial).
    pub frames: usize,
}

/// Measures `samples` (16 kHz mono) in [`LEVEL_FRAME`]s.
///
/// **Worker or pump.** Allocates one `f32` per frame.
pub fn levels(samples: &[f32]) -> Levels {
    let mut peaks: Vec<f32> = samples.chunks(LEVEL_FRAME).map(frame_peak).collect();
    let frames = peaks.len();
    if frames == 0 {
        return Levels {
            robust_peak: 0.0,
            quiet: 0.0,
            frames: 0,
        };
    }
    // Loudest first. Frame peaks are never NaN (see `frame_peak`), so the order is total.
    peaks.sort_unstable_by(|a, b| b.total_cmp(a));
    Levels {
        robust_peak: peaks[TRANSIENT_FRAMES.min(frames - 1)],
        quiet: peaks[frames - 1 - frames * QUIET_PERCENTILE / 100],
        frames,
    }
}

/// The robust peak of `samples` (16 kHz mono). See the module docs.
pub fn robust_peak(samples: &[f32]) -> f32 {
    levels(samples).robust_peak
}

/// The largest absolute sample. A NaN sample is ignored (`f32::max` drops it), so the result is
/// never NaN.
pub(crate) fn frame_peak(frame: &[f32]) -> f32 {
    frame.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Root mean square of `samples`, accumulated in `f64`. Zero for an empty slice.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// A linear full-scale amplitude in dBFS. Zero gives negative infinity.
pub fn to_dbfs(amplitude: f32) -> f32 {
    20.0 * amplitude.log10()
}

/// A dBFS level as a linear full-scale amplitude.
pub fn from_dbfs(dbfs: f32) -> f32 {
    10f32.powf(dbfs / 20.0)
}

/// What a normalise call did.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum GainOutcome {
    /// The buffer was lifted by `gain` (at most [`MAX_GAIN`]).
    Applied {
        /// The linear gain.
        gain: f32,
    },
    /// The level it keys on is at or above the target: untouched.
    Healthy,
    /// Robust peak below [`NOISE_FLOOR`]: untouched.
    Silence,
    /// Fewer than [`MIN_FRAMES`] level frames: untouched.
    TooShort,
    /// [`normalise_speech`]: the VAD found no speech. Untouched, and the caller discards the take:
    /// no engine should see it.
    NoSpeech,
    /// [`normalise_without_vad`]: the speech band is stationary (room tone, hum): untouched.
    Stationary,
}

/// What a normalise call measured, beyond the take's levels.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GainEvidence {
    /// Nothing: the take was silent or too short to judge.
    NotJudged,
    /// [`normalise_speech`]: what the VAD found.
    Vad {
        /// The provisional gain the VAD heard the take through.
        provisional_gain: f32,
        /// Level frames of the take that lie in speech.
        speech_frames: usize,
        /// The range to keep, as [`trim_ends`](crate::trim_ends) would give it for the lifted
        /// copy. `None` exactly when no speech was found.
        speech: Option<Range<usize>>,
    },
    /// [`normalise_without_vad`]: the speech band's contrast, in dB.
    NoVad {
        /// Robust peak over the 10th-percentile frame of the band envelope.
        band_contrast_db: f32,
    },
}

/// The result of a normalise call, for logs: levels and counts only, never audio content.
#[derive(Clone, Debug, PartialEq)]
pub struct GainReport {
    /// The whole take's levels before any gain.
    pub before: Levels,
    /// What was measured to decide.
    pub evidence: GainEvidence,
    /// What was done.
    pub outcome: GainOutcome,
}

impl GainReport {
    /// The gain that was applied (1.0 when the buffer was left alone).
    pub fn gain(&self) -> f32 {
        match self.outcome {
            GainOutcome::Applied { gain } => gain,
            _ => 1.0,
        }
    }
}

/// The gain that takes `robust_peak` to [`TARGET_PEAK`]: capped at [`MAX_GAIN`], never below 1.
pub fn gain_for(robust_peak: f32) -> f32 {
    if robust_peak >= TARGET_PEAK {
        1.0
    } else if robust_peak > 0.0 {
        (TARGET_PEAK / robust_peak).min(MAX_GAIN)
    } else {
        MAX_GAIN
    }
}

/// The provisional gain a VAD hears a take through: [`gain_for`] the whole take's robust peak,
/// with no judgement of what the take holds.
pub fn provisional_gain(levels: &Levels) -> f32 {
    gain_for(levels.robust_peak)
}

/// Multiplies `samples` by `gain`, clamping to ±1.0. Allocation-free.
pub fn apply_gain(samples: &mut [f32], gain: f32) {
    for s in samples.iter_mut() {
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
}

/// The levels of the level frames of `samples` that lie in `segments` (a frame belongs to a
/// segment when its middle sample does), measured as [`levels`] measures a whole buffer. `None`
/// when no frame does. **Allocates** one `f32` per speech frame.
pub fn speech_levels(samples: &[f32], segments: &[Segment]) -> Option<Levels> {
    let in_speech = |frame: usize| {
        let middle = frame * LEVEL_FRAME + LEVEL_FRAME / 2;
        segments
            .iter()
            .any(|s| (s.start * VAD_WINDOW..s.end * VAD_WINDOW).contains(&middle))
    };
    let mut peaks: Vec<f32> = samples
        .chunks(LEVEL_FRAME)
        .enumerate()
        .filter(|&(k, _)| in_speech(k))
        .map(|(_, f)| frame_peak(f))
        .collect();
    let frames = peaks.len();
    if frames == 0 {
        return None;
    }
    peaks.sort_unstable_by(|a, b| b.total_cmp(a));
    Some(Levels {
        robust_peak: peaks[TRANSIENT_FRAMES.min(frames - 1)],
        quiet: peaks[frames - 1 - frames * QUIET_PERCENTILE / 100],
        frames,
    })
}

/// **The call to use whenever a VAD is installed.** Lifts one utterance (16 kHz mono) in place so
/// the robust peak of its speech reaches [`TARGET_PEAK`], learning the level from speech alone.
///
/// The flow (module docs): [`provisional_gain`] from the whole take; `vad` over the take lifted
/// by it; the final gain from [`speech_levels`] of the original. No speech means no gain and
/// [`GainOutcome::NoSpeech`]: discard the take. The report's evidence carries the range to keep
/// from the same VAD pass, so the caller need not run the VAD again to trim.
///
/// **Worker.** One gain for the whole buffer, so relative dynamics are preserved exactly, except
/// for samples the clamp to ±1.0 shaves. Allocates a lifted copy of the take and a few `f32`s per
/// frame. A VAD error is returned, never guessed around.
pub fn normalise_speech(
    samples: &mut [f32],
    vad: &mut dyn SpeechProbability,
    cfg: &VadConfig,
) -> Result<GainReport, EngineError> {
    let before = levels(samples);
    if let Some(outcome) = unjudgeable(&before) {
        return Ok(GainReport {
            before,
            evidence: GainEvidence::NotJudged,
            outcome,
        });
    }
    let provisional = provisional_gain(&before);
    let mut heard = samples.to_vec();
    apply_gain(&mut heard, provisional);
    let segments = vad::speech_segments(&heard, vad, cfg)?;
    let speech = speech_levels(samples, &segments);
    let evidence = GainEvidence::Vad {
        provisional_gain: provisional,
        speech_frames: speech.map_or(0, |l| l.frames),
        speech: vad::keep_range(&segments, samples.len(), cfg),
    };
    let outcome = match speech {
        None => GainOutcome::NoSpeech,
        Some(l) if l.robust_peak >= TARGET_PEAK => GainOutcome::Healthy,
        Some(l) => {
            let gain = gain_for(l.robust_peak);
            apply_gain(samples, gain);
            GainOutcome::Applied { gain }
        }
    };
    Ok(GainReport {
        before,
        evidence,
        outcome,
    })
}

/// **The fallback, only while no VAD is available** (the model missing, or still downloading),
/// with the app saying that voice detection is unavailable. Lifts one utterance (16 kHz mono) in
/// place so its robust peak reaches [`TARGET_PEAK`], unless its speech band is stationary
/// ([`speech_band`]). It errs toward lifting, and can lift rumble and other non-speech.
///
/// **Worker.** One gain for the whole buffer. Allocates a few `f32`s per 20 ms frame.
pub fn normalise_without_vad(samples: &mut [f32]) -> GainReport {
    let before = levels(samples);
    if let Some(outcome) = unjudgeable(&before) {
        return GainReport {
            before,
            evidence: GainEvidence::NotJudged,
            outcome,
        };
    }
    let band_contrast_db = speech_band::contrast_db(&speech_band::envelope(samples));
    let outcome = if before.robust_peak >= TARGET_PEAK {
        GainOutcome::Healthy
    } else if band_contrast_db < MIN_DYNAMICS_DB {
        GainOutcome::Stationary
    } else {
        let gain = gain_for(before.robust_peak);
        apply_gain(samples, gain);
        GainOutcome::Applied { gain }
    };
    GainReport {
        before,
        evidence: GainEvidence::NoVad { band_contrast_db },
        outcome,
    }
}

/// Takes neither call judges: silent, or too short.
fn unjudgeable(levels: &Levels) -> Option<GainOutcome> {
    if levels.robust_peak < NOISE_FLOOR {
        Some(GainOutcome::Silence)
    } else if levels.frames < MIN_FRAMES {
        Some(GainOutcome::TooShort)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_of_nothing_are_zero() {
        let l = levels(&[]);
        assert_eq!((l.robust_peak, l.quiet, l.frames), (0.0, 0.0, 0));
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn robust_peak_skips_exactly_the_transient_frames() {
        // Twenty frames with peaks 1..=20 (in thousandths): skipping the eight loudest leaves 12.
        let mut samples = Vec::new();
        for k in 1..=20 {
            let mut frame = vec![0.0f32; LEVEL_FRAME];
            frame[7] = k as f32 / 1000.0;
            samples.extend(frame);
        }
        let l = levels(&samples);
        assert_eq!(l.robust_peak, 0.012);
        // The 10th percentile of 20 frames sits 2 places above the quietest: the third quietest.
        assert_eq!(l.quiet, 0.003);
        assert_eq!(l.frames, 20);
    }

    #[test]
    fn a_nan_sample_does_not_poison_the_level() {
        assert_eq!(frame_peak(&[0.1, f32::NAN, -0.2]), 0.2);
    }

    #[test]
    fn the_dynamics_ratio_is_its_db_value() {
        assert!((to_dbfs(MIN_DYNAMICS) - MIN_DYNAMICS_DB).abs() < 1e-5);
    }

    #[test]
    fn db_helpers_round_trip() {
        assert!((to_dbfs(from_dbfs(-75.0)) + 75.0).abs() < 1e-4);
        assert!((to_dbfs(1.0)).abs() < 1e-6);
    }
}
