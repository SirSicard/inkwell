//! Mock-provider tests: the request each provider sends, how its answer is read, how errors
//! map, and the three guarantees that no text leaves the machine when it must not (local-only
//! mode, a denied keychain, a cancelled call). No server: a counting transport stands in.

mod support;

use std::sync::Arc;

use ink_core::{CancelToken, Endpoint, Llm, LlmError, LlmRequest};
use ink_llm::provider::configured_providers;
use ink_llm::{
    ByokConfig, ByokLlm, EndpointError, EndpointUrl, KeyStore, LocalOnly, Provider, TransportError,
};
use serde_json::json;
use support::{CountingTransport, MemKeys};

const OPENAI_OK: &str =
    r#"{"choices":[{"message":{"role":"assistant","content":"Polished text."}}]}"#;
const ANTHROPIC_OK: &str = r#"{"content":[{"type":"text","text":"Polished "},{"type":"tool_use","id":"x"},{"type":"text","text":"text."}]}"#;

fn request(json_schema: Option<&str>) -> LlmRequest {
    LlmRequest {
        system: "Clean up the dictation.".into(),
        user: "um the synthetic sentence".into(),
        max_tokens: 256,
        temperature: 0.3,
        json_schema: json_schema.map(str::to_owned),
    }
}

fn byok(
    config: ByokConfig,
    keys: &Arc<MemKeys>,
    transport: &Arc<CountingTransport>,
    local_only: bool,
) -> ByokLlm {
    ByokLlm::new(
        config,
        keys.clone(),
        transport.clone(),
        LocalOnly::new(local_only),
    )
    .expect("a valid provider")
}

fn run(llm: &ByokLlm, req: &LlmRequest) -> Result<String, LlmError> {
    llm.complete(req, &CancelToken::new()).map(|r| r.text)
}

#[test]
fn openai_request_shape() {
    let keys = Arc::new(MemKeys::with("openai", "sk-synthetic-openai"));
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    let llm = byok(ByokConfig::new(Provider::OpenAi), &keys, &transport, false);

    assert_eq!(run(&llm, &request(None)).unwrap(), "Polished text.");
    let seen = transport.last();
    assert_eq!(seen.url, "https://api.openai.com/v1/chat/completions");
    assert_eq!(
        seen.header("Authorization"),
        Some("Bearer sk-synthetic-openai")
    );
    assert_eq!(seen.header("Content-Type"), Some("application/json"));
    assert_eq!(
        seen.body,
        json!({
            "model": "gpt-4o-mini",
            "messages": [
                {"role": "system", "content": "Clean up the dictation."},
                {"role": "user", "content": "um the synthetic sentence"}
            ],
            "max_tokens": 256,
            "temperature": 0.3
        })
    );
    assert!(!seen.loopback_only);
}

#[test]
fn openai_compatible_providers_ask_for_json_mode_for_structured_tasks() {
    for (provider, url) in [
        (
            Provider::OpenAi,
            "https://api.openai.com/v1/chat/completions",
        ),
        (
            Provider::Groq,
            "https://api.groq.com/openai/v1/chat/completions",
        ),
        (
            Provider::OpenRouter,
            "https://openrouter.ai/api/v1/chat/completions",
        ),
    ] {
        let keys = Arc::new(MemKeys::with(provider.id(), "sk-synthetic"));
        let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
        let llm = byok(ByokConfig::new(provider), &keys, &transport, false);
        run(&llm, &request(Some("{}"))).unwrap();
        let seen = transport.last();
        assert_eq!(seen.url, url);
        assert_eq!(seen.body["model"], provider.default_model());
        assert_eq!(seen.body["response_format"], json!({"type": "json_object"}));
        assert_eq!(seen.header("Authorization"), Some("Bearer sk-synthetic"));
    }
}

