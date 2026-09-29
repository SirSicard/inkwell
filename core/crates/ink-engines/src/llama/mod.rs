//! llama.cpp adapters (the `engine-llama` feature).
//!
//! | Type | Implements | For |
//! |---|---|---|
//! | [`QwenAsr`] | [`OfflineEngine`] | Qwen3-ASR (a GGUF text model plus an `mmproj` audio projector) through llama.cpp's multimodal library, `mtmd`: the dictation final and the meeting final (architecture rule 10). |
//! | [`QwenAsrLoader`] | [`Loader`] | Loads a registry row's files for [`Residency`], which is the only owner the app should give the model: residency then decides when it is unloaded (after idling, or before an update replaces its files). |
//! | [`LlamaLlm`] | [`Llm`] | A local GGUF chat model for polish and summaries, running in this process ([`Endpoint::InProcess`]). |
//!
//! llama.cpp and its ggml are linked statically, so its ggml stays inside the core and cannot
//! collide with the diarizer's own ggml libraries (docs/ARCHITECTURE.md, "ggml";
//! `tests/ggml_link.rs` checks it).
//!
//! **Logs.** llama.cpp, ggml and mtmd log through llama-cpp-2 into `tracing` (target
//! `llama-cpp-2`). Only their warnings and errors may be kept: **every tracing subscriber in the
//! app must apply [`log_allowed`]**, which drops the engines' DEBUG and INFO and every event of
//! llama-cpp-2's own. Those can quote the text being processed (a lazy grammar's debug lines quote
//! generated tokens, and the binding re-emits buffered lines as warnings), and nothing a model
//! hears or says may reach a log (I5). The warnings
//! and errors quote model files, vocabulary tokens and error codes, not user text;
//! `tests/llama_logs.rs` checks that on real runs. With no subscriber, everything is dropped at the
//! source. The filter lives with the app's subscriber because llama-cpp-2 asks that subscriber
//! whether a level is wanted; capping it inside the core would need a log callback of our own, in
//! `unsafe` code. Failures also come back as the returned errors, which name the step and the file,
//! never the text.
//!
//! **Before exit, drop every model.** ggml's Metal backend aborts the process at exit if a model is
//! still loaded (its device teardown asserts that every buffer was freed). So the app's shutdown
//! drops its [`Residency`], and with it every model, before the process exits, and nothing keeps a
//! model in a `static`. `tests/llama_asr.rs` pins this.
//!
//! **Where it computes** ([`compute`]): on a GPU when ggml reports one (Metal on the Mac, Vulkan on
//! Windows with `engine-llama-vulkan`), every layer offloaded; else on the CPU, nothing offloaded and
//! one thread per physical core, set explicitly (llama.cpp's own default is 4). Chosen once per
//! process from ggml's devices by [`crate::choose`], so a Vulkan build on a machine without a
//! Vulkan device runs on the CPU.
//!
//! **Threads.** Everything here is a **worker**-thread call, `Send + Sync`, and may block for as
//! long as the work takes.
//!
//! [`OfflineEngine`]: ink_core::OfflineEngine
//! [`Llm`]: ink_core::Llm
//! [`Endpoint::InProcess`]: ink_core::Endpoint::InProcess
//! [`Loader`]: crate::Loader
//! [`Residency`]: crate::Residency

mod asr;
mod llm;

pub use asr::{MAX_NEW_TOKENS, MAX_WINDOW_SECONDS, QwenAsr, QwenAsrLoader};
pub use llm::{JSON_OBJECT_GRAMMAR, LlamaLlm};

/// Whether a `tracing` event or span may be kept.
///
/// - Target `llama-cpp-2`, where every llama.cpp, ggml and mtmd line arrives (the module is a
///   field): kept at WARN and ERROR only.
/// - llama-cpp-2's log bridge and crate root (targets `llama_cpp_2::log` and `llama_cpp_2`): dropped
///   at every level. The bridge re-emits a buffered line's raw text at WARN, whatever that line's
///   level was.
/// - llama-cpp-2's other modules (`llama_cpp_2::*`): kept at WARN and ERROR only. Below that they
///   log paths. In 0.1.157 (pinned) the only WARN or ERROR outside the bridge is `model.rs`'s
///   "Unexpected rope type", which carries a number: a wrong-model signal worth keeping. Re-read
///   the binding's WARN and ERROR call sites whenever the pin moves.
/// - Anything else: not this filter's business, kept.
///
/// Every subscriber in the app applies it (see the module docs). S1.7 makes that structural: one
/// core subscriber applying this and ink-llm's `log_record_allowed`, with a planted-secret test.
pub fn log_allowed(metadata: &tracing::Metadata<'_>) -> bool {
    let target = metadata.target();
    let warn_or_error = matches!(
        *metadata.level(),
        tracing::Level::WARN | tracing::Level::ERROR
    );
    if target == "llama_cpp_2"
        || target == "llama_cpp_2::log"
        || target.starts_with("llama_cpp_2::log::")
    {
        return false;
    }
    if target == "llama-cpp-2" || target.starts_with("llama_cpp_2::") {
        return warn_or_error;
    }
    true
}

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::OnceLock;

