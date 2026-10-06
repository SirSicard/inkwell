//! Settings > AI with the core's own model on this machine (`on_device`): what the features use
//! between it, an own-key provider and a model the shell registered; `llm.choose on_device`;
//! its `llm.providers` entry; `llm.test` on it. The model is a mock and the network a fake.

mod cloud_support;
mod common;

use std::ffi::{CString, c_char, c_void};

use cloud_support::{Rig, WAIT, entry};
use common::*;
use ink_engines::{EngineRow, LanguageSize};
use serde_json::{Value, json};

fn chat_row() -> EngineRow {
    language_row("test-chat", LanguageSize::Default, "Test Chat")
}

fn local_rig(label: &str) -> Rig {
    Rig::with_local(label, &[chat_row()], MockLocalLoader::new("OK"))
}

fn setting_model(rig: &Rig, id: &str) -> Value {
    let listed = rig.ask(json!({"cmd": "modes.list"}), id);
    listed
        .get("setting_polish_model")
        .cloned()
        .unwrap_or(Value::Null)
}

fn stored(rig: &Rig, key: &str) -> Option<String> {
    rig.core.shared().store.setting(key).unwrap()
}

unsafe extern "C" fn never_generate(_: *mut c_void, _: u64, _: *const c_char) {
    unreachable!("never called in these tests");
}

unsafe extern "C" fn no_release(_: *mut c_void) {}

