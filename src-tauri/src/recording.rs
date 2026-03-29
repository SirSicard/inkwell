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
