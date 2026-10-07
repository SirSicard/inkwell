//! The core's own language model on this machine (Windows' Qwen3; `ink_ffi::local`), through the
//! core: once a language row is installed it is `engine:local`, the AI setting's model while no
//! provider is chosen, behind local-only mode and the on-device consent like every model; one
//! installed at a time; unloaded at shutdown. The model is a mock: CI has no weights.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use ink_core::{CancelToken, EventSink, LlmRequest};
use ink_engines::{DownloadError, DownloadProgress, EngineRow, ModelDir, Registry};
use ink_ffi::runtime::{Core, Parts};
use ink_pipeline::update::ModelInstaller;
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(10);

fn default_row() -> EngineRow {
    language_row("test-chat", "Test Chat")
}

fn small_row() -> EngineRow {
    language_row("test-chat-small", "Test Chat Small")
}

/// Installs as the downloader would: every file at its place, and the marker.
struct Installs(ModelDir);

impl ModelInstaller for Installs {
    fn install(
        &self,
        row: &EngineRow,
        _: &CancelToken,
        _: EventSink<DownloadProgress>,
    ) -> Result<(), DownloadError> {
        install(&self.0, row);
        Ok(())
    }
}

struct Rig {
    core: Option<Core>,
    events: Arc<Recorder>,
    local: Arc<MockLocalLoader>,
    store: Arc<ink_store::SqliteStore>,
    models: ModelDir,
    _dir: TempDir,
}

impl Rig {
    /// A core whose registry lists the test speech row and both language rows, with `installed`
    /// installed and `settings` stored before it starts.
    fn new(label: &str, installed: &[EngineRow], settings: &[(&str, &str)]) -> Self {
        let dir = TempDir::new(label);
        let models = ModelDir::new(dir.path().join("models"));
        let speech = test_row(ROW_ID);
        install(&models, &speech);
        for row in installed {
            install(&models, row);
        }
        let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
        for (k, v) in settings {
            ink_core::Store::set_setting(store.as_ref(), k, v).unwrap();
        }
        let local = MockLocalLoader::new("Hello, world!");
        let loader = MockLoader::new(Behaviour::Say("hello world".into()));
        let (core, events) = start_parts(Parts {
            store: store.clone(),
            clock: clock(),
            registry: Registry::new(vec![speech, default_row(), small_row()]).unwrap(),
            models: models.clone(),
            loader,
            installer: Arc::new(Installs(models.clone())),
            data_dir: dir.path().to_owned(),
            permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
            local: local.parts(None),
            meetings: Default::default(),
        });
        Self {
            core: Some(core),
            events,
            local,
            store,
            models,
            _dir: dir,
        }
    }

    fn core(&self) -> &Core {
        self.core.as_ref().unwrap()
    }

    fn ask(&self, mut cmd: Value, id: &str) -> Value {
        cmd["id"] = id.into();
        self.core().command(&cmd.to_string()).unwrap();
        self.events
            .wait_for(WAIT, |v| {
                v["ref"] == id || (v["type"] == "command.failed" && v["id"] == id)
            })
            .unwrap_or_else(|| panic!("no answer to {id}: {:?}", self.events.types()))
    }

    fn modes(&self, id: &str) -> Value {
        self.ask(json!({"cmd": "modes.list"}), id)
    }

    fn update(&self, row: &str) -> Value {
        let n = self.events.count("model.update_finished");
        self.core()
            .command(&json!({"cmd": "model.update", "model": row, "next": row}).to_string())
            .unwrap();
        assert!(self.events.wait_count("model.update_finished", n + 1, WAIT));
        self.events
            .all()
            .into_iter()
            .filter(|e| e["type"] == "model.update_finished")
            .nth(n)
            .unwrap()
    }

    fn setting(&self, key: &str) -> Option<String> {
        ink_core::Store::setting(self.store.as_ref(), key).unwrap()
    }

    fn finish(mut self) -> ink_ffi::runtime::Stopped {
        let stopped = self.core.take().unwrap().shutdown();
        self.events.assert_valid();
        stopped
    }
}

