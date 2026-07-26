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

    let mut resampler = SincFixedIn::<f32>::new(
        ratio,
        2.0,
        params,
        samples.len(),
        1, // mono
    )
    .map_err(|e| format!("Failed to create resampler: {}", e))?;

    let input = vec![samples.to_vec()]; // 1 channel
    let output = resampler
        .process(&input, None)
        .map_err(|e| format!("Resampling failed: {}", e))?;

    Ok(output[0].clone())
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
/// the noise floor are returned untouched — there is no signal there to rescue.
pub fn normalize_peak(mut samples: Vec<f32>) -> Vec<f32> {
    const TARGET_PEAK: f32 = 0.35;
    /// Below this the buffer is silence, not quiet speech. Leave it alone so
    /// VAD still sees silence and reports honestly.
    const NOISE_FLOOR: f32 = 0.0005;
    /// Caps how far a faint buffer can be lifted, so room tone stays room tone.
    const MAX_GAIN: f32 = 60.0;

    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak < NOISE_FLOOR || peak >= TARGET_PEAK {
        return samples;
    }

    let gain = (TARGET_PEAK / peak).min(MAX_GAIN);
    for s in samples.iter_mut() {
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
    log::info!("Normalised audio: peak {:.6} -> {:.6} (gain {:.1}x)", peak, peak * gain, gain);
    samples
}

#[cfg(test)]
mod normalize_tests {
    use super::normalize_peak;

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
