//! Parakeet TDT v3 (int8) through sherpa-onnx's C API on the CPU (`engine-sherpa`, Windows): the
//! non-Apple Parakeet, as an [`OfflineEngine`]. On the Mac, Parakeet runs in FluidAudio instead.
//!
//! - **One pass per call**, greedy search, up to [`MAX_SECONDS`] of audio; longer audio is refused
//!   rather than cut (the callers that use it hand over short windows: live partials re-decode a
//!   trailing window, a dictation take is short). The whole call is one segment.
//! - **The CPU**, with one thread per physical core. ONNX Runtime's other execution providers are
//!   not in this build of sherpa-onnx.
//! - **Cancellation** is checked before the decode, which cannot be interrupted once started.
//!
//! # Building
//!
//! The library is not built by cargo. sherpa-onnx 1.13.4's prebuilt Windows libraries, the
//! "shared, MD, Release, no-tts" archive, are unpacked outside the repository; `SHERPA_ONNX_DIR`
//! names that directory, and `build.rs` checks each file it uses against its pinned SHA-256 before
//! linking. That archive is built without text-to-speech, so it carries none of espeak-ng
//! (GPL-3.0) or piper-phonemize, which sherpa-onnx's default static bundle links. Nothing is
//! downloaded at build time: the sherpa-onnx crates, whose build script fetches archives, are not
//! used. The declarations below follow sherpa-onnx 1.13.4's C API (`c-api.h`, Apache-2.0), and
//! [`SherpaParakeet::load`] refuses a library that reports another version.
//!
//! # Threads
//!
//! Every call is a **worker** call. A recognizer may create streams on several threads, but one
//! decode at a time runs here: the recognizer is used under a mutex, which costs nothing for the
//! short windows this engine takes.

#![warn(clippy::undocumented_unsafe_blocks)]

use std::ffi::{CStr, CString, c_char};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Mutex;

use ink_core::{
    CANONICAL_RATE, EngineError, EngineInfo, OfflineEngine, TimedText, TranscribeOptions,
    Transcript,
};

use crate::compute::physical_cores;
use crate::lock;

/// The sherpa-onnx release the declarations below are written against.
pub const SHERPA_ONNX_VERSION: &str = "1.13.4";

/// The longest audio transcribed in one call, in seconds.
pub const MAX_SECONDS: u32 = 90;

/// sherpa-onnx 1.13.4's C API: the offline recognizer, as `c-api.h` declares it. Every struct is
/// complete: the library reads the whole configuration.
#[allow(dead_code)] // Most fields are there for the layout only.
mod ffi {
    use std::ffi::{c_char, c_float};

    #[repr(C)]
    pub struct FeatureConfig {
        pub sample_rate: i32,
        pub feature_dim: i32,
    }

    #[repr(C)]
    pub struct HomophoneReplacerConfig {
        pub dict_dir: *const c_char,
        pub lexicon: *const c_char,
        pub rule_fsts: *const c_char,
    }

    #[repr(C)]
    pub struct TransducerModelConfig {
        pub encoder: *const c_char,
        pub decoder: *const c_char,
        pub joiner: *const c_char,
    }

    #[repr(C)]
    pub struct OneModelConfig {
        pub model: *const c_char,
    }

    #[repr(C)]
    pub struct WhisperModelConfig {
        pub encoder: *const c_char,
        pub decoder: *const c_char,
        pub language: *const c_char,
        pub task: *const c_char,
        pub tail_paddings: i32,
        pub enable_token_timestamps: i32,
        pub enable_segment_timestamps: i32,
    }

    #[repr(C)]
    pub struct CanaryModelConfig {
        pub encoder: *const c_char,
        pub decoder: *const c_char,
        pub src_lang: *const c_char,
        pub tgt_lang: *const c_char,
        pub use_pnc: i32,
    }

    #[repr(C)]
    pub struct EncoderDecoderConfig {
        pub encoder: *const c_char,
        pub decoder: *const c_char,
    }

