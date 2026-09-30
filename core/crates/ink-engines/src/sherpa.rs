//! Parakeet TDT v3 (int8) through sherpa-onnx's C API on the CPU (`engine-sherpa`, Windows): the
//! non-Apple Parakeet, as an [`OfflineEngine`]. On the Mac, Parakeet runs in FluidAudio instead.
//!
//! - **One pass** of greedy search for up to [`MAX_SECONDS`] of audio. Longer audio (a long
//!   dictation take or voice edit: push-to-talk holds run to 180 s and toggle takes are uncapped) is
//!   cut into windows of at most that, each at the quietest 20 ms in its last [`CUT_SEARCH_SECONDS`]
//!   (`ink_audio::window`), decoded one after another; cancellation is checked between them. Cutting
//!   shorter would cost accuracy: AMI IHM's 70-88 s clips score 27.93 % WER whole
//!   (`tests/sherpa.rs`), and 27.22 in 45 s windows but 29.6-31.6 in 10-30 s ones (measured once,
//!   cut the same way).
//! - **One segment per word**, placed by the model's token times: a word runs from its first
//!   token's start to its last token's end. The live-partials scheme (`crate::live`) reads them;
//!   a dictation reads only the text.
//! - **The CPU**, with one thread per physical core. ONNX Runtime's other execution providers are
//!   not in this build of sherpa-onnx.
//! - **Cancellation** is checked before each window's decode, which cannot be interrupted once
//!   started.
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
//! # onnxruntime.dll
//!
//! Windows 11 has its own, older `onnxruntime.dll` in System32, and Windows looks there before
//! `PATH` (after the executable's own directory). sherpa-onnx given that one crashes the process
//! when it creates the recognizer. So the app ships the archive's DLLs beside its executable
//! (`build.rs` puts them beside the tests too), and [`SherpaParakeet::load`] asks the ONNX Runtime
//! the process has loaded for its version first, and refuses any but [`ONNXRUNTIME_VERSION`].
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

use ink_audio::window::{WindowConfig, plan_windows};
use ink_core::{
    CANONICAL_RATE, EngineError, EngineInfo, OfflineEngine, TimedText, TranscribeOptions,
    Transcript,
};

use crate::compute::physical_cores;
use crate::lock;

/// The sherpa-onnx release the declarations below are written against.
pub const SHERPA_ONNX_VERSION: &str = "1.13.4";

/// The ONNX Runtime that sherpa-onnx 1.13.4's Windows archive carries (the DLL's own version).
pub const ONNXRUNTIME_VERSION: &str = "1.27.0";

/// The longest audio decoded in one pass, in seconds. Longer audio is cut into windows.
pub const MAX_SECONDS: u32 = 90;

