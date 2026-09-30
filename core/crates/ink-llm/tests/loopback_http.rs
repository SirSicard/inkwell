//! The real HTTP client against a server on 127.0.0.1. Nothing leaves the machine: every server
//! here binds the loopback interface only.

mod support;

use std::net::TcpListener;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use ink_core::{CancelToken, Llm, LlmError, LlmRequest};
use ink_llm::transport::TransportConfig;
use ink_llm::{ByokConfig, ByokLlm, LocalOnly, Provider, UreqTransport};
use support::{MemKeys, ToLoopback, http_response, loopback_server};

const OPENAI_OK: &str =
    r#"{"choices":[{"message":{"role":"assistant","content":"Local answer."}}]}"#;

fn request() -> LlmRequest {
    LlmRequest {
        system: "Be brief.".into(),
        user: "synthetic question".into(),
        max_tokens: 64,
        temperature: 0.0,
        json_schema: None,
    }
}

fn transport() -> Arc<UreqTransport> {
    Arc::new(UreqTransport::new(TransportConfig {
        connect_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(5),
        write_timeout: Duration::from_secs(5),
        max_body_bytes: 64 * 1024,
    }))
}

fn custom(base_url: String, keys: MemKeys, local_only: bool) -> ByokLlm {
    ByokLlm::new(
        ByokConfig {
            base_url: Some(base_url),
            model: Some("local-model".into()),
            ..ByokConfig::new(Provider::Custom)
        },
        Arc::new(keys),
        transport(),
        LocalOnly::new(local_only),
    )
    .unwrap()
}

#[test]
fn the_real_client_posts_to_a_loopback_server() {
    let (port, received) = loopback_server(http_response(200, "OK", OPENAI_OK), 1);
    let llm = custom(
        format!("http://127.0.0.1:{port}/v1"),
        MemKeys::with("custom", "local-synthetic-key"),
        true,
    );
    let answer = llm.complete(&request(), &CancelToken::new()).unwrap();
    assert_eq!(answer.text, "Local answer.");

    let got = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(got.request_line, "POST /v1/chat/completions HTTP/1.1");
    assert_eq!(
        got.header("Authorization"),
        Some("Bearer local-synthetic-key")
    );
    assert_eq!(got.header("Content-Type"), Some("application/json"));
    assert_eq!(got.header("User-Agent"), Some("inkwell"));
    let body: serde_json::Value = serde_json::from_slice(&got.body).unwrap();
    assert_eq!(body["messages"][1]["content"], "synthetic question");
}

#[test]
fn localhost_resolves_to_the_loopback_server() {
    let (port, received) = loopback_server(http_response(200, "OK", OPENAI_OK), 1);
    let llm = custom(
        format!("http://LOCALHOST:{port}/v1"),
        MemKeys::default(),
        true,
    );
    assert_eq!(
        llm.complete(&request(), &CancelToken::new()).unwrap().text,
        "Local answer."
    );
    assert!(received.recv_timeout(Duration::from_secs(5)).is_ok());
}

#[test]
fn the_anthropic_shape_reaches_a_loopback_server() {
    let answer = r#"{"content":[{"type":"text","text":"Local answer."}]}"#;
    let (port, received) = loopback_server(http_response(200, "OK", answer), 1);
    // Anthropic's endpoint is fixed, so the real request is redirected to the test server by
    // the transport, not by configuration.
    let llm = ByokLlm::new(
        ByokConfig::new(Provider::Anthropic),
        Arc::new(MemKeys::with("anthropic", "sk-ant-synthetic")),
        Arc::new(ToLoopback::new(port)),
        LocalOnly::new(false),
    )
    .unwrap();
    assert_eq!(
        llm.complete(&request(), &CancelToken::new()).unwrap().text,
        "Local answer."
    );
    let got = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(got.request_line, "POST /v1/messages HTTP/1.1");
    assert_eq!(got.header("x-api-key"), Some("sk-ant-synthetic"));
    assert_eq!(got.header("anthropic-version"), Some("2023-06-01"));
}

#[test]
fn a_real_http_error_body_is_not_echoed_into_the_error() {
    let echo = r#"{"error":{"message":"Invalid input: synthetic canary sentence","code":"invalid_value"}}"#;
    let (port, _received) = loopback_server(http_response(400, "Bad Request", echo), 1);
    let llm = custom(
        format!("http://127.0.0.1:{port}/v1"),
        MemKeys::default(),
        false,
    );
    let err = llm.complete(&request(), &CancelToken::new()).unwrap_err();
    assert_eq!(err, LlmError::Http { status: 400 });
    assert!(!format!("{err} {err:?}").contains("canary"));
}

