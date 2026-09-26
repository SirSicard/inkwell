//! The speech band, and the no-VAD fallback's test for a stationary room.
//!
//! With a VAD installed, the gain stages learn level from speech alone and this module is not
//! consulted (see [`gain`](crate::gain)). Without one (the model missing, or still downloading),
//! the fallback has to guess, and it errs toward lifting: losing a quiet dictation is worse than
//! transcribing noise, and the app says that voice detection is unavailable.
//!
//! The guess is made on the **speech band** (300 Hz–3.4 kHz, [`SpeechBandFilter`]) and its
//! **envelope**, the RMS of each 20 ms level frame. Audio is treated as a stationary room, and
//! left alone, when the envelope's robust peak stands less than
//! [`MIN_DYNAMICS_DB`](crate::gain::MIN_DYNAMICS_DB) (4 dB) above
//! its 10th-percentile frame (the same robust peak and quiet frame the gain stage uses). Anything
//! else is lifted.
//!
//! What that does, as the committed tests hold it:
//!
//! - White room tone and gently low-passed rumble are left alone
//!   (`fallback_leaves_white_room_tone_and_gentle_rumble_alone`,
//!   `agc_without_vad_never_lifts_white_room_tone_or_gentle_rumble`).
//! - Speech and breathy speech, syllables of 0.12 s to 0.26 s, clean or 8 dB over white noise or
//!   rumble, are lifted (`fallback_lifts_fast_and_slow_speech`,
//!   `agc_without_vad_lifts_fast_and_slow_speech`).
//! - Steeply low-passed rumble, rumble swinging slowly in level, a cycling fan and knocks can be
//!   lifted too (`fallback_can_lift_steep_and_swinging_rumble`,
//!   `agc_without_vad_can_lift_steep_and_swinging_rumble`). That is the price of the fallback,
//!   and why the pipeline uses the VAD-gated calls whenever a VAD is installed.

use std::f64::consts::TAU;
use std::ops::Range;

use ink_core::CANONICAL_RATE;

use crate::gain::{LEVEL_FRAME, QUIET_PERCENTILE, TRANSIENT_FRAMES};

/// The speech band's edges in Hz.
pub const SPEECH_BAND_HZ: Range<f32> = 300.0..3_400.0;

/// Envelope values are floored here (−180 dBFS) so digital silence has a finite ratio.
const ENVELOPE_FLOOR: f32 = 1.0e-9;

/// The speech-band filter: two 2nd-order Butterworth high-passes at 300 Hz (24 dB per octave below
/// the band) and one 2nd-order Butterworth low-pass at 3.4 kHz, at 16 kHz.
///
/// **Pump or worker.** Never allocates; its state carries across calls.
#[derive(Clone, Debug)]
pub struct SpeechBandFilter {
    sections: [Biquad; 3],
}

impl Default for SpeechBandFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechBandFilter {
    /// A filter at rest.
    pub fn new() -> Self {
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let low = f64::from(SPEECH_BAND_HZ.start);
        let high = f64::from(SPEECH_BAND_HZ.end);
        Self {
            sections: [
                Biquad::high_pass(low, q),
                Biquad::high_pass(low, q),
                Biquad::low_pass(high, q),
            ],
        }
    }

    /// Filters one sample.
    pub fn process(&mut self, x: f32) -> f32 {
        self.sections
            .iter_mut()
            .fold(f64::from(x), |acc, s| s.process(acc)) as f32
    }

    /// Returns the filter to rest.
    pub fn reset(&mut self) {
        self.sections.iter_mut().for_each(Biquad::reset);
    }
}

/// The speech-band envelope of `samples` (16 kHz mono): the RMS of the band-passed signal in each
/// level frame (the last may be partial). **Allocates** one `f32` per frame.
pub fn envelope(samples: &[f32]) -> Vec<f32> {
    let mut filter = SpeechBandFilter::new();
    samples
        .chunks(LEVEL_FRAME)
        .map(|frame| {
            let energy: f64 = frame
                .iter()
                .map(|&x| f64::from(filter.process(x)).powi(2))
                .sum();
            (energy / frame.len() as f64).sqrt() as f32
        })
        .collect()
}