    #[repr(C)]
    pub struct MoonshineModelConfig {
        pub preprocessor: *const c_char,
        pub encoder: *const c_char,
        pub uncached_decoder: *const c_char,
        pub cached_decoder: *const c_char,
        pub merged_decoder: *const c_char,
    }

    #[repr(C)]
    pub struct LmConfig {
        pub model: *const c_char,
        pub scale: c_float,
    }

    #[repr(C)]
    pub struct SenseVoiceModelConfig {
        pub model: *const c_char,
        pub language: *const c_char,
        pub use_itn: i32,
    }

    #[repr(C)]
    pub struct FunAsrNanoModelConfig {
        pub encoder_adaptor: *const c_char,
        pub llm: *const c_char,
        pub embedding: *const c_char,
        pub tokenizer: *const c_char,
        pub system_prompt: *const c_char,
        pub user_prompt: *const c_char,
        pub max_new_tokens: i32,
        pub temperature: c_float,
        pub top_p: c_float,
        pub seed: i32,
        pub language: *const c_char,
        pub itn: i32,
        pub hotwords: *const c_char,
    }

    #[repr(C)]
    pub struct Qwen3AsrModelConfig {
        pub conv_frontend: *const c_char,
        pub encoder: *const c_char,
        pub decoder: *const c_char,
        pub tokenizer: *const c_char,
        pub max_total_len: i32,
        pub max_new_tokens: i32,
        pub temperature: c_float,
        pub top_p: c_float,
        pub seed: i32,
        pub hotwords: *const c_char,
    }

    #[repr(C)]
    pub struct CohereTranscribeModelConfig {
        pub encoder: *const c_char,
        pub decoder: *const c_char,
        pub language: *const c_char,
        pub use_punct: i32,
        pub use_itn: i32,
    }

    #[repr(C)]
    pub struct ModelConfig {
        pub transducer: TransducerModelConfig,
        pub paraformer: OneModelConfig,
        pub nemo_ctc: OneModelConfig,
        pub whisper: WhisperModelConfig,
        pub tdnn: OneModelConfig,
        pub tokens: *const c_char,
        pub num_threads: i32,
        pub debug: i32,
        pub provider: *const c_char,
        pub model_type: *const c_char,
        pub modeling_unit: *const c_char,
        pub bpe_vocab: *const c_char,
        pub telespeech_ctc: *const c_char,
        pub sense_voice: SenseVoiceModelConfig,
        pub moonshine: MoonshineModelConfig,
        pub fire_red_asr: EncoderDecoderConfig,
        pub dolphin: OneModelConfig,
        pub zipformer_ctc: OneModelConfig,
        pub canary: CanaryModelConfig,
        pub wenet_ctc: OneModelConfig,
        pub omnilingual: OneModelConfig,
        pub medasr: OneModelConfig,
        pub funasr_nano: FunAsrNanoModelConfig,
        pub fire_red_asr_ctc: OneModelConfig,
        pub qwen3_asr: Qwen3AsrModelConfig,
        pub cohere_transcribe: CohereTranscribeModelConfig,
    }

    #[repr(C)]
    pub struct RecognizerConfig {
        pub feat_config: FeatureConfig,
        pub model_config: ModelConfig,
        pub lm_config: LmConfig,
        pub decoding_method: *const c_char,
        pub max_active_paths: i32,
        pub hotwords_file: *const c_char,
        pub hotwords_score: c_float,
        pub rule_fsts: *const c_char,
        pub rule_fars: *const c_char,
        pub blank_penalty: c_float,
        pub hr: HomophoneReplacerConfig,
    }

    /// `SherpaOnnxOfflineRecognizer`: opaque.
    #[repr(C)]
    pub struct Recognizer {
        _private: [u8; 0],
    }

    /// `SherpaOnnxOfflineStream`: opaque.
    #[repr(C)]
    pub struct Stream {
        _private: [u8; 0],
    }

