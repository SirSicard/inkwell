//! Echo scenes for the meeting chain: a far end played through a room into the mic, over a
//! near-end talker, generated from seeds.
//!
//! The room is ink-echo's seeded room (a direct path, reflections at 2.9, 6.3 and 11.7 ms and a
//! decaying noise tail, RT60 0.4 s), regenerated here with the same seed and the same arithmetic:
//! test helpers cannot be shared between crates, and ink-echo keeps its own private. The echo
//! reaches the mic late and on a drifting clock, by an independent windowed-sinc resampler.

#![allow(dead_code)] // each test binary uses a different subset

use std::f64::consts::PI;

use ink_audio::synth::{Lcg, SpeechShape, speech_with};
use num_complex::Complex;
use rustfft::FftPlanner;

pub const RATE: usize = 16_000;

/// How the echo is made.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// The mic hears the far end this late, ms.
    pub delay_ms: f64,
    /// The mic's clock runs this fast against the far end's, ppm.
    pub drift_ppm: f64,
    /// Whether the mic hears the far end at all (false: earbuds).
    pub echo: bool,
    pub seed: u64,
}

impl Spec {
    /// Laptop speakers: +75 ms, +50 ppm (the gate's specification case).
    pub fn speakers() -> Self {
        Self {
            delay_ms: 75.0,
            drift_ppm: 50.0,
            echo: true,
            seed: 20_260_924,
        }
    }

    /// Earbuds: the mic never hears the far end.
    pub fn earbuds() -> Self {
        Self {
            echo: false,
            ..Self::speakers()
        }
    }
}

/// A scene: every signal on the mic's clock except `far`, which is on its own.
#[derive(Clone)]
pub struct Scene {
    pub far: Vec<f32>,
    pub mic: Vec<f32>,
    /// The near-end talker as the mic has it.
    pub near: Vec<f32>,
    /// The echo as the mic has it.
    pub echo: Vec<f32>,
}

const FAR_SHAPE: SpeechShape = SpeechShape {
    syllable: 0.2,
    gap: 0.25,
    floor: 0.0,
    breath: 0.2,
};

const NEAR_SHAPE: SpeechShape = SpeechShape {
    syllable: 0.22,
    gap: 0.3,
    floor: 0.0,
    breath: 0.2,
};

fn active_rms(x: &[f64]) -> f64 {
    let e: Vec<f64> = x
        .chunks(RATE / 100)
        .map(|c| c.iter().map(|v| v * v).sum::<f64>() / c.len() as f64)
        .collect();
    let top = e.iter().copied().fold(0.0, f64::max);
    let keep: Vec<f64> = e.into_iter().filter(|v| *v > top * 1e-4).collect();
    (keep.iter().sum::<f64>() / keep.len().max(1) as f64).sqrt()
}

fn scale_to(x: &mut [f64], dbfs: f64) {
    let g = 10f64.powf(dbfs / 20.0) / active_rms(x).max(1e-30);
    x.iter_mut().for_each(|v| *v *= g);
}

/// Talk laid onto a timeline of `total_s`: utterances of 1.5–4 s with 0.3–0.9 s pauses, filling
/// each of `segments`.
fn talker(total_s: f64, segments: &[(f64, f64)], shape: SpeechShape, seed: u64) -> Vec<f64> {
    let mut rng = Lcg::new(seed);
    let mut utterances = Vec::new();
    for &(a, b) in segments {
        let mut t = a;
        while t < b {
            let len = (1.5 + 2.5 * rng.next_f64()).min(b - t);
            utterances.push((t, len));
            t += len + 0.3 + 0.6 * rng.next_f64();
        }
    }
    lay(total_s, &utterances, shape, seed)
}

/// Utterances `(start_s, length_s)` laid onto a timeline of `total_s`.
fn lay(total_s: f64, utterances: &[(f64, f64)], shape: SpeechShape, seed: u64) -> Vec<f64> {
    let n = (total_s * RATE as f64) as usize;
    let mut out = vec![0.0; n];
    for (k, &(at, len)) in utterances.iter().enumerate() {
        let speech = speech_with(len, -20.0, shape, seed.wrapping_add(k as u64));
        let start = (at * RATE as f64) as usize;
        for (o, s) in out[start.min(n)..].iter_mut().zip(&speech) {
            *o = f64::from(*s);
        }
    }
    out
}