fn request() -> LlmRequest {
    LlmRequest {
        system: "Reply OK.".into(),
        user: "synthetic".into(),
        max_tokens: 8,
        temperature: 0.0,
        json_schema: None,
    }
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let until = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < until, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn an_installed_language_model_is_engine_local_and_the_ai_settings_model() {
    let rig = Rig::new("local-listed", &[default_row()], &[]);
    let listed = rig.modes("l1");
    assert_eq!(listed["setting_polish_model"], "engine:local", "{listed}");
    let models = listed["polish_models"].as_array().unwrap();
    assert_eq!(models.len(), 1, "{listed}");
    assert_eq!(models[0]["id"], "engine:local");
    assert_eq!(models[0]["name"], "Test Chat");
    assert_eq!(models[0]["to"], "on_device");
    assert_eq!(
        models[0]["blocked_local_only"], false,
        "local-only lets it through"
    );
    assert_eq!(models[0]["allowed"], false, "no consent yet");

    let state = rig.ask(json!({"cmd": "consent.get", "feature": "polish"}), "c1");
    assert_eq!(state["to"], "on_device");
    assert_eq!(state["name"], "Test Chat");
    assert!(state.get("endpoint").is_none());
    assert_eq!(
        ink_ffi::engines::context_tokens(rig.core().shared()),
        ink_ffi::local::LOCAL_CONTEXT_TOKENS
    );
    assert_eq!(ink_ffi::local::LOCAL_CONTEXT_TOKENS, 8_192);
    assert_eq!(rig.local.loads(), 0, "listing never loads it");
    rig.finish();
}

#[test]
fn with_no_language_model_installed_nothing_changes() {
    // The Mac's case: no language row installed (on the Mac, none exists), so a model the shell
    // registers stays the AI setting's, as before.
    let rig = Rig::new("local-none", &[], &[]);
    let listed = rig.modes("l1");
    assert!(listed.get("setting_polish_model").is_none(), "{listed}");
    assert!(listed["polish_models"].as_array().unwrap().is_empty());
    assert!(rig.core().shared().llms.local().is_none());
    assert_eq!(
        ink_ffi::engines::context_tokens(rig.core().shared()),
        ink_ffi::engines::DEFAULT_CONTEXT_TOKENS
    );
    rig.finish();
}

unsafe extern "C" fn never_generate(_: *mut std::ffi::c_void, _: u64, _: *const std::ffi::c_char) {
    unreachable!("a refused registration is never called");
}

unsafe extern "C" fn no_release(_: *mut std::ffi::c_void) {}

#[test]
fn a_shell_cannot_register_a_language_model_as_local() {
    use ink_ffi::external::{InkEngineVTable, KIND_LLM, Registration};
    let rig = Rig::new("local-register", &[], &[]);
    let info = std::ffi::CString::new(r#"{"id":"local","licence":"MIT","model":"x","local":true}"#)
        .unwrap();
    let table = InkEngineVTable {
        kind: KIND_LLM,
        info_json: info.as_ptr(),
        release: Some(no_release),
        generate: Some(never_generate),
        ..Default::default()
    };
    // SAFETY: a valid table with no context; a refused registration calls nothing in it.
    let registration =
        unsafe { Registration::from_table(&table, rig.core().shared().shutdown.clone()) }.unwrap();
    let refused = rig.core().register(registration).unwrap_err();
    assert!(refused.contains("core's own"), "{refused}");
    rig.finish();
}

#[test]
fn a_call_loads_it_once_through_local_only_and_shutdown_unloads_it() {
    let rig = Rig::new("local-call", &[default_row()], &[]);
    let shared = rig.core().shared().clone();
    assert!(shared.local_only.is_on(), "local-only is on by default");
    let llm = ink_ffi::engines::llm(&shared).expect("the local model");
    for _ in 0..2 {
        let answer = llm.complete(&request(), &CancelToken::new()).unwrap();
        assert_eq!(answer.text, "Hello, world!");
    }
    assert_eq!((rig.local.loads(), rig.local.calls()), (1, 2));
    assert_eq!(shared.local.resident(), ["test-chat".to_owned()]);
    drop((llm, shared));
    let journal = rig.local.journal.clone();
    let stopped = rig.finish();
    assert_eq!(
        stopped.models_unloaded, 1,
        "the language model is unloaded at shutdown"
    );
    assert_eq!(journal.drops.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn installing_a_language_model_makes_it_the_one_in_use() {
    let rig = Rig::new("local-install", &[], &[]);
    assert!(
        rig.modes("l0")["polish_models"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let finished = rig.update("test-chat");
    assert_eq!(finished["ok"], true, "{finished}");
    let listed = rig.modes("l1");
    assert_eq!(listed["setting_polish_model"], "engine:local");
    assert_eq!(listed["polish_models"][0]["name"], "Test Chat");
    assert_eq!(
        rig.setting(ink_ffi::local::CURRENT_KEY).as_deref(),
        Some("test-chat")
    );
    rig.finish();
}

#[test]
fn installing_the_other_size_replaces_it_once_it_has_installed() {
    let rig = Rig::new(
        "local-replace",
        &[default_row()],
        &[(ink_ffi::local::CURRENT_KEY, "test-chat")],
    );
    // In use, and loaded, before the other size is installed.
    let llm = ink_ffi::engines::llm(rig.core().shared()).unwrap();
    llm.complete(&request(), &CancelToken::new()).unwrap();
    drop(llm);
    assert_eq!(rig.local.loads(), 1);

    let finished = rig.update("test-chat-small");
    assert_eq!(finished["ok"], true, "{finished}");
    assert!(rig.models.is_installed(&small_row()));
    assert!(!rig.models.is_installed(&default_row()), "replaced");
    assert!(
        !rig.models.root().join("test-chat").exists(),
        "its files are gone"
    );
    assert!(
        rig.core().shared().local.resident().is_empty(),
        "unloaded first"
    );
    let listed = rig.modes("l1");
    assert_eq!(
        listed["setting_polish_model"], "engine:local",
        "the same alias"
    );
    assert_eq!(listed["polish_models"][0]["name"], "Test Chat Small");
    assert_eq!(
        rig.setting(ink_ffi::local::CURRENT_KEY).as_deref(),
        Some("test-chat-small")
    );
    rig.finish();
}

#[test]
fn a_size_a_call_held_during_its_replacement_is_deleted_at_the_next_launch() {
    // What a replacement could not delete (a call held it then) is left installed, with the new
    // one named in use: the launch deletes it before anything can load it.
    let rig = Rig::new(
        "local-tidy",
        &[default_row(), small_row()],
        &[(ink_ffi::local::CURRENT_KEY, "test-chat-small")],
    );
    assert!(!rig.models.is_installed(&default_row()));
    assert!(rig.models.is_installed(&small_row()));
    assert_eq!(
        rig.modes("l1")["polish_models"][0]["name"],
        "Test Chat Small"
    );
    rig.finish();
}

#[test]
fn without_a_record_of_which_is_in_use_the_launch_deletes_neither() {
    let rig = Rig::new("local-tidy-unknown", &[default_row(), small_row()], &[]);
    assert!(rig.models.is_installed(&default_row()));
    assert!(rig.models.is_installed(&small_row()));
    // The first in the registry's order is used.
    assert_eq!(rig.modes("l1")["polish_models"][0]["name"], "Test Chat");
    rig.finish();
}

#[test]
fn a_call_in_flight_holds_the_model_through_its_replacement() {
    let gate = Arc::new(Gate::default());
    let rig = Rig::new(
        "local-held",
        &[default_row()],
        &[(ink_ffi::local::CURRENT_KEY, "test-chat")],
    );
    *rig.local.gate.lock().unwrap() = Some(gate.clone());
    let llm = ink_ffi::engines::llm(rig.core().shared()).unwrap();
    let call = std::thread::spawn(move || llm.complete(&request(), &CancelToken::new()));
    wait_until("the call reaches the model", || rig.local.calls() == 1);

    let finished = rig.update("test-chat-small");
    assert_eq!(finished["ok"], true, "{finished}");
    // The new one is in use; the old one's files stay while the call holds them.
    assert_eq!(
        rig.modes("l1")["polish_models"][0]["name"],
        "Test Chat Small"
    );
    assert!(rig.models.is_installed(&default_row()), "held: not deleted");
    gate.open();
    assert_eq!(call.join().unwrap().unwrap().text, "Hello, world!");
    rig.finish();
}