    // Linked by build.rs (`sherpa-onnx-c-api`), after it has checked the library.
    unsafe extern "C" {
        pub fn SherpaOnnxGetVersionStr() -> *const c_char;
        pub fn SherpaOnnxCreateOfflineRecognizer(
            config: *const RecognizerConfig,
        ) -> *const Recognizer;
        pub fn SherpaOnnxDestroyOfflineRecognizer(recognizer: *const Recognizer);
        pub fn SherpaOnnxCreateOfflineStream(recognizer: *const Recognizer) -> *const Stream;
        pub fn SherpaOnnxDestroyOfflineStream(stream: *const Stream);
        pub fn SherpaOnnxAcceptWaveformOffline(
            stream: *const Stream,
            sample_rate: i32,
            samples: *const f32,
            n: i32,
        );
        pub fn SherpaOnnxDecodeOfflineStream(recognizer: *const Recognizer, stream: *const Stream);
        pub fn SherpaOnnxGetOfflineStreamResultAsJson(stream: *const Stream) -> *const c_char;
        pub fn SherpaOnnxDestroyOfflineStreamResultJson(json: *const c_char);
    }
}

/// The recognizer handle. Used only under [`SherpaParakeet`]'s mutex.
struct Recognizer(NonNull<ffi::Recognizer>);

// SAFETY: the handle has no thread affinity (sherpa-onnx's C API ties none of its objects to a
// thread), and it is only used under a mutex, so never from two threads at once.
unsafe impl Send for Recognizer {}

impl Drop for Recognizer {
    fn drop(&mut self) {
        // SAFETY: the handle came from SherpaOnnxCreateOfflineRecognizer and is destroyed once,
        // here; every stream made from it was destroyed inside the call that made it.
        unsafe { ffi::SherpaOnnxDestroyOfflineRecognizer(self.0.as_ptr()) }
    }
}

/// Parakeet TDT v3 int8, loaded. Implements [`OfflineEngine`] (see the module docs).
pub struct SherpaParakeet {
    info: EngineInfo,
    recognizer: Mutex<Recognizer>,
}

