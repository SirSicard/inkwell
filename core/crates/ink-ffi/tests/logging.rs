//! One logger, both filters. The core installs the only `log` logger and `tracing` subscriber, and
//! each drops what would leak: ureq's debug lines carry the `x-api-key` header, and llama.cpp's
//! debug lines quote generated text. A secret and a transcript word are planted at debug through
//! each path; neither reaches the sink, while control lines at the same level do, so the sink is
//! listening and only the filters stop them.
//!
//! Its own test binary: the logger is process-wide. Run it with `--features engine-llama` too, so
//! the llama.cpp filter applied is `ink_engines::llama::log_allowed` itself (CI does, on macOS).

use std::sync::{Arc, Mutex};

use ink_ffi::logging::{self, Line};

const SECRET: &str = "sk-ant-PLANTED-0123456789";
const WORD: &str = "zebrafish";

#[test]
fn the_core_logger_drops_api_keys_and_generated_text_at_debug() {
    logging::install(log::LevelFilter::Trace, false).unwrap();
    let lines: Arc<Mutex<Vec<Line>>> = Arc::default();
    let sink = lines.clone();
    logging::set_sink(Some(Arc::new(move |l: &Line| {
        sink.lock().unwrap().push(l.clone())
    })));

    // The `log` path: ureq's request dump (ink-llm, ink-engines' downloader).
    log::debug!(target: "ureq::unit", "writing prelude: POST /v1/messages x-api-key: {SECRET}");
    log::trace!(target: "ureq", "header x-api-key: {SECRET}");
    // A llama.cpp line arriving on the `log` path (were tracing's `log` feature ever on).
    log::debug!(target: "llama-cpp-2", "generated: {WORD}");
    // The `tracing` path: llama.cpp through llama-cpp-2, and its log bridge.
    tracing::debug!(target: "llama-cpp-2", module = "llama", "generated token: {WORD}");
    tracing::info!(target: "llama-cpp-2", "sampled: {WORD}");
    tracing::warn!(target: "llama_cpp_2::log", "re-emitted buffered line: {WORD}");
    tracing::debug!(target: "llama_cpp_2::context", "decode: {WORD}");

    // Controls: the same levels on other targets are written.
    log::debug!(target: "ink_ffi_test", "control log debug");
    log::info!(target: "ureq::pool", "control ureq info");
    tracing::debug!(target: "ink_ffi_test", answer = 42, "control tracing debug");
    #[cfg(feature = "engine-llama")]
    tracing::warn!(target: "llama-cpp-2", "control llama warning: model file missing");

    logging::set_sink(None);
    let lines = lines.lock().unwrap().clone();
    let all: String = lines
        .iter()
        .map(|l| format!("{} {} {}\n", l.level, l.target, l.message))
        .collect();
    assert!(!all.contains(SECRET), "the API key was written:\n{all}");
    assert!(!all.contains(WORD), "generated text was written:\n{all}");
    for control in [
        "control log debug",
        "control ureq info",
        "control tracing debug answer=42",
    ] {
        assert!(all.contains(control), "{control:?} missing:\n{all}");
    }
    #[cfg(feature = "engine-llama")]
    assert!(
        all.contains("control llama warning"),
        "llama warnings are kept:\n{all}"
    );
    let expected = if cfg!(feature = "engine-llama") { 4 } else { 3 };
    assert_eq!(lines.len(), expected, "{all}");

    // The level applies on top: at info, the debug controls go too.
    logging::install(log::LevelFilter::Info, false).unwrap();
    let quiet: Arc<Mutex<Vec<Line>>> = Arc::default();
    let sink = quiet.clone();
    logging::set_sink(Some(Arc::new(move |l: &Line| {
        sink.lock().unwrap().push(l.clone())
    })));
    log::debug!(target: "ink_ffi_test", "hidden");
    tracing::debug!(target: "ink_ffi_test", "hidden");
    log::info!(target: "ink_ffi_test", "shown");
    logging::set_sink(None);
    let quiet = quiet.lock().unwrap();
    assert_eq!(quiet.len(), 1, "{quiet:?}");
    assert_eq!(quiet[0].message, "shown");

    // And nobody else can take over: the logger and the subscriber are ours for the process.
    struct Other;
    impl log::Log for Other {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, _: &log::Record<'_>) {}
        fn flush(&self) {}
    }
    assert!(log::set_logger(&Other).is_err());
}