/// How far back from the end of a full window the cut may land, in seconds: a long take is cut in
/// its quietest 20 ms between 60 and 90 s.
pub const CUT_SEARCH_SECONDS: u32 = 30;

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

    /// ONNX Runtime's `OrtApiBase` (`onnxruntime_c_api.h`, MIT): the two entry points every
    /// version keeps. Only the version string is read.
    #[repr(C)]
    pub struct OrtApiBase {
        pub get_api: unsafe extern "C" fn(version: u32) -> *const std::ffi::c_void,
        pub get_version_string: unsafe extern "C" fn() -> *const c_char,
    }

    // Linked by build.rs (`onnxruntime`), after it has checked the library: the same
    // `onnxruntime.dll` sherpa-onnx's library uses, since Windows loads one module by that name.
    unsafe extern "C" {
        pub fn OrtGetApiBase() -> *const OrtApiBase;
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
        let ort = onnxruntime_version();
        if ort.as_deref() != Some(ONNXRUNTIME_VERSION) {
            return Err(failed(format!(
                "the process loaded ONNX Runtime {}, not the {ONNXRUNTIME_VERSION} sherpa-onnx \
                 was built with (Windows' own onnxruntime.dll, found before the app's?): the \
                 archive's DLLs must sit beside the executable",
                ort.as_deref().unwrap_or("of no known version")
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

    /// One decode of `audio`, which is finite and at most [`MAX_SECONDS`] long: its words.
    fn decode(&self, audio: &[f32]) -> Result<Vec<TimedText>, EngineError> {
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
            let audio_ms = audio.len() as u64 * 1000 / u64::from(CANONICAL_RATE);
            words_of(&bytes, audio_ms).map_err(|e| self.failed(e))
        };
        // SAFETY: `stream` came from SherpaOnnxCreateOfflineStream and is destroyed once, after its
        // last use.
        unsafe { ffi::SherpaOnnxDestroyOfflineStream(stream) };
        result
    }
}

/// The version of the ONNX Runtime this process loaded, if it reports one.
fn onnxruntime_version() -> Option<String> {
    // SAFETY: OrtGetApiBase takes no arguments and returns null or a pointer to a static struct
    // whose first two members every ONNX Runtime version keeps.
    let base = unsafe { ffi::OrtGetApiBase() };
    if base.is_null() {
        return None;
    }
    // SAFETY: `base` is non-null and points to that static struct; its version function takes no
    // arguments and returns null or a static NUL-terminated string.
    let version = unsafe { ((*base).get_version_string)() };
    if version.is_null() {
        return None;
    }
    // SAFETY: non-null, static and NUL-terminated (above).
    Some(
        unsafe { CStr::from_ptr(version) }
            .to_string_lossy()
            .into_owned(),
    )
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

/// The words of a result's JSON, in ms, kept inside the `audio_ms` decoded: its `tokens` joined
/// into words (a token that starts with a space starts a word; punctuation joins the word before
/// it), each from its first token's `timestamps` entry to the latest end (`timestamps` plus
/// `durations`, taken as 0 where the model gives none) of its tokens. The error names the problem,
/// never the text.
fn words_of(json: &[u8], audio_ms: u64) -> Result<Vec<TimedText>, String> {
    use serde_json::Value;
    // Parsed as a `Value` and read by hand: a typed parse's errors can quote the text.
    let value: Value =
        serde_json::from_slice(json).map_err(|e| format!("unreadable result JSON ({e})"))?;
    let has_text = match value.get("text") {
        Some(Value::String(text)) => !text.trim().is_empty(),
        _ => return Err("the result JSON has no text".into()),
    };
    let array = |key: &str| match value.get(key) {
        None => Ok(&[][..]),
        Some(Value::Array(items)) => Ok(items.as_slice()),
        Some(_) => Err(format!("the result's {key} is not a list")),
    };
    let (tokens, starts, durations) = (array("tokens")?, array("timestamps")?, array("durations")?);
    if starts.len() != tokens.len() || !(durations.is_empty() || durations.len() == tokens.len()) {
        return Err(format!(
            "the result's {} tokens have {} start times and {} durations",
            tokens.len(),
            starts.len(),
            durations.len()
        ));
    }
    if has_text && tokens.is_empty() {
        return Err("the result has text but no tokens".into());
    }
    let seconds = |v: &Value, key: &str| {
        v.as_f64()
            .filter(|s| s.is_finite() && *s >= 0.0)
            .ok_or_else(|| format!("the result's {key} holds something that is not a time"))
    };
    let ms = |s: f64| ((s * 1000.0).round() as u64).min(audio_ms);
    let mut words: Vec<TimedText> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let token = token
            .as_str()
            .ok_or("the result's tokens hold something that is not text")?;
        let start = seconds(&starts[i], "timestamps")?;
        let duration = durations
            .get(i)
            .map_or(Ok(0.0), |d| seconds(d, "durations"))?;
        let (start_ms, end_ms) = (ms(start), ms(start + duration));
        match words.last_mut() {
            Some(word) if !token.starts_with(' ') => {
                word.text.push_str(token);
                word.end_ms = word.end_ms.max(end_ms);
            }
            _ => words.push(TimedText {
                start_ms,
                end_ms,
                text: token.trim_start().to_owned(),
            }),
        }
    }
    words.retain(|w| !w.text.trim().is_empty());
    Ok(words)
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
        Ok(Transcript {
            segments: in_windows(audio, |window| {
                if options.cancel.is_cancelled() {
                    return Err(EngineError::Cancelled);
                }
                self.decode(window)
            })?,
        })
    }
}

/// `audio` decoded a window at a time by `decode` (see the module docs), its words placed in ms
/// from the start of `audio`. No audio is no windows.
fn in_windows(
    audio: &[f32],
    mut decode: impl FnMut(&[f32]) -> Result<Vec<TimedText>, EngineError>,
) -> Result<Vec<TimedText>, EngineError> {
    const RATE: usize = CANONICAL_RATE as usize;
    let config = WindowConfig {
        max_len: MAX_SECONDS as usize * RATE,
        overlap: 0,
        search: CUT_SEARCH_SECONDS as usize * RATE,
    };
    let windows = plan_windows(audio, config)
        .map_err(|e| EngineError::Failed(format!("cutting long audio into windows: {e}")))?;
    let mut words = Vec::new();
    for window in windows {
        let (start, end) = (window.start as usize, window.end as usize);
        let offset_ms = window.start * 1000 / u64::from(CANONICAL_RATE);
        words.extend(decode(&audio[start..end])?.into_iter().map(|w| TimedText {
            start_ms: w.start_ms + offset_ms,
            end_ms: w.end_ms + offset_ms,
            text: w.text,
        }));
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::{MAX_SECONDS, in_windows, words_of};
    use ink_core::{EngineError, TimedText};

    fn word(text: &str, start_ms: u64, end_ms: u64) -> TimedText {
        TimedText {
            start_ms,
            end_ms,
            text: text.into(),
        }
    }

    #[test]
    fn tokens_join_into_words_placed_by_their_times() {
        // The shape sherpa-onnx 1.13.4 gives Parakeet TDT: a token opening a word starts with a
        // space; punctuation joins the word before it; a duration may be zero.
        let json = br#"{"lang": "", "text": "Yeah, we're here.",
            "timestamps": [1.60, 1.68, 1.76, 2.08, 2.16, 2.24, 2.40, 2.56],
            "durations": [0.08, 0.08, 0.08, 0.08, 0.00, 0.08, 0.16, 0.08],
            "tokens": [" Ye", "ah", ",", " we", "'", "re", " here", "."], "words": []}"#;
        let words = words_of(json, 10_000).unwrap();
        assert_eq!(
            words,
            [
                word("Yeah,", 1600, 1840),
                word("we're", 2080, 2320),
                word("here.", 2400, 2640),
            ]
        );
        let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(
            text.join(" "),
            "Yeah, we're here.",
            "the text, word for word"
        );
    }

    #[test]
    fn times_stay_inside_the_audio_and_nothing_said_is_no_words() {
        let json =
            br#"{"text": "late", "timestamps": [0.96], "durations": [0.16], "tokens": [" late"]}"#;
        assert_eq!(words_of(json, 1_000).unwrap(), [word("late", 960, 1000)]);
        let json = br#"{"text": "", "timestamps": [], "durations": [], "tokens": []}"#;
        assert!(words_of(json, 1_000).unwrap().is_empty());
    }

    /// A take longer than one pass is cut in its quietest stretch between 60 and 90 s, and each
    /// window's words are placed from the start of the take.
    #[test]
    fn a_long_take_is_decoded_in_windows_placed_on_its_timeline() {
        const RATE: usize = 16_000;
        // 100 s of loud audio with one silent 20 ms frame at 72 s: the cut lands there.
        let mut audio = vec![0.5f32; 100 * RATE];
        audio[72 * RATE..72 * RATE + 320].fill(0.0);
        let mut lengths = Vec::new();
        let words = in_windows(&audio, |window| {
            lengths.push(window.len());
            Ok(vec![word("w", 1_000, 1_500)])
        })
        .unwrap();
        assert_eq!(lengths, [72 * RATE + 160, 28 * RATE - 160]);
        assert_eq!(words, [word("w", 1_000, 1_500), word("w", 73_010, 73_510)]);
        // Up to the limit, one pass; no audio, none.
        let mut calls = 0;
        in_windows(&vec![0.1; MAX_SECONDS as usize * RATE], |_| {
            calls += 1;
            Ok(Vec::new())
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(in_windows(&[], |_| unreachable!()), Ok(Vec::new()));
        // A window's error ends the take.
        let mut calls = 0;
        let err = in_windows(&audio, |_| {
            calls += 1;
            Err(EngineError::Cancelled)
        });
        assert_eq!((err, calls), (Err(EngineError::Cancelled), 1));
    }

    #[test]
    fn a_malformed_result_is_an_error_that_quotes_nothing() {
        for json in [
            &br#"{"text": "secret", "timestamps": [0.1], "durations": [0.1]}"#[..],
            br#"{"text": "secret", "timestamps": [0.1, 0.2], "durations": [0.1], "tokens": [" secret"]}"#,
            br#"{"text": "secret", "timestamps": ["x"], "durations": [0.1], "tokens": [" secret"]}"#,
            b"not json at all: secret",
        ] {
            let err = words_of(json, 1_000).unwrap_err();
            assert!(!err.contains("secret"), "{err}");
        }
    }
}