#[test]
fn groq_and_openrouter_request_shape() {
    for provider in [Provider::Groq, Provider::OpenRouter] {
        let keys = Arc::new(MemKeys::with(provider.id(), "sk-synthetic"));
        let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
        let config = ByokConfig {
            model: Some("some-model".into()),
            ..ByokConfig::new(provider)
        };
        let llm = byok(config, &keys, &transport, false);
        run(&llm, &request(None)).unwrap();
        let body = transport.last().body;
        assert_eq!(body["model"], "some-model");
        assert_eq!(body["messages"][1]["content"], "um the synthetic sentence");
        assert!(body.get("response_format").is_none());
    }
}

#[test]
fn anthropic_request_shape() {
    let keys = Arc::new(MemKeys::with("anthropic", "sk-ant-synthetic"));
    let transport = Arc::new(CountingTransport::ok(ANTHROPIC_OK));
    let llm = byok(
        ByokConfig::new(Provider::Anthropic),
        &keys,
        &transport,
        false,
    );

    assert_eq!(run(&llm, &request(Some("{}"))).unwrap(), "Polished text.");
    let seen = transport.last();
    assert_eq!(seen.url, "https://api.anthropic.com/v1/messages");
    assert_eq!(seen.header("x-api-key"), Some("sk-ant-synthetic"));
    assert_eq!(seen.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(seen.header("Authorization"), None);
    assert_eq!(
        seen.body,
        json!({
            "model": "claude-haiku-4-5-20251001",
            "system": "Clean up the dictation.",
            "messages": [{"role": "user", "content": "um the synthetic sentence"}],
            "max_tokens": 256,
            "temperature": 0.3
        })
    );
}

#[test]
fn an_empty_system_prompt_is_left_out() {
    let mut req = request(None);
    req.system.clear();

    let keys = Arc::new(MemKeys::with("anthropic", "k"));
    let transport = Arc::new(CountingTransport::ok(ANTHROPIC_OK));
    run(
        &byok(
            ByokConfig::new(Provider::Anthropic),
            &keys,
            &transport,
            false,
        ),
        &req,
    )
    .unwrap();
    assert!(transport.last().body.get("system").is_none());

    let keys = Arc::new(MemKeys::with("openai", "k"));
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    run(
        &byok(ByokConfig::new(Provider::OpenAi), &keys, &transport, false),
        &req,
    )
    .unwrap();
    assert_eq!(
        transport.last().body["messages"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn custom_request_shape_without_a_key_sends_no_authorization() {
    let keys = Arc::new(MemKeys::default());
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    let llm = byok(ByokConfig::new(Provider::Custom), &keys, &transport, true);

    assert_eq!(
        llm.info().endpoint,
        Endpoint::Loopback("http://localhost:11434/v1".into())
    );
    run(&llm, &request(Some("{}"))).unwrap();
    let seen = transport.last();
    assert_eq!(seen.url, "http://localhost:11434/v1/chat/completions");
    assert_eq!(seen.header("Authorization"), None);
    assert_eq!(seen.body["model"], "llama3");
    // Custom servers are not asked for JSON mode; some reject the parameter.
    assert!(seen.body.get("response_format").is_none());
    assert!(
        seen.loopback_only,
        "local-only mode tells the transport to check again"
    );
}

#[test]
fn custom_server_with_a_key_sends_it() {
    let keys = Arc::new(MemKeys::with("custom", "local-synthetic-key"));
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    let config = ByokConfig {
        base_url: Some("http://127.0.0.1:8080/v1/".into()),
        model: Some("local-model".into()),
        ..ByokConfig::new(Provider::Custom)
    };
    run(&byok(config, &keys, &transport, false), &request(None)).unwrap();
    let seen = transport.last();
    assert_eq!(seen.url, "http://127.0.0.1:8080/v1/chat/completions");
    assert_eq!(
        seen.header("Authorization"),
        Some("Bearer local-synthetic-key")
    );
}

/// A built-in provider's key only ever goes to that provider: its endpoint is fixed, and a
/// configuration that points it elsewhere is refused before the keychain is asked.
#[test]
fn a_built_in_providers_endpoint_cannot_be_overridden() {
    for provider in [
        Provider::OpenAi,
        Provider::Groq,
        Provider::Anthropic,
        Provider::OpenRouter,
    ] {
        for url in [
            "https://proxy.example.com/v1",
            "http://127.0.0.1:8080/v1",
            provider.default_base_url(),
        ] {
            let keys = Arc::new(MemKeys::with(provider.id(), "sk-synthetic"));
            let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
            let config = ByokConfig {
                base_url: Some(url.into()),
                ..ByokConfig::new(provider)
            };
            let built = ByokLlm::new(
                config,
                keys.clone(),
                transport.clone(),
                LocalOnly::new(false),
            );
            assert_eq!(
                built.err(),
                Some(EndpointError::FixedEndpoint),
                "{provider:?} accepted {url}"
            );
            assert_eq!(keys.reads(), 0);
            assert_eq!(transport.calls(), 0);
        }
    }
}

#[test]
fn a_custom_endpoint_override_works() {
    let keys = Arc::new(MemKeys::with("custom", "custom-synthetic-key"));
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    let config = ByokConfig {
        base_url: Some("https://models.example.com/v1".into()),
        ..ByokConfig::new(Provider::Custom)
    };
    run(&byok(config, &keys, &transport, false), &request(None)).unwrap();
    let seen = transport.last();
    assert_eq!(seen.url, "https://models.example.com/v1/chat/completions");
    assert_eq!(
        seen.header("Authorization"),
        Some("Bearer custom-synthetic-key")
    );
}

#[test]
fn a_custom_server_over_plain_http_elsewhere_never_gets_the_key() {
    let keys = Arc::new(MemKeys::with("custom", "sk-synthetic"));
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    let config = ByokConfig {
        base_url: Some("http://192.0.2.10:11434/v1".into()),
        ..ByokConfig::new(Provider::Custom)
    };
    run(&byok(config, &keys, &transport, false), &request(None)).unwrap();
    assert_eq!(transport.last().header("Authorization"), None);
    assert_eq!(keys.reads(), 0, "the key is not even read");
}

#[test]
fn openai_compatible_response_parsing() {
    for (body, expected) in [
        (OPENAI_OK, Ok("Polished text.".to_owned())),
        (
            r#"{"choices":[{"message":{"content":null}}]}"#,
            Err("choices[0].message.content"),
        ),
        (r#"{"choices":[]}"#, Err("choices[0].message.content")),
        ("<html>not json</html>", Err("not JSON")),
    ] {
        let keys = Arc::new(MemKeys::with("openai", "k"));
        let transport = Arc::new(CountingTransport::ok(body));
        let got = run(
            &byok(ByokConfig::new(Provider::OpenAi), &keys, &transport, false),
            &request(None),
        );
        match expected {
            Ok(text) => assert_eq!(got.unwrap(), text),
            Err(field) => match got {
                Err(LlmError::BadResponse(msg)) => assert!(msg.contains(field), "{msg}"),
                other => panic!("{body}: {other:?}"),
            },
        }
    }
}

#[test]
fn anthropic_response_parsing() {
    for (body, ok) in [
        (ANTHROPIC_OK, true),
        (r#"{"content":[{"type":"tool_use","id":"x"}]}"#, false),
        (r#"{"content":"text"}"#, false),
        (r#"{"type":"message"}"#, false),
    ] {
        let keys = Arc::new(MemKeys::with("anthropic", "k"));
        let transport = Arc::new(CountingTransport::ok(body));
        let got = run(
            &byok(
                ByokConfig::new(Provider::Anthropic),
                &keys,
                &transport,
                false,
            ),
            &request(None),
        );
        assert_eq!(got.is_ok(), ok, "{body}: {got:?}");
        if !ok {
            assert!(matches!(got, Err(LlmError::BadResponse(_))));
        }
    }
}

#[test]
fn http_errors_map_to_their_status_without_the_body() {
    // Providers echo the request in validation errors; the canary stands for the user's text.
    let echo = r#"{"error":{"message":"Invalid value for 'input': synthetic canary sentence","type":"invalid_request_error","code":"invalid_value"}}"#;
    for status in [400, 401, 404, 429, 500, 503, 301] {
        let keys = Arc::new(MemKeys::with("openai", "sk-canary-key"));
        let transport = Arc::new(CountingTransport::status(status, echo));
        let err = run(
            &byok(ByokConfig::new(Provider::OpenAi), &keys, &transport, false),
            &request(None),
        )
        .unwrap_err();
        assert_eq!(err, LlmError::Http { status });
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains("canary"), "{shown}");
    }
}

#[test]
fn transport_failures_map_to_fixed_messages() {
    for (error, want) in [
        (
            TransportError::Dns,
            LlmError::Network("could not resolve the host".into()),
        ),
        (
            TransportError::Connect,
            LlmError::Network("could not connect".into()),
        ),
        (
            TransportError::Timeout,
            LlmError::Network("timed out".into()),
        ),
        (
            TransportError::TooLarge,
            LlmError::BadResponse("the answer was too large".into()),
        ),
        (
            TransportError::NotLoopback,
            LlmError::LocalOnly {
                endpoint: "https://api.openai.com/v1".into(),
            },
        ),
    ] {
        let keys = Arc::new(MemKeys::with("openai", "k"));
        let transport = Arc::new(CountingTransport::failing(error));
        let got = run(
            &byok(ByokConfig::new(Provider::OpenAi), &keys, &transport, false),
            &request(None),
        );
        assert_eq!(got.unwrap_err(), want);
    }
}

#[test]
fn a_non_loopback_url_is_refused_in_local_only_mode() {
    for provider in Provider::ALL {
        let config = ByokConfig {
            base_url: (provider == Provider::Custom)
                .then(|| "https://models.example.com/v1".into()),
            ..ByokConfig::new(provider)
        };
        let keys = Arc::new(MemKeys::with(provider.id(), "sk-synthetic"));
        let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
        let llm = byok(config, &keys, &transport, true);
        let err = run(&llm, &request(None)).unwrap_err();
        assert!(
            matches!(err, LlmError::LocalOnly { .. }),
            "{provider:?}: {err:?}"
        );
        assert_eq!(transport.calls(), 0, "{provider:?} made a network call");
        assert_eq!(keys.reads(), 0, "{provider:?} asked the keychain");
    }
}

#[test]
fn the_local_only_switch_applies_on_the_next_call() {
    let keys = Arc::new(MemKeys::with("openai", "k"));
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    let switch = LocalOnly::new(false);
    let llm = ByokLlm::new(
        ByokConfig::new(Provider::OpenAi),
        keys.clone(),
        transport.clone(),
        switch.clone(),
    )
    .unwrap();
    run(&llm, &request(None)).unwrap();
    assert_eq!(transport.calls(), 1);

    switch.set(true);
    assert!(matches!(
        run(&llm, &request(None)),
        Err(LlmError::LocalOnly { .. })
    ));
    assert_eq!(transport.calls(), 1);
}

#[test]
fn llm_info_endpoint_agrees_with_the_guard() {
    // Every trick URL from the guard's tests: whatever `info` says is what the guard does.
    for raw in [
        "http://localhost:11434/v1",
        "http://LOCALHOST/v1",
        "http://127.0.0.1:8080/v1",
        "http://[::1]:8080/v1",
        "http://localhost.example.com/v1",
        "http://127.0.0.1.nip.io/v1",
        "http://localhost./v1",
        "http://0.0.0.0/v1",
        "http://[::ffff:127.0.0.1]/v1",
        "http://2130706433/v1",
        "https://models.example.com/v1",
    ] {
        let keys = Arc::new(MemKeys::default());
        let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
        let config = ByokConfig {
            base_url: Some(raw.into()),
            ..ByokConfig::new(Provider::Custom)
        };
        let llm = byok(config, &keys, &transport, true);
        let local = llm.info().endpoint.is_local();
        assert_eq!(
            local,
            EndpointUrl::parse(raw).unwrap().is_loopback(),
            "{raw}"
        );
        let refused = matches!(run(&llm, &request(None)), Err(LlmError::LocalOnly { .. }));
        assert_eq!(refused, !local, "{raw}: info and guard disagree");
        assert_eq!(transport.calls(), usize::from(local), "{raw}");
    }
    // URLs the guard cannot vouch for never become a provider at all.
    for raw in [
        "http://127.0.0.1@example.com/v1",
        "http://example.com#@127.0.0.1",
        "ftp://127.0.0.1/",
        "/v1",
    ] {
        let config = ByokConfig {
            base_url: Some(raw.into()),
            ..ByokConfig::new(Provider::Custom)
        };
        let built = ByokLlm::new(
            config,
            Arc::new(MemKeys::default()),
            Arc::new(CountingTransport::ok(OPENAI_OK)),
            LocalOnly::new(true),
        );
        assert!(built.is_err(), "{raw}");
    }
}

#[test]
fn a_denied_keychain_makes_zero_network_calls() {
    for provider in [Provider::OpenAi, Provider::Anthropic, Provider::Custom] {
        let keys = Arc::new(MemKeys::denying());
        let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
        let llm = byok(ByokConfig::new(provider), &keys, &transport, false);
        assert_eq!(
            run(&llm, &request(None)),
            Err(LlmError::KeychainDenied),
            "{provider:?}"
        );
        assert_eq!(keys.reads(), 1);
        assert_eq!(transport.calls(), 0, "{provider:?} sent something");
    }
}

#[test]
fn a_missing_key_makes_zero_network_calls() {
    let keys = Arc::new(MemKeys::default());
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    let llm = byok(ByokConfig::new(Provider::Groq), &keys, &transport, false);
    assert_eq!(run(&llm, &request(None)), Err(LlmError::NoKey));
    assert_eq!(transport.calls(), 0);
}

#[test]
fn a_cancelled_call_sends_nothing() {
    let keys = Arc::new(MemKeys::with("openai", "k"));
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    let llm = byok(ByokConfig::new(Provider::OpenAi), &keys, &transport, false);
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        llm.complete(&request(None), &cancel),
        Err(LlmError::Cancelled)
    );
    assert_eq!(transport.calls(), 0);
    assert_eq!(keys.reads(), 0);
}

#[test]
fn configured_providers_never_read_a_key() {
    let keys = MemKeys::with("groq", "k");
    keys.save_key("anthropic", "k").unwrap();
    assert_eq!(
        configured_providers(&keys),
        [Provider::Groq, Provider::Anthropic]
    );
    assert_eq!(keys.reads(), 0, "the settings screen must not prompt");
}

#[test]
fn one_transport_serves_every_call_and_provider() {
    let transport = Arc::new(CountingTransport::ok(OPENAI_OK));
    let keys = Arc::new(MemKeys::with("openai", "k"));
    keys.save_key("groq", "k").unwrap();
    let a = byok(ByokConfig::new(Provider::OpenAi), &keys, &transport, false);
    let b = byok(ByokConfig::new(Provider::Groq), &keys, &transport, false);
    for _ in 0..3 {
        run(&a, &request(None)).unwrap();
        run(&b, &request(None)).unwrap();
    }
    assert_eq!(transport.calls(), 6);
    // Two providers, one client.
    assert_eq!(Arc::strong_count(&transport), 3);
}
