//! The log filter every logger in the app must apply ([`ink_llm::log_record_allowed`]).
//!
//! ureq's `debug` log writes each request's header block and masks only `Authorization` and
//! `Cookie`, so Anthropic's `x-api-key` is logged in clear as soon as a logger runs at `debug`.
//! These tests drive a real Anthropic request through the real client, against a loopback
//! server, with a logger capturing everything at `trace`.
//!
//! Its own test binary because a logger is process-global: installed here, it cannot leak into
//! or be disturbed by the other tests. The two tests share one logger and take turns.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use ink_core::{CancelToken, Llm, LlmRequest};
use ink_llm::{ByokConfig, ByokLlm, LocalOnly, Provider, log_record_allowed};
use log::{LevelFilter, Log, Metadata, Record};
use support::{MemKeys, ToLoopback, http_response, loopback_server};

const KEY: &str = "sk-ant-synthetic-log-canary";

/// Captures every record, through the crate's filter or not.
struct Capture {
    filtered: AtomicBool,
    records: Mutex<Vec<String>>,
}

impl Log for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        !self.filtered.load(Ordering::SeqCst) || log_record_allowed(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        // The `log!` macros call `log`, not `enabled`, so the filter is applied here.
        if self.enabled(record.metadata()) {
            self.records
                .lock()
                .unwrap()
                .push(format!("{} {}", record.target(), record.args()));
        }
    }

    fn flush(&self) {}
}

static LOGGER: Capture = Capture {
    filtered: AtomicBool::new(true),
    records: Mutex::new(Vec::new()),
};
static TURN: Mutex<()> = Mutex::new(());

/// Sends one Anthropic request through the real client with the logger set to `filtered`, and
/// returns what was logged meanwhile.
fn log_of_one_anthropic_request(filtered: bool) -> Vec<String> {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        log::set_logger(&LOGGER).expect("the only logger in this binary");
        log::set_max_level(LevelFilter::Trace);
    });
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    LOGGER.filtered.store(filtered, Ordering::SeqCst);
    LOGGER.records.lock().unwrap().clear();

    let answer = r#"{"content":[{"type":"text","text":"ok"}]}"#;
    let (port, received) = loopback_server(http_response(200, "OK", answer), 1);
    let llm = ByokLlm::new(
        ByokConfig::new(Provider::Anthropic),
        Arc::new(MemKeys::with("anthropic", KEY)),
        Arc::new(ToLoopback::new(port)),
        LocalOnly::new(false),
    )
    .unwrap();
    let request = LlmRequest {
        system: String::new(),
        user: "synthetic".into(),
        max_tokens: 8,
        temperature: 0.0,
        json_schema: None,
    };
    assert_eq!(
        llm.complete(&request, &CancelToken::new()).unwrap().text,
        "ok"
    );
    let got = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        got.header("x-api-key"),
        Some(KEY),
        "the key was really sent"
    );

    LOGGER.records.lock().unwrap().clone()
}

#[test]
fn with_the_filter_the_anthropic_key_never_reaches_the_log() {
    let records = log_of_one_anthropic_request(true);
    for record in &records {
        assert!(!record.contains(KEY), "logged: {record}");
    }
}

/// Proves the filter is load-bearing: without it, the same request writes the key into the log
/// through ureq's debug output. If ureq ever stops doing that, this test fails, and the filter
/// can be reconsidered.
#[test]
fn without_the_filter_the_anthropic_key_is_logged() {
    let records = log_of_one_anthropic_request(false);
    assert!(
        records
            .iter()
            .any(|r| r.starts_with("ureq") && r.contains(KEY)),
        "expected ureq to log the key; got {} records",
        records.len()
    );
}
