//! Deterministic synthetic signals for tests, benches and later steps' fixtures.
//!
//! Real audio never enters the repository, so every level test runs on these. They are pure
//! functions of their arguments: the same call gives the same samples on every OS (up to the last
//! bit of `sin`, which is why tests compare with tolerances).

use std::f64::consts::TAU;

use ink_core::CANONICAL_RATE;

use crate::speech_band::Biquad;

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
/// Its loud frames are even, so its robust peak sits 9–14 dB above its RMS (checked below).
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

/// Breathy, continuous speech-like audio at 16 kHz: the same voiced syllables as [`speech_like`]
/// but run together with no gaps, under an envelope that never falls below `floor` (0–1) of its
/// peak, with aspiration noise riding on the voicing. This is low-crest speech: `floor` sets how
/// far its quiet frames sit below its loud ones (0.35 gives 7–9.5 dB between the robust peak and
/// the 10th-percentile frame, checked by `low_crest_breathy_speech_is_lifted_to_the_target`).
/// Scaled to `rms_dbfs` RMS.
pub fn breathy_speech(seconds: f64, rms_dbfs: f32, floor: f64, seed: u64) -> Vec<f32> {
    let rate = f64::from(CANONICAL_RATE);
    let total = (seconds * rate).round() as usize;
    let mut rng = Lcg::new(seed);
    let mut out: Vec<f64> = Vec::with_capacity(total + CANONICAL_RATE as usize);
    while out.len() < total {
        let dur = 0.16 + 0.10 * rng.next_f64();
        let f0 = 100.0 + 80.0 * rng.next_f64();
        let n = (dur * rate) as usize;
        let mut phase = 0.0f64;
        for i in 0..n {
            let t = i as f64 / rate;
            phase += TAU * f0 * (1.0 + 0.1 * t / dur) / rate;
            let mut v = 0.0;
            let mut k = 1.0;
            while k * f0 * 1.1 < 3_800.0 {
                v += (k * phase).sin() / k;
                k += 1.0;
            }
            let breath = 0.3 * rng.next_gaussian();
            let env = floor + (1.0 - floor) * (std::f64::consts::PI * t / dur).sin().powi(2);
            out.push((v + breath) * env);
        }
    }
    out.truncate(total);
    scale_to_rms(&out, rms_dbfs)
}

/// `signal` plus Gaussian noise `snr_db` below it (RMS to RMS), the sum rescaled to `rms_dbfs`.
pub fn with_noise(signal: &[f32], snr_db: f32, rms_dbfs: f32, seed: u64) -> Vec<f32> {
    let ms = signal.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / signal.len().max(1) as f64;
    let noise_rms = ms.sqrt() * 10f64.powf(-f64::from(snr_db) / 20.0);
    let mut rng = Lcg::new(seed);
    let mixed: Vec<f64> = signal
        .iter()
        .map(|&s| f64::from(s) + noise_rms * rng.next_gaussian())
        .collect();
    scale_to_rms(&mixed, rms_dbfs)
}

/// `signal` plus `noise` scaled to sit `snr_db` below it (RMS to RMS), the sum rescaled to
/// `rms_dbfs`. `noise` must be at least as long as `signal`.
pub fn mix(signal: &[f32], noise: &[f32], snr_db: f32, rms_dbfs: f32) -> Vec<f32> {
    let power =
        |x: &[f32]| x.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    let noise = &noise[..signal.len()];
    let scale = (power(signal) / power(noise).max(f64::MIN_POSITIVE)).sqrt()
        * 10f64.powf(-f64::from(snr_db) / 20.0);
    let mixed: Vec<f64> = signal
        .iter()
        .zip(noise)
        .map(|(&s, &n)| f64::from(s) + scale * f64::from(n))
        .collect();
    scale_to_rms(&mixed, rms_dbfs)
}

/// Intermittent knocks at 16 kHz and no speech: short thumps (a 150–400 Hz resonance decaying
/// over about 15 ms) every 0.3–0.9 s over room tone 40 dB below them. Scaled to `rms_dbfs` RMS.
pub fn knocks(seconds: f64, rms_dbfs: f32, seed: u64) -> Vec<f32> {
    let rate = f64::from(CANONICAL_RATE);
    let total = (seconds * rate).round() as usize;
    let mut rng = Lcg::new(seed);
    let mut out: Vec<f64> = (0..total).map(|_| 0.01 * rng.next_gaussian()).collect();
    let mut at = (0.2 * rate) as usize;
    while at < total {
        let hz = 150.0 + 250.0 * rng.next_f64();
        for (i, v) in out[at..]
            .iter_mut()
            .take((0.08 * rate) as usize)
            .enumerate()
        {
            let t = i as f64 / rate;
            *v += (TAU * hz * t).sin() * (-t / 0.015).exp();
        }
        at += ((0.3 + 0.6 * rng.next_f64()) * rate) as usize;
    }
    scale_to_rms(&out, rms_dbfs)
}

