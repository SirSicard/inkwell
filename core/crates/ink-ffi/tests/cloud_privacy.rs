//! Own-key providers under the core's logger at its most verbose: the API key and the words sent
//! to a provider never reach a log line, an event or an error, whatever succeeds or fails. Its own
//! test binary: the logger is process-wide.

mod cloud_support;
mod common;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use cloud_support::*;
use ink_core::{CancelToken, Llm, LlmRequest};
use ink_ffi::logging::{self, Line};
use ink_llm::{KeyStoreError, TransportError};
use ink_pipeline::consent::{self, Consented, Feature};
use serde_json::json;

/// Synthetic words sent to the provider, looked for everywhere they must not be.
const WORDS: &str = "synthetic-prompt-canary-3b9c";

#[test]
fn neither_the_key_nor_the_words_reach_a_log_an_event_or_an_error() {
    logging::install(log::LevelFilter::Trace, false).unwrap();
    let lines: Arc<Mutex<Vec<Line>>> = Arc::default();
    let sink = lines.clone();
    logging::set_sink(Some(Arc::new(move |l: &Line| {
        sink.lock().unwrap().push(l.clone())
    })));

    let rig = Rig::new("cloud-privacy", &[]);
    rig.save_key("openai", "k1");
    // Refused saves, one of them with the key in it.
    rig.ask(
        json!({"cmd": "llm.key.save", "provider": "openai", "key": format!("{KEY} {KEY}")}),
        "bad1",
    );
    rig.ask(
        json!({"cmd": "llm.key.save", "provider": "nobody", "key": KEY}),
        "bad2",
    );
    // A command that does not read is refused before it is queued (ink_command logs the reason).
    for unread in [
        json!({"cmd": "llm.key.save", "provider": "openai", "key": KEY, "extra": KEY}),
        json!({"cmd": "llm.key.save", "provider": KEY}),
        json!({"cmd": "llm.key.save", "provider": "openai", "key": [KEY]}),
    ] {
        let refused = rig.core.command(&unread.to_string()).unwrap_err();
        assert!(!refused.contains(KEY), "{refused}");
    }
    *rig.keys.refuse.lock().unwrap() = Some(KeyStoreError::Failed);
    rig.ask(
        json!({"cmd": "llm.key.save", "provider": "groq", "key": KEY}),
        "bad4",
    );
    *rig.keys.refuse.lock().unwrap() = None;
    rig.choose_openai("c1");
    rig.ask(
        json!({"cmd": "consent.allow", "feature": "meetings", "to": "cloud", "endpoint": "https://api.openai.com/v1"}),
        "a1",
    );

    // The key test, answered, refused and unreachable.
    for (i, answer) in [
        Ok((200, openai_answer("OK"))),
        Ok((401, String::new())),
        Ok((500, String::new())),
        Err(TransportError::Timeout),
    ]
    .into_iter()
    .enumerate()
    {
        rig.net.set(answer);
        let tested = rig.ask(json!({"cmd": "llm.test"}), &format!("t{i}"));
        assert_eq!(tested["type"], "llm.tested");
    }

    // The words, through the features' path (summaries and Ask), answered and failing, and
    // refused by local-only mode.
    let shared = rig.core.shared().clone();
    let llm = ink_ffi::engines::llm(&shared).unwrap();
    let allowed = consent::stored(shared.store.as_ref(), Feature::Meetings);
    let request = LlmRequest {
        system: format!("Synthetic instructions {WORDS}."),
        user: format!("What was said: {WORDS}"),
        max_tokens: 64,
        temperature: 0.0,
        json_schema: None,
    };
    for answer in [
        Ok((200, openai_answer("Synthetic answer."))),
        Ok((200, "not json".to_owned())),
        Ok((429, String::new())),
        Err(TransportError::Io),
    ] {
        rig.net.set(answer);
        let got = Consented {
            inner: llm.as_ref(),
            consents: &allowed,
        }
        .complete(&request, &CancelToken::new());
        if let Err(e) = got {
            assert!(!e.to_string().contains(WORDS), "{e}");
            assert!(!e.to_string().contains(KEY), "{e}");
        }
    }
    rig.set_local_only("on");
    assert!(llm.complete(&request, &CancelToken::new()).is_err());
    assert!(
        rig.keys.reads.load(Ordering::SeqCst) >= 4,
        "the key was used"
    );
    assert!(
        rig.net
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.body.contains(WORDS)),
        "the words were sent"
    );
    rig.ask(json!({"cmd": "llm.key.delete", "provider": "openai"}), "d1");
    drop((llm, shared));
    rig.core.shutdown();
    logging::set_sink(None);

    let lines = lines.lock().unwrap();
    assert!(
        lines.iter().any(|l| l.message.contains("llm.test")),
        "the capture works: the failed tests are logged by name"
    );
    for l in lines.iter() {
        assert!(!l.message.contains(KEY), "a log line holds the key: {l:?}");
        assert!(
            !l.message.contains(WORDS),
            "a log line holds the words: {l:?}"
        );
    }
    let events = rig.events.all();
    assert!(events.iter().any(|e| e["type"] == "command.failed"));
    for e in &events {
        let text = e.to_string();
        assert!(!text.contains(KEY), "an event holds the key: {text}");
        assert!(!text.contains(WORDS), "an event holds the words: {text}");
    }
    rig.events.assert_valid();
}
