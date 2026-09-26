//! The speech band, and whether the audio in it is stationary.
//!
//! Both gain stages lift quiet audio to a target, and neither may lift a stationary room: room
//! tone, hum, HVAC rumble, a fridge. They decide that here, on the **speech band** (300 Hz–3.4 kHz,
//! [`SpeechBandFilter`]), where rumble below it contributes almost nothing and speech keeps its
//! syllables, and on the band's **envelope**: the RMS of each 20 ms level frame.
//!
//! Audio is **stationary** unless its envelope does both of these ([`Stationarity`]):
//!
//! - **stands out**: the robust peak of the envelope is at least [`MIN_DYNAMICS_DB`] (4 dB) above
//!   its 10th-percentile frame (the same robust peak and quiet frame the gain stage uses), and
//! - **moves in runs**: consecutive frames are correlated, lag-1 autocorrelation of the log
//!   envelope at least [`MIN_CORRELATION`] (0.6). Speech's loud frames come in syllables, many
//!   frames long. A stationary noise's frames are loud or quiet at random, one frame to the next,
//!   however large its swings: rumble low-passed steeply has little left in the band but a narrow
//!   skirt, whose frames swing by several dB, uncorrelated. Contrast alone lifted it.
//!
//! Neither condition asks whether the audio is speech. A cycling fan (loud and quiet for a second
//! at a time) moves in runs and stands out, and is lifted; knocks can be. What is speech is the
//! VAD's call, downstream.
//!
//! The committed tests hold this to account: rumble at 100, 250, 500 and 1000 Hz, gentle and steep,
//! and white room tone are never lifted (`rumble_takes_are_not_lifted`, `agc_never_lifts_rumble`,
//! `agc_never_lifts_a_long_stretch_of_room_tone`); breathy and ordinary speech 8 dB over white
//! noise or 100 Hz rumble always are (`speech_in_noise_and_over_rumble_at_8_db_snr_is_lifted_to_the_target`,
//! `agc_lifts_speech_in_noise_and_over_rumble_to_the_target`); and each condition's edge has its
//! own test (`the_dynamics_guard_stops_lifting_below_4_db_of_contrast`,
//! `the_guard_needs_the_envelope_to_move_in_runs`).
//!
//! Only the decision is made on the band. The level the gain is keyed to stays the full-band
//! robust peak.

use std::f64::consts::TAU;
use std::ops::Range;

use ink_core::CANONICAL_RATE;

use crate::gain::{LEVEL_FRAME, MIN_DYNAMICS_DB, QUIET_PERCENTILE, TRANSIENT_FRAMES};

/// The speech band's edges in Hz.
pub const SPEECH_BAND_HZ: Range<f32> = 300.0..3_400.0;

/// The lag-1 autocorrelation of the log band envelope that non-stationary audio reaches.
pub const MIN_CORRELATION: f32 = 0.6;

/// Envelope values are floored here (−180 dBFS) so digital silence has a finite logarithm.
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

/// How an envelope moves. See the module docs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stationarity {
    /// The envelope's robust peak over its 10th-percentile frame, in dB.
    pub contrast_db: f32,
    /// Lag-1 autocorrelation of the log envelope (−1 to 1; 0 when it does not move at all).
    pub correlation: f32,
}

impl Stationarity {
    /// Measures an envelope (frames in time order). **Allocates** a copy to rank it.
    pub fn of_envelope(envelope: &[f32]) -> Self {
        let mut scratch = envelope.to_vec();
        Self::measure(envelope.iter().copied(), &mut scratch).0
    }

    /// Whether the audio is stationary: its envelope does not both stand out and move in runs.
    pub fn is_stationary(&self) -> bool {
        self.contrast_db < MIN_DYNAMICS_DB || self.correlation < MIN_CORRELATION
    }

    /// Measures the envelope `frames` yields in time order, ranking a copy in `scratch` (which must
    /// hold them all). Returns the measure and the envelope's quiet (10th-percentile) frame.
    /// Allocation-free.
    pub(crate) fn measure(
        frames: impl Iterator<Item = f32> + Clone,
        scratch: &mut [f32],
    ) -> (Self, f32) {
        let mut n = 0;
        for (slot, v) in scratch.iter_mut().zip(frames.clone()) {
            *slot = v.max(ENVELOPE_FLOOR);
            n += 1;
        }
        if n == 0 {
            let flat = Self {
                contrast_db: 0.0,
                correlation: 0.0,
            };
            return (flat, 0.0);
        }
        let ranked = &mut scratch[..n];
        let loudest_first = |a: &f32, b: &f32| b.total_cmp(a);
        let robust = *ranked
            .select_nth_unstable_by(TRANSIENT_FRAMES.min(n - 1), loudest_first)
            .1;
        let quiet = *ranked
            .select_nth_unstable_by(n - 1 - n * QUIET_PERCENTILE / 100, loudest_first)
            .1;
        let contrast_db = 20.0 * (robust / quiet).log10();

        let log = |v: f32| f64::from(v.max(ENVELOPE_FLOOR)).ln();
        let mean = frames.clone().map(log).sum::<f64>() / n as f64;
        let variance: f64 = frames.clone().map(|v| (log(v) - mean).powi(2)).sum();
        let lagged: f64 = frames
            .clone()
            .zip(frames.skip(1))
            .map(|(a, b)| (log(a) - mean) * (log(b) - mean))
            .sum();
        // A flat envelope (a steady tone, digital silence) does not move: correlation 0.
        let correlation = if variance > 1e-12 {
            (lagged / variance) as f32
        } else {
            0.0
        };
        (
            Self {
                contrast_db,
                correlation,
            },
            quiet,
        )
    }
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
    use crate::gain::{from_dbfs, to_dbfs};
    use crate::synth::tone;

    /// The filter's gain at `hz`, from a steady tone's RMS in and out (the first 0.1 s skipped).
    fn gain_db(hz: f64) -> f32 {
        let x = tone(0.5, hz, 0.5, CANONICAL_RATE);
        let mut f = SpeechBandFilter::new();
        let y: Vec<f32> = x.iter().map(|&s| f.process(s)).collect();
        to_dbfs(crate::gain::rms(&y[1_600..])) - to_dbfs(crate::gain::rms(&x[1_600..]))
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
    fn a_flat_envelope_is_stationary_and_uncorrelated() {
        let s = Stationarity::of_envelope(&[0.01; 50]);
        assert_eq!((s.contrast_db, s.correlation), (0.0, 0.0));
        assert!(s.is_stationary());
        assert!(Stationarity::of_envelope(&[]).is_stationary());
    }

    #[test]
    fn runs_correlate_and_flicker_anticorrelates() {
        let level = |loud: bool| {
            if loud {
                from_dbfs(-40.0)
            } else {
                from_dbfs(-50.0)
            }
        };
        let runs: Vec<f32> = (0..100u32)
            .map(|k| level((k / 5).is_multiple_of(2)))
            .collect();
        let flicker: Vec<f32> = (0..100u32).map(|k| level(k.is_multiple_of(2))).collect();
        let r = Stationarity::of_envelope(&runs);
        let f = Stationarity::of_envelope(&flicker);
        assert!((r.contrast_db - 10.0).abs() < 1e-3 && (f.contrast_db - 10.0).abs() < 1e-3);
        assert!(r.correlation > 0.5 && !r.is_stationary(), "{r:?}");
        assert!(f.correlation < -0.9 && f.is_stationary(), "{f:?}");
    }
}