/// An envelope's contrast: its robust peak over its 10th-percentile frame, in dB. **Allocates** a
/// copy to rank it.
pub fn contrast_db(envelope: &[f32]) -> f32 {
    let mut scratch = envelope.to_vec();
    contrast_in(envelope.iter().copied(), &mut scratch).0
}

/// The contrast of the envelope `frames` yields, ranking a copy in `scratch` (which must hold them
/// all), and the envelope's quiet (10th-percentile) frame. Allocation-free.
pub(crate) fn contrast_in(frames: impl Iterator<Item = f32>, scratch: &mut [f32]) -> (f32, f32) {
    let mut n = 0;
    for (slot, v) in scratch.iter_mut().zip(frames) {
        *slot = v.max(ENVELOPE_FLOOR);
        n += 1;
    }
    if n == 0 {
        return (0.0, 0.0);
    }
    let ranked = &mut scratch[..n];
    let loudest_first = |a: &f32, b: &f32| b.total_cmp(a);
    let robust = *ranked
        .select_nth_unstable_by(TRANSIENT_FRAMES.min(n - 1), loudest_first)
        .1;
    let quiet = *ranked
        .select_nth_unstable_by(n - 1 - n * QUIET_PERCENTILE / 100, loudest_first)
        .1;
    (20.0 * (robust / quiet).log10(), quiet)
}

/// One second-order section (RBJ cookbook, transposed direct form II), in `f64` so the 300 Hz
/// high-pass, whose poles sit close to the unit circle at 16 kHz, stays exact enough.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// A low-pass at `hz` with quality `q`, at 16 kHz.
    pub(crate) fn low_pass(hz: f64, q: f64) -> Self {
        let (cos, alpha) = Self::prewarp(hz, q);
        Self::normalised(
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    /// A high-pass at `hz` with quality `q`, at 16 kHz.
    pub(crate) fn high_pass(hz: f64, q: f64) -> Self {
        let (cos, alpha) = Self::prewarp(hz, q);
        Self::normalised(
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    fn prewarp(hz: f64, q: f64) -> (f64, f64) {
        let w0 = TAU * hz / f64::from(CANONICAL_RATE);
        (w0.cos(), w0.sin() / (2.0 * q))
    }

    fn normalised(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Filters one sample.
    pub(crate) fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Clears the state.
    pub(crate) fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gain::{from_dbfs, rms, to_dbfs};
    use crate::synth::tone;

    /// The filter's gain at `hz`, from a steady tone's RMS in and out (the first 0.1 s skipped).
    fn gain_db(hz: f64) -> f32 {
        let x = tone(0.5, hz, 0.5, CANONICAL_RATE);
        let mut f = SpeechBandFilter::new();
        let y: Vec<f32> = x.iter().map(|&s| f.process(s)).collect();
        to_dbfs(rms(&y[1_600..])) - to_dbfs(rms(&x[1_600..]))
    }

    #[test]
    fn the_filter_passes_the_speech_band_and_rejects_rumble() {
        // Butterworth sections: −6 dB at 300 Hz (two −3 dB sections) and −3 dB at 3.4 kHz.
        assert!(gain_db(1_000.0).abs() < 0.5, "{}", gain_db(1_000.0));
        assert!((gain_db(300.0) + 6.0).abs() < 0.5, "{}", gain_db(300.0));
        assert!((gain_db(3_400.0) + 3.0).abs() < 0.5, "{}", gain_db(3_400.0));
        assert!(gain_db(100.0) < -35.0, "{}", gain_db(100.0));
        assert!(gain_db(50.0) < -60.0, "{}", gain_db(50.0));
    }

    #[test]
    fn contrast_is_the_robust_peak_over_the_quiet_frame() {
        assert_eq!(contrast_db(&[0.01; 50]), 0.0);
        assert_eq!(contrast_db(&[]), 0.0);
        let two: Vec<f32> = (0..100u32)
            .map(|k| from_dbfs(if k.is_multiple_of(2) { -40.0 } else { -50.0 }))
            .collect();
        assert!((contrast_db(&two) - 10.0).abs() < 1e-3);
    }
}
