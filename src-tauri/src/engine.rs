use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
use std::path::{Path, PathBuf};

/// sherpa-onnx is linked statically here, and static builds are CPU-only.
/// Every accelerator path (CoreML on macOS, CUDA elsewhere) needs a shared
/// build with provider-specific archives, so there is nothing to probe for.
const PROVIDER: &str = "cpu";

/// Create a recognizer with the appropriate provider.
fn create_with_provider(config: &mut OfflineRecognizerConfig) -> Option<OfflineRecognizer> {
    config.model_config.provider = Some(PROVIDER.to_string());
    OfflineRecognizer::create(config)
}

/// Longest run of words tested for a repeat at a chunk seam. 0.5s of overlap
/// is at most ~4 words of speech; 24 leaves headroom for fast speakers.
const MAX_SEAM_WORDS: usize = 24;

/// Shortest normalized match accepted at a seam. Stops a bare "a" or "the"
/// landing on both sides from eating a real word.
const MIN_SEAM_CHARS: usize = 4;

/// Compare words ignoring case and punctuation — the model does not place the
/// same comma in the same spot on both sides of a seam.
fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// How many leading words of `next` restate the trailing words of `prev`.
fn seam_overlap(prev: &[String], next: &[String]) -> usize {
    let max = MAX_SEAM_WORDS.min(prev.len()).min(next.len());
    for k in (1..=max).rev() {
        let tail = &prev[prev.len() - k..];
        let head = &next[..k];
        if !tail.iter().zip(head).all(|(a, b)| normalize_word(a) == normalize_word(b)) {
            continue;
        }
        let matched_chars: usize = head.iter().map(|w| normalize_word(w).chars().count()).sum();
        if matched_chars >= MIN_SEAM_CHARS {
            return k;
        }
    }
    0
}

/// Join chunk transcripts, dropping the words the overlap made the model emit
/// twice. Chunks share OVERLAP_SAMPLES of audio, so the tail of one chunk and
/// the head of the next transcribe the same speech.
fn merge_chunks(parts: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        let next: Vec<String> = part.split_whitespace().map(|s| s.to_string()).collect();
        if next.is_empty() {
            continue;
        }
        if out.is_empty() {
            out = next;
            continue;
        }
        let skip = seam_overlap(&out, &next);
        if skip > 0 {
            log::debug!("Seam dedup: dropped {} repeated word(s)", skip);
        }
        out.extend_from_slice(&next[skip..]);
    }
    out.join(" ")
}

