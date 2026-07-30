use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

const WINDOW_SIZE: usize = 512;

/// Whether the Silero model is usable. Tracked globally so the pipeline can say
/// *why* VAD is off (never installed / still downloading / fetch failed) rather
/// than skipping silence removal in silence. Set by `setup::ensure_vad_model`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VadStatus {
    Missing = 0,
    Downloading = 1,
    Ready = 2,
    Failed = 3,
}

static STATUS: AtomicU8 = AtomicU8::new(VadStatus::Missing as u8);

pub fn set_status(status: VadStatus) {
    STATUS.store(status as u8, Ordering::Relaxed);
}

pub fn status() -> VadStatus {
    match STATUS.load(Ordering::Relaxed) {
        1 => VadStatus::Downloading,
        2 => VadStatus::Ready,
        3 => VadStatus::Failed,
        _ => VadStatus::Missing,
    }
}

/// User-facing explanation for a dictation that had to run without VAD.
pub fn unavailable_reason() -> &'static str {
    match status() {
        VadStatus::Downloading => {
            "Voice-activity model is still downloading, so this recording was transcribed without silence removal."
        }
        VadStatus::Failed => {
            "Voice-activity model could not be downloaded. Dictation still works, but silence is not trimmed. Run scripts/download-models.sh to install it manually."
        }
        _ => "Voice-activity model is not installed. Dictation still works, but silence is not trimmed.",
    }
}

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

/// Trim leading and trailing silence from 16kHz mono samples using Silero VAD.
/// Everything between the first and last detected speech, pauses included,
/// survives; only the dead air at the ends is removed. Returns an empty vec
/// when no speech is detected, and callers fall back to the raw buffer.
///
/// The detector is cached across calls and rebuilt only when the model path or
/// threshold changes. Concurrent callers serialize on the cache mutex.
pub fn trim_silence(samples: &[f32], model_path: &str, threshold: f32) -> Result<Vec<f32>, String> {
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

    // Trim, don't gate. This used to butt-join every detected speech segment
    // and throw the pauses away, which harmed accuracy twice over: mid-speech
    // pauses longer than 250ms were excised (splicing unrelated phonemes hard
    // against each other), and the spliced stream left the long-audio chunker
    // no silence to cut at, so chunk seams landed mid-word by construction.
    // The transcript history showed both signatures. Dictation only needs the
    // dead air at the ends removed; the model handles pauses fine.
    let mut first_start: Option<usize> = None;
    let mut last_end: usize = 0;

    let mut note = |seg_start: i32, seg_len: usize| {
        let start = seg_start.max(0) as usize;
        if first_start.is_none() {
            first_start = Some(start);
        }
        last_end = last_end.max(start + seg_len);
    };

    for chunk in samples.chunks(WINDOW_SIZE) {
        vad.accept_waveform(chunk);
        while let Some(seg) = vad.front() {
            note(seg.start(), seg.samples().len());
            vad.pop();
        }
    }
    vad.flush();
    while let Some(seg) = vad.front() {
        note(seg.start(), seg.samples().len());
        vad.pop();
    }

    let Some(first_start) = first_start else {
        // No speech found at all; the caller falls back to the raw buffer.
        log::info!("VAD: no speech detected in {} samples", samples.len());
        return Ok(Vec::new());
    };

    // Silero's onset trigger lags soft attacks, and the last word's decay
    // matters too; keep a pad on both ends so no phoneme is shaved off.
    const EDGE_PAD: usize = 4000; // 250ms at 16kHz
    let lo = first_start.saturating_sub(EDGE_PAD);
    let hi = (last_end + EDGE_PAD).min(samples.len());
    let trimmed = samples[lo..hi].to_vec();

    log::info!(
        "VAD: trimmed {} -> {} samples ({:.1}s of edge silence removed)",
        samples.len(),
        trimmed.len(),
        (samples.len() - trimmed.len()) as f32 / 16000.0
    );

    Ok(trimmed)
}