/// The seeded room at 16 kHz.
pub fn room(seed: u64) -> Vec<f64> {
    let n = (0.6 * RATE as f64) as usize;
    let mut h = vec![0.0; n];
    h[0] = 1.0;
    for (t, a) in [(0.0029, 0.55), (0.0063, -0.40), (0.0117, 0.30)] {
        h[(t * RATE as f64).round() as usize] += a;
    }
    let mut rng = Lcg::new(seed);
    let direct: f64 = h.iter().map(|v| v * v).sum();
    let mut tail: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / RATE as f64;
            let g = rng.next_gaussian() * (-6.9078 * t / 0.4).exp();
            if t < 0.005 {
                0.0
            } else if t < 0.008 {
                g * (t - 0.005) / 0.003
            } else {
                g
            }
        })
        .collect();
    let te: f64 = tail.iter().map(|v| v * v).sum();
    let scale = (direct / 10f64.powf(0.3) / te).sqrt();
    tail.iter_mut().for_each(|v| *v *= scale);
    h.iter().zip(&tail).map(|(a, b)| a + b).collect()
}

/// `x ⊛ h`, cut to `x`'s length, by FFT.
fn convolve(x: &[f64], h: &[f64]) -> Vec<f64> {
    let n = (x.len() + h.len()).next_power_of_two();
    let mut planner = FftPlanner::<f64>::new();
    let (fwd, inv) = (planner.plan_fft_forward(n), planner.plan_fft_inverse(n));
    let pad = |v: &[f64]| {
        let mut c = vec![Complex::new(0.0, 0.0); n];
        for (o, s) in c.iter_mut().zip(v) {
            o.re = *s;
        }
        c
    };
    let (mut a, mut b) = (pad(x), pad(h));
    fwd.process(&mut a);
    fwd.process(&mut b);
    for (p, q) in a.iter_mut().zip(&b) {
        *p *= q;
    }
    inv.process(&mut a);
    a[..x.len()].iter().map(|c| c.re / n as f64).collect()
}

/// `y` heard on a clock `drift_ppm` fast and `delay_s` late: `out[k] = y(k / (1 + drift) − delay)`,
/// by a Kaiser-windowed sinc (32 zero crossings a side).
fn onto_mic_clock(y: &[f64], delay_s: f64, drift_ppm: f64) -> Vec<f64> {
    const HALF: usize = 32;
    const OS: usize = 512;
    let beta = 9.0;
    let i0 = |x: f64| {
        let (mut s, mut t) = (1.0, 1.0);
        for k in 1..50 {
            t *= (x / 2.0) * (x / 2.0) / (k * k) as f64;
            s += t;
        }
        s
    };
    let table: Vec<f64> = (0..=HALF * OS + 1)
        .map(|i| {
            let x = i as f64 / OS as f64;
            if x >= HALF as f64 {
                return 0.0;
            }
            let sinc = if x == 0.0 {
                1.0
            } else {
                (PI * x).sin() / (PI * x)
            };
            let u = x / HALF as f64;
            sinc * i0(beta * (1.0 - u * u).sqrt()) / i0(beta)
        })
        .collect();
    let kernel = |x: f64| {
        let ax = x.abs() * OS as f64;
        let i = ax as usize;
        if i + 1 >= table.len() {
            0.0
        } else {
            let f = ax - i as f64;
            table[i] * (1.0 - f) + table[i + 1] * f
        }
    };
    let eps = drift_ppm * 1e-6;
    let d = delay_s * RATE as f64;
    (0..y.len())
        .map(|k| {
            let t = k as f64 / (1.0 + eps) - d;
            let base = t.floor() as i64;
            let mut acc = 0.0;
            for j in (base - HALF as i64 + 1)..=(base + HALF as i64) {
                if j >= 0 && (j as usize) < y.len() {
                    acc += y[j as usize] * kernel(t - j as f64);
                }
            }
            acc
        })
        .collect()
}

/// A scene from a far end and a near end already laid out (both `total` samples, 16 kHz): the
/// far end at −30 dBFS, the near end at −34, the echo as loud at the mic as the near end (laptop
/// speakers), and a −70 dBFS noise floor.
pub fn mix(far: Vec<f64>, near: Vec<f64>, spec: Spec) -> Scene {
    let (mut far, mut near) = (far, near);
    scale_to(&mut far, -30.0);
    if near.iter().any(|v| *v != 0.0) {
        scale_to(&mut near, -34.0);
    }
    let n = far.len();
    let echo = if spec.echo {
        let heard = convolve(&far, &room(spec.seed));
        let mut e = onto_mic_clock(&heard, spec.delay_ms / 1000.0, spec.drift_ppm);
        scale_to(&mut e, -34.0);
        e
    } else {
        vec![0.0; n]
    };
    let mut rng = Lcg::new(spec.seed ^ 7);
    let noise_rms = 10f64.powf(-70.0 / 20.0);
    let mic: Vec<f32> = (0..n)
        .map(|i| (near[i] + echo[i] + noise_rms * rng.next_gaussian()) as f32)
        .collect();
    let to32 = |v: &[f64]| v.iter().map(|s| *s as f32).collect::<Vec<f32>>();
    Scene {
        far: to32(&far),
        mic,
        near: to32(&near),
        echo: to32(&echo),
    }
}

