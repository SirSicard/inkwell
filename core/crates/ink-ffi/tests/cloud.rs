//! Own-key (BYOK) language models (`cloud`): the providers listed, a key kept only in the key
//! store, a cloud provider chosen only with local-only mode turned off by the shell's say-so, the
//! features sending to it only with their own consent for its endpoint, and the key test. Every
//! key, model and text here is synthetic; the key store and the network are fakes.

mod cloud_support;
mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use cloud_support::*;
use common::*;
use ink_core::mock::MockPlatform;
use ink_core::{CancelToken, Endpoint, HotkeyEvent, Llm, LlmError, LlmRequest};
use ink_ffi::dictation::{Block, DictationInbox};
use ink_ffi::runtime::{Core, DictationParts};
use ink_llm::{KeyStoreError, TransportError};
use ink_pipeline::chain::DictationSettings;
use ink_pipeline::consent::{self, Consented, Feature, LlmConsent};
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::gain_stage::Vad;
use serde_json::json;

const OPENAI: &str = "https://api.openai.com/v1";

#[test]
fn every_provider_is_listed_and_a_key_goes_only_to_the_key_store() {
    let rig = Rig::new("cloud-keys", &[]);
    let listed = rig.ask(json!({"cmd": "llm.providers"}), "p1");
    assert_eq!(listed["type"], "llm.providers");
    let ids: Vec<&str> = listed["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["openai", "groq", "anthropic", "openrouter", "custom"]);
    assert_eq!(entry(&listed, "openai")["endpoint"], OPENAI);
    assert_eq!(entry(&listed, "openai")["custom_url"], false);
    assert_eq!(entry(&listed, "custom")["custom_url"], true);
    assert_eq!(entry(&listed, "custom")["needs_key"], false);
    assert!(
        listed["providers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["has_key"] == false)
    );
    assert_eq!(listed["local_only"], true, "on by default");
    assert_eq!(listed["ready"], false);
    assert!(listed.get("chosen").is_none());

    // Saved, trimmed of what a paste brings along.
    let saved = rig.ask(
        json!({"cmd": "llm.key.save", "provider": "openai", "key": format!("  {KEY}\n")}),
        "k1",
    );
    assert_eq!(entry(&saved, "openai")["has_key"], true);
    assert_eq!(entry(&saved, "groq")["has_key"], false);
    assert_eq!(rig.keys.keys.lock().unwrap()["openai"], KEY);

    // What no key looks like is refused, and the failure never repeats it.
    for (i, bad) in [
        "",
        "   ",
        "sk-synthetic with-space",
        "sk-synthetic\u{7}bell",
        "sk-synthetic-é",
    ]
    .into_iter()
    .enumerate()
    {
        let failed = rig.ask(
            json!({"cmd": "llm.key.save", "provider": "openai", "key": bad}),
            &format!("bad{i}"),
        );
        assert_eq!(failed["type"], "command.failed", "{bad:?}");
        assert!(
            !failed["message"].as_str().unwrap().contains("sk-"),
            "{failed}"
        );
    }
    assert_eq!(rig.keys.keys.lock().unwrap()["openai"], KEY, "left alone");
    let long = "k".repeat(ink_ffi::cloud::MAX_KEY_CHARS + 1);
    let failed = rig.ask(
        json!({"cmd": "llm.key.save", "provider": "openai", "key": long}),
        "long",
    );
    assert_eq!(failed["type"], "command.failed");
    let failed = rig.ask(
        json!({"cmd": "llm.key.save", "provider": "nobody", "key": KEY}),
        "who",
    );
    assert_eq!(failed["type"], "command.failed");

    // A store that refuses is said, never read as "no key".
    *rig.keys.refuse.lock().unwrap() = Some(KeyStoreError::Denied);
    let failed = rig.ask(
        json!({"cmd": "llm.key.save", "provider": "groq", "key": KEY}),
        "k2",
    );
    assert_eq!(
        failed["message"],
        "couldn't save the key: the key store refused"
    );
    let unread = rig.ask(json!({"cmd": "llm.providers"}), "p2");
    assert_eq!(unread["error"], "couldn't read the stored keys");
    *rig.keys.refuse.lock().unwrap() = None;

    let deleted = rig.ask(json!({"cmd": "llm.key.delete", "provider": "openai"}), "d1");
    assert_eq!(entry(&deleted, "openai")["has_key"], false);
    assert!(rig.keys.keys.lock().unwrap().is_empty());
    // Deleting what is not there is not a failure.
    let again = rig.ask(json!({"cmd": "llm.key.delete", "provider": "openai"}), "d2");
    assert_eq!(again["type"], "llm.providers");

    // Nowhere but the key store: in no event, and in none of the library's files.
    let Rig {
        core, events, dir, ..
    } = rig;
    core.shutdown();
    for e in events.all() {
        assert!(!e.to_string().contains(KEY), "{e}");
    }
    assert!(!any_file_holds(dir.path(), KEY));
    events.assert_valid();
}

#[test]
fn a_cloud_provider_is_chosen_only_with_local_only_mode_turned_off_by_the_shell() {
    let rig = Rig::new("cloud-choose", &[]);
    let shared = rig.core.shared().clone();

    // Not said: refused, and nothing changes.
    let failed = rig.ask(json!({"cmd": "llm.choose", "provider": "openai"}), "c1");
    assert_eq!(failed["type"], "command.failed");
    assert!(
        failed["message"].as_str().unwrap().contains("local_only"),
        "{failed}"
    );
    assert!(shared.local_only.is_on());
    assert!(shared.llms.cloud().is_none());

    // Said: the choice and local-only mode off, in one write; each feature's state names the
    // provider, and none is allowed to send to it yet.
    let chosen = rig.ask(
        json!({"cmd": "llm.choose", "provider": "openai", "model": " gpt-synthetic ", "local_only": "off"}),
        "c2",
    );
    assert_eq!(chosen["chosen"], "openai");
    assert_eq!(chosen["model"], "gpt-synthetic");
    assert_eq!(chosen["endpoint"], OPENAI);
    assert_eq!(chosen["to"], "cloud");
    assert_eq!(chosen["local_only"], false);
    assert_eq!(chosen["ready"], false, "no key yet");
    assert!(!shared.local_only.is_on());
    let setting = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "setting.value" && v["key"] == "llm.local_only"
        })
        .unwrap();
    assert_eq!(setting["value"], "off");
    for feature in ["polish", "edit", "meetings"] {
        let state = rig
            .events
            .wait_for(WAIT, |v| {
                v["type"] == "consent.state" && v["feature"] == feature && v["to"] == "cloud"
            })
            .unwrap_or_else(|| panic!("no consent.state for {feature}"));
        assert_eq!(state["endpoint"], OPENAI);
        assert_eq!(state["name"], "gpt-synthetic (openai)");
        assert_eq!(state["allowed"], false);
    }
    assert_eq!(rig.save_key("openai", "k1")["ready"], true);

    // A built-in provider's address is fixed, so its key only ever goes to it.
    let failed = rig.ask(
        json!({"cmd": "llm.choose", "provider": "openai", "base_url": "https://example.com/v1", "local_only": "off"}),
        "c3",
    );
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .contains("cannot be changed")
    );
    let failed = rig.ask(
        json!({"cmd": "llm.choose", "provider": "openai", "model": "a\nb", "local_only": "off"}),
        "c4",
    );
    assert_eq!(failed["type"], "command.failed");
    assert_eq!(
        shared.llms.cloud().unwrap().info().model,
        "gpt-synthetic",
        "a refused choice changes nothing"
    );

    // A server on this machine keeps local-only mode on, and cannot be told otherwise.
    let local =
        json!({"cmd": "llm.choose", "provider": "custom", "base_url": "http://127.0.0.1:11434/v1"});
    let mut off = local.clone();
    off["local_only"] = "off".into();
    assert_eq!(rig.ask(off, "c5")["type"], "command.failed");
    let chosen = rig.ask(local, "c6");
    assert_eq!(chosen["chosen"], "custom");
    assert_eq!(chosen["base_url"], "http://127.0.0.1:11434/v1");
    assert_eq!(chosen["to"], "on_device");
    assert_eq!(chosen["local_only"], true);
    assert_eq!(chosen["ready"], true, "a custom server needs no key");
    assert!(shared.local_only.is_on());

    // A cloud server named as custom is a cloud provider like any other.
    let failed = rig.ask(
        json!({"cmd": "llm.choose", "provider": "custom", "base_url": "https://llm.example.com/v1"}),
        "c7",
    );
    assert_eq!(failed["type"], "command.failed");

    // None: local-only mode on again.
    rig.choose_openai("c8");
    assert!(!shared.local_only.is_on());
    assert_eq!(
        rig.ask(
            json!({"cmd": "llm.choose", "provider": "none", "local_only": "off"}),
            "c9"
        )["type"],
        "command.failed"
    );
    let none = rig.ask(json!({"cmd": "llm.choose", "provider": "none"}), "c10");
    assert!(none.get("chosen").is_none());
    assert_eq!(none["local_only"], true);
    assert!(shared.local_only.is_on());
    assert!(shared.llms.cloud().is_none());
    drop(shared);

    // The choice outlives a restart, and its key stays in the key store.
    rig.choose_openai("c11");
    let rig = rig.restart();
    let listed = rig.ask(json!({"cmd": "llm.providers"}), "p1");
    assert_eq!(listed["chosen"], "openai");
    assert_eq!(listed["local_only"], false);
    assert_eq!(listed["ready"], true);
    let state = rig.ask(json!({"cmd": "consent.get", "feature": "polish"}), "g1");
    assert_eq!(state["to"], "cloud");
    assert_eq!(state["endpoint"], OPENAI);
    rig.core.shutdown();
    rig.events.assert_valid();
}