/// A fan that cycles, at 16 kHz and no speech: broadband noise switching between two levels 15 dB
/// apart every 0.7–1.3 s. Scaled to `rms_dbfs` RMS.
pub fn cycling_fan(seconds: f64, rms_dbfs: f32, seed: u64) -> Vec<f32> {
    let rate = f64::from(CANONICAL_RATE);
    let total = (seconds * rate).round() as usize;
    let mut rng = Lcg::new(seed);
    let mut out = Vec::with_capacity(total);
    let mut high = true;
    while out.len() < total {
        let level = if high { 1.0 } else { 10f64.powf(-15.0 / 20.0) };
        let n = ((0.7 + 0.6 * rng.next_f64()) * rate) as usize;
        out.extend((0..n).map(|_| level * rng.next_gaussian()));
        high = !high;
    }
    out.truncate(total);
    scale_to_rms(&out, rms_dbfs)
}

/// How steeply a [`rumble`] falls above its cutoff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slope {
    /// A one-pole low-pass: 6 dB per octave.
    Gentle,
    /// A 4th-order Butterworth run forwards and backwards (zero phase): 48 dB per octave, nothing
    /// left above the cutoff but the filter's own skirt.
    Steep,
}

/// Room rumble at 16 kHz: Gaussian noise low-passed at `cutoff_hz` (HVAC, a fridge, traffic),
/// stationary, no speech. Scaled to `rms_dbfs` RMS.
pub fn rumble(seconds: f64, rms_dbfs: f32, cutoff_hz: f64, slope: Slope, seed: u64) -> Vec<f32> {
    let rate = f64::from(CANONICAL_RATE);
    // A second of filter warm-up either side, trimmed off.
    let margin = CANONICAL_RATE as usize;
    let total = (seconds * rate).round() as usize;
    let mut rng = Lcg::new(seed);
    let mut x: Vec<f64> = (0..total + 2 * margin)
        .map(|_| rng.next_gaussian())
        .collect();
    match slope {
        Slope::Gentle => {
            let a = (-TAU * cutoff_hz / rate).exp();
            let mut y = 0.0;
            for v in &mut x {
                y += (1.0 - a) * (*v - y);
                *v = y;
            }
        }
        Slope::Steep => {
            // The two sections of a 4th-order Butterworth low-pass.
            let sections = [
                Biquad::low_pass(cutoff_hz, 0.541_196_1),
                Biquad::low_pass(cutoff_hz, 1.306_563_0),
            ];
            // Backwards, then forwards: the phase cancels and the order is restored.
            for _ in 0..2 {
                let mut s = sections;
                x.reverse();
                for v in &mut x {
                    *v = s.iter_mut().fold(*v, |acc, b| b.process(acc));
                }
            }
        }
    }
    scale_to_rms(&x[margin..margin + total], rms_dbfs)
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
    fn the_other_fixtures_have_the_asked_rms_and_are_deterministic() {
        let speech = breathy_speech(2.0, -75.0, 0.35, 1);
        for (what, x) in [
            ("breathy", speech.clone()),
            ("with noise", with_noise(&speech, 8.0, -75.0, 2)),
            ("knocks", knocks(2.0, -75.0, 3)),
            ("cycling fan", cycling_fan(2.0, -75.0, 4)),
        ] {
            assert_eq!(x.len(), 32_000, "{what}");
            assert!((to_dbfs(rms(&x)) + 75.0).abs() < 0.01, "{what}");
        }
        assert_eq!(speech, breathy_speech(2.0, -75.0, 0.35, 1));
        assert_eq!(knocks(2.0, -75.0, 3), knocks(2.0, -75.0, 3));
    }

    #[test]
    fn noise_has_the_asked_rms() {
        let n = noise(2.0, -70.0, 3);
        assert!((to_dbfs(rms(&n)) + 70.0).abs() < 0.01);
    }
}