/// The gate's protocol, 45 s: far end alone 1–21 s, near end alone 21–27 s, both 27–40 s, far end
/// alone 40–44 s.
pub fn protocol(spec: Spec) -> Scene {
    let total = 45.0;
    let far = talker(total, &[(1.0, 21.0), (27.0, 44.0)], FAR_SHAPE, spec.seed);
    let near = talker(total, &[(21.0, 40.0)], NEAR_SHAPE, spec.seed ^ 0x5eed);
    mix(far, near, spec)
}

/// Far-end talk only, 1–`total_s − 1` s: the mic hears nothing but its echo.
pub fn echo_only(total_s: f64, spec: Spec) -> Scene {
    let far = talker(total_s, &[(1.0, total_s - 1.0)], FAR_SHAPE, spec.seed);
    mix(far, vec![0.0; (total_s * RATE as f64) as usize], spec)
}

/// Utterances at given times, `(start_s, length_s)`, on both sides.
pub fn scripted(total_s: f64, far: &[(f64, f64)], near: &[(f64, f64)], spec: Spec) -> Scene {
    let f = lay(total_s, far, FAR_SHAPE, spec.seed);
    let n = lay(total_s, near, NEAR_SHAPE, spec.seed ^ 0x5eed);
    mix(f, n, spec)
}

/// The far end continuous 1–`total_s − 1` s, echoed along `spec` until `switch_s` and along
/// `after` from then on (the same far end, a different room path).
pub fn path_changes(total_s: f64, switch_s: f64, spec: Spec, after: Spec) -> Scene {
    let far = talker(total_s, &[(1.0, total_s - 1.0)], FAR_SHAPE, spec.seed);
    let a = mix(far.clone(), vec![0.0; far.len()], spec);
    let b = mix(far, vec![0.0; a.far.len()], after);
    let cut = (switch_s * RATE as f64) as usize;
    let mic = a.mic[..cut].iter().chain(&b.mic[cut..]).copied().collect();
    let echo = a.echo[..cut]
        .iter()
        .chain(&b.echo[cut..])
        .copied()
        .collect();
    Scene {
        far: a.far,
        mic,
        near: a.near,
        echo,
    }
}

/// The far end continuous 1–`total_s − 1` s and echoed along `spec`; from `switch_s` the mic
/// hears the echo of other audio instead, which the far-end stream does not carry (a second app
/// playing, the tap on the wrong one).
pub fn untapped(total_s: f64, switch_s: f64, spec: Spec) -> Scene {
    let far = talker(total_s, &[(1.0, total_s - 1.0)], FAR_SHAPE, spec.seed);
    let other = talker(
        total_s,
        &[(1.0, total_s - 1.0)],
        FAR_SHAPE,
        spec.seed ^ 0xa11,
    );
    let a = mix(far, vec![0.0; (total_s * RATE as f64) as usize], spec);
    let b = mix(other, vec![0.0; a.far.len()], spec);
    let cut = (switch_s * RATE as f64) as usize;
    let mic = a.mic[..cut].iter().chain(&b.mic[cut..]).copied().collect();
    let echo = a.echo[..cut]
        .iter()
        .chain(&b.echo[cut..])
        .copied()
        .collect();
    Scene {
        far: a.far,
        mic,
        near: a.near,
        echo,
    }
}

/// Least-squares weights of `near` and `echo` in `x` over `range`: how much of each the signal
/// holds. Returns the echo's weight relative to the near end's, in dB.
pub fn echo_to_near_db(x: &[f32], near: &[f32], echo: &[f32]) -> f64 {
    let n = x.len().min(near.len()).min(echo.len());
    let dot = |a: &[f32], b: &[f32]| -> f64 {
        a[..n]
            .iter()
            .zip(&b[..n])
            .map(|(p, q)| f64::from(*p) * f64::from(*q))
            .sum()
    };
    let (nn, ee, ne) = (dot(near, near), dot(echo, echo), dot(near, echo));
    let (xn, xe) = (dot(x, near), dot(x, echo));
    let det = nn * ee - ne * ne;
    let a = (xn * ee - xe * ne) / det;
    let b = (xe * nn - xn * ne) / det;
    20.0 * (b.abs() / a.abs().max(1e-30)).log10()
}