/// Summaries and Ask are sized for the chosen provider's context, not the on-device model's: one
/// pass and the whole meeting for a cloud model, the small default for a custom server.
#[test]
fn summaries_and_ask_are_sized_for_the_chosen_providers_context() {
    use ink_ffi::engines::{DEFAULT_CONTEXT_TOKENS, ask_options, context_tokens, summary_options};
    use ink_llm::tasks::ask::AskOptions;
    use ink_llm::tasks::summary::SummaryOptions;

    let rig = Rig::new("cloud-context", &[]);
    let shared = rig.core.shared().clone();
    assert_eq!(context_tokens(&shared), DEFAULT_CONTEXT_TOKENS);

    rig.choose_openai("c1");
    assert_eq!(context_tokens(&shared), 128_000);
    assert_eq!(ask_options(&shared), AskOptions::default());
    assert_eq!(summary_options(&shared), SummaryOptions::default());
    rig.ask(
        json!({"cmd": "llm.choose", "provider": "anthropic", "local_only": "off"}),
        "c2",
    );
    assert_eq!(context_tokens(&shared), 200_000);

    rig.ask(
        json!({"cmd": "llm.choose", "provider": "custom", "base_url": "http://127.0.0.1:11434/v1"}),
        "c3",
    );
    assert_eq!(context_tokens(&shared), DEFAULT_CONTEXT_TOKENS);
    drop(shared);
    rig.core.shutdown();
    rig.events.assert_valid();
}

