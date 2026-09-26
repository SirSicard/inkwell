//! Deterministic synthetic signals for tests, benches and later steps' fixtures.
//!
//! Real audio never enters the repository, so every level test runs on these. They are pure
//! functions of their arguments: the same call gives the same samples on every OS (up to the last
//! bit of `sin`, which is why tests compare with tolerances).

use std::f64::consts::TAU;

use ink_core::CANONICAL_RATE;

/// A seeded PCG-style generator: 64-bit LCG state, 31-bit output. Not for anything but test signals.
#[derive(Clone, Debug)]
pub struct Lcg(u64);

impl Lcg {
    /// A generator for `seed`.
    pub fn new(seed: u64) -> Self {
        Self(
            seed.wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407),
        )
    }

    /// The next value in `0.0..1.0`.
    pub fn next_f64(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as f64 / (1u64 << 31) as f64
    }

    /// An approximately Gaussian value, mean 0, standard deviation 1 (sum of twelve uniforms).
    pub fn next_gaussian(&mut self) -> f64 {
        (0..12).map(|_| self.next_f64()).sum::<f64>() - 6.0
    }
}

/// Speech-like audio at 16 kHz: voiced syllables of 160–260 ms (a harmonic series on a gliding
/// 100–180 Hz fundamental, falling 6 dB per octave, under a smooth sin² envelope) separated by
/// 40–100 ms of digital silence. Scaled so the whole buffer's RMS is `rms_dbfs`.
///
/// Its loud frames are even, so its robust peak sits about 11.5 dB above its RMS.
pub fn speech_like(seconds: f64, rms_dbfs: f32, seed: u64) -> Vec<f32> {
    let rate = f64::from(CANONICAL_RATE);
    let total = (seconds * rate).round() as usize;
    let mut rng = Lcg::new(seed);
    let mut out: Vec<f64> = Vec::with_capacity(total + CANONICAL_RATE as usize);
    while out.len() < total {
        let dur = 0.16 + 0.10 * rng.next_f64();
        let gap = 0.04 + 0.06 * rng.next_f64();
        let f0 = 100.0 + 80.0 * rng.next_f64();
        let n = (dur * rate) as usize;
        let mut phase = 0.0f64;
        for i in 0..n {
            let t = i as f64 / rate;
            // A 10 % upward glide across the syllable.
            phase += TAU * f0 * (1.0 + 0.1 * t / dur) / rate;
            let mut v = 0.0;
            let mut k = 1.0;
            while k * f0 * 1.1 < 3_800.0 {
                v += (k * phase).sin() / k;
                k += 1.0;
            }
            let env = (std::f64::consts::PI * t / dur).sin().powi(2);
            out.push(v * env);
        }
        out.extend(std::iter::repeat_n(0.0, (gap * rate) as usize));
    }
    out.truncate(total);
    scale_to_rms(&out, rms_dbfs)
}

/// Gaussian noise at 16 kHz with the given RMS (room tone, hiss).
pub fn noise(seconds: f64, rms_dbfs: f32, seed: u64) -> Vec<f32> {
    let total = (seconds * f64::from(CANONICAL_RATE)).round() as usize;
    let mut rng = Lcg::new(seed);
    let raw: Vec<f64> = (0..total).map(|_| rng.next_gaussian()).collect();
    scale_to_rms(&raw, rms_dbfs)
}

/// A sine at `hz` and `rate`, with peak amplitude `amplitude`.
pub fn tone(seconds: f64, hz: f64, amplitude: f32, rate: u32) -> Vec<f32> {
    let total = (seconds * f64::from(rate)).round() as usize;
    (0..total)
        .map(|i| ((TAU * hz * i as f64 / f64::from(rate)).sin() * f64::from(amplitude)) as f32)
        .collect()
}

fn scale_to_rms(samples: &[f64], rms_dbfs: f32) -> Vec<f32> {
    let ms = samples.iter().map(|v| v * v).sum::<f64>() / samples.len().max(1) as f64;
    if ms == 0.0 {
        return vec![0.0; samples.len()];
    }
    let scale = 10f64.powf(f64::from(rms_dbfs) / 20.0) / ms.sqrt();
    samples.iter().map(|v| (v * scale) as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gain::{rms, robust_peak, to_dbfs};

    #[test]
    fn speech_like_has_the_asked_rms_and_is_deterministic() {
        let a = speech_like(3.0, -75.0, 7);
        assert_eq!(a.len(), 48_000);
        assert!((to_dbfs(rms(&a)) + 75.0).abs() < 0.01);
        assert_eq!(a, speech_like(3.0, -75.0, 7));
        assert_ne!(a, speech_like(3.0, -75.0, 8));
        // The crest the gain constants were chosen against.
        let crest = to_dbfs(robust_peak(&a)) - to_dbfs(rms(&a));
        assert!(
            (9.0..14.0).contains(&crest),
            "robust peak {crest} dB over RMS"
        );
    }

    #[test]
    fn noise_has_the_asked_rms() {
        let n = noise(2.0, -70.0, 3);
        assert!((to_dbfs(rms(&n)) + 70.0).abs() < 0.01);
    }
}
