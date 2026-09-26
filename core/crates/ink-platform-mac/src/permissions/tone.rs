//! The self-tap tone probe: how the app learns whether System Audio capture is granted.
//!
//! **Why a probe.** A process tap without the permission is not refused: it delivers silence. And
//! macOS offers no public, non-prompting query for this permission. So the probe asks the
//! question the only way that has an answer: it plays a quiet tone from this process, taps only
//! this process, and listens. The tone back means granted; digital silence means denied.
//!
//! **Inaudible.** The tap mutes this process's output while the tap is read
//! (`MutedWhenTapped`), and the tone starts only once the tap is calling back. The tone is also
//! quiet (−30 dBFS) and short (0.4 s), for the case where muting does not apply.
//!
//! [`analyse`] is pure and tested; running the probe needs the audio hardware and the
//! permission, so the capture checklist covers it.
#![cfg(target_os = "macos")]

use std::thread;
use std::time::{Duration, Instant};

use ink_audio::{capture_ring, unguarded};
use ink_core::{AudioSource, Clock, PermissionState, PlatformError};
use objc2_core_audio::{CATapMuteBehavior, kAudioObjectPropertyScopeOutput};

use crate::capture::hal::{self, Direction, capture_format};
use crate::capture::{
    LevelMeter, MacFarEndSource, ProcessHal, RunningIo, SystemHal, TapScope, ToneContext, own_pid,
    tone_proc,
};
use crate::clock::MacClock;

/// The probe tone's frequency.
pub const TONE_HZ: f32 = 1_000.0;
/// Its amplitude: −30 dBFS.
pub const TONE_AMPLITUDE: f32 = 0.031_6;
/// How long the tone plays.
const TONE_DURATION: Duration = Duration::from_millis(400);
/// The start of the tone that is not analysed (output and tap latency).
const SETTLE: Duration = Duration::from_millis(100);
/// How long to wait for this process to appear as an audio process object.
const OWN_PROCESS_TIMEOUT: Duration = Duration::from_millis(500);
/// How long to wait for the tap to call back before playing the tone anyway.
const TAP_START_TIMEOUT: Duration = Duration::from_millis(300);
/// The share of the energy at the tone's frequency above which the tone counts as heard.
const HEARD_FRACTION: f32 = 0.5;
/// Below this peak the capture is silence (−120 dBFS).
const SILENCE_PEAK: f32 = 1e-6;

/// What the probe heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneVerdict {
    /// The tone came back: System Audio capture works.
    Heard,
    /// The tap called back with silence while the tone played: the permission is denied.
    Silence,
    /// Audio came back but not the tone. Not an answer.
    NotTheTone,
    /// Nothing came back at all. Not an answer.
    NoAudio,
}

impl ToneVerdict {
    /// The permission state this verdict supports.
    pub fn permission(self) -> PermissionState {
        match self {
            Self::Heard => PermissionState::Granted,
            Self::Silence => PermissionState::Denied,
            Self::NotTheTone | Self::NoAudio => PermissionState::Unknown,
        }
    }
}

/// What a probe run measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeReport {
    /// The verdict.
    pub verdict: ToneVerdict,
    /// Tap callbacks during the run.
    pub tap_callbacks: u64,
    /// Tone generator callbacks during the run (zero means the output never ran).
    pub output_callbacks: u64,
    /// Samples analysed.
    pub samples: usize,
    /// The share of their energy at the tone's frequency.
    pub tone_fraction: f32,
    /// Their peak.
    pub peak: f32,
}

/// The share of the energy of `samples` (mono) at `hz`: about 1 for a pure tone at `hz`, about 0
/// for other sounds. A Goertzel filter at the exact frequency, normalised by the total energy.
pub fn tone_fraction(samples: &[f32], sample_rate: u32, hz: f32) -> f32 {
    let n = samples.len();
    if n == 0 || sample_rate == 0 {
        return 0.0;
    }
    let w = std::f64::consts::TAU * f64::from(hz) / f64::from(sample_rate);
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2, mut energy) = (0.0f64, 0.0f64, 0.0f64);
    for &x in samples {
        let x = f64::from(x);
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
        energy += x * x;
    }
    if energy == 0.0 {
        return 0.0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    // A sine of amplitude A over n samples has |X|² ≈ (A·n/2)² and energy ≈ A²·n/2.
    (2.0 * power / (n as f64 * energy)) as f32
}

