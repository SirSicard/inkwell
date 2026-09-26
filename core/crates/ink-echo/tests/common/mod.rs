//! The echo fixture, generated from seeds: nothing is read from disk.
//!
//! A far end of synthetic speech plays through a synthetic room (a direct path, three early
//! reflections and a decaying noise tail, RT60 0.4 s: S0.3's seeded room, regenerated here at
//! 16 kHz), reaches the mic late and on a drifting clock, and sits under a near-end talker and a
//! noise floor. The timeline follows S0.3's protocol: far end alone, then the near end alone,
//! then both, then the far end alone again.

#![allow(dead_code)]

use std::f64::consts::PI;
use std::sync::OnceLock;

use ink_audio::synth::{Lcg, SpeechShape, speech_with};
use ink_echo::{
    Alignment, CancellerConfig, EchoCanceller, EchoFrame, FRAME, PathFinder, PathReport,
};
use num_complex::Complex;
use rustfft::FftPlanner;

pub const RATE: usize = 16_000;
pub const TOTAL_S: f64 = 45.0;
/// Far end alone 1–21 s, near end alone 21–27 s, both 27–40 s, far end alone 40–44 s.
pub const FAR_SEGMENTS: [(f64, f64); 2] = [(1.0, 21.0), (27.0, 44.0)];
pub const NEAR_SEGMENTS: [(f64, f64); 1] = [(21.0, 40.0)];

/// How a scene is built.
#[derive(Clone, Copy, Debug)]
pub struct SceneSpec {
    /// The mic hears the far end this late (negative: the far-end stream is the late one).
    pub delay_ms: f64,
    /// The mic's clock runs this fast against the far end's.
    pub drift_ppm: f64,
    /// Whether the mic hears the far end at all (false: earbuds).
    pub echo: bool,
    pub seed: u64,
}

impl SceneSpec {
    /// S0.3's specification case: +75 ms, +50 ppm.
    pub fn speakers() -> Self {
        Self {
            delay_ms: 75.0,
            drift_ppm: 50.0,
            echo: true,
            seed: 20_260_924,
        }
    }

