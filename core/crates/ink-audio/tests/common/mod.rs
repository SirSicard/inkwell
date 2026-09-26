//! Helpers shared by the integration tests: temp directories, synthetic signals, WAV writing.
//!
//! Everything is synthetic and deterministic. Temp directories live under the OS temp dir, so the
//! tests run the same on macOS and Windows.

#![allow(dead_code)] // each test binary uses a different subset

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ink_core::StreamFormat;

/// A directory under the OS temp dir, removed on drop. Declare it before anything that holds
/// files open in it, so it drops last (Windows cannot remove open files).
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ink-audio-test-{}-{label}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub const MONO_16K: StreamFormat = StreamFormat {
    sample_rate: 16_000,
    channels: 1,
};

pub const STEREO_48K: StreamFormat = StreamFormat {
    sample_rate: 48_000,
    channels: 2,
};

/// A deterministic test signal: two tones plus seeded noise, in -0.9..0.9, interleaved. Each
/// channel differs, so a channel swap would show.
pub fn signal(frames: usize, format: StreamFormat, seed: u64) -> Vec<f32> {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let rate = format.sample_rate as f32;
    let mut out = Vec::with_capacity(frames * usize::from(format.channels));
    for i in 0..frames {
        let t = i as f32 / rate;
        for c in 0..format.channels {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let noise = ((state >> 40) as f32 / (1u64 << 24) as f32) - 0.5;
            let f = 220.0 * f32::from(c + 1);
            let v = 0.5 * (std::f32::consts::TAU * f * t).sin()
                + 0.2 * (std::f32::consts::TAU * 3.1 * f * t).sin()
                + 0.2 * noise;
            out.push(v.clamp(-0.9, 0.9));
        }
    }
    out
}

/// Writes 16-bit PCM and returns the samples exactly as a reader decodes them (`i16 / 32768`).
pub fn write_wav_i16(path: &Path, format: StreamFormat, samples: &[f32]) -> Vec<f32> {
    let spec = hound::WavSpec {
        channels: format.channels,
        sample_rate: format.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    let mut decoded = Vec::with_capacity(samples.len());
    for &s in samples {
        let q = (s * 32_767.0).round() as i16;
        writer.write_sample(q).unwrap();
        decoded.push(f32::from(q) / 32_768.0);
    }
    writer.finalize().unwrap();
    decoded
}

/// Writes 32-bit float samples.
pub fn write_wav_f32(path: &Path, format: StreamFormat, samples: &[f32]) {
    let spec = hound::WavSpec {
        channels: format.channels,
        sample_rate: format.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for &s in samples {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();
}

/// Every file in `dir` as (name, bytes), sorted by name.
pub fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

/// Host time of frame `frame` at `rate`, from `start_ns`.
pub fn at(start_ns: u64, frame: u64, rate: u32) -> u64 {
    start_ns + (u128::from(frame) * 1_000_000_000 / u128::from(rate)) as u64
}

// --- Scripted VADs for the gain stages. -------------------------------------------------------

use ink_audio::vad::{SpeechProbability, VAD_WINDOW};
use ink_core::EngineError;

fn window_rms(w: &[f32]) -> f32 {
    (w.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / w.len().max(1) as f64).sqrt() as f32
}

/// The truth, per VAD window, from a clean speech signal: a window is speech when its RMS is within
/// 30 dB of the loudest window's.
pub fn speech_mask(clean: &[f32]) -> Vec<bool> {
    let rms: Vec<f32> = clean.chunks(VAD_WINDOW).map(window_rms).collect();
    let loudest = rms.iter().fold(0.0f32, |m, &r| m.max(r));
    rms.iter()
        .map(|&r| r > 0.0 && r >= loudest * 0.031_6)
        .collect()
}

/// A VAD that knows the truth: 0.9 on the mask's speech windows, 0.05 elsewhere (and past its
/// end). Windows are counted from the last reset.
pub struct Oracle {
    mask: Vec<bool>,
    next: usize,
}

impl Oracle {
    pub fn new(mask: Vec<bool>) -> Self {
        Self { mask, next: 0 }
    }

    pub fn boxed(mask: Vec<bool>) -> Box<dyn SpeechProbability> {
        Box::new(Self::new(mask))
    }
}

impl SpeechProbability for Oracle {
    fn reset(&mut self) {
        self.next = 0;
    }

    fn probability(&mut self, _: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
        let speech = self.mask.get(self.next).copied().unwrap_or(false);
        self.next += 1;
        Ok(if speech { 0.9 } else { 0.05 })
    }
}

/// As [`Oracle`], but deaf to windows quieter than `hearing_dbfs` RMS, as a real VAD is: it cannot
/// hear −75 dBFS speech.
pub struct DeafOracle {
    oracle: Oracle,
    hearing: f32,
}

impl DeafOracle {
    pub const HEARING_DBFS: f32 = -50.0;

    pub fn new(mask: Vec<bool>) -> Self {
        Self {
            oracle: Oracle::new(mask),
            hearing: 10f32.powf(Self::HEARING_DBFS / 20.0),
        }
    }

    pub fn boxed(mask: Vec<bool>) -> Box<dyn SpeechProbability> {
        Box::new(Self::new(mask))
    }
}

impl SpeechProbability for DeafOracle {
    fn reset(&mut self) {
        self.oracle.reset();
    }

    fn probability(&mut self, window: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
        let p = self.oracle.probability(window)?;
        Ok(if window_rms(window) >= self.hearing {
            p
        } else {
            0.05
        })
    }
}

/// A VAD that says the same thing about every window.
pub struct Always(pub f32);

impl SpeechProbability for Always {
    fn reset(&mut self) {}

    fn probability(&mut self, _: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
        Ok(self.0)
    }
}

/// A VAD whose model is missing.
pub struct Failing;

impl SpeechProbability for Failing {
    fn reset(&mut self) {}

    fn probability(&mut self, _: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
        Err(EngineError::ModelMissing("vad".into()))
    }
}
