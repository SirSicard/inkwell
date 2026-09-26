//! Band-limited interpolation at fractional, slowly moving positions: the drift-cancelling
//! resample.
//!
//! The far end is read at positions that advance by `1 / (1 + drift)` per output sample, so its
//! clock lands on the mic's. Linear interpolation would not do: its low-pass depends on the
//! fractional position, which drifts slowly, so the echo path the canceller has to learn would
//! wobble at the drift rate and cap the cancellation. A Kaiser-windowed sinc keeps the
//! interpolation error near −90 dB whatever the fraction.
//!
//! Both streams are already 16 kHz here (resampling happens once, in `ink-audio`), so the kernel
//! cuts at 95 % of Nyquist: the canonical resampler has already removed what lies above, and the
//! response is then flat and the same at every fractional position.

use std::f64::consts::PI;

/// Zero crossings each side of the kernel's centre.
const HALF_ZC: usize = 16;
/// Table entries per zero crossing.
const OVERSAMPLE: usize = 1024;
/// The Kaiser window's shape: about 90 dB of stopband.
const KAISER_BETA: f64 = 9.0;
/// The low-pass, as a fraction of Nyquist.
pub(crate) const CUTOFF: f64 = 0.95;

/// Input samples the kernel reaches on each side of a position.
pub(crate) const REACH: usize = (HALF_ZC as f64 / CUTOFF) as usize + 1;

/// A tabulated Kaiser-windowed sinc, built once per canceller (130 KB).
pub(crate) struct Interpolator {
    /// kernel(x) for x in 0..=HALF_ZC in steps of 1/OVERSAMPLE; the kernel is symmetric.
    table: Vec<f64>,
}

/// The zeroth-order modified Bessel function of the first kind, by its power series. It converges
/// fast for the arguments used here (at most `KAISER_BETA`).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..60 {
        let k = f64::from(k);
        term *= q / (k * k);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

impl Interpolator {
    pub(crate) fn new() -> Self {
        let n = HALF_ZC * OVERSAMPLE + 2;
        let i0b = bessel_i0(KAISER_BETA);
        let table = (0..n)
            .map(|i| {
                let x = i as f64 / OVERSAMPLE as f64;
                if x >= HALF_ZC as f64 {
                    return 0.0;
                }
                let sinc = if x == 0.0 {
                    1.0
                } else {
                    (PI * x).sin() / (PI * x)
                };
                let u = x / HALF_ZC as f64;
                sinc * bessel_i0(KAISER_BETA * (1.0 - u * u).max(0.0).sqrt()) / i0b
            })
            .collect();
        Self { table }
    }

    #[inline]
    fn kernel(&self, x: f64) -> f64 {
        let ax = x.abs() * OVERSAMPLE as f64;
        // Truncation is the table lookup: ax is non-negative and finite here.
        let i = ax as usize;
        if i + 1 >= self.table.len() {
            return 0.0;
        }
        let f = ax - i as f64;
        self.table[i] * (1.0 - f) + self.table[i + 1] * f
    }