    /// The mic leads: the far-end stream arrives 75 ms late and its clock runs 50 ppm fast.
    pub fn mic_leads() -> Self {
        Self {
            delay_ms: -75.0,
            drift_ppm: -50.0,
            ..Self::speakers()
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

/// A generated scene: every component on the mic's clock except `far`, which is on its own.
pub struct Scene {
    pub spec: SceneSpec,
    pub far: Vec<f32>,
    pub mic: Vec<f32>,
    /// The near-end talker as the mic has it: the truth for double talk.
    pub near: Vec<f32>,
    /// The echo as the mic has it.
    pub echo: Vec<f32>,
}

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

/// Talk laid onto a timeline: utterances of 1.5–4 s with 0.3–0.9 s pauses, in `segments`.
fn talker(segments: &[(f64, f64)], shape: SpeechShape, seed: u64) -> Vec<f64> {
    let n = (TOTAL_S * RATE as f64) as usize;
    let mut out = vec![0.0; n];
    let mut rng = Lcg::new(seed);
    let mut utter = 0u64;
    for &(a, b) in segments {
        let mut t = a;
        while t < b {
            let len = (1.5 + 2.5 * rng.next_f64()).min(b - t);
            let speech = speech_with(len, -20.0, shape, seed.wrapping_add(utter));
            utter += 1;
            let at = (t * RATE as f64) as usize;
            for (o, s) in out[at..].iter_mut().zip(&speech) {
                *o = f64::from(*s);
            }
            t += len + 0.3 + 0.6 * rng.next_f64();
        }
    }
    out
}

/// S0.3's synthetic room at 16 kHz: a direct path, reflections at 2.9, 6.3 and 11.7 ms, and a
/// Gaussian tail decaying 60 dB in 0.4 s, 3 dB below the direct and early energy.
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

/// `y` heard on a clock `drift` fast and `delay_s` late: `out[k] = y(k / (1 + drift) − delay)`,
/// by a Kaiser-windowed sinc (32 zero crossings a side), independent of the crate's own.
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

/// Builds the scene for `spec`.
pub fn scene(spec: SceneSpec) -> Scene {
    let far_shape = SpeechShape {
        syllable: 0.2,
        gap: 0.25,
        floor: 0.0,
        breath: 0.2,
    };
    let near_shape = SpeechShape {
        syllable: 0.22,
        gap: 0.3,
        floor: 0.0,
        breath: 0.2,
    };
    let mut far = talker(&FAR_SEGMENTS, far_shape, spec.seed);
    let mut near = talker(&NEAR_SEGMENTS, near_shape, spec.seed ^ 0x5eed);
    scale_to(&mut far, -30.0);
    scale_to(&mut near, -34.0);
    let n = far.len();
    let mut echo = if spec.echo {
        let heard = convolve(&far, &room(spec.seed));
        onto_mic_clock(&heard, spec.delay_ms / 1000.0, spec.drift_ppm)
    } else {
        vec![0.0; n]
    };
    if spec.echo {
        // Echo as loud at the mic as the near-end talker: laptop speakers.
        scale_to(&mut echo, -34.0);
    }
    let mut rng = Lcg::new(spec.seed ^ 7);
    let noise_rms = 10f64.powf(-70.0 / 20.0);
    let mic: Vec<f32> = (0..n)
        .map(|i| (near[i] + echo[i] + noise_rms * rng.next_gaussian()) as f32)
        .collect();
    let to32 = |v: &[f64]| v.iter().map(|s| *s as f32).collect::<Vec<f32>>();
    Scene {
        spec,
        far: to32(&far),
        mic,
        near: to32(&near),
        echo: to32(&echo),
    }
}

/// The path, found the way the pipeline will: both streams in 100 ms pushes.
pub fn find_path(scene: &Scene) -> PathReport {
    let mut finder = PathFinder::new();
    for (m, f) in scene.mic.chunks(1_600).zip(scene.far.chunks(1_600)) {
        finder.push(m, f).expect("in step");
    }
    finder.estimate()
}

/// Everything the canceller put out, as whole streams on the mic's grid.
#[derive(Default)]
pub struct Outputs {
    pub mic: Vec<f32>,
    pub reference: Vec<f32>,
    pub linear: Vec<f32>,
    pub full: Vec<f32>,
    pub frames: Vec<u64>,
}

impl Outputs {
    pub fn take(&mut self, f: &EchoFrame) {
        self.frames.push(f.index);
        self.mic.extend_from_slice(&f.mic[..f.len]);
        self.reference.extend_from_slice(&f.reference[..f.len]);
        self.linear.extend_from_slice(&f.linear[..f.len]);
        self.full.extend_from_slice(&f.full[..f.len]);
    }
}

/// Runs the canceller over the scene along `path`, pushing the two streams in uneven, offset
/// chunks as a pump would.
pub fn cancel(mic: &[f32], far: &[f32], path: Alignment) -> Outputs {
    let mut c = EchoCanceller::new(path, CancellerConfig::default()).expect("canceller");
    let mut out = Outputs::default();
    let (mut i, mut j, mut turn) = (0, 0, 0usize);
    while i < mic.len() || j < far.len() {
        let (mn, fnn) = if turn % 2 == 0 {
            (441, 160)
        } else {
            (199, 480)
        };
        turn += 1;
        let m = &mic[i..(i + mn).min(mic.len())];
        let f = &far[j..(j + fnn).min(far.len())];
        i += m.len();
        j += f.len();
        c.push(m, f, |fr| out.take(fr)).expect("in step");
    }
    let frames = c.finish(|fr| out.take(fr)).expect("finish");
    assert_eq!(frames, mic.len().div_ceil(FRAME) as u64);
    out
}

/// The speakers scene, its path search and its cancelled outputs, computed once per test binary.
pub struct Run {
    pub scene: Scene,
    pub report: PathReport,
    pub out: Outputs,
}

pub fn speakers_run() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| {
        let scene = scene(SceneSpec::speakers());
        let report = find_path(&scene);
        let path = report.path.expect("the speakers scene has a path");
        let out = cancel(&scene.mic, &scene.far, path);
        Run { scene, report, out }
    })
}