/// Judges what the tap delivered while the tone played. `callbacks` is how often the tap called
/// back; `samples` is mono audio from the tone's window.
pub fn analyse(callbacks: u64, samples: &[f32], sample_rate: u32) -> ToneVerdict {
    if callbacks == 0 || samples.is_empty() {
        return ToneVerdict::NoAudio;
    }
    let mut meter = LevelMeter::new();
    meter.add(samples);
    if meter.peak() < SILENCE_PEAK {
        return ToneVerdict::Silence;
    }
    if tone_fraction(samples, sample_rate, TONE_HZ) >= HEARD_FRACTION {
        ToneVerdict::Heard
    } else {
        ToneVerdict::NotTheTone
    }
}

/// Waits for this process to show up as an audio process object.
fn own_process_object(hal: &dyn ProcessHal) -> Result<u32, PlatformError> {
    let start = Instant::now();
    loop {
        if let Some(object) = hal.process_object_for_pid(own_pid())? {
            return Ok(object);
        }
        if start.elapsed() > OWN_PROCESS_TIMEOUT {
            return Err(PlatformError::Failed(
                "this process did not appear as an audio process while playing the probe tone"
                    .into(),
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Runs the probe: about 0.6 to 1 s. **Worker.**
///
/// It opens an output IOProc on the default output device and a tap of this process. If the
/// permission was never asked, starting the tap is what makes macOS ask, so callers that must
/// not prompt run it only after the app has asked once.
pub(crate) fn run(clock: MacClock) -> Result<ProbeReport, PlatformError> {
    let output = hal::default_device(Direction::Output)?
        .ok_or_else(|| PlatformError::Device("no default output device for the probe".into()))?;
    let output_format = hal::first_stream_format(output, kAudioObjectPropertyScopeOutput)?
        .ok_or_else(|| PlatformError::Device("the output device has no output stream".into()))?;
    let output_format = capture_format(&output_format)
        .map_err(|problem| PlatformError::Device(format!("probe output: {problem}")))?;
    let tone = ToneContext::new(
        unguarded(),
        TONE_HZ,
        TONE_AMPLITUDE,
        output_format.sample_rate,
    );
    // Silence first: this makes the process an audio client, so it has a process object to tap.
    let output_io = RunningIo::start(output, Some(tone_proc), tone).map_err(|(_, e)| e)?;
    let own = own_process_object(&SystemHal)?;

    let mut tap = MacFarEndSource::open(
        TapScope::Only(vec![own]),
        CATapMuteBehavior::MutedWhenTapped,
        clock,
        unguarded(),
    )?;
    let format = tap.format();
    let (producer, mut consumer) = capture_ring(format, Duration::from_secs(2))
        .map_err(|e| PlatformError::Failed(format!("probe ring: {e}")))?;
    tap.start(Box::new(producer))?;
    let start = Instant::now();
    while tap.stats().callbacks == 0 && start.elapsed() < TAP_START_TIMEOUT {
        thread::sleep(Duration::from_millis(10));
    }
    output_io.context().set_playing(true);
    let tone_on = clock.now_ns();
    thread::sleep(TONE_DURATION);
    output_io.context().set_playing(false);
    let tap_callbacks = tap.stats().callbacks;
    let tap_result = tap.stop();
    let output_callbacks = output_io.context().callbacks();
    let output_panics = output_io.context().panics();
    let (_, output_result) = output_io.stop();
    tap_result?;
    output_result.map_err(|e| PlatformError::Device(format!("probe output: {e}")))?;
    if output_panics > 0 {
        return Err(PlatformError::Failed(
            "the probe tone generator panicked".into(),
        ));
    }

    // The tone's window, by host time; the first of each channel only (the tap is mono).
    let from = tone_on + SETTLE.as_nanos() as u64;
    let until = tone_on + TONE_DURATION.as_nanos() as u64;
    let channels = usize::from(format.channels.max(1));
    let mut samples = Vec::new();
    while let Some(captured) = consumer.pop() {
        let block = captured.block;
        if (from..until).contains(&block.host_time_ns) {
            samples.extend(block.samples.iter().step_by(channels));
        }
    }
    let mut meter = LevelMeter::new();
    meter.add(&samples);
    Ok(ProbeReport {
        verdict: analyse(tap_callbacks, &samples, format.sample_rate),
        tap_callbacks,
        output_callbacks,
        samples: samples.len(),
        tone_fraction: tone_fraction(&samples, format.sample_rate, TONE_HZ),
        peak: meter.peak(),
    })
}

/// What a probe report means for the far end in words, for the checklist binary.
pub fn describe(report: &ProbeReport) -> &'static str {
    match report.verdict {
        ToneVerdict::Heard => "the tone came back: system audio capture works",
        ToneVerdict::Silence => "the tap delivered silence: system audio capture is denied",
        ToneVerdict::NotTheTone => "audio came back but not the tone: no answer",
        ToneVerdict::NoAudio if report.output_callbacks == 0 => {
            "the output device never ran: no answer"
        }
        ToneVerdict::NoAudio => "the tap never called back: no answer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, amplitude: f32, rate: u32, seconds: f32) -> Vec<f32> {
        let n = (rate as f32 * seconds) as usize;
        (0..n)
            .map(|i| amplitude * (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn the_tone_back_means_granted() {
        for rate in [44_100, 48_000, 96_000] {
            let samples = sine(TONE_HZ, TONE_AMPLITUDE, rate, 0.3);
            let fraction = tone_fraction(&samples, rate, TONE_HZ);
            assert!(fraction > 0.95, "{rate} Hz: {fraction}");
            assert_eq!(analyse(20, &samples, rate), ToneVerdict::Heard);
        }
        assert_eq!(ToneVerdict::Heard.permission(), PermissionState::Granted);
    }

    /// A denied tap delivers silence, so silence while the tone plays is the denial.
    #[test]
    fn a_denied_tap_delivers_silence_and_that_means_denied() {
        let silence = vec![0.0f32; 14_400];
        assert_eq!(analyse(30, &silence, 48_000), ToneVerdict::Silence);
        assert_eq!(ToneVerdict::Silence.permission(), PermissionState::Denied);
    }

    #[test]
    fn other_audio_or_no_audio_is_no_answer() {
        let other = sine(440.0, 0.3, 48_000, 0.3);
        assert_eq!(analyse(30, &other, 48_000), ToneVerdict::NotTheTone);
        assert_eq!(analyse(0, &[], 48_000), ToneVerdict::NoAudio);
        assert_eq!(analyse(5, &[], 48_000), ToneVerdict::NoAudio);
        assert_eq!(
            ToneVerdict::NotTheTone.permission(),
            PermissionState::Unknown
        );
        assert_eq!(ToneVerdict::NoAudio.permission(), PermissionState::Unknown);
    }

    #[test]
    fn the_tone_is_heard_under_other_sound_but_not_when_buried() {
        let rate = 48_000;
        let tone = sine(TONE_HZ, TONE_AMPLITUDE, rate, 0.3);
        let hum = sine(120.0, TONE_AMPLITUDE * 0.3, rate, 0.3);
        let mixed: Vec<f32> = tone.iter().zip(&hum).map(|(a, b)| a + b).collect();
        assert_eq!(analyse(10, &mixed, rate), ToneVerdict::Heard);
        let loud = sine(300.0, TONE_AMPLITUDE * 3.0, rate, 0.3);
        let buried: Vec<f32> = tone.iter().zip(&loud).map(|(a, b)| a + b).collect();
        assert_eq!(analyse(10, &buried, rate), ToneVerdict::NotTheTone);
    }

    #[test]
    fn the_verdicts_read_in_words() {
        let report = |verdict, output_callbacks| ProbeReport {
            verdict,
            tap_callbacks: 0,
            output_callbacks,
            samples: 0,
            tone_fraction: 0.0,
            peak: 0.0,
        };
        assert!(describe(&report(ToneVerdict::Silence, 5)).contains("denied"));
        assert!(describe(&report(ToneVerdict::NoAudio, 0)).contains("output"));
        assert!(describe(&report(ToneVerdict::NoAudio, 5)).contains("tap"));
    }
}