use ink_core::{CancelToken, EngineError};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::{
    LlamaBackendDeviceType, LogOptions, TokenToStringError, list_llama_ggml_backend_devices,
    send_logs_to_tracing,
};

use crate::compute::{Compute, Device, DeviceKind, choose, physical_cores};

/// The process's llama.cpp backend. llama.cpp is initialised once per process (the binding refuses
/// a second `init`), so every model shares this one, and it lives until the process exits. A model
/// unloaded by residency and loaded again reuses it.
fn backend() -> Result<&'static LlamaBackend, EngineError> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| {
            // Before `init`, so the start-up lines take the same route. Which of them are kept is
            // the subscriber's call, through `log_allowed` (see the module docs).
            send_logs_to_tracing(LogOptions::default());
            LlamaBackend::init().map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| EngineError::Failed(format!("the llama.cpp backend did not start: {e}")))
}

/// **Worker.** Where this process's llama.cpp models compute: the GPU ggml reports, or the CPU on
/// every physical core (see the module docs). Starts the backend if it has not started, and is
/// decided once: ggml registers its devices when the backend starts, and they do not change.
pub fn compute() -> Result<&'static Compute, EngineError> {
    static COMPUTE: OnceLock<Compute> = OnceLock::new();
    // The devices are only listed once the backend (and with it ggml's device registry) is up.
    backend()?;
    Ok(COMPUTE.get_or_init(|| {
        let devices: Vec<Device> = list_llama_ggml_backend_devices()
            .into_iter()
            .map(|d| Device {
                backend: d.backend,
                description: d.description,
                kind: match d.device_type {
                    LlamaBackendDeviceType::Cpu => DeviceKind::Cpu,
                    LlamaBackendDeviceType::Gpu => DeviceKind::Gpu,
                    LlamaBackendDeviceType::IntegratedGpu => DeviceKind::IntegratedGpu,
                    LlamaBackendDeviceType::Accelerator => DeviceKind::Accelerator,
                    LlamaBackendDeviceType::Unknown => DeviceKind::Unknown,
                },
            })
            .collect();
        choose(&devices, physical_cores())
    }))
}

/// Model parameters for `compute`: llama.cpp's defaults on a GPU (every layer offloaded,
/// memory-mapped: what was measured), and no layer offloaded on the CPU.
fn model_params(compute: &Compute) -> LlamaModelParams {
    let params = LlamaModelParams::default();
    if compute.is_gpu() {
        params
    } else {
        params.with_n_gpu_layers(0)
    }
}

/// A context's parameters for `compute`, holding `n_ctx` tokens: on the CPU, generation and prompt
/// reading both get one thread per physical core; on a GPU, llama.cpp's defaults.
fn context_params(compute: &Compute, n_ctx: u32) -> LlamaContextParams {
    let params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(n_ctx));
    match compute.cpu_threads() {
        None => params,
        Some(threads) => {
            let threads = i32::try_from(threads.get()).unwrap_or(i32::MAX);
            params.with_n_threads(threads).with_n_threads_batch(threads)
        }
    }
}

/// A path's file name, for errors: the directory can hold the user's name, and errors end up in
/// logs.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "<no file name>".into(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The model file must exist before llama.cpp is asked to open it: the binding only
/// debug-asserts that, and llama.cpp's own answer to a missing file is a bare null.
fn require_file(path: &Path, what: &str) -> Result<(), EngineError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(EngineError::ModelMissing(format!(
            "{what}: {} is not there",
            file_name(path)
        )))
    }
}

/// Why generation stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    /// The model ended its turn.
    EndOfText,
    /// The token budget ran out first.
    Budget,
}

/// Why generation failed. Carries codes only, never text.
#[derive(Debug)]
enum GenerateError {
    Cancelled,
    Failed(String),
}

/// Samples and decodes one token at a time from `n_past`, until the model ends its turn or
/// `budget` tokens have been produced. Checks `cancel` before every token.
///
/// The text is assembled from raw bytes and decoded once at the end: a token can end in the middle
/// of a UTF-8 character. Bytes that still do not form UTF-8 (only possible if the budget cuts a
/// character) become U+FFFD rather than failing the whole answer.
fn generate(
    model: &LlamaModel,
    ctx: &mut LlamaContext<'_>,
    sampler: &mut LlamaSampler,
    mut n_past: i32,
    budget: u32,
    cancel: &CancelToken,
) -> Result<(String, Stop), GenerateError> {
    let mut batch = LlamaBatch::new(1, 1);
    let mut bytes = Vec::new();
    let mut stop = Stop::Budget;
    for _ in 0..budget {
        if cancel.is_cancelled() {
            return Err(GenerateError::Cancelled);
        }
        // Samples from the last decoded position's logits and advances the sampler's state.
        let token = sampler.sample(ctx, -1);
        if model.is_eog_token(token) {
            stop = Stop::EndOfText;
            break;
        }
        bytes.extend(piece(model, token).map_err(GenerateError::Failed)?);
        batch.clear();
        batch
            .add(token, n_past, &[0], true)
            .map_err(|e| GenerateError::Failed(format!("batch: {e}")))?;
        n_past += 1;
        ctx.decode(&mut batch)
            .map_err(|e| GenerateError::Failed(format!("decode: {e}")))?;
    }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), stop))
}

