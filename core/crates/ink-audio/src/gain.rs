//! The per-utterance gain stage ahead of every engine (architecture rule 11).
//!
//! A built-in microphone array read through the raw HAL skips the voice processing that normally
//! lifts it, so ordinary speech arrives around −65 dBFS, and a recogniser handed that level returns
//! empty text (one did from about −70 dBFS RMS). [`normalise`] lifts a whole utterance (a
//! dictation take, an imported file's window) with one gain, so its dynamics survive. The meeting
//! path uses the slow [`Agc`](crate::Agc) toward the same [`TARGET_PEAK`].
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
//! - **Stationary audio:** a robust peak less than [`MIN_DYNAMICS`] above the take's quiet frames
//!   (its [`QUIET_PERCENTILE`]th-percentile frame). Room tone, hum and fan noise have no loud
//!   frames standing out from quiet ones; speech does. This guard is new in 1.0: the gain cap had
//!   to rise from 60× to [`MAX_GAIN`] for a −75 dBFS talker to reach the target, and without it a
//!   quiet room just above the floor would be lifted into full-level hiss.
//! - **Anything shorter than [`MIN_FRAMES`]:** too short to tell speech from noise.
//!
//! The gain is capped at [`MAX_GAIN`]. Samples are clamped to ±1.0 afterwards, so the rare
//! transient above the robust peak is shaved rather than wrapped: a shaved click is harmless where
//! an unamplified dictation is not.

/// The length of a level frame: 20 ms at 16 kHz. Every level in this crate is measured on these.
pub const LEVEL_FRAME: usize = 320;

/// The robust peak everything is lifted to (about −9 dBFS). Engines see speech at this level.
pub const TARGET_PEAK: f32 = 0.35;

/// A robust peak below this (−80 dBFS) is silence, not quiet speech, and is left alone. A −75 dBFS
/// RMS talker's robust peak sits around −63 dBFS, well above it.
pub const NOISE_FLOOR: f32 = 1.0e-4;

/// The largest gain applied (60 dB). A −75 dBFS RMS talker needs about 54 dB to reach the target.
pub const MAX_GAIN: f32 = 1000.0;

/// How many of the loudest frames the robust peak skips as possible transients. A keyboard click
/// spans one or two 20 ms frames; real speech spans dozens.
pub const TRANSIENT_FRAMES: usize = 8;

/// The percentile of frame peaks that stands for the take's quiet frames (its noise).
pub const QUIET_PERCENTILE: usize = 10;

/// How far (as a ratio, 6 dB) the robust peak must stand above the quiet frames for the take to
/// count as more than stationary noise. Frame peaks of steady noise stay within about 3 dB of each
/// other; speech's loud frames stand 20 dB or more above its pauses.
pub const MIN_DYNAMICS: f32 = 2.0;

/// The fewest level frames (200 ms) a buffer needs before it is judged at all.
pub const MIN_FRAMES: usize = 10;

/// Level statistics of a buffer, in linear full-scale units (1.0 = 0 dBFS).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    /// The robust peak: the loudest frame peak after skipping [`TRANSIENT_FRAMES`].
    pub robust_peak: f32,
    /// The [`QUIET_PERCENTILE`]th-percentile frame peak.
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

/// What [`normalise`] did.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum GainOutcome {
    /// The buffer was lifted by `gain` (at most [`MAX_GAIN`]).
    Applied {
        /// The linear gain.
        gain: f32,
    },
    /// Robust peak at or above the target: untouched.
    Healthy,
    /// Robust peak below [`NOISE_FLOOR`]: untouched.
    Silence,
    /// No loud frames stand out from the quiet ones (room tone, hum): untouched.
    Stationary,
    /// Fewer than [`MIN_FRAMES`] level frames: untouched.
    TooShort,
}

/// The result of [`normalise`], for logs: levels only, never audio content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GainReport {
    /// The buffer's levels before any gain.
    pub before: Levels,
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

/// Lifts one utterance (16 kHz mono) so its robust peak reaches [`TARGET_PEAK`], in place.
///
/// **Worker.** One gain for the whole buffer, so relative dynamics are preserved exactly, except
/// for samples the clamp to ±1.0 shaves. Allocates one `f32` per 20 ms frame to measure.
pub fn normalise(samples: &mut [f32]) -> GainReport {
    let before = levels(samples);
    let outcome = decide(&before);
    if let GainOutcome::Applied { gain } = outcome {
        for s in samples.iter_mut() {
            *s = (*s * gain).clamp(-1.0, 1.0);
        }
    }
    GainReport { before, outcome }
}

fn decide(levels: &Levels) -> GainOutcome {
    let peak = levels.robust_peak;
    if peak < NOISE_FLOOR {
        GainOutcome::Silence
    } else if peak >= TARGET_PEAK {
        GainOutcome::Healthy
    } else if levels.frames < MIN_FRAMES {
        GainOutcome::TooShort
    } else if peak < levels.quiet * MIN_DYNAMICS {
        GainOutcome::Stationary
    } else {
        GainOutcome::Applied {
            gain: (TARGET_PEAK / peak).min(MAX_GAIN),
        }
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
    fn db_helpers_round_trip() {
        assert!((to_dbfs(from_dbfs(-75.0)) + 75.0).abs() < 1e-4);
        assert!((to_dbfs(1.0)).abs() < 1e-6);
    }
}
