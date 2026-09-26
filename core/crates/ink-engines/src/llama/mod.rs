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
//! `llama-cpp-2`, and llama-cpp-2's own module targets). Only warnings and errors may be kept:
//! **every tracing subscriber in the app must apply [`log_allowed`]**, which drops the engines'
//! DEBUG and INFO. Those levels can quote the text being processed (a lazy grammar's debug lines
//! quote generated tokens), and nothing a model hears or says may reach a log (I5). The warnings
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

/// Whether a `tracing` event or span may be kept: llama.cpp's, ggml's, mtmd's and llama-cpp-2's
/// only at WARN or ERROR; everything else is not this filter's business and passes. Every
/// subscriber in the app applies it (see the module docs).
pub fn log_allowed(metadata: &tracing::Metadata<'_>) -> bool {
    let target = metadata.target();
    let engine = target == "llama-cpp-2" || target.starts_with("llama_cpp_2");
    !engine
        || matches!(
            *metadata.level(),
            tracing::Level::WARN | tracing::Level::ERROR
        )
}

use std::path::Path;
use std::sync::OnceLock;

use ink_core::{CancelToken, EngineError};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::{LogOptions, TokenToStringError, send_logs_to_tracing};

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

    #[test]
    fn engine_logs_below_warn_are_dropped_and_others_pass() {
        let seen = Filtered::default();
        tracing::subscriber::with_default(seen.clone(), || {
            // Where llama.cpp, ggml and mtmd lines arrive, and llama-cpp-2's own events.
            tracing::debug!(target: "llama-cpp-2", "debug line");
            tracing::info!(target: "llama-cpp-2", "info line");
            tracing::warn!(target: "llama-cpp-2", "warn line");
            tracing::error!(target: "llama-cpp-2", "error line");
            tracing::debug!(target: "llama_cpp_2::model", "loaded");
            tracing::info!(target: "llama_cpp_2::log", "no level");
            tracing::warn!(target: "llama_cpp_2::log", "buffered");
            // Anything else is none of this filter's business.
            tracing::debug!(target: "ink_pipeline", "other");
        });
        let seen = seen.0.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![
                (Level::WARN, "llama-cpp-2".to_owned()),
                (Level::ERROR, "llama-cpp-2".to_owned()),
                (Level::WARN, "llama_cpp_2::log".to_owned()),
                (Level::DEBUG, "ink_pipeline".to_owned()),
            ]
        );
    }
}