/// The bytes one token renders to, with special tokens rendered as nothing (what llama.cpp's own
/// tools print). The binding reports a token that renders to nothing as `UnknownTokenType`; that
/// is a control token here, not an error.
fn piece(model: &LlamaModel, token: LlamaToken) -> Result<Vec<u8>, String> {
    const FIRST_TRY: usize = 32;
    match model.token_to_piece_bytes(token, FIRST_TRY, false, None) {
        Ok(bytes) => Ok(bytes),
        Err(TokenToStringError::UnknownTokenType) => Ok(Vec::new()),
        Err(TokenToStringError::InsufficientBufferSpace(needed)) => {
            let size = usize::try_from(needed.unsigned_abs())
                .map_err(|_| format!("token {} needs an impossible buffer", token.0))?;
            model
                .token_to_piece_bytes(token, size, false, None)
                .map_err(|e| format!("token {}: {e}", token.0))
        }
        Err(e) => Err(format!("token {}: {e}", token.0)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Level, Metadata, Subscriber};

    use super::log_allowed;

    /// A subscriber that applies `log_allowed`, as the app's must, and records what gets through.
    #[derive(Clone, Default)]
    struct Filtered(Arc<Mutex<Vec<(Level, String)>>>);

    impl Subscriber for Filtered {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            log_allowed(metadata)
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, event: &Event<'_>) {
            let m = event.metadata();
            self.0
                .lock()
                .unwrap()
                .push((*m.level(), m.target().to_owned()));
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    /// Runs `emit` under the filtering subscriber and returns what got through.
    fn kept(emit: impl FnOnce()) -> Vec<(Level, String)> {
        let seen = Filtered::default();
        tracing::subscriber::with_default(seen.clone(), emit);
        seen.0.lock().unwrap().clone()
    }

    #[test]
    fn engine_lines_are_kept_at_warn_and_error_only() {
        // llama-cpp-2 sends every llama.cpp, ggml and mtmd line to target `llama-cpp-2`, with the
        // module in a field.
        let got = kept(|| {
            for module in [
                "llama.cpp::llama_model_loader",
                "ggml::ggml_metal_init",
                "mtmd::clip",
            ] {
                tracing::trace!(target: "llama-cpp-2", module, "trace line");
                tracing::debug!(target: "llama-cpp-2", module, "debug line");
                tracing::info!(target: "llama-cpp-2", module, "info line");
                tracing::warn!(target: "llama-cpp-2", module, "warn line");
                tracing::error!(target: "llama-cpp-2", module, "error line");
            }
        });
        let engine = |level| (level, "llama-cpp-2".to_owned());
        assert_eq!(
            got,
            [Level::WARN, Level::ERROR]
                .repeat(3)
                .into_iter()
                .map(engine)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_bindings_log_bridge_is_dropped_at_every_level() {
        // `llama_cpp_2::log` re-emits a buffered line's raw text at WARN whatever its level was
        // (llama-cpp-2 0.1.157, src/log.rs).
        let got = kept(|| {
            for level in ["trace", "debug", "info", "warn", "error"] {
                tracing::trace!(target: "llama_cpp_2::log", level, "text");
                tracing::debug!(target: "llama_cpp_2::log", level, "text");
                tracing::info!(target: "llama_cpp_2::log", level, "text");
                tracing::warn!(target: "llama_cpp_2::log", level, text = "buffered", "re-emit");
                tracing::error!(target: "llama_cpp_2::log", level, "text");
            }
            tracing::warn!(target: "llama_cpp_2", "crate root");
        });
        assert!(got.is_empty(), "kept {got:?}");
    }

    #[test]
    fn the_bindings_other_modules_are_kept_at_warn_and_error_only() {
        // Below WARN they log paths; model.rs's rope-type error (a number) is a wrong-model signal.
        let got = kept(|| {
            tracing::debug!(target: "llama_cpp_2::model", "Loaded model");
            tracing::info!(target: "llama_cpp_2::context", "context");
            tracing::warn!(target: "llama_cpp_2::context", "warn");
            tracing::error!(target: "llama_cpp_2::model", rope_type = 7, "Unexpected rope type");
        });
        assert_eq!(
            got,
            vec![
                (Level::WARN, "llama_cpp_2::context".to_owned()),
                (Level::ERROR, "llama_cpp_2::model".to_owned()),
            ]
        );
    }

    #[test]
    fn other_targets_are_not_this_filters_business() {
        let got = kept(|| {
            tracing::debug!(target: "ink_pipeline", "other");
            tracing::info!(target: "llama_cpp_2x", "a different crate");
        });
        assert_eq!(
            got,
            vec![
                (Level::DEBUG, "ink_pipeline".to_owned()),
                (Level::INFO, "llama_cpp_2x".to_owned()),
            ]
        );
    }
}
