//! The local chat model adapter with real weights. Every test here is `#[ignore]`: CI has no model.
//!
//! - `a_chat_model_answers_*` needs a GGUF chat model at `$INK_LLM_GGUF`. No chat weights are
//!   approved for download yet, so when it is not set the test says so and returns.
//! - The rest run llama.cpp's text path on the Qwen3-ASR model's own decoder (a Qwen3 language
//!   model with a ChatML template) from `$INK_BENCH_DIR`: the plumbing, not the quality.

#![cfg(feature = "engine-llama")]

mod bench;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ink_core::{CancelToken, EngineError, Llm, LlmError, LlmRequest};
use ink_engines::llama::LlamaLlm;

/// One model at a time per binary, and never one in a `static` (see `llama_asr.rs`).
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn request(user: &str, json: bool) -> LlmRequest {
    LlmRequest {
        system: if json {
            "Answer with one JSON object with a string field \"answer\".".into()
        } else {
            "Answer in one short sentence.".into()
        },
        user: user.into(),
        max_tokens: 64,
        temperature: 0.0,
        json_schema: json.then(|| r#"{"type":"object"}"#.to_owned()),
    }
}

fn asr_decoder() -> PathBuf {
    bench::bench_dir().join("models/qwen3-asr-1.7b-gguf/Qwen3-ASR-1.7B-Q8_0.gguf")
}

fn is_json_object(text: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(text.trim()),
        Ok(serde_json::Value::Object(_))
    )
}

#[test]
#[ignore = "needs a GGUF chat model at $INK_LLM_GGUF; skips without one"]
fn a_chat_model_answers_plain_and_json_requests() {
    let Some(path) = std::env::var_os("INK_LLM_GGUF").map(PathBuf::from) else {
        eprintln!("SKIPPED: set INK_LLM_GGUF to a GGUF chat model to run this test");
        return;
    };
    if !path.is_file() {
        eprintln!("SKIPPED: INK_LLM_GGUF does not name a file");
        return;
    }
    let _serial = serial();
    let llm = LlamaLlm::load(&path, "local-test").unwrap();
    let cancel = CancelToken::new();
    let plain = llm
        .complete(
            &request("What colour is the sky on a clear day?", false),
            &cancel,
        )
        .unwrap();
    assert!(!plain.text.trim().is_empty());
    let json = llm
        .complete(&request("What colour is grass?", true), &cancel)
        .unwrap();
    assert!(is_json_object(&json.text), "not one JSON object");
}

#[test]
#[ignore = "needs the Qwen3-ASR model under $INK_BENCH_DIR; run locally"]
fn the_text_path_runs_on_a_real_model() {
    let _serial = serial();
    let llm = LlamaLlm::load(&asr_decoder(), "qwen3-asr-decoder").unwrap();
    let info = llm.info();
    assert_eq!(info.provider, "local");
    assert!(info.endpoint.is_local(), "local-only mode must allow it");

    let cancel = CancelToken::new();
    // Greedy is deterministic: the same request gives the same answer.
    let a = llm
        .complete(&request("Say hello.", false), &cancel)
        .unwrap();
    let b = llm
        .complete(&request("Say hello.", false), &cancel)
        .unwrap();
    assert_eq!(a, b);
    // Sampled answers use a fixed seed, so they repeat too.
    let warm = LlmRequest {
        temperature: 0.7,
        ..request("Say hello.", false)
    };
    assert_eq!(
        llm.complete(&warm, &cancel).unwrap(),
        llm.complete(&warm, &cancel).unwrap()
    );
    // The JSON grammar parses in llama.cpp and holds the answer to one object. This decoder does
    // not answer in JSON by itself, so the grammar is what makes the difference.
    assert!(!is_json_object(&a.text));
    let json = llm.complete(&request("Say hello.", true), &cancel).unwrap();
    assert!(is_json_object(&json.text), "not one JSON object");
}

#[test]
#[ignore = "needs the Qwen3-ASR model under $INK_BENCH_DIR; run locally"]
fn bad_requests_fail_without_quoting_the_text() {
    let _serial = serial();
    let llm = LlamaLlm::load(&asr_decoder(), "qwen3-asr-decoder").unwrap();

    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert_eq!(
        llm.complete(&request("Say hello.", false), &cancelled),
        Err(LlmError::Cancelled)
    );

    let cancel = CancelToken::new();
    let no_budget = LlmRequest {
        max_tokens: 0,
        ..request("Say hello.", false)
    };
    assert!(matches!(
        llm.complete(&no_budget, &cancel),
        Err(LlmError::BadResponse(_))
    ));

    // A NUL cannot cross into C; the error names the message, not its text.
    match llm.complete(&request("private words\0here", false), &cancel) {
        Err(LlmError::Network(msg)) => {
            assert!(msg.starts_with("local model:"), "{msg}");
            assert!(!msg.contains("private"), "the error quotes the request");
        }
        other => panic!("expected a local failure, got {other:?}"),
    }

    // More than the model's context is refused before anything runs.
    let huge = LlmRequest {
        max_tokens: u32::MAX,
        ..request("private words", false)
    };
    match llm.complete(&huge, &cancel) {
        Err(LlmError::Network(msg)) => {
            assert!(msg.contains("context"), "{msg}");
            assert!(!msg.contains("private"), "the error quotes the request");
        }
        other => panic!("expected a local failure, got {other:?}"),
    }
}

#[test]
#[ignore = "needs the Qwen3-ASR model under $INK_BENCH_DIR; run locally"]
fn what_is_not_a_chat_model_is_refused_at_load() {
    let _serial = serial();
    // The audio projector is a GGUF file, but not a language model.
    let projector = asr_decoder().with_file_name("mmproj-Qwen3-ASR-1.7B-Q8_0.gguf");
    assert!(matches!(
        LlamaLlm::load(&projector, "projector"),
        Err(EngineError::Failed(_))
    ));
    assert!(matches!(
        LlamaLlm::load(&asr_decoder().with_file_name("absent.gguf"), "absent"),
        Err(EngineError::ModelMissing(_))
    ));
}
