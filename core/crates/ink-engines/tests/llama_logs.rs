//! What llama.cpp logs while it works, as the app's logger would keep it: the engines' lines go to
//! `tracing`, and every subscriber applies `llama::log_allowed`. Real runs, `#[ignore]`d:
//!
//! ```text
//! INK_BENCH_DIR=<bench data> cargo test -p ink-engines --features engine-llama \
//!     --test llama_logs -- --ignored --nocapture
//! ```
//!
//! The capture sees every line at every level from before the model loads, and marks which ones
//! the filter keeps. Each test checks that:
//! - the filter keeps real lines (the model's load warnings), so the check below is not vacuous,
//!   and those lines carry the target the filter expects;
//! - no kept line quotes the request or the model's output, while it reports how many dropped lines
//!   do. A line "quotes" when it holds a word of five letters or more from the request or output
//!   that no line before the call held (so the model file's own vocabulary does not count).

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
use tracing::{Event, Level, Metadata, Subscriber};

/// One captured line, and whether `log_allowed` keeps it.
struct Line {
    kept: bool,
    level: Level,
    target: String,
    /// The `module` field llama-cpp-2 puts llama.cpp's, ggml's and mtmd's module in.
    module: String,
    text: String,
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<Line>>>);

impl Capture {
    fn lines(&self) -> MutexGuard<'_, Vec<Line>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Fields<'a> {
    text: &'a mut String,
    module: &'a mut String,
}

impl Visit for Fields<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "module" {
            value.clone_into(self.module);
        }
        let _ = write!(self.text, " {}={value}", field.name());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.text, " {}={value:?}", field.name());
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
        let (mut text, mut module) = (String::new(), String::new());
        event.record(&mut Fields {
            text: &mut text,
            module: &mut module,
        });
        self.lines().push(Line {
            kept: log_allowed(meta),
            level: *meta.level(),
            target: meta.target().to_owned(),
            module,
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

/// The checks in the module docs. `loaded` is where this test's model load began, `from` where the
/// call began; `text` is the request and output together.
fn check(capture: &Capture, loaded: usize, from: usize, text: &str, what: &str) {
    let lines = capture.lines();
    let this_test = &lines[loaded..];
    let kept_here = this_test.iter().filter(|l| l.kept).count();
    assert!(kept_here > 0, "{what}: the filter kept no line at all");
    assert!(
        this_test.iter().any(|l| l.target == "llama-cpp-2"
            && ["llama.cpp", "ggml", "mtmd"]
                .iter()
                .any(|m| l.module.starts_with(m))),
        "{what}: no llama.cpp line arrived under the target the filter expects"
    );
    assert!(
        this_test
            .iter()
            .filter(|l| l.kept)
            .all(|l| l.target == "llama-cpp-2" && matches!(l.level, Level::WARN | Level::ERROR)),
        "{what}: the filter kept something other than an engine warning or error"
    );

    let known: HashSet<String> = lines[..from]
        .iter()
        .flat_map(|l| bench::normalise(&l.text))
        .collect();
    let words: HashSet<String> = bench::normalise(text)
        .into_iter()
        .filter(|w| w.chars().count() >= 5 && !known.contains(w))
        .collect();
    assert!(!words.is_empty(), "{what}: nothing distinctive to look for");
    let quotes = |l: &&Line| bench::normalise(&l.text).iter().any(|w| words.contains(w));
    let after = &lines[from..];
    let kept_quoting = after.iter().filter(|l| l.kept).filter(quotes).count();
    let dropped_quoting = after.iter().filter(|l| !l.kept).filter(quotes).count();
    let binding_own = this_test
        .iter()
        .filter(|l| l.target.starts_with("llama_cpp_2"))
        .count();
    println!(
        "{what}: {} lines from load on ({kept_here} kept); during and after the call {} lines, \
         kept lines quoting the output {kept_quoting}, dropped lines quoting it \
         {dropped_quoting}; binding's own events {binding_own}",
        this_test.len(),
        after.len(),
    );
    if dropped_quoting == 0 {
        println!(
            "{what}: no dropped line quoted the output in this run; the unit tests on \
             `log_allowed` carry the proof that the filter is needed"
        );
    }
    assert_eq!(kept_quoting, 0, "{what}: a kept log line quotes the output");
}

fn models() -> std::path::PathBuf {
    bench::bench_dir().join("models/qwen3-asr-1.7b-gguf")
}

#[test]
#[ignore = "needs the Qwen3-ASR model and AMI IHM under $INK_BENCH_DIR; run locally"]
fn a_transcription_logs_no_word_of_its_transcript() {
    let (capture, _serial) = capture();
    let loaded = capture.lines().len();
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
    check(capture, loaded, from, &text, "transcription");
}

#[test]
#[ignore = "needs the Qwen3-ASR model under $INK_BENCH_DIR; run locally"]
fn a_completion_logs_no_word_of_its_answer() {
    let (capture, _serial) = capture();
    let loaded = capture.lines().len();
    let llm = LlamaLlm::load(
        &models().join("Qwen3-ASR-1.7B-Q8_0.gguf"),
        "qwen3-asr-decoder",
    )
    .unwrap();
    let from = capture.lines().len();
    let request = LlmRequest {
        system: "Answer with one JSON object with a string field \"answer\".".into(),
        // A planted word, so there is something distinctive to find.
        user: "Say hello to the team from Quokkazephyr.".into(),
        max_tokens: 64,
        temperature: 0.0,
        json_schema: Some(r#"{"type":"object"}"#.into()),
    };
    let answer = llm.complete(&request, &CancelToken::new()).unwrap().text;
    // The answer's words, and the request's own words, which the prompt carried in.
    let words = format!("{answer} {} {}", request.system, request.user);
    check(capture, loaded, from, &words, "completion");
}
