use rubato::{SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction, Resampler};
use std::path::PathBuf;

const TARGET_SAMPLE_RATE: usize = 16000;

/// Resample audio from source rate to 16kHz mono
pub fn resample_to_16k(samples: &[f32], source_rate: usize) -> Result<Vec<f32>, String> {
    if source_rate == TARGET_SAMPLE_RATE {
        return Ok(samples.to_vec());
    }

    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let ratio = TARGET_SAMPLE_RATE as f64 / source_rate as f64;

    // Input length includes the flush padding added below; SincFixedIn wants
    // its exact chunk size up front.
    let pad = 256; // >= sinc_len / 2 input samples of flush
    let mut resampler = SincFixedIn::<f32>::new(
        ratio,
        2.0,
        params,
        samples.len() + pad,
        1, // mono
    )
    .map_err(|e| format!("Failed to create resampler: {}", e))?;

    // The sinc filter delays its output by half the kernel and keeps that much
    // input as history a single process() call never emits, which used to
    // silently drop the final ~2.7ms of every recording, i.e. the end of the
    // last word. Pad the input with enough zeros to flush the history through,
    // then trim the warm-up delay from the head and cut to the exact expected
    // length, so the output aligns 1:1 with the input.
    let mut padded = Vec::with_capacity(samples.len() + pad);
    padded.extend_from_slice(samples);
    padded.resize(samples.len() + pad, 0.0);

    let input = vec![padded]; // 1 channel
    let mut output = resampler
        .process(&input, None)
        .map_err(|e| format!("Resampling failed: {}", e))?;

    let mut result = output.swap_remove(0);
    let delay = resampler.output_delay();
    if delay < result.len() {
        result.drain(..delay);
    }
    let expected = (samples.len() as f64 * ratio).round() as usize;
    result.truncate(expected);
    Ok(result)
}

/// Save f32 mono samples as a 16kHz WAV file (for debugging)
pub fn save_wav(samples: &[f32], path: &PathBuf) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| format!("Failed to create WAV: {}", e))?;

    for &sample in samples {
        writer
            .write_sample(sample)
            .map_err(|e| format!("Failed to write sample: {}", e))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize WAV: {}", e))?;

    log::info!("Saved WAV: {} ({} samples, {:.1}s)",
        path.display(),
        samples.len(),
        samples.len() as f32 / TARGET_SAMPLE_RATE as f32
    );

    Ok(())
}

