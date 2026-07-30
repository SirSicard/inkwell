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
pub struct SpeechEngine {
    recognizer: OfflineRecognizer,
    model_type: ModelType,
    /// True only when the recognizer was built with a BPE vocab and beam
    /// search, i.e. when a hotword stream will actually bias correctly. Asking
    /// for hotwords without the vocab does not fail cleanly: sherpa falls back
    /// to character-unit encoding and builds a nonsense bias graph.
    hotwords_ready: bool,
}

// Safety: the engine is owned by a single thread (see `engine_service`), which
// receives it once by value and never shares it. `Send` covers that move; the
// underlying sherpa-onnx handle is not aliased across threads.
//
// `Sync` was also asserted here, with no argument beyond "the C API is thread
// safe". It was only ever sound because non-async Tauri commands serialise on
// the main thread, and would have quietly stopped holding the moment one of
// them became async, which switch_model now is. Confining the engine to one
// thread removes the need for it entirely.
unsafe impl Send for SpeechEngine {}

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
        Ok(Self { recognizer, model_type, hotwords_ready: false })
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

        // Greedy search was the implicit default, and it is what turned every
        // out-of-vocabulary word into whatever single path the decoder fell
        // into ("Inkwell" went 0-for-5 in the owner's own history: Inco,
        // Ingwell, Enkwell...). Beam search keeps alternatives alive, and it is
        // the prerequisite for hotword biasing below. Only these two method
        // strings exist for NeMo transducers; anything else aborts the process.
        config.decoding_method = Some("modified_beam_search".to_string());

        // Hotword biasing needs the model's BPE vocabulary with merge scores.
        // The published tarball does not ship one, but tokens.txt is the same
        // vocabulary in id order, and sentencepiece's BPE convention scores a
        // piece as minus its merge rank, so the file can be synthesized from
        // what is already on disk. Written once, next to the model.
        let bpe_vocab = model_dir.join("bpe.vocab");
        if !bpe_vocab.exists() {
            match synthesize_bpe_vocab(&tokens, &bpe_vocab) {
                Ok(n) => log::info!("Synthesized bpe.vocab ({} pieces) for hotword biasing", n),
                Err(e) => log::warn!("Could not synthesize bpe.vocab, hotwords disabled: {}", e),
            }
        }
        let hotwords_ready = bpe_vocab.exists();
        if hotwords_ready {
            config.model_config.modeling_unit = Some("bpe".to_string());
            config.model_config.bpe_vocab = Some(bpe_vocab.to_string_lossy().to_string());
        }

        log::info!("Creating Parakeet {} recognizer from {}", variant.to_uppercase(), model_dir.display());
        let recognizer = create_with_provider(&mut config)
            .ok_or(format!("Failed to create Parakeet {} recognizer", variant.to_uppercase()))?;

        log::info!(
            "Loaded Parakeet {} successfully (hotwords {})",
            variant.to_uppercase(),
            if hotwords_ready { "ready" } else { "unavailable" }
        );
        let model_type = match variant {
            "v2" => ModelType::ParakeetV2,
            _ => ModelType::Parakeet,
        };
        Ok(Self { recognizer, model_type, hotwords_ready })
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
        Ok(Self { recognizer, model_type: ModelType::Whisper(variant.to_string()), hotwords_ready: false })
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
        Ok(Self { recognizer, model_type: ModelType::SenseVoice, hotwords_ready: false })
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
        Ok(Self { recognizer, model_type: ModelType::CanaryFlash, hotwords_ready: false })
    }

    /// Transcribe 16kHz mono f32 audio samples.
    ///
    /// `hotwords`: newline-separated phrases (the user's dictionary words) fed
    /// to the decoder as contextual bias, so "Inkwell" can win the beam instead
    /// of being repaired after the fact. Only transducer models support it;
    /// other engines ignore it.
    pub fn transcribe(&self, samples: &[f32], hotwords: Option<&str>) -> Result<String, String> {
        // Long recordings are chunked. The old 15s window came from a
        // misremembered "~20s Parakeet limit"; the model is long-form (the
        // owner's transcript history shows every duplicated-word artifact came
        // from clips crossing the old 15s seams, and none from short clips).
        // 60s keeps memory bounded while making seams rare, and each cut point
        // is chosen at the quietest moment near the target instead of a blind
        // sample offset, so a cut lands in a pause rather than mid-word.
        const CHUNK_SAMPLES: usize = 60 * 16000;
        const OVERLAP_SAMPLES: usize = 32000; // 2s: enough context to re-anchor the merge
        const CUT_SEARCH_RADIUS: usize = 24000; // look up to 1.5s around the target for quiet

        if samples.len() <= CHUNK_SAMPLES + CUT_SEARCH_RADIUS {
            return self.transcribe_chunk(samples, hotwords);
        }

        log::info!("Long audio ({:.1}s): chunking at quiet points near 60s", samples.len() as f32 / 16000.0);
        let mut parts: Vec<String> = Vec::new();
        let mut start = 0;
        while start < samples.len() {
            let target_end = start + CHUNK_SAMPLES;
            let end = if target_end >= samples.len() {
                samples.len()
            } else {
                quietest_cut(samples, target_end, CUT_SEARCH_RADIUS)
            };
            let chunk = &samples[start..end];
            match self.transcribe_chunk(chunk, hotwords) {
                Ok(t) if !t.is_empty() => parts.push(t),
                Ok(_) => {}
                Err(e) => log::warn!("Chunk transcription failed: {}", e),
            }
            if end == samples.len() { break; }
            start = end.saturating_sub(OVERLAP_SAMPLES);
        }

        // Chunks share OVERLAP_SAMPLES of audio, so the tail of one chunk and
        // the head of the next transcribe the same speech; merge drops the
        // duplicated words.
        let text = crate::merge::merge_chunks(&parts);
        log::info!(
            "Transcribed ({:?}): \"{}\" ({:.1}s audio, {} chunks)",
            self.model_type,
            text,
            samples.len() as f32 / 16000.0,
            parts.len()
        );
        Ok(text)
    }

    fn transcribe_chunk(&self, samples: &[f32], hotwords: Option<&str>) -> Result<String, String> {
        // Gated on hotwords_ready, not just the model family: without the BPE
        // vocab wired at construction, sherpa encodes hotwords character-wise
        // and biases toward nonsense instead of failing.
        let stream = match hotwords {
            Some(words) if self.hotwords_ready && !words.trim().is_empty() => {
                self.recognizer.create_stream_with_hotwords(words)
            }
            _ => self.recognizer.create_stream(),
        };
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

/// Write a sentencepiece-style BPE vocab (piece TAB score) derived from the
/// model's own tokens.txt, scoring each piece as minus its id. tokens.txt is
/// the vocabulary in merge-rank order, and sentencepiece's BPE vocab dump uses
/// exactly -rank as the score, so this reconstructs the file the hotword
/// tokenizer needs from what the model already ships. Derived from the deployed
/// tokens.txt rather than downloaded, so it can never disagree with the model.
fn synthesize_bpe_vocab(tokens_path: &std::path::Path, out_path: &std::path::Path) -> Result<usize, String> {
    let content = std::fs::read_to_string(tokens_path)
        .map_err(|e| format!("read {}: {}", tokens_path.display(), e))?;
    let mut lines_out = String::new();
    let mut count = 0usize;
    for line in content.lines() {
        // tokens.txt lines are "piece id"; the piece may itself contain spaces
        // only in pathological vocabs, so split from the right.
        let Some((piece, id)) = line.rsplit_once(' ') else { continue };
        let Ok(id_num) = id.trim().parse::<i64>() else { continue };
        lines_out.push_str(piece);
        lines_out.push('\t');
        lines_out.push_str(&format!("{}", -(id_num as f64)));
        lines_out.push('\n');
        count += 1;
    }
    if count == 0 {
        return Err("tokens.txt yielded no pieces".to_string());
    }
    // Write-then-rename: a crash mid-write must not leave a truncated vocab
    // that exists() would trust forever and mis-tokenize every hotword with.
    let tmp = out_path.with_extension("vocab.partial");
    std::fs::write(&tmp, lines_out).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, out_path).map_err(|e| format!("rename {}: {}", out_path.display(), e))?;
    Ok(count)
}