/// Find first existing file from a list of candidates in a directory
fn find_file(dir: &Path, candidates: &[&str]) -> Option<PathBuf> {
    for name in candidates {
        let path = dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Supported model types
#[derive(Debug, Clone, PartialEq)]
pub enum ModelType {
    MoonshineTiny,
    MoonshineBase,
    MoonshineMedium,
    Parakeet,
    ParakeetV2,
    Whisper(String), // model name, e.g. "small", "turbo"
    SenseVoice,
    CanaryFlash,
}

/// A speech recognition engine wrapping sherpa-onnx OfflineRecognizer.
/// Safety: sherpa-onnx's C API is thread-safe. The raw pointer inside
/// OfflineRecognizer is only accessed through its safe Rust methods.
pub struct SpeechEngine {
    recognizer: OfflineRecognizer,
    model_type: ModelType,
}

unsafe impl Send for SpeechEngine {}
unsafe impl Sync for SpeechEngine {}

impl SpeechEngine {
    /// Create a Moonshine engine
    /// V1 models need: preprocessor, encoder, uncached_decoder, cached_decoder
    /// V2 models need: encoder, merged_decoder
    pub fn moonshine(models_dir: &Path, variant: &str) -> Result<Self, String> {
        let model_dir = models_dir.join(format!("moonshine-{}", variant));

        let tokens = model_dir.join("tokens.txt");
        if !tokens.exists() {
            return Err(format!("Moonshine {} tokens not found at {}", variant, tokens.display()));
        }

        let mut config = OfflineRecognizerConfig::default();

        // Check if this is a V2 model (has merged_decoder) or V1 (has separate files)
        let merged_decoder = find_file(&model_dir, &["merged_decoder.int8.onnx", "merged_decoder.onnx"]);

        if let Some(merged) = merged_decoder {
            // V2: encoder + merged_decoder
            let encoder = find_file(&model_dir, &["encoder.int8.onnx", "encoder.onnx", "encode.int8.onnx", "encode.onnx"])
                .ok_or(format!("Moonshine {} encoder not found in {}", variant, model_dir.display()))?;

            config.model_config.moonshine.encoder = Some(encoder.to_string_lossy().into_owned());
            config.model_config.moonshine.merged_decoder = Some(merged.to_string_lossy().into_owned());
            log::info!("Using Moonshine V2 layout (encoder + merged_decoder)");
        } else {
            // V1: preprocessor + encoder + uncached_decoder + cached_decoder
            let preprocessor = find_file(&model_dir, &["preprocess.onnx", "preprocessor.onnx"])
                .ok_or(format!("Moonshine {} preprocessor not found in {}", variant, model_dir.display()))?;
            let encoder = find_file(&model_dir, &["encode.int8.onnx", "encoder.int8.onnx", "encode.onnx", "encoder.onnx"])
                .ok_or(format!("Moonshine {} encoder not found in {}", variant, model_dir.display()))?;
            let uncached = find_file(&model_dir, &["uncached_decode.int8.onnx", "uncached_decoder.int8.onnx", "uncached_decode.onnx"])
                .ok_or(format!("Moonshine {} uncached_decoder not found in {}", variant, model_dir.display()))?;
            let cached = find_file(&model_dir, &["cached_decode.int8.onnx", "cached_decoder.int8.onnx", "cached_decode.onnx"])
                .ok_or(format!("Moonshine {} cached_decoder not found in {}", variant, model_dir.display()))?;

            config.model_config.moonshine.preprocessor = Some(preprocessor.to_string_lossy().into_owned());
            config.model_config.moonshine.encoder = Some(encoder.to_string_lossy().into_owned());
            config.model_config.moonshine.uncached_decoder = Some(uncached.to_string_lossy().into_owned());
            config.model_config.moonshine.cached_decoder = Some(cached.to_string_lossy().into_owned());
            log::info!("Using Moonshine V1 layout (preprocessor + encoder + uncached + cached)");
        }

        config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
        config.model_config.num_threads = 4;

        log::info!("Creating Moonshine {} recognizer from {}", variant, model_dir.display());
        let recognizer = create_with_provider(&mut config)
            .ok_or(format!("Failed to create Moonshine {} recognizer", variant))?;

        let model_type = match variant {
            "tiny" => ModelType::MoonshineTiny,
            "base" => ModelType::MoonshineBase,
            _ => ModelType::MoonshineMedium,
        };

        log::info!("Loaded Moonshine {} successfully", variant);
        Ok(Self { recognizer, model_type })
    }

    /// Create a Parakeet engine (NeMo transducer)
    /// variant: "v3" (25 European languages) or "v2" (English only)
    pub fn parakeet(models_dir: &Path) -> Result<Self, String> {
        Self::parakeet_variant(models_dir, "v3")
    }

    pub fn parakeet_v2(models_dir: &Path) -> Result<Self, String> {
        Self::parakeet_variant(models_dir, "v2")
    }

    fn parakeet_variant(models_dir: &Path, variant: &str) -> Result<Self, String> {
        let model_dir = models_dir.join(format!("parakeet-{}", variant));

        let encoder = find_file(&model_dir, &["encoder.int8.onnx", "encoder.onnx"])
            .ok_or(format!("Parakeet V3 encoder not found in {}", model_dir.display()))?;
        let decoder = find_file(&model_dir, &["decoder.int8.onnx", "decoder.onnx"])
            .ok_or(format!("Parakeet V3 decoder not found in {}", model_dir.display()))?;
        let joiner = find_file(&model_dir, &["joiner.int8.onnx", "joiner.onnx"])
            .ok_or(format!("Parakeet V3 joiner not found in {}", model_dir.display()))?;
        let tokens = model_dir.join("tokens.txt");

        if !tokens.exists() {
            return Err(format!("Parakeet V3 tokens not found at {}", tokens.display()));
        }

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer.encoder = Some(encoder.to_string_lossy().to_string());
        config.model_config.transducer.decoder = Some(decoder.to_string_lossy().to_string());
        config.model_config.transducer.joiner = Some(joiner.to_string_lossy().to_string());
        config.model_config.tokens = Some(tokens.to_string_lossy().to_string());
        config.model_config.model_type = Some("nemo_transducer".to_string());
        config.model_config.num_threads = 4;

        log::info!("Creating Parakeet {} recognizer from {}", variant.to_uppercase(), model_dir.display());
        let recognizer = create_with_provider(&mut config)
            .ok_or(format!("Failed to create Parakeet {} recognizer", variant.to_uppercase()))?;

        log::info!("Loaded Parakeet {} successfully", variant.to_uppercase());
        let model_type = match variant {
            "v2" => ModelType::ParakeetV2,
            _ => ModelType::Parakeet,
        };
        Ok(Self { recognizer, model_type })
    }

    /// Create a Whisper engine (multilingual, 99 languages)
    pub fn whisper(models_dir: &Path, variant: &str) -> Result<Self, String> {
        let model_dir = models_dir.join(format!("whisper-{}", variant));

        let encoder = find_file(&model_dir, &[
            &format!("{}-encoder.int8.onnx", variant),
            &format!("{}-encoder.onnx", variant),
            "encoder.int8.onnx", "encoder.onnx",
        ]).ok_or(format!("Whisper {} encoder not found in {}", variant, model_dir.display()))?;

        let decoder = find_file(&model_dir, &[
            &format!("{}-decoder.int8.onnx", variant),
            &format!("{}-decoder.onnx", variant),
            "decoder.int8.onnx", "decoder.onnx",
        ]).ok_or(format!("Whisper {} decoder not found in {}", variant, model_dir.display()))?;

        let tokens = find_file(&model_dir, &[
            &format!("{}-tokens.txt", variant),
            "tokens.txt",
        ]).ok_or(format!("Whisper {} tokens not found in {}", variant, model_dir.display()))?;

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.whisper.encoder = Some(encoder.to_string_lossy().into_owned());
        config.model_config.whisper.decoder = Some(decoder.to_string_lossy().into_owned());
        config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
        config.model_config.num_threads = 4;

        log::info!("Creating Whisper {} recognizer from {}", variant, model_dir.display());
        let recognizer = create_with_provider(&mut config)
            .ok_or(format!("Failed to create Whisper {} recognizer", variant))?;

        log::info!("Loaded Whisper {} successfully", variant);
        Ok(Self { recognizer, model_type: ModelType::Whisper(variant.to_string()) })
    }

    /// Create a SenseVoice engine (zh, en, ja, ko, yue)
    pub fn sense_voice(models_dir: &Path) -> Result<Self, String> {
        let model_dir = models_dir.join("sense-voice");

        let model = find_file(&model_dir, &["model.int8.onnx", "model.onnx"])
            .ok_or(format!("SenseVoice model not found in {}", model_dir.display()))?;
        let tokens = model_dir.join("tokens.txt");

        if !tokens.exists() {
            return Err(format!("SenseVoice tokens not found at {}", tokens.display()));
        }

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.sense_voice.model = Some(model.to_string_lossy().to_string());
        config.model_config.tokens = Some(tokens.to_string_lossy().to_string());
        config.model_config.num_threads = 4;

        log::info!("Creating SenseVoice recognizer from {}", model_dir.display());
        let recognizer = create_with_provider(&mut config)
            .ok_or("Failed to create SenseVoice recognizer")?;

        log::info!("Loaded SenseVoice successfully");
        Ok(Self { recognizer, model_type: ModelType::SenseVoice })
    }

    /// Create a Canary 180M Flash engine (en, es, de, fr)
    pub fn canary_flash(models_dir: &Path) -> Result<Self, String> {
        let model_dir = models_dir.join("canary-flash");

        let encoder = find_file(&model_dir, &["encoder.int8.onnx", "encoder.onnx"])
            .ok_or(format!("Canary Flash encoder not found in {}", model_dir.display()))?;
        let decoder = find_file(&model_dir, &["decoder.int8.onnx", "decoder.onnx"])
            .ok_or(format!("Canary Flash decoder not found in {}", model_dir.display()))?;
        let tokens = model_dir.join("tokens.txt");

        if !tokens.exists() {
            return Err(format!("Canary Flash tokens not found at {}", tokens.display()));
        }

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.canary.encoder = Some(encoder.to_string_lossy().into_owned());
        config.model_config.canary.decoder = Some(decoder.to_string_lossy().into_owned());
        config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
        config.model_config.num_threads = 4;

        log::info!("Creating Canary Flash recognizer from {}", model_dir.display());
        let recognizer = create_with_provider(&mut config)
            .ok_or("Failed to create Canary Flash recognizer")?;

        log::info!("Loaded Canary Flash successfully");
        Ok(Self { recognizer, model_type: ModelType::CanaryFlash })
    }

    /// Transcribe 16kHz mono f32 audio samples
    pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
        // Chunk long recordings to avoid Parakeet's ~20s limit.
        // Split at 15s with 0.5s overlap, join results.
        const CHUNK_SAMPLES: usize = 15 * 16000; // 15s at 16kHz
        const OVERLAP_SAMPLES: usize = 8000;     // 0.5s overlap

        if samples.len() <= CHUNK_SAMPLES {
            return self.transcribe_chunk(samples);
        }

        log::info!("Long audio ({:.1}s): chunking into 15s segments", samples.len() as f32 / 16000.0);
        let mut parts: Vec<String> = Vec::new();
        let mut start = 0;
        while start < samples.len() {
            let end = (start + CHUNK_SAMPLES).min(samples.len());
            let chunk = &samples[start..end];
            match self.transcribe_chunk(chunk) {
                Ok(t) if !t.is_empty() => parts.push(t),
                Ok(_) => {}
                Err(e) => log::warn!("Chunk transcription failed: {}", e),
            }
            if end == samples.len() { break; }
            start += CHUNK_SAMPLES - OVERLAP_SAMPLES;
        }

        let text = merge_chunks(&parts);
        log::info!(
            "Transcribed ({:?}): \"{}\" ({:.1}s audio, {} chunks)",
            self.model_type,
            text,
            samples.len() as f32 / 16000.0,
            parts.len()
        );
        Ok(text)
    }

    fn transcribe_chunk(&self, samples: &[f32]) -> Result<String, String> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(16000, samples);
        self.recognizer.decode(&stream);

        let result = stream.get_result()
            .ok_or("Failed to get recognition result")?;

        let text = result.text.trim().to_string();
        log::info!(
            "Transcribed ({:?}): \"{}\" ({:.1}s audio)",
            self.model_type,
            text,
            samples.len() as f32 / 16000.0
        );

        Ok(text)
    }

    pub fn model_type(&self) -> &ModelType {
        &self.model_type
    }
}