    /// The value of `x` at fractional position `t` (in samples of `x`), low-passed at
    /// [`CUTOFF`]. Positions near or past the ends see zeros beyond them.
    #[inline]
    pub(crate) fn sample_at(&self, x: &[f32], t: f64) -> f32 {
        let reach = HALF_ZC as f64 / CUTOFF;
        let lo = (t - reach).ceil().max(0.0);
        let hi = (t + reach).floor().min(x.len() as f64 - 1.0);
        if hi < lo {
            return 0.0;
        }
        // Both bounds are whole, non-negative and inside `x` after the clamps above.
        let (lo, hi) = (lo as usize, hi as usize);
        let mut acc = 0.0f64;
        for (k, &v) in x.iter().enumerate().take(hi + 1).skip(lo) {
            acc += f64::from(v) * self.kernel(CUTOFF * (t - k as f64));
        }
        (acc * CUTOFF) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, hz: f64, phase: f64) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * hz * i as f64 / 16_000.0 + phase).sin() as f32)
            .collect()
    }

    fn error_db(y: &[f32], expect: &[f32]) -> f64 {
        let (mut e, mut s) = (0.0, 0.0);
        for i in 200..y.len() - 200 {
            e += f64::from(y[i] - expect[i]).powi(2);
            s += f64::from(expect[i]).powi(2);
        }
        10.0 * (e / s).log10()
    }

    #[test]
    fn a_fractional_delay_of_a_tone_is_exact_to_minus_80_db() {
        let x = tone(16_000, 1_000.0, 0.0);
        let interp = Interpolator::new();
        let d = 0.37;
        let y: Vec<f32> = (0..x.len())
            .map(|m| interp.sample_at(&x, m as f64 - d))
            .collect();
        let expect = tone(16_000, 1_000.0, -2.0 * PI * 1_000.0 * d / 16_000.0);
        let err = error_db(&y, &expect);
        assert!(err < -80.0, "error {err:.1} dB");
    }

    #[test]
    fn a_drifting_read_follows_the_drift_to_minus_80_db() {
        // Read a 1 kHz tone at positions m / (1 + 50 ppm): the result is the tone at 1 kHz / (1 +
        // 50 ppm), whatever the fraction does along the way.
        let eps = 50e-6;
        let x = tone(32_000, 1_000.0, 0.0);
        let interp = Interpolator::new();
        let y: Vec<f32> = (0..30_000)
            .map(|m| interp.sample_at(&x, m as f64 / (1.0 + eps)))
            .collect();
        let expect = tone(30_000, 1_000.0 / (1.0 + eps), 0.0);
        let err = error_db(&y, &expect);
        assert!(err < -80.0, "error {err:.1} dB");
    }

    /// The response is flat through the speech band and the same at every fractional position,
    /// so a slow drift of the fraction cannot modulate the echo path. The top 400 Hz, which the
    /// canonical resampler has already removed, is cut.
    #[test]
    fn the_response_is_flat_and_the_same_at_every_fraction() {
        let interp = Interpolator::new();
        let rms = |v: &[f32]| {
            (v[500..v.len() - 500]
                .iter()
                .map(|s| f64::from(*s).powi(2))
                .sum::<f64>()
                / (v.len() - 1000) as f64)
                .sqrt()
        };
        let gain_db = |hz: f64, frac: f64| {
            let x = tone(16_000, hz, 0.0);
            let y: Vec<f32> = (0..x.len())
                .map(|m| interp.sample_at(&x, m as f64 + frac))
                .collect();
            20.0 * (rms(&y) / std::f64::consts::FRAC_1_SQRT_2).log10()
        };
        for hz in [300.0, 1_000.0, 3_000.0, 6_000.0, 7_000.0] {
            let gains: Vec<f64> = [0.0, 0.1, 0.25, 0.5, 0.9]
                .iter()
                .map(|&frac| gain_db(hz, frac))
                .collect();
            let (lo, hi) = gains
                .iter()
                .fold((f64::MAX, f64::MIN), |(lo, hi), g| (lo.min(*g), hi.max(*g)));
            assert!(
                hi - lo < 0.01,
                "{hz} Hz varies with the fraction: {gains:?}"
            );
            if hz <= 6_000.0 {
                assert!(lo.abs() < 0.01 && hi.abs() < 0.01, "{hz} Hz: {gains:?}");
            }
        }
        let top = gain_db(7_950.0, 0.5);
        assert!(top < -20.0, "7.95 kHz passes at {top:.1} dB");
    }

    #[test]
    fn positions_past_the_ends_fade_to_zero_without_panicking() {
        let interp = Interpolator::new();
        let x = vec![1.0f32; 100];
        assert_eq!(interp.sample_at(&x, -1_000.0), 0.0);
        assert_eq!(interp.sample_at(&x, 1_000.0), 0.0);
        assert_eq!(interp.sample_at(&[], 3.0), 0.0);
        assert!((interp.sample_at(&x, 50.25) - 1.0).abs() < 1e-3);
    }
}