/// The sample index of the quietest 20ms frame within `radius` of `target`,
/// returned as a cut point for chunking. Cutting at an energy minimum lands in
/// a pause or between words instead of mid-phoneme, which is what turned 15s
/// chunk seams into duplicated half-words ("pro prolific") in the transcript
/// history.
fn quietest_cut(samples: &[f32], target: usize, radius: usize) -> usize {
    const FRAME: usize = 320; // 20ms at 16kHz
    let lo = target.saturating_sub(radius);
    let hi = (target + radius).min(samples.len().saturating_sub(1));
    if lo + FRAME >= hi {
        return target.min(samples.len());
    }
    let mut best_start = target;
    let mut best_energy = f32::MAX;
    let mut i = lo;
    while i + FRAME <= hi {
        let energy: f32 = samples[i..i + FRAME].iter().map(|s| s * s).sum();
        if energy < best_energy {
            best_energy = energy;
            best_start = i;
        }
        i += FRAME;
    }
    // Cut in the middle of the quiet frame, not at its edge.
    (best_start + FRAME / 2).min(samples.len())
}

#[cfg(test)]
mod chunk_tests {
    use super::*;

    fn tone(len: usize) -> Vec<f32> {
        (0..len).map(|i| (i as f32 * 0.3).sin() * 0.5).collect()
    }

