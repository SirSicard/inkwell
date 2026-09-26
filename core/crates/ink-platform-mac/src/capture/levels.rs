//! Telling a silent capture from a quiet one.
//!
//! The failure this exists for: a microphone that delivers **digital zeros** reads on a meter as
//! "very quiet" (a Bluetooth mic inside an aggregate device, a denied permission), and a quiet
//! room reads the same at a glance. They are far apart in meaning: zeros are no data at all. So
//! the meter counts samples that are exactly zero, separately from the level.
//!
//! Pump or worker side only (it runs on what the ring hands back, never in an IOProc). Pure, so
//! the rules are tested directly.

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
    /// The far end delivered no callbacks: nothing is playing. A tap-only aggregate fires no
    /// callbacks while no process makes a sound, so this is **not** a failure.
    Idle,
    /// Callbacks arrived and every sample was exactly zero. On the mic: permission denied, a
    /// Bluetooth mic inside an aggregate device, or a Bluetooth headset mic while the user says
    /// nothing (it gates to zeros). On the far end: a denied system-audio tap.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Seeded noise at about −86 dBFS: the level an earlier implementation read as "a quiet room"
    /// when the mic was in fact delivering zeros.
    fn quiet_room(n: usize) -> Vec<f32> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 1.7e-4
            })
            .collect()
    }

    #[test]
    fn digital_zeros_are_not_a_quiet_room() {
        let mut zeros = LevelMeter::new();
        zeros.add(&[0.0; 4_800]);
        assert!(zeros.all_zero());
        assert_eq!(zeros.rms_dbfs(), f64::NEG_INFINITY);
        assert_eq!(
            assess(Channel::Mic, 10, &zeros),
            CaptureHealth::DigitalSilence
        );

        let mut room = LevelMeter::new();
        room.add(&quiet_room(4_800));
        let db = room.rms_dbfs();
        assert!((-95.0..-80.0).contains(&db), "{db} dBFS");
        assert!(room.zero_fraction() < 0.01);
        assert_eq!(assess(Channel::Mic, 10, &room), CaptureHealth::Signal);
    }

    /// A Bluetooth mic inside an aggregate device delivers only zeros, which is why the mic is
    /// never put in one (the dual graph). If one ever is, this is how it reads.
    #[test]
    fn a_bluetooth_mic_inside_an_aggregate_reads_as_digital_silence() {
        let mut meter = LevelMeter::new();
        for _ in 0..50 {
            meter.add(&[0.0; 320]); // 20 ms blocks at 16 kHz, the call-link rate
        }
        assert_eq!(
            assess(Channel::Mic, 50, &meter),
            CaptureHealth::DigitalSilence
        );
    }

    /// A Bluetooth headset mic gates to digital zeros while the user is silent and carries 16 kHz
    /// call audio while they talk: a high zero share is normal there, and it is still signal.
    #[test]
    fn a_bluetooth_headset_mic_gates_to_zeros_while_silent_and_is_still_signal() {
        let mut meter = LevelMeter::new();
        meter.add(&[0.0; 16_000]); // a second of silence: gated
        let speech: Vec<f32> = (0..16_000).map(|i| 0.1 * (i as f32 * 0.07).sin()).collect();
        meter.add(&speech);
        assert!((meter.zero_fraction() - 0.5).abs() < 0.01);
        assert_eq!(assess(Channel::Mic, 100, &meter), CaptureHealth::Signal);
    }

    /// A tap-only aggregate fires no callbacks while nothing is playing: "no callbacks" on the far
    /// end is idle, not broken. On the mic it is a stalled device.
    #[test]
    fn a_far_end_without_callbacks_is_idle_not_broken() {
        let empty = LevelMeter::new();
        assert_eq!(assess(Channel::Far, 0, &empty), CaptureHealth::Idle);
        assert_eq!(assess(Channel::Mic, 0, &empty), CaptureHealth::NoCallbacks);
    }

    /// A denied system-audio tap delivers silence, so zeros with callbacks on the far end are the
    /// permission failure, never idle.
    #[test]
    fn a_far_end_of_zeros_with_callbacks_is_digital_silence() {
        let mut meter = LevelMeter::new();
        meter.add(&[0.0; 960]);
        assert_eq!(
            assess(Channel::Far, 2, &meter),
            CaptureHealth::DigitalSilence
        );
    }

    #[test]
    fn the_meter_reports_level_and_peak() {
        let mut meter = LevelMeter::new();
        meter.add(&[0.5, -0.5, 0.5, -0.5]);
        assert!((meter.rms_dbfs() - (-6.0206)).abs() < 1e-3);
        assert_eq!(meter.peak(), 0.5);
        assert_eq!(meter.samples(), 4);
        assert_eq!(LevelMeter::new().zero_fraction(), 0.0);
        assert!(!LevelMeter::new().all_zero());
    }
}
