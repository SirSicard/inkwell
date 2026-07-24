use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::sync::{Mutex, OnceLock};

const WINDOW_SIZE: usize = 512;

/// The live detector plus the config it was built from.
/// Creating one loads the Silero model off disk on every dictation, which is
/// the single biggest fixed cost in the pipeline, so we keep the last one.
struct CachedVad {
    model_path: String,
    threshold_bits: u32,
    vad: VoiceActivityDetector,
}

// SAFETY: sherpa-onnx's C API is thread-safe, and the detector is only ever
// reachable while the cache mutex is held, so at most one thread drives it at a
// time. Same contract as SpeechEngine in engine.rs.
unsafe impl Send for CachedVad {}

fn cache() -> &'static Mutex<Option<CachedVad>> {
    static CACHE: OnceLock<Mutex<Option<CachedVad>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn create_detector(model_path: &str, threshold: f32) -> Result<VoiceActivityDetector, String> {
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

    VoiceActivityDetector::create(&vad_config, 30.0)
        .ok_or_else(|| "Failed to create VoiceActivityDetector".to_string())
}

/// Run Silero VAD on audio samples to extract only speech segments.
/// Input: 16kHz mono f32 samples.
/// Returns: samples with silence removed.
///
/// The detector is cached across calls and rebuilt only when the model path or
/// threshold changes. Concurrent callers serialize on the cache mutex.
pub fn remove_silence(samples: &[f32], model_path: &str, threshold: f32) -> Result<Vec<f32>, String> {
    // A panic mid-detection must not disable VAD for the rest of the session.
    let mut guard = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let threshold_bits = threshold.to_bits();
    let reusable = guard
        .as_ref()
        .is_some_and(|c| c.model_path == model_path && c.threshold_bits == threshold_bits);

    if !reusable {
        *guard = Some(CachedVad {
            model_path: model_path.to_string(),
            threshold_bits,
            vad: create_detector(model_path, threshold)?,
        });
        log::info!("VAD: detector built for {}", model_path);
    }

    let vad = &guard.as_ref().expect("cache populated above").vad;

    // A reused detector still holds state and queued segments from last time.
    vad.reset();
    vad.clear();

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