    #[test]
    fn cut_lands_in_the_quiet_gap_not_at_the_blind_offset() {
        // Loud speech everywhere except a 100ms silence 0.8s after the target.
        let mut s = tone(40 * 16000);
        let gap_at = 20 * 16000 + 12800;
        for x in &mut s[gap_at..gap_at + 1600] {
            *x = 0.0;
        }
        let cut = quietest_cut(&s, 20 * 16000, 24000);
        assert!(cut >= gap_at && cut <= gap_at + 1600, "cut {} not in gap {}..{}", cut, gap_at, gap_at + 1600);
    }

    #[test]
    fn uniform_audio_cuts_near_the_target() {
        let s = tone(40 * 16000);
        let cut = quietest_cut(&s, 20 * 16000, 24000);
        assert!((cut as i64 - (20 * 16000) as i64).unsigned_abs() as usize <= 24000 + 320);
    }

    #[test]
    fn cut_never_exceeds_buffer() {
        let s = tone(1000);
        assert!(quietest_cut(&s, 900, 24000) <= 1000);
        assert!(quietest_cut(&s, 5, 2) <= 1000);
    }

    #[test]
    fn bpe_vocab_is_piece_tab_negative_rank() {
        let dir = std::env::temp_dir().join("inkwell-bpe-vocab-test");
        std::fs::create_dir_all(&dir).unwrap();
        let tokens = dir.join("tokens.txt");
        let out = dir.join("bpe.vocab");
        std::fs::write(&tokens, "<blk> 0\n\u{2581}the 1\ning 2\n").unwrap();
        let n = synthesize_bpe_vocab(&tokens, &out).unwrap();
        assert_eq!(n, 3);
        let written = std::fs::read_to_string(&out).unwrap();
        assert_eq!(written, "<blk>\t-0\n\u{2581}the\t-1\ning\t-2\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_tokens_file_is_an_error_not_an_empty_vocab() {
        let dir = std::env::temp_dir().join("inkwell-bpe-vocab-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let tokens = dir.join("tokens.txt");
        std::fs::write(&tokens, "").unwrap();
        assert!(synthesize_bpe_vocab(&tokens, &dir.join("bpe.vocab")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