#[test]
fn redirects_are_not_followed() {
    // ureq would follow 301, 302 and 303 as a GET, keeping every header but `Authorization` and
    // `Cookie` (so Anthropic's `x-api-key` would go along); 307 and 308 it never follows for a
    // POST. The second server stands for "somewhere else": it must never be contacted.
    for (status, reason) in [
        (301, "Moved Permanently"),
        (302, "Found"),
        (303, "See Other"),
        (307, "Temporary Redirect"),
        (308, "Permanent Redirect"),
    ] {
        let (elsewhere, contacted) = loopback_server(http_response(200, "OK", OPENAI_OK), 1);
        let redirect = format!(
            "HTTP/1.1 {status} {reason}\r\nLocation: http://127.0.0.1:{elsewhere}/v1/chat/completions\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let (port, received) = loopback_server(redirect, 1);
        let llm = custom(
            format!("http://127.0.0.1:{port}/v1"),
            MemKeys::with("custom", "local-synthetic-key"),
            false,
        );
        assert_eq!(
            llm.complete(&request(), &CancelToken::new()).unwrap_err(),
            LlmError::Http { status }
        );
        assert!(received.recv_timeout(Duration::from_secs(5)).is_ok());
        assert!(
            contacted.recv_timeout(Duration::from_millis(300)).is_err(),
            "the {status} redirect was followed"
        );
    }
}

#[test]
fn an_oversized_answer_is_refused() {
    let big = format!(
        r#"{{"choices":[{{"message":{{"content":"{}"}}}}]}}"#,
        "x".repeat(100 * 1024)
    );
    let (port, _received) = loopback_server(http_response(200, "OK", &big), 1);
    let llm = custom(
        format!("http://127.0.0.1:{port}/v1"),
        MemKeys::default(),
        false,
    );
    assert_eq!(
        llm.complete(&request(), &CancelToken::new()).unwrap_err(),
        LlmError::BadResponse("the answer was too large".into())
    );
}

#[test]
fn a_closed_port_is_a_network_error() {
    // Bind and drop to find a port nothing listens on.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let llm = custom(
        format!("http://127.0.0.1:{port}/v1"),
        MemKeys::default(),
        true,
    );
    assert!(matches!(
        llm.complete(&request(), &CancelToken::new()),
        Err(LlmError::Network(_))
    ));
}

/// A server on 127.0.0.1 that takes one connection and never answers. It holds the connection
/// until the returned sender is dropped, and says on the receiver when it has taken it.
fn silent_server() -> (u16, mpsc::Sender<()>, mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let (hold, released) = mpsc::channel::<()>();
    let (taken, connected) = mpsc::channel::<()>();
    thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let _ = taken.send(());
        let _ = released.recv();
        drop(stream);
    });
    (port, hold, connected)
}

/// A budget stops a request on the wire: a server that takes the request and never answers costs
/// the token's deadline, not the client's 300 s read timeout, and the call reads as cancelled (the
/// budget ran out), not as a network failure.
#[test]
fn a_deadline_stops_a_request_the_server_never_answers() {
    let (port, _hold, _) = silent_server();
    let llm = ByokLlm::new(
        ByokConfig {
            base_url: Some(format!("http://127.0.0.1:{port}/v1")),
            ..ByokConfig::new(Provider::Custom)
        },
        Arc::new(MemKeys::default()),
        Arc::new(UreqTransport::new(TransportConfig::default())),
        LocalOnly::new(true),
    )
    .unwrap();
    let started = Instant::now();
    let deadline = started + Duration::from_millis(300);
    let outcome = llm.complete(&request(), &CancelToken::with_deadline(deadline));
    assert!(matches!(outcome, Err(LlmError::Cancelled)), "{outcome:?}");
    assert!(Instant::now() >= deadline);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

/// A cancel with no deadline (the core's shutdown, a meeting stopped) stops a call whose request
/// is on the wire at once, not when the client's 300 s read timeout ends the request: the call
/// reads as cancelled, and the request is left to end on its own.
#[test]
fn a_cancel_stops_a_request_the_server_never_answers() {
    let (port, _hold, connected) = silent_server();
    let llm = Arc::new(
        ByokLlm::new(
            ByokConfig {
                base_url: Some(format!("http://127.0.0.1:{port}/v1")),
                ..ByokConfig::new(Provider::Custom)
            },
            Arc::new(MemKeys::default()),
            Arc::new(UreqTransport::new(TransportConfig::default())),
            LocalOnly::new(true),
        )
        .unwrap(),
    );
    let cancel = CancelToken::new();
    let (answered, answer) = mpsc::channel();
    {
        let (llm, cancel) = (llm.clone(), cancel.clone());
        thread::spawn(move || {
            let _ = answered.send(llm.complete(&request(), &cancel));
        });
    }
    connected
        .recv_timeout(Duration::from_secs(5))
        .expect("the request reached the server");
    let cancelled = Instant::now();
    cancel.cancel();
    let outcome = answer
        .recv_timeout(Duration::from_secs(10))
        .expect("the call returned once cancelled");
    assert!(matches!(outcome, Err(LlmError::Cancelled)), "{outcome:?}");
    assert!(
        cancelled.elapsed() < Duration::from_secs(2),
        "{:?}",
        cancelled.elapsed()
    );
}
