//! Telling a silent capture from a quiet one: the rules a capture watchdog judges delivered audio
//! by.
//!
//! The failure this exists for: a microphone that delivers **digital zeros** reads on a meter as
//! "very quiet" (a denied permission, a Bluetooth mic inside a macOS aggregate device), and a quiet
//! room reads the same at a glance. They are far apart in meaning: zeros are no data at all. So
//! the meter counts samples that are exactly zero, separately from the level. A pipeline treats
//! [`CaptureHealth::DigitalSilence`] as a failure to report, never as a quiet stretch.
//!
//! **Pump or worker.** It runs on what the ring hands back, never in a capture callback. Pure, so
//! the rules are tested directly (`tests/levels.rs`).

use ink_core::Channel;

/// Accumulates the level of a stream: sample count, exact zeros, RMS and peak.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LevelMeter {
    samples: u64,
    zero_samples: u64,
    sum_squares: f64,
    peak: f32,
}

impl LevelMeter {
    /// An empty meter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds samples (any channel layout: the level is over all of them).
    pub fn add(&mut self, samples: &[f32]) {
        for &s in samples {
            if s == 0.0 {
                self.zero_samples += 1;
            }
            self.sum_squares += f64::from(s) * f64::from(s);
            self.peak = self.peak.max(s.abs());
        }
        self.samples += samples.len() as u64;
    }

    /// Samples seen.
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// The share of samples that were exactly zero, 0 to 1 (0 when nothing was seen).
    pub fn zero_fraction(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.zero_samples as f64 / self.samples as f64
        }
    }

    /// Whether every sample seen was exactly zero (false when nothing was seen).
    pub fn all_zero(&self) -> bool {
        self.samples > 0 && self.zero_samples == self.samples
    }

    /// RMS level in dBFS; negative infinity for digital silence or no samples.
    pub fn rms_dbfs(&self) -> f64 {
        if self.samples == 0 || self.sum_squares == 0.0 {
            f64::NEG_INFINITY
        } else {
            10.0 * (self.sum_squares / self.samples as f64).log10()
        }
    }

    /// The largest absolute sample.
    pub fn peak(&self) -> f32 {
        self.peak
    }
}

/// What a stretch of capture shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureHealth {
    /// The mic delivered no callbacks at all. A running input device always calls back, so this is
    /// a stalled or vanished device, never a quiet room.
    NoCallbacks,
    /// The far end delivered no callbacks: nothing is playing. Some far-end sources call back only
    /// while a process makes a sound (a macOS tap-only aggregate does), so this is **not** a
    /// failure.
    Idle,
    /// Callbacks arrived and every sample was exactly zero. On the mic: permission denied, a
    /// Bluetooth mic inside a macOS aggregate device, or a Bluetooth headset mic while the user says
    /// nothing (it gates to zeros). On the far end: a denied system-audio capture.
    DigitalSilence,
    /// Real samples, however quiet.
    Signal,
}

/// Judges a stretch of capture from its callback count and its level.
pub fn assess(channel: Channel, callbacks: u64, meter: &LevelMeter) -> CaptureHealth {
    if callbacks == 0 || meter.samples() == 0 {
        return match channel {
            Channel::Mic => CaptureHealth::NoCallbacks,
            Channel::Far => CaptureHealth::Idle,
        };
    }
    if meter.all_zero() {
        CaptureHealth::DigitalSilence
    } else {
        CaptureHealth::Signal
    }
}