/// The model files, by the names the sherpa-onnx conversion gives them.
const FILES: [&str; 4] = [
    "encoder.int8.onnx",
    "decoder.int8.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];

impl SherpaParakeet {
    /// **Worker.** Loads the model in `dir` (the four files of the int8 conversion) on the CPU,
    /// with one thread per physical core. Takes seconds. A missing file is
    /// [`EngineError::ModelMissing`]; a library that is not sherpa-onnx [`SHERPA_ONNX_VERSION`]
    /// is refused.
    pub fn load(dir: &Path, info: EngineInfo) -> Result<Self, EngineError> {
        let failed = |what: String| EngineError::Failed(format!("{}: {what}", info.id));
        // SAFETY: the function takes no arguments and returns a static C string.
        let version = unsafe { CStr::from_ptr(ffi::SherpaOnnxGetVersionStr()) };
        if version.to_bytes() != SHERPA_ONNX_VERSION.as_bytes() {
            return Err(failed(format!(
                "the sherpa-onnx library is version {}, not {SHERPA_ONNX_VERSION}",
                version.to_string_lossy()
            )));
        }
        let mut paths = Vec::with_capacity(FILES.len());
        for name in FILES {
            let path = dir.join(name);
            if !path.is_file() {
                return Err(EngineError::ModelMissing(format!(
                    "{}: {name} is not there",
                    info.id
                )));
            }
            // The file name only in errors: the directory can hold the user's name.
            let text = path
                .to_str()
                .ok_or_else(|| failed(format!("{name} has a path that is not UTF-8")))?;
            paths.push(
                CString::new(text).map_err(|_| failed(format!("{name}'s path holds a NUL")))?,
            );
        }
        let threads = i32::try_from(physical_cores().get()).unwrap_or(i32::MAX);
        let provider = c"cpu";
        let model_type = c"nemo_transducer";
        let greedy = c"greedy_search";
        let empty: *const c_char = c"".as_ptr();
        let config = recognizer_config(
            [paths[0].as_ptr(), paths[1].as_ptr(), paths[2].as_ptr()],
            paths[3].as_ptr(),
            threads,
            [provider.as_ptr(), model_type.as_ptr(), greedy.as_ptr()],
            empty,
        );
        // SAFETY: `config` and every string it points to outlive the call, and it is the complete
        // struct `c-api.h` declares for 1.13.4 (the version checked above).
        let handle = unsafe { ffi::SherpaOnnxCreateOfflineRecognizer(&config) };
        let handle = NonNull::new(handle.cast_mut()).ok_or_else(|| {
            failed("sherpa-onnx could not load the model (see its log for the reason)".into())
        })?;
        Ok(Self {
            info,
            recognizer: Mutex::new(Recognizer(handle)),
        })
    }

    fn failed(&self, what: String) -> EngineError {
        EngineError::Failed(format!("{}: {what}", self.info.id))
    }

    /// One decode of `audio`, which is finite and within the length limit.
    fn decode(&self, audio: &[f32]) -> Result<String, EngineError> {
        let n = i32::try_from(audio.len()).map_err(|_| self.failed("too much audio".into()))?;
        let rate = i32::try_from(CANONICAL_RATE).unwrap_or(i32::MAX);
        let recognizer = lock(&self.recognizer);
        // SAFETY: the recognizer is valid for as long as `self` lives, and the mutex keeps this the
        // only use of it.
        let stream = unsafe { ffi::SherpaOnnxCreateOfflineStream(recognizer.0.as_ptr()) };
        if stream.is_null() {
            return Err(self.failed("sherpa-onnx could not open a stream".into()));
        }
        // SAFETY: `stream` was just created from this recognizer; `audio` is `n` valid samples
        // that outlive the calls (the library copies them).
        let json = unsafe {
            ffi::SherpaOnnxAcceptWaveformOffline(stream, rate, audio.as_ptr(), n);
            ffi::SherpaOnnxDecodeOfflineStream(recognizer.0.as_ptr(), stream);
            ffi::SherpaOnnxGetOfflineStreamResultAsJson(stream)
        };
        let result = if json.is_null() {
            Err(self.failed("sherpa-onnx returned no result".into()))
        } else {
            // SAFETY: a non-null result is a NUL-terminated string the library owns until it is
            // destroyed below.
            let bytes = unsafe { CStr::from_ptr(json) }.to_bytes().to_vec();
            // SAFETY: `json` came from SherpaOnnxGetOfflineStreamResultAsJson and is freed once.
            unsafe { ffi::SherpaOnnxDestroyOfflineStreamResultJson(json) };
            text_of(&bytes).map_err(|e| self.failed(e))
        };
        // SAFETY: `stream` came from SherpaOnnxCreateOfflineStream and is destroyed once, after its
        // last use.
        unsafe { ffi::SherpaOnnxDestroyOfflineStream(stream) };
        result
    }
}

/// The recognizer's configuration: the transducer's `[encoder, decoder, joiner]`, the tokens,
/// the thread count, `[provider, model type, decoding method]`, and `empty` for every string the
/// library should treat as unset. Everything else is zero, the library's defaults.
fn recognizer_config(
    transducer: [*const c_char; 3],
    tokens: *const c_char,
    threads: i32,
    [provider, model_type, decoding_method]: [*const c_char; 3],
    empty: *const c_char,
) -> ffi::RecognizerConfig {
    let one = || ffi::OneModelConfig { model: empty };
    ffi::RecognizerConfig {
        feat_config: ffi::FeatureConfig {
            sample_rate: i32::try_from(CANONICAL_RATE).unwrap_or(i32::MAX),
            // The model's own feature size is read from its metadata; 80 is the library's default.
            feature_dim: 80,
        },
        model_config: ffi::ModelConfig {
            transducer: ffi::TransducerModelConfig {
                encoder: transducer[0],
                decoder: transducer[1],
                joiner: transducer[2],
            },
            paraformer: one(),
            nemo_ctc: one(),
            whisper: ffi::WhisperModelConfig {
                encoder: empty,
                decoder: empty,
                language: empty,
                task: empty,
                tail_paddings: 0,
                enable_token_timestamps: 0,
                enable_segment_timestamps: 0,
            },
            tdnn: one(),
            tokens,
            num_threads: threads,
            debug: 0,
            provider,
            model_type,
            modeling_unit: empty,
            bpe_vocab: empty,
            telespeech_ctc: empty,
            sense_voice: ffi::SenseVoiceModelConfig {
                model: empty,
                language: empty,
                use_itn: 0,
            },
            moonshine: ffi::MoonshineModelConfig {
                preprocessor: empty,
                encoder: empty,
                uncached_decoder: empty,
                cached_decoder: empty,
                merged_decoder: empty,
            },
            fire_red_asr: ffi::EncoderDecoderConfig {
                encoder: empty,
                decoder: empty,
            },
            dolphin: one(),
            zipformer_ctc: one(),
            canary: ffi::CanaryModelConfig {
                encoder: empty,
                decoder: empty,
                src_lang: empty,
                tgt_lang: empty,
                use_pnc: 0,
            },
            wenet_ctc: one(),
            omnilingual: one(),
            medasr: one(),
            funasr_nano: ffi::FunAsrNanoModelConfig {
                encoder_adaptor: empty,
                llm: empty,
                embedding: empty,
                tokenizer: empty,
                system_prompt: empty,
                user_prompt: empty,
                max_new_tokens: 0,
                temperature: 0.0,
                top_p: 0.0,
                seed: 0,
                language: empty,
                itn: 0,
                hotwords: empty,
            },
            fire_red_asr_ctc: one(),
            qwen3_asr: ffi::Qwen3AsrModelConfig {
                conv_frontend: empty,
                encoder: empty,
                decoder: empty,
                tokenizer: empty,
                max_total_len: 0,
                max_new_tokens: 0,
                temperature: 0.0,
                top_p: 0.0,
                seed: 0,
                hotwords: empty,
            },
            cohere_transcribe: ffi::CohereTranscribeModelConfig {
                encoder: empty,
                decoder: empty,
                language: empty,
                use_punct: 0,
                use_itn: 0,
            },
        },
        lm_config: ffi::LmConfig {
            model: empty,
            scale: 0.0,
        },
        decoding_method,
        max_active_paths: 4,
        hotwords_file: empty,
        hotwords_score: 0.0,
        rule_fsts: empty,
        rule_fars: empty,
        blank_penalty: 0.0,
        hr: ffi::HomophoneReplacerConfig {
            dict_dir: empty,
            lexicon: empty,
            rule_fsts: empty,
        },
    }
}

/// The `text` field of a result's JSON. The error names the problem, never the text.
fn text_of(json: &[u8]) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| format!("unreadable result JSON ({e})"))?;
    match value.get("text") {
        Some(serde_json::Value::String(text)) => Ok(text.trim().to_owned()),
        _ => Err("the result JSON has no text".into()),
    }
}