/// Registers a language model on this machine as a shell would (the Mac's Apple model).
fn register_shell_model(rig: &Rig) {
    use ink_ffi::external::{InkEngineVTable, KIND_LLM, Registration};
    let info =
        CString::new(r#"{"id":"shell-llm","licence":"MIT","model":"shell","local":true}"#).unwrap();
    let table = InkEngineVTable {
        kind: KIND_LLM,
        info_json: info.as_ptr(),
        release: Some(no_release),
        generate: Some(never_generate),
        ..Default::default()
    };
    // SAFETY: a valid table with no context; nothing in these tests calls it.
    let registration =
        unsafe { Registration::from_table(&table, rig.core.shared().shutdown.clone()) }.unwrap();
    rig.core.register(registration).unwrap();
}

#[test]
fn a_machine_with_no_language_models_of_its_own_lists_no_on_device_entry() {
    // The Mac: no language row, so llm.providers is as it was.
    let rig = Rig::new("on-device-none", &[]);
    let listed = rig.ask(json!({"cmd": "llm.providers"}), "p1");
    let ids: Vec<&str> = listed["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["openai", "groq", "anthropic", "openrouter", "custom"]);
    let refused = rig.ask(json!({"cmd": "llm.choose", "provider": "on_device"}), "c1");
    assert_eq!(refused["type"], "command.failed", "{refused}");
    rig.events.assert_valid();
}

#[test]
fn the_downloaded_model_is_used_while_no_provider_is_chosen_and_a_chosen_one_comes_first() {
    let rig = local_rig("on-device-order");
    // A model the shell registered too: the core's own still comes first.
    register_shell_model(&rig);
    assert_eq!(setting_model(&rig, "m1"), "engine:local");
    let listed = rig.ask(json!({"cmd": "llm.providers"}), "p1");
    assert!(listed.get("chosen").is_none(), "{listed}");
    assert_eq!(
        entry(&listed, "on_device"),
        &json!({
            "id": "on_device",
            "default_model": "test-chat",
            "endpoint": "this process",
            "custom_url": false,
            "needs_key": false,
            "has_key": false,
            "installed": true,
        })
    );

    rig.save_key("openai", "k1");
    rig.choose_openai("c1");
    assert_eq!(setting_model(&rig, "m2"), "provider:openai");
    let modes = rig.ask(json!({"cmd": "modes.list"}), "m3");
    let ids: Vec<&str> = modes["polish_models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["provider:openai", "engine:local", "engine:shell-llm"]);

    let none = rig.ask(json!({"cmd": "llm.choose", "provider": "none"}), "c2");
    assert_eq!(none["local_only"], true);
    assert_eq!(setting_model(&rig, "m4"), "engine:local");
    rig.events.assert_valid();
}

#[test]
fn choosing_this_machines_model_keeps_local_only_on_and_lasts() {
    let rig = local_rig("on-device-choose");
    rig.save_key("openai", "k1");
    rig.choose_openai("c1");
    assert!(!rig.core.shared().local_only.is_on());

    // What it refuses changes nothing.
    for (i, bad) in [
        json!({"cmd": "llm.choose", "provider": "on_device", "local_only": "off"}),
        json!({"cmd": "llm.choose", "provider": "on_device", "base_url": "http://127.0.0.1:1/v1"}),
        json!({"cmd": "llm.choose", "provider": "on_device", "model": "another-model"}),
    ]
    .into_iter()
    .enumerate()
    {
        let failed = rig.ask(bad, &format!("bad{i}"));
        assert_eq!(failed["type"], "command.failed", "{failed}");
    }
    assert_eq!(setting_model(&rig, "m0"), "provider:openai");

    let chosen = rig.ask(
        json!({"cmd": "llm.choose", "provider": "on_device", "model": "test-chat"}),
        "c2",
    );
    assert_eq!(chosen["type"], "llm.providers", "{chosen}");
    assert_eq!(chosen["chosen"], "on_device");
    assert_eq!(chosen["model"], "test-chat");
    assert_eq!(chosen["to"], "on_device");
    assert_eq!(chosen["endpoint"], "this process");
    assert_eq!(
        (&chosen["local_only"], &chosen["ready"]),
        (&json!(true), &json!(true))
    );
    assert!(chosen.get("base_url").is_none());
    assert!(rig.core.shared().local_only.is_on(), "local-only back on");
    let said = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "setting.value" && v["key"] == "llm.local_only" && v["value"] == "on"
        })
        .expect("local-only said");
    assert!(said.get("ref").is_none());
    assert_eq!(
        stored(&rig, "llm.cloud").as_deref(),
        Some(r#"{"provider":"on_device"}"#)
    );
    assert_eq!(setting_model(&rig, "m1"), "engine:local");

    let rig = rig.restart_with(&[chat_row()]);
    let listed = rig.ask(json!({"cmd": "llm.providers"}), "p1");
    assert_eq!(listed["chosen"], "on_device", "{listed}");
    assert_eq!(setting_model(&rig, "m2"), "engine:local");
    rig.events.assert_valid();
}

#[test]
fn moving_from_a_provider_to_this_machine_needs_the_on_device_consent() {
    let rig = local_rig("on-device-consent");
    rig.save_key("openai", "k1");
    rig.choose_openai("c1");
    let cloud = rig.ask(
        json!({"cmd": "consent.allow", "feature": "polish", "to": "cloud",
               "endpoint": "https://api.openai.com/v1"}),
        "a1",
    );
    assert_eq!(cloud["allowed"], true, "{cloud}");

    rig.ask(json!({"cmd": "llm.choose", "provider": "on_device"}), "c2");
    let state = rig.ask(json!({"cmd": "consent.get", "feature": "polish"}), "g1");
    assert_eq!(state["to"], "on_device");
    assert_eq!(state["name"], "Test Chat");
    assert_eq!(
        state["allowed"], false,
        "the cloud consent does not cover it: {state}"
    );

    let allowed = rig.ask(
        json!({"cmd": "consent.allow", "feature": "polish", "to": "on_device"}),
        "a2",
    );
    assert_eq!(allowed["allowed"], true, "{allowed}");
    rig.events.assert_valid();
}

#[test]
fn with_this_machines_model_chosen_and_removed_nothing_stands_in() {
    let rig = local_rig("on-device-removed");
    register_shell_model(&rig);
    rig.ask(json!({"cmd": "llm.choose", "provider": "on_device"}), "c1");
    let row = chat_row();
    ink_ffi::local::remove(rig.core.shared(), &row).unwrap();

    // Neither the shell's model nor anything else: the features have none.
    assert_eq!(setting_model(&rig, "m1"), Value::Null);
    assert!(rig.core.shared().llms.pick().is_none());
    let listed = rig.ask(json!({"cmd": "llm.providers"}), "p1");
    assert_eq!(listed["chosen"], "on_device");
    assert!(listed.get("model").is_none(), "{listed}");
    assert_eq!(listed["ready"], false);
    assert_eq!(entry(&listed, "on_device")["installed"], false);
    assert_eq!(
        entry(&listed, "on_device")["default_model"],
        "test-chat",
        "the size it would download"
    );
    let failed = rig.ask(json!({"cmd": "llm.test"}), "t1");
    assert_eq!(failed["type"], "command.failed");
    assert_eq!(
        failed["message"],
        "no language model is downloaded on this computer"
    );
    let again = rig.ask(json!({"cmd": "llm.choose", "provider": "on_device"}), "c2");
    assert_eq!(again["type"], "command.failed", "{again}");

    // Choosing none goes back to the shell's model.
    rig.ask(json!({"cmd": "llm.choose", "provider": "none"}), "c3");
    assert_eq!(setting_model(&rig, "m2"), "engine:shell-llm");
    rig.events.assert_valid();
}

#[test]
fn the_test_loads_this_machines_model_and_times_the_load_and_the_answer() {
    let rig = local_rig("on-device-test");
    let local = rig.local.clone().unwrap();
    let tested = rig.ask(json!({"cmd": "llm.test"}), "t1");
    assert_eq!(tested["type"], "llm.tested", "{tested}");
    assert_eq!(tested["provider"], "on_device");
    assert_eq!(tested["model"], "test-chat");
    assert_eq!(tested["ok"], true);
    assert!(tested["load_ms"].is_u64(), "{tested}");
    assert!(tested["answer_ms"].is_u64(), "{tested}");
    assert_eq!((local.loads(), local.calls()), (1, 1));
    let again = rig.ask(json!({"cmd": "llm.test"}), "t2");
    assert_eq!(again["ok"], true);
    assert_eq!(local.loads(), 1, "loaded once");
    assert_eq!(rig.net.calls(), 0, "nothing on the network");

    // A chosen provider is what is tested then: timed too, with no load.
    rig.save_key("openai", "k1");
    rig.choose_openai("c1");
    let cloud = rig.ask(json!({"cmd": "llm.test"}), "t3");
    assert_eq!(cloud["provider"], "openai");
    assert!(cloud["answer_ms"].is_u64(), "{cloud}");
    assert!(cloud.get("load_ms").is_none(), "{cloud}");
    assert_eq!(local.calls(), 2, "the local model is not asked");
    rig.events.assert_valid();
}