/// Peak-normalise a recording to a usable level for VAD and recognition.
///
/// Built-in mic arrays on macOS are read through the raw HAL, which bypasses the
/// voice processing (AGC, beamforming) that normally lifts them, so ordinary
/// speech arrives around -65 dBFS. Left alone, VAD scores the whole buffer as
/// silence and the recogniser decodes a near-flat signal.
///
/// Gain is capped so a quiet room is not amplified into hiss, and buffers below
/// the noise floor are returned untouched, since there is no signal there to rescue.
pub fn normalize_peak(mut samples: Vec<f32>) -> Vec<f32> {
    const TARGET_PEAK: f32 = 0.35;
    /// Below this the buffer is silence, not quiet speech. Leave it alone so
    /// VAD still sees silence and reports honestly.
    const NOISE_FLOOR: f32 = 0.0005;
    /// Caps how far a faint buffer can be lifted, so room tone stays room tone.
    const MAX_GAIN: f32 = 60.0;
    /// 20ms at the 16kHz this function receives (it runs after resampling).
    const FRAME: usize = 320;
    /// How many of the loudest frames to discount as possible transients. A
    /// keyboard click spans one or two 20ms frames; real speech spans dozens.
    const TRANSIENT_FRAMES: usize = 8;

    // Gain is keyed to a robust loudness estimate, not the absolute peak. The
    // absolute peak belongs to whichever single sample is loudest, and on a
    // push-to-talk app that is routinely the hotkey press itself: one keyboard
    // transient at full scale used to make this function decide the take was
    // already loud enough and leave -60 dBFS speech untouched, losing the whole
    // dictation.
    //
    // The estimate is the Nth-loudest frame peak, not a percentile of all
    // frames: a percentile over the whole take keys the gain to however much
    // silence surrounds the speech, so a long recording with a short utterance
    // in it read as "room tone" and got the speech amplified into a square
    // wave. Skipping a fixed handful of top frames ignores clicks whatever the
    // speech-to-silence ratio is, and lands inside the speech itself for any
    // utterance longer than a fraction of a second.
    let mut frame_peaks: Vec<f32> = samples
        .chunks(FRAME)
        .map(|f| f.iter().map(|s| s.abs()).fold(0.0f32, f32::max))
        .collect();
    if frame_peaks.is_empty() {
        return samples;
    }
    frame_peaks.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let robust_peak = frame_peaks[TRANSIENT_FRAMES.min(frame_peaks.len() - 1)];

    if robust_peak < NOISE_FLOOR || robust_peak >= TARGET_PEAK {
        return samples;
    }

    let gain = (TARGET_PEAK / robust_peak).min(MAX_GAIN);
    for s in samples.iter_mut() {
        // The rare transient above the robust estimate hard-clips; a shaved
        // click is harmless where an unamplified dictation was not.
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
    log::info!(
        "Normalised audio: robust peak {:.6} -> {:.6} (gain {:.1}x)",
        robust_peak,
        robust_peak * gain,
        gain
    );
    samples
}

#[cfg(test)]
mod normalize_tests {
    use super::normalize_peak;

    #[test]
    fn a_single_click_does_not_veto_amplifying_quiet_speech() {
        // 2s of -54dBFS speech-level signal with one full-scale keyboard click.
        // The old absolute-peak logic saw the click and returned the buffer
        // untouched; the robust estimate must ignore it and lift the speech.
        let mut samples = vec![0.002f32; 96000];
        samples[48000] = 0.9;
        let out = normalize_peak(samples);
        let speech_level = out[1000].abs();
        assert!(
            speech_level > 0.05,
            "quiet speech was not amplified past a transient: {speech_level}"
        );
    }

    #[test]
    fn lifts_a_quiet_recording_to_the_target() {
        // Peak 0.01 needs 35x, inside the cap, so it reaches the target exactly.
        let quiet = vec![0.01, -0.01, 0.005, -0.005];
        let out = normalize_peak(quiet);
        let peak = out.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!((peak - 0.35).abs() < 0.01, "expected ~0.35 peak, got {peak}");
    }

    #[test]
    fn preserves_relative_dynamics() {
        let out = normalize_peak(vec![0.01, -0.005]);
        // The quieter sample must stay half the loud one, not be squashed flat.
        assert!((out[0] / out[1].abs() - 2.0).abs() < 0.01);
    }

    #[test]
    fn leaves_a_healthy_recording_alone() {
        let healthy = vec![0.5, -0.5, 0.25];
        let out = normalize_peak(healthy.clone());
        assert_eq!(out, healthy);
    }

    #[test]
    fn does_not_amplify_silence_into_hiss() {
        let silence = vec![0.00001, -0.00002, 0.0];
        let out = normalize_peak(silence.clone());
        assert_eq!(out, silence, "sub-noise-floor buffers must pass through");
    }

    #[test]
    fn gain_is_capped() {
        // Just above the noise floor: uncapped gain would be ~583x.
        let faint = vec![0.0006, -0.0006];
        let out = normalize_peak(faint);
        let peak = out.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(peak <= 0.0006 * 60.0 + 1e-6, "gain exceeded the cap: {peak}");
    }

    #[test]
    fn never_clips() {
        let out = normalize_peak(vec![0.01, -0.01, 0.005]);
        assert!(out.iter().all(|s| s.abs() <= 1.0));
    }
}
