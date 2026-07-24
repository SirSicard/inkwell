//! Synthesized audio feedback for hotkey events.
//! Generates tones at runtime — no external files needed.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};

static SOUNDS_ENABLED: AtomicBool = AtomicBool::new(true);
static AGENT_SOUNDS_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_dictation_sounds(enabled: bool) {
    SOUNDS_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn set_agent_sounds(enabled: bool) {
    AGENT_SOUNDS_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn dictation_sounds_enabled() -> bool {
    SOUNDS_ENABLED.load(Ordering::Relaxed)
}

pub fn agent_sounds_enabled() -> bool {
    AGENT_SOUNDS_ENABLED.load(Ordering::Relaxed)
}

/// Dictation recording started — soft ascending two-note chime.
pub fn play_dictation_start() {
    if !SOUNDS_ENABLED.load(Ordering::Relaxed) { return; }
    std::thread::spawn(|| play_tone(&[
        Note { freq: 880.0, duration_ms: 60, volume: 0.15 },
        Note { freq: 1108.73, duration_ms: 80, volume: 0.12 },  // C#6, ascending
    ]));
}

/// Dictation recording stopped — soft descending tone.
pub fn play_dictation_stop() {
    if !SOUNDS_ENABLED.load(Ordering::Relaxed) { return; }
    std::thread::spawn(|| play_tone(&[
        Note { freq: 1108.73, duration_ms: 50, volume: 0.10 },
        Note { freq: 830.61, duration_ms: 70, volume: 0.08 },   // Ab5, descending
    ]));
}

/// Agent recording started — techy synth pulse (higher pitch, sharper attack).
pub fn play_agent_start() {
    if !AGENT_SOUNDS_ENABLED.load(Ordering::Relaxed) { return; }
    std::thread::spawn(|| play_tone(&[
        Note { freq: 1318.51, duration_ms: 40, volume: 0.15 },  // E6
        Note { freq: 1567.98, duration_ms: 40, volume: 0.15 },  // G6
        Note { freq: 1975.53, duration_ms: 60, volume: 0.12 },  // B6 — ascending triad
    ]));
}

/// Agent recording stopped — descending synth.
pub fn play_agent_stop() {
    if !AGENT_SOUNDS_ENABLED.load(Ordering::Relaxed) { return; }
    std::thread::spawn(|| play_tone(&[
        Note { freq: 1567.98, duration_ms: 40, volume: 0.10 },
        Note { freq: 1174.66, duration_ms: 60, volume: 0.08 },  // D6, descending
    ]));
}

struct Note {
    freq: f32,
    duration_ms: u32,
    volume: f32,
}

fn play_tone(notes: &[Note]) {
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            log::warn!("No audio output device for feedback sounds");
            return;
        }
    };

    let config = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to get output config: {}", e);
            return;
        }
    };

    let sample_rate_f = config.sample_rate() as f32;
    let channels = config.channels() as usize;

    // Pre-render all samples
    let mut samples: Vec<f32> = Vec::new();
    for note in notes {
        let num_samples = (sample_rate_f * note.duration_ms as f32 / 1000.0) as usize;
        for i in 0..num_samples {
            let t = i as f32 / sample_rate_f;
            // Sine wave with fast exponential decay (soft, non-harsh)
            let envelope = (-t * 1000.0 / note.duration_ms as f32 * 3.0).exp();
            // Fade in first 5% to avoid click
            let fade_in = if i < num_samples / 20 {
                i as f32 / (num_samples as f32 / 20.0)
            } else {
                1.0
            };
            let sample = (t * note.freq * 2.0 * std::f32::consts::PI).sin()
                * note.volume
                * envelope
                * fade_in;
            // Duplicate for all channels
            for _ in 0..channels {
                samples.push(sample);
            }
        }
    }

    let total_samples = samples.len();
    let sample_idx = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sample_data = std::sync::Arc::new(samples);
    let done = std::sync::Arc::new(AtomicBool::new(false));

    let idx = sample_idx.clone();
    let data = sample_data.clone();
    let done_flag = done.clone();

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config.into(),
            move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for sample in output.iter_mut() {
                    let i = idx.fetch_add(1, Ordering::Relaxed);
                    if i < data.len() {
                        *sample = data[i];
                    } else {
                        *sample = 0.0;
                        done_flag.store(true, Ordering::Relaxed);
                    }
                }
            },
            |e| log::warn!("Audio output error: {}", e),
            None,
        ),
        _ => {
            log::warn!("Unsupported sample format for feedback sounds");
            return;
        }
    };

    match stream {
        Ok(s) => {
            if let Err(e) = s.play() {
                log::warn!("Failed to play feedback sound: {}", e);
                return;
            }
            // Wait for playback to finish (with timeout)
            let total_duration_ms: u32 = notes.iter().map(|n| n.duration_ms).sum();
            let timeout = std::time::Duration::from_millis(total_duration_ms as u64 + 100);
            let start = std::time::Instant::now();
            while !done.load(Ordering::Relaxed) && start.elapsed() < timeout {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            // Small tail for decay
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        Err(e) => log::warn!("Failed to build audio stream: {}", e),
    }
}