#[cfg(test)]
mod merge_tests {
    use super::{merge_chunks, seam_overlap};

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(|w| w.to_string()).collect()
    }

    #[test]
    fn joins_chunks_with_no_overlap() {
        let parts = vec!["the quick brown fox".to_string(), "jumps over".to_string()];
        assert_eq!(merge_chunks(&parts), "the quick brown fox jumps over");
    }

    #[test]
    fn drops_repeated_seam_words() {
        let parts = vec![
            "we should ship it on monday".to_string(),
            "on monday morning at nine".to_string(),
        ];
        assert_eq!(
            merge_chunks(&parts),
            "we should ship it on monday morning at nine"
        );
    }

    #[test]
    fn seam_match_ignores_case_and_punctuation() {
        let parts = vec![
            "that is the plan, agreed".to_string(),
            "Agreed? then let's go".to_string(),
        ];
        assert_eq!(merge_chunks(&parts), "that is the plan, agreed then let's go");
    }

    #[test]
    fn prefers_the_longest_overlap() {
        let prev = words("alpha beta gamma delta");
        let next = words("beta gamma delta epsilon");
        assert_eq!(seam_overlap(&prev, &next), 3);
    }

    #[test]
    fn short_common_word_is_not_treated_as_overlap() {
        // "the" straddling the seam is a coincidence, not a repeat.
        let prev = words("we looked at the");
        let next = words("the report was late");
        assert_eq!(seam_overlap(&prev, &next), 0);
        assert_eq!(
            merge_chunks(&[prev.join(" "), next.join(" ")]),
            "we looked at the the report was late"
        );
    }

    #[test]
    fn skips_empty_chunks() {
        let parts = vec!["hello".to_string(), String::new(), "world".to_string()];
        assert_eq!(merge_chunks(&parts), "hello world");
    }

    #[test]
    fn single_chunk_is_unchanged() {
        let parts = vec!["just one chunk".to_string()];
        assert_eq!(merge_chunks(&parts), "just one chunk");
    }

    #[test]
    fn full_chunk_repeat_collapses() {
        let parts = vec!["identical phrase here".to_string(), "identical phrase here".to_string()];
        assert_eq!(merge_chunks(&parts), "identical phrase here");
    }
}