fn request(user: &str) -> LlmRequest {
    LlmRequest {
        system: "Synthetic instructions.".into(),
        user: user.into(),
        max_tokens: 64,
        temperature: 0.0,
        json_schema: None,
    }
}

#[test]
fn nothing_is_sent_without_the_features_consent_for_that_endpoint() {
    let rig = Rig::new("cloud-consent", &[]);
    rig.save_key("openai", "k1");
    rig.save_key("anthropic", "k2");
    rig.choose_openai("c1");
    let shared = rig.core.shared().clone();
    let store = shared.store.clone();
    let llm = ink_ffi::engines::llm(&shared).expect("the chosen provider");
    assert_eq!(llm.info().endpoint, Endpoint::Remote(OPENAI.into()));
    let words = request("synthetic meeting words");
    let send = |consent: Option<&LlmConsent>| {
        Consented {
            inner: llm.as_ref(),
            consent,
        }
        .complete(&words, &CancelToken::new())
    };
    let not_allowed = |r: Result<_, LlmError>| matches!(r, Err(LlmError::NotAllowed { .. }));

    // Never agreed, agreed for this machine, or agreed for another provider: nothing is sent, and
    // the key is not even read.
    assert!(not_allowed(send(None)));
    assert!(not_allowed(send(Some(&LlmConsent::OnDevice))));
    assert!(not_allowed(send(Some(&LlmConsent::Cloud {
        endpoint: "https://api.anthropic.com".into(),
        name: "claude (anthropic)".into(),
    }))));
    assert_eq!(rig.net.calls(), 0);
    assert_eq!(rig.keys.reads.load(Ordering::SeqCst), 0);

    // The user agrees for summaries and Ask, with this endpoint: they send, and only they do.
    let allowed = rig.ask(
        json!({"cmd": "consent.allow", "feature": "meetings", "to": "cloud", "endpoint": OPENAI}),
        "a1",
    );
    assert_eq!(allowed["allowed"], true, "{allowed}");
    let meetings = consent::stored(store.as_ref(), Feature::Meetings);
    assert_eq!(send(meetings.as_ref()).map(|r| r.text), Ok("OK".to_owned()));
    assert_eq!(rig.net.calls(), 1);
    let seen = rig.net.seen.lock().unwrap()[0].clone();
    assert_eq!(seen.url, format!("{OPENAI}/chat/completions"));
    assert_eq!(
        seen.header("Authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert!(seen.body.contains("synthetic meeting words"));
    assert!(!seen.loopback_only);
    for feature in [Feature::Polish, Feature::Edit] {
        let other = consent::stored(store.as_ref(), feature);
        assert!(not_allowed(send(other.as_ref())), "{feature:?}");
    }
    assert_eq!(rig.net.calls(), 1);

    // Another provider chosen: the consent was for OpenAI's endpoint, so it is paused.
    rig.ask(
        json!({"cmd": "llm.choose", "provider": "anthropic", "local_only": "off"}),
        "c2",
    );
    let anthropic = ink_ffi::engines::llm(&shared).unwrap();
    assert_eq!(
        anthropic.info().endpoint,
        Endpoint::Remote("https://api.anthropic.com".into())
    );
    let paused = Consented {
        inner: anthropic.as_ref(),
        consent: meetings.as_ref(),
    }
    .complete(&words, &CancelToken::new());
    assert!(not_allowed(paused));
    let state = rig.ask(json!({"cmd": "consent.get", "feature": "meetings"}), "g1");
    assert_eq!(state["on"], true);
    assert_eq!(
        state["allowed"], false,
        "paused until the user agrees again"
    );
    assert_eq!(rig.net.calls(), 1);

    // Back to OpenAI, but local-only mode turned on again by the user: even with consent,
    // nothing is sent.
    rig.choose_openai("c3");
    assert!(send(meetings.as_ref()).is_ok());
    assert_eq!(rig.net.calls(), 2);
    rig.set_local_only("on");
    assert!(matches!(
        send(meetings.as_ref()),
        Err(LlmError::LocalOnly { .. })
    ));
    assert_eq!(rig.net.calls(), 2);
    drop((llm, anthropic, shared));
    rig.core.shutdown();
    rig.events.assert_valid();
}

const BLOCK: usize = 160;
const BLOCK_NS: u64 = 10_000_000;

/// A take: room, a press, speech, a release, quiet.
fn take(core: &Core, inbox: &DictationInbox, seed: u64) {
    let mut t = core.shared().clock.now_ns();
    let hotkey = inbox.hotkey_sink();
    let audio = |samples: &[f32], t: &mut u64| {
        for block in samples.chunks(BLOCK) {
            inbox.push_audio(Block {
                samples: block.to_vec(),
                host_time_ns: *t,
                dropped_frames: 0,
            });
            *t += BLOCK_NS;
        }
    };
    audio(&[0.0; 8_000], &mut t);
    hotkey(HotkeyEvent::Pressed { at_ns: t });
    audio(&ink_audio::synth::speech_like(1.5, -30.0, seed), &mut t);
    hotkey(HotkeyEvent::Released { at_ns: t });
    audio(&[0.0; 16_000], &mut t);
}

/// Dictation polish, through the chain, goes to the chosen provider only with polish's consent for
/// its endpoint; without it the take goes in as said, and nothing is sent.
#[test]
fn dictation_polish_goes_to_the_chosen_provider_only_with_its_consent() {
    let rig = Rig::new("cloud-polish", &[test_row(ROW_ID)]);
    rig.save_key("openai", "k1");
    rig.choose_openai("c1");
    rig.net
        .set(Ok((200, openai_answer("Polished synthetic words."))));
    let dictate = |consent: Option<LlmConsent>| {
        let platform = Arc::new(MockPlatform::new());
        let mut settings = DictationSettings::default();
        settings.modes.modes[0].polish_enabled = true;
        settings.polish_consent = consent;
        let inbox = rig
            .core
            .start_dictation(DictationParts {
                inserter: platform.clone(),
                focus: platform.clone(),
                llm: None,
                settings,
                vad: Vad::Unavailable(VadUnavailable::ModelMissing),
            })
            .unwrap();
        (platform, inbox)
    };

    let (platform, inbox) = dictate(None);
    take(&rig.core, &inbox, 1);
    assert!(
        rig.events
            .wait_count("dictation.inserted", 1, Duration::from_secs(20))
    );
    assert!(
        rig.events
            .wait_for(WAIT, |v| {
                v["type"] == "dictation.warning" && v["kind"] == "polish_not_allowed"
            })
            .is_some()
    );
    assert!(!platform.inserted().last().unwrap().contains("Polished"));
    assert_eq!(rig.net.calls(), 0);

    let (platform, inbox) = dictate(Some(LlmConsent::Cloud {
        endpoint: OPENAI.into(),
        name: "gpt-4o-mini (openai)".into(),
    }));
    take(&rig.core, &inbox, 2);
    assert!(
        rig.events
            .wait_count("dictation.inserted", 2, Duration::from_secs(20))
    );
    assert_eq!(
        platform.inserted().last().map(|s| s.trim().to_owned()),
        Some("Polished synthetic words.".into())
    );
    assert_eq!(rig.net.calls(), 1);
    rig.core.shutdown();
    rig.events.assert_valid();
}

/// A provider that takes the request and never answers costs a take its polish budget, not the
/// client's read timeout: the budget reaches the request on the wire, the take goes in as said
/// with `polish_timed_out` (not `polish_failed`), and the next take is processed as usual.
#[test]
fn an_own_key_provider_that_never_answers_costs_a_take_its_polish_budget() {
    let rig = Rig::new("cloud-stall", &[test_row(ROW_ID)]);
    rig.save_key("openai", "k1");
    rig.choose_openai("c1");
    let platform = Arc::new(MockPlatform::new());
    let mut settings = DictationSettings::default();
    settings.modes.modes[0].polish_enabled = true;
    settings.polish_consent = Some(LlmConsent::Cloud {
        endpoint: OPENAI.into(),
        name: "gpt-4o-mini (openai)".into(),
    });
    settings.polish_budget = Duration::from_millis(300);
    let inbox = rig
        .core
        .start_dictation(DictationParts {
            inserter: platform.clone(),
            focus: platform.clone(),
            llm: None,
            settings,
            vad: Vad::Unavailable(VadUnavailable::ModelMissing),
        })
        .unwrap();

    rig.net.stall.store(true, Ordering::SeqCst);
    let started = std::time::Instant::now();
    take(&rig.core, &inbox, 1);
    assert!(
        rig.events
            .wait_count("dictation.inserted", 1, Duration::from_secs(20))
    );
    assert!(
        started.elapsed() < WAIT,
        "the take waited on the provider past its budget"
    );
    assert!(rig.net.seen.lock().unwrap()[0].deadline.is_some());
    assert!(!platform.inserted().last().unwrap().contains("Polished"));
    let warnings: Vec<_> = rig
        .events
        .all()
        .into_iter()
        .filter(|v| v["type"] == "dictation.warning")
        .collect();
    assert!(
        warnings.iter().any(|v| v["kind"] == "polish_timed_out"),
        "{warnings:?}"
    );
    assert!(
        !warnings.iter().any(|v| v["kind"] == "polish_failed"),
        "{warnings:?}"
    );

    rig.net.stall.store(false, Ordering::SeqCst);
    rig.net
        .set(Ok((200, openai_answer("Polished synthetic words."))));
    take(&rig.core, &inbox, 2);
    assert!(
        rig.events
            .wait_count("dictation.inserted", 2, Duration::from_secs(20))
    );
    assert_eq!(
        platform.inserted().last().map(|s| s.trim().to_owned()),
        Some("Polished synthetic words.".into())
    );
    rig.core.shutdown();
    rig.events.assert_valid();
}

#[test]
fn the_test_sends_one_fixed_request_and_says_how_it_went() {
    let rig = Rig::new("cloud-test", &[]);
    let failed = rig.ask(json!({"cmd": "llm.test"}), "t1");
    assert_eq!(failed["type"], "command.failed");
    assert_eq!(failed["message"], "no provider is chosen");

    rig.save_key("openai", "k1");
    rig.choose_openai("c1");
    let ok = rig.ask(json!({"cmd": "llm.test"}), "t2");
    assert_eq!(ok["type"], "llm.tested");
    assert_eq!(ok["ok"], true);
    assert_eq!(ok["provider"], "openai");
    assert_eq!(ok["model"], "gpt-4o-mini");
    assert!(ok.get("error").is_none());
    let seen = rig.net.seen.lock().unwrap()[0].clone();
    assert_eq!(seen.url, format!("{OPENAI}/chat/completions"));
    assert_eq!(
        seen.header("Authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert!(seen.body.contains("Is this connection working?"));

    rig.net.set(Ok((401, String::new())));
    let refused = rig.ask(json!({"cmd": "llm.test"}), "t3");
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["status"], 401);
    assert_eq!(
        refused["error"],
        "couldn't get an answer: the provider refused the key"
    );
    rig.net.set(Err(TransportError::Connect));
    let unreachable = rig.ask(json!({"cmd": "llm.test"}), "t4");
    assert!(unreachable.get("status").is_none());
    assert!(
        unreachable["error"]
            .as_str()
            .unwrap()
            .starts_with("couldn't reach the provider")
    );

    // A custom server over plain http on another machine never gets the key, so its refusal is
    // not blamed on the key.
    rig.net.set(Ok((401, String::new())));
    rig.ask(
        json!({"cmd": "llm.key.save", "provider": "custom", "key": KEY}),
        "k2",
    );
    rig.ask(
        json!({"cmd": "llm.choose", "provider": "custom", "base_url": "http://192.0.2.10:8000/v1", "local_only": "off"}),
        "c2",
    );
    let withheld = rig.ask(json!({"cmd": "llm.test"}), "t4b");
    assert_eq!(withheld["status"], 401);
    assert_eq!(
        withheld["error"],
        "couldn't get an answer: the server asks for a key, and keys are sent only over https or \
         to this computer"
    );
    let seen = rig.net.seen.lock().unwrap().last().unwrap().clone();
    assert_eq!(seen.url, "http://192.0.2.10:8000/v1/chat/completions");
    assert_eq!(seen.header("Authorization"), None);
    rig.ask(json!({"cmd": "llm.key.delete", "provider": "custom"}), "d0");
    rig.choose_openai("c3");

    // No key: said, and nothing sent.
    rig.ask(json!({"cmd": "llm.key.delete", "provider": "openai"}), "d1");
    let calls = rig.net.calls();
    let keyless = rig.ask(json!({"cmd": "llm.test"}), "t5");
    assert_eq!(
        keyless["error"],
        "couldn't test it: no key is stored for it"
    );
    assert_eq!(rig.net.calls(), calls);

    // Local-only mode on: nothing sent.
    rig.save_key("openai", "k2");
    rig.set_local_only("on");
    let local = rig.ask(json!({"cmd": "llm.test"}), "t6");
    assert_eq!(local["ok"], false);
    assert!(local["error"].as_str().unwrap().contains("local-only"));
    assert_eq!(rig.net.calls(), calls);

    // One at a time: another asked while one is on the wire is refused, not queued.
    rig.set_local_only("off");
    rig.net.set(Ok((200, openai_answer("OK"))));
    let gate = Arc::new(Gate::default());
    *rig.net.gate.lock().unwrap() = Some(gate.clone());
    rig.core
        .command(&json!({"cmd": "llm.test", "id": "t7"}).to_string())
        .unwrap();
    assert!(gate.until_waiting(WAIT));
    let busy = rig.ask(json!({"cmd": "llm.test"}), "t8");
    assert_eq!(busy["type"], "command.failed");
    assert!(
        busy["message"]
            .as_str()
            .unwrap()
            .contains("already running")
    );
    gate.open();
    let first = rig
        .events
        .wait_for(WAIT, |v| v["type"] == "llm.tested" && v["ref"] == "t7")
        .unwrap();
    assert_eq!(first["ok"], true);
    let again = rig.ask(json!({"cmd": "llm.test"}), "t9");
    assert_eq!(again["ok"], true, "free again once answered");
    rig.core.shutdown();
    rig.events.assert_valid();
}

/// A test in flight when the core stops is joined before shutdown returns, and nothing arrives
/// after `core.stopped`.
#[test]
fn a_test_in_flight_is_finished_before_shutdown_returns() {
    let rig = Rig::new("cloud-shutdown", &[]);
    rig.save_key("openai", "k1");
    rig.choose_openai("c1");
    let gate = Arc::new(Gate::default());
    *rig.net.gate.lock().unwrap() = Some(gate.clone());
    rig.core
        .command(&json!({"cmd": "llm.test", "id": "t1"}).to_string())
        .unwrap();
    assert!(gate.until_waiting(WAIT));
    let opener = {
        let gate = gate.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            gate.open();
        })
    };
    rig.core.shutdown();
    opener.join().unwrap();
    let types = rig.events.types();
    assert_eq!(types.last().map(String::as_str), Some("core.stopped"));
    rig.events.assert_valid();
}
