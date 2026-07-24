use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};

const WINDOW_SIZE: usize = 512;

/// Run Silero VAD on audio samples to extract only speech segments.
/// Input: 16kHz mono f32 samples.
/// Returns: samples with silence removed.
pub fn remove_silence(samples: &[f32], model_path: &str, threshold: f32) -> Result<Vec<f32>, String> {
    let mut silero_config = SileroVadModelConfig::default();
    silero_config.model = Some(model_path.to_string());
    silero_config.threshold = threshold;
    silero_config.min_silence_duration = 0.25;
    silero_config.min_speech_duration = 0.25;
    silero_config.max_speech_duration = 30.0; // allow long utterances

    let vad_config = VadModelConfig {
        silero_vad: silero_config,
        ten_vad: Default::default(),
        sample_rate: 16000,
        num_threads: 1,
        provider: Some("cpu".to_string()),
        debug: false,
    };

    let vad = VoiceActivityDetector::create(&vad_config, 30.0)
        .ok_or("Failed to create VoiceActivityDetector")?;

    let mut speech_samples = Vec::new();

    for chunk in samples.chunks(WINDOW_SIZE) {
        vad.accept_waveform(chunk);

        while let Some(seg) = vad.front() {
            speech_samples.extend_from_slice(seg.samples());
            vad.pop();
        }
    }

    // Flush remaining
    vad.flush();
    while let Some(seg) = vad.front() {
        speech_samples.extend_from_slice(seg.samples());
        vad.pop();
    }

    let removed_pct = if !samples.is_empty() {
        100.0 * (1.0 - speech_samples.len() as f32 / samples.len() as f32)
    } else {
        0.0
    };

    log::info!(
        "VAD: {} -> {} samples ({:.1}% silence removed)",
        samples.len(),
        speech_samples.len(),
        removed_pct
    );

    Ok(speech_samples)
}