impl OfflineEngine for SherpaParakeet {
    fn info(&self) -> EngineInfo {
        self.info.clone()
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        if options.cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        if let Some(i) = audio.iter().position(|s| !s.is_finite()) {
            return Err(self.failed(format!("audio sample {i} is not a finite number")));
        }
        if audio.is_empty() {
            return Ok(Transcript::default());
        }
        let limit = MAX_SECONDS as usize * CANONICAL_RATE as usize;
        if audio.len() > limit {
            return Err(self.failed(format!(
                "{} s of audio is more than one pass takes ({MAX_SECONDS} s)",
                audio.len() / CANONICAL_RATE as usize
            )));
        }
        let text = self.decode(audio)?;
        let end_ms = audio.len() as u64 * 1000 / u64::from(CANONICAL_RATE);
        Ok(Transcript {
            segments: vec![TimedText {
                start_ms: 0,
                end_ms,
                text,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::text_of;

    #[test]
    fn the_text_is_read_from_the_result_json() {
        let json = br#"{"lang": "", "text": " hello there ", "timestamps": [0.1, 0.4]}"#;
        assert_eq!(text_of(json).unwrap(), "hello there");
    }

    #[test]
    fn a_result_without_text_is_an_error_that_quotes_nothing() {
        let err = text_of(br#"{"tokens": ["secret"]}"#).unwrap_err();
        assert!(!err.contains("secret"), "{err}");
        let err = text_of(b"not json at all: secret").unwrap_err();
        assert!(!err.contains("secret"), "{err}");
    }
}
