use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
use std::path::{Path, PathBuf};

/// Detect GPU availability and log it. Returns the provider to use.
/// sherpa-onnx static builds are CPU-only. GPU needs shared builds with
/// CUDA-specific archives. For now we detect and log for future use.
fn detect_provider() -> &'static str {
    use std::sync::OnceLock;
    static PROVIDER: OnceLock<&'static str> = OnceLock::new();

    PROVIDER.get_or_init(|| {
        let has_nvidia = std::process::Command::new("nvidia-smi")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if has_nvidia {
            // GPU detected but static sherpa-onnx builds are CPU-only.
            // CUDA support requires shared builds with CUDA-specific archives.
            // TODO: support shared+CUDA builds for GPU acceleration
            log::info!("NVIDIA GPU detected. Static build = CPU-only inference. \
                        GPU acceleration requires shared+CUDA build (future release).");
        } else {
            log::info!("No NVIDIA GPU detected. Using CPU provider.");
        }

        "cpu"
    })
}

/// Create a recognizer with the appropriate provider.
fn create_with_provider(config: &mut OfflineRecognizerConfig) -> Option<OfflineRecognizer> {
    let provider = detect_provider();
    config.model_config.provider = Some(provider.to_string());
    OfflineRecognizer::create(config)
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

    /// Transcribe 16kHz mono f32 audio samples
    pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
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
