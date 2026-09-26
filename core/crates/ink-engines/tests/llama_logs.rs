//! What llama.cpp logs while it works, as the app's logger would keep it: the engines' lines go to
//! `tracing`, and every subscriber applies `llama::log_allowed`. Real runs, `#[ignore]`d:
//!
//! ```text
//! INK_BENCH_DIR=<bench data> cargo test -p ink-engines --features engine-llama \
//!     --test llama_logs -- --ignored --nocapture
//! ```
//!
//! The capture sees every line at every level (so it can show the pipe is live), and marks which
//! ones the filter keeps. Only the kept lines must never hold a word of what the model wrote.

#![cfg(feature = "engine-llama")]

mod bench;

use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use ink_core::{CancelToken, Channel, Llm, LlmRequest, OfflineEngine, TranscribeOptions};
use ink_engines::Registry;
use ink_engines::llama::{LlamaLlm, QwenAsr, log_allowed};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

/// One captured line, and whether `log_allowed` keeps it.
struct Line {
    kept: bool,
    text: String,
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<Line>>>);

impl Capture {
    fn lines(&self) -> MutexGuard<'_, Vec<Line>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Fields<'a>(&'a mut String);

impl Visit for Fields<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        let _ = write!(self.0, " {}={value}", field.name());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, " {}={value:?}", field.name());
    }
}

impl Subscriber for Capture {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let meta = event.metadata();
        let mut text = format!("{} {}", meta.level(), meta.target());
        event.record(&mut Fields(&mut text));
        self.lines().push(Line {
            kept: log_allowed(meta),
            text,
        });
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

/// The process-wide capture (llama.cpp logs from several threads, so it is the global default),
/// and a lock so one test at a time runs a model.
fn capture() -> (&'static Capture, MutexGuard<'static, ()>) {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    static SERIAL: Mutex<()> = Mutex::new(());
    let capture = CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        tracing::subscriber::set_global_default(capture.clone())
            .expect("no other subscriber in this test binary");
        capture
    });
    (
        capture,
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner),
    )
}

/// Checks the lines captured since `from` against `output`, and says what it saw.
fn assert_no_output_words(capture: &Capture, from: usize, output: &str, what: &str) {
    let words: HashSet<String> = bench::normalise(output).into_iter().collect();
    assert!(
        !words.is_empty(),
        "{what}: the model wrote nothing to look for"
    );
    let lines = capture.lines();
    let during = &lines[from..];
    assert!(
        !during.is_empty(),
        "{what}: nothing was logged during the call, so the capture proves nothing"
    );
    let kept: Vec<&Line> = during.iter().filter(|l| l.kept).collect();
    for line in &kept {
        let leaked: Vec<String> = bench::normalise(&line.text)
            .into_iter()
            .filter(|w| words.contains(w))
            .collect();
        assert!(
            leaked.is_empty(),
            "{what}: a kept log line holds {leaked:?}"
        );
    }
    // Diagnostic only: how much of the dropped DEBUG/INFO stream overlaps the output at all.
    let dropped_with_words = during
        .iter()
        .filter(|l| !l.kept)
        .filter(|l| {
            bench::normalise(&l.text)
                .iter()
                .any(|w| w.len() >= 6 && words.contains(w))
        })
        .count();
    println!(
        "{what}: {} lines logged during the call, {} kept, {} dropped lines share a long word with \
         the output",
        during.len(),
        kept.len(),
        dropped_with_words
    );
}

fn models() -> std::path::PathBuf {
    bench::bench_dir().join("models/qwen3-asr-1.7b-gguf")
}

#[test]
#[ignore = "needs the Qwen3-ASR model and AMI IHM under $INK_BENCH_DIR; run locally"]
fn a_transcription_logs_no_word_of_its_transcript() {
    let (capture, _serial) = capture();
    let row = Registry::builtin()
        .unwrap()
        .get("qwen3-asr-1.7b-q8")
        .unwrap()
        .clone();
    let engine = QwenAsr::load(
        &models().join(&row.files[0].name),
        &models().join(&row.files[1].name),
        row.info(),
    )
    .unwrap();
    let (rate, audio) =
        bench::read_wav(&bench::bench_dir().join("ami-ihm/ami-ihm-00.wav")).unwrap();
    assert_eq!(rate, 16_000);

    let from = capture.lines().len();
    let options = TranscribeOptions {
        channel: Channel::Mic,
        context: None,
        cancel: CancelToken::new(),
    };
    let text = engine.transcribe(&audio, &options).unwrap().text();
    assert_no_output_words(capture, from, &text, "transcription");
}

#[test]
#[ignore = "needs the Qwen3-ASR model under $INK_BENCH_DIR; run locally"]
fn a_completion_logs_no_word_of_its_answer() {
    let (capture, _serial) = capture();
    let llm = LlamaLlm::load(
        &models().join("Qwen3-ASR-1.7B-Q8_0.gguf"),
        "qwen3-asr-decoder",
    )
    .unwrap();
    let from = capture.lines().len();
    let request = LlmRequest {
        system: "Answer with one JSON object with a string field \"answer\".".into(),
        user: "Say hello to the team.".into(),
        max_tokens: 64,
        temperature: 0.0,
        json_schema: Some(r#"{"type":"object"}"#.into()),
    };
    let answer = llm.complete(&request, &CancelToken::new()).unwrap().text;
    // The answer's words, and the request's own words, which the prompt carried in.
    let words = format!("{answer} {} {}", request.system, request.user);
    assert_no_output_words(capture, from, &words, "completion");
}
