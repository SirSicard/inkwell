//! Own-key (BYOK) language models, for a shell with no language model of its own on this machine
//! (Windows): the user picks a provider in Settings > AI, stores its API key in the OS key store,
//! picks a model, and in choosing a cloud provider turns local-only mode off. Polish, voice edit,
//! summaries and Ask then use it, each only with the user's consent for that provider's endpoint
//! ([`consent`](crate::consent)): choosing a provider sends nothing on its own.
//!
//! | Command | Answer |
//! |---|---|
//! | `llm.providers` | `llm.providers`: every provider, whether its key is stored, and the choice |
//! | `llm.key.save {provider, key}` | `llm.providers`, once the key is in the OS key store |
//! | `llm.key.delete {provider}` | `llm.providers` |
//! | `llm.choose {provider, model?, base_url?, local_only?}` | `setting.value` of `llm.local_only`, a `consent.state` per feature, then `llm.providers` |
//! | `llm.test` | `llm.tested`, from its own thread: one short fixed request to the chosen provider |
//!
//! Each answer echoes the command's `id` as `ref`; a failure is `command.failed` with that id.
//!
//! # The key
//!
//! A key travels once, in `llm.key.save`, into the OS key store (the macOS keychain, the Windows
//! Credential Manager: [`ink_llm::keys`]). It is never written to the library's settings, never
//! echoed in an event, and never named in an error or a log line: errors say what failed ("the key
//! store refused"), never what was sent. [`KeyText`] carries it through the command and never
//! shows it. It is read back only when a request is about to go out ([`ink_llm::provider`]).
//!
//! # Local-only mode
//!
//! `llm.local_only` stays on by default. Choosing a provider that is not on this machine turns it
//! off, and the command must say so (`"local_only":"off"`): it is never a side effect the shell did
//! not ask for. Choosing a provider on this machine (a local server), or none, turns it back on.
//! The choice (provider, model, a custom server's address) is the core's own setting
//! [`CLOUD_KEY`]; the key is never in it.
//!
//! # Which model a feature uses
//!
//! A model the shell registered comes first (the Mac's Foundation Models), then the chosen provider
//! ([`ShellLlms::pick`](crate::llms::ShellLlms::pick)). The Mac's screens never choose one, so
//! nothing changes there.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::thread::{self, JoinHandle};

use ink_core::{Endpoint, Llm, LlmError, LlmRequest};
use ink_llm::{
    ApiKey, ByokConfig, ByokLlm, KeyStore, KeyStoreError, OsKeyStore, Provider, Transport,
    UreqTransport,
};
use ink_pipeline::consent::Feature;
use serde_json::{Map, Value, json};

use crate::events::{self, event};
use crate::llms::LOCAL_ONLY_KEY;
use crate::runtime::{Shared, lock};

/// The core's setting holding the chosen provider, its model and a custom server's address, as a
/// JSON object (`{"provider","model"?,"base_url"?}`), or [`NONE`]. Never the key. Not a shell
/// setting: only `llm.choose` writes it.
pub const CLOUD_KEY: &str = "llm.cloud";

/// [`CLOUD_KEY`]'s value, and `llm.choose`'s provider, for no provider.
pub const NONE: &str = "none";

/// The longest key `llm.key.save` takes, in characters: far beyond any provider's.
pub const MAX_KEY_CHARS: usize = 1_024;

/// The longest model id `llm.choose` takes, in characters.
pub const MAX_MODEL_CHARS: usize = 256;

/// The longest custom server address `llm.choose` takes, in characters.
pub const MAX_URL_CHARS: usize = 2_048;

/// An API key on its way to the key store. Its `Debug` never shows it, and its memory is wiped when
/// it is dropped. Copies made before it (the command's JSON) are out of reach.
pub struct KeyText(ApiKey);

impl KeyText {
    /// Wraps a key.
    pub fn new(key: String) -> Self {
        Self(ApiKey::new(key))
    }

    fn expose(&self) -> &str {
        self.0.expose()
    }
}

impl Clone for KeyText {
    fn clone(&self) -> Self {
        Self::new(self.expose().to_owned())
    }
}

impl PartialEq for KeyText {
    fn eq(&self, other: &Self) -> bool {
        self.expose() == other.expose()
    }
}

impl Eq for KeyText {}

impl std::fmt::Debug for KeyText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyText(<redacted>)")
    }
}

/// A command of this module, read. Its text fields are checked when it runs, so what is wrong
/// with them reaches the screen as `command.failed`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudQuery {
    /// `llm.providers`.
    Providers,
    /// `llm.key.save`.
    SaveKey {
        /// The provider's id.
        provider: String,
        /// The key.
        key: KeyText,
    },
    /// `llm.key.delete`.
    DeleteKey {
        /// The provider's id.
        provider: String,
    },
    /// `llm.choose`.
    Choose {
        /// The provider's id, or [`NONE`].
        provider: String,
        /// The model; the provider's default when absent or empty.
        model: Option<String>,
        /// A custom server's address.
        base_url: Option<String>,
        /// `off` for a provider that is not on this machine: the shell says it turns local-only
        /// mode off.
        local_only: Option<String>,
    },
    /// `llm.test`.
    Test,
}

/// Reads `v` as one of this module's commands: `None` when `name` is none of them.
pub fn parse(name: &str, v: &Value) -> Option<Result<CloudQuery, String>> {
    let fields: &[&str] = match name {
        "llm.providers" | "llm.test" => &[],
        "llm.key.save" => &["provider", "key"],
        "llm.key.delete" => &["provider"],
        "llm.choose" => &["provider", "model", "base_url", "local_only"],
        _ => return None,
    };
    Some(parse_known(name, fields, v))
}

fn parse_known(name: &str, allowed: &[&str], v: &Value) -> Result<CloudQuery, String> {
    let obj = v.as_object().ok_or("command: not an object")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["cmd", "id"].contains(&k.as_str()) && !allowed.contains(&k.as_str()))
    {
        return Err(format!("{name}: unknown field \"{k}\""));
    }
    // Only the field's name is ever said: a key's value must not reach an error.
    let text = |k: &str| -> Result<String, String> {
        obj.get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{name}: needs a string \"{k}\""))
    };
    let optional = |k: &str| -> Result<Option<String>, String> {
        match obj.get(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("{name}: \"{k}\" must be a string")),
        }
    };
    Ok(match name {
        "llm.providers" => CloudQuery::Providers,
        "llm.test" => CloudQuery::Test,
        "llm.key.save" => CloudQuery::SaveKey {
            provider: text("provider")?,
            key: KeyText::new(text("key")?),
        },
        "llm.key.delete" => CloudQuery::DeleteKey {
            provider: text("provider")?,
        },
        "llm.choose" => CloudQuery::Choose {
            provider: text("provider")?,
            model: optional("model")?,
            base_url: optional("base_url")?,
            local_only: optional("local_only")?,
        },
        _ => unreachable!("parse() lists every command"),
    })
}

/// The chosen provider, as [`CLOUD_KEY`] keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The provider.
    pub provider: Provider,
    /// The model; the provider's default when `None`.
    pub model: Option<String>,
    /// For [`Provider::Custom`], the server's address.
    pub base_url: Option<String>,
}

/// A stored choice that does not read. Treated as none, and said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnreadableChoice;

impl Choice {
    /// The stored form.
    pub fn to_setting(&self) -> String {
        let mut obj = Map::new();
        obj.insert("provider".into(), self.provider.id().into());
        if let Some(model) = &self.model {
            obj.insert("model".into(), model.as_str().into());
        }
        if let Some(url) = &self.base_url {
            obj.insert("base_url".into(), url.as_str().into());
        }
        Value::Object(obj).to_string()
    }

    /// Reads the stored form: `Ok(None)` for nothing stored or [`NONE`].
    pub fn from_setting(value: Option<&str>) -> Result<Option<Self>, UnreadableChoice> {
        let Some(value) = value.filter(|v| *v != NONE) else {
            return Ok(None);
        };
        let v: Value = serde_json::from_str(value).map_err(|_| UnreadableChoice)?;
        let obj = v.as_object().ok_or(UnreadableChoice)?;
        if obj
            .keys()
            .any(|k| !["provider", "model", "base_url"].contains(&k.as_str()))
        {
            return Err(UnreadableChoice);
        }
        let text = |k: &str| match obj.get(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(UnreadableChoice),
        };
        let provider = text("provider")?
            .as_deref()
            .and_then(Provider::from_id)
            .ok_or(UnreadableChoice)?;
        Ok(Some(Self {
            provider,
            model: text("model")?,
            base_url: text("base_url")?,
        }))
    }

    fn config(&self) -> ByokConfig {
        ByokConfig {
            provider: self.provider,
            model: self.model.clone(),
            base_url: self.base_url.clone(),
        }
    }
}

/// Where own-key providers keep their keys and how they reach the network.
#[derive(Clone)]
struct Services {
    keys: Arc<dyn KeyStore>,
    transport: Arc<dyn Transport>,
}

/// What [`Shared`] holds for own-key providers: the key store and the HTTP client (made on first
/// use, so a shell that never chooses one never opens the key store), and the test thread's
/// mailbox.
#[derive(Default)]
pub struct Cloud {
    services: RwLock<Option<Services>>,
    tests: OnceLock<Mailbox>,
}

/// The test thread's mailbox, and whether a test is running or waiting.
struct Mailbox {
    tx: Mutex<Sender<Msg>>,
    busy: Arc<AtomicBool>,
}

impl Cloud {
    /// The services: the OS key store and the process's HTTP client unless replaced.
    fn services(&self) -> Services {
        if let Some(s) = self
            .services
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return s;
        }
        let mut slot = self
            .services
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        slot.get_or_insert_with(|| {
            let keys: Arc<dyn KeyStore> = match OsKeyStore::native() {
                Ok(store) => Arc::new(store),
                Err(e) => {
                    log::warn!("own-key providers: no OS key store ({e:?}); no key can be kept");
                    Arc::new(NoKeyStore(e))
                }
            };
            Services {
                keys,
                transport: UreqTransport::shared(),
            }
        })
        .clone()
    }

    fn replace(&self, keys: Arc<dyn KeyStore>, transport: Arc<dyn Transport>) {
        *self
            .services
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(Services { keys, transport });
    }
}

/// The key store on a platform without one: every call says so.
struct NoKeyStore(KeyStoreError);

impl KeyStore for NoKeyStore {
    fn has_key(&self, _: &str) -> Result<bool, KeyStoreError> {
        Err(self.0)
    }

    fn read_key(&self, _: &str) -> Result<ApiKey, KeyStoreError> {
        Err(self.0)
    }

    fn save_key(&self, _: &str, _: &str) -> Result<(), KeyStoreError> {
        Err(self.0)
    }

    fn delete_key(&self, _: &str) -> Result<(), KeyStoreError> {
        Err(self.0)
    }
}

/// **Worker.** Replaces the key store and the HTTP client (a test's fakes) and builds the chosen
/// provider again over them. See [`Core::set_cloud_services`](crate::runtime::Core::set_cloud_services).
pub(crate) fn set_services(
    shared: &Shared,
    keys: Arc<dyn KeyStore>,
    transport: Arc<dyn Transport>,
) {
    shared.cloud.replace(keys, transport);
    load(shared);
}

/// **Worker.** The stored choice, or none; one that cannot be read is none, and logged by name.
fn stored(shared: &Shared) -> Result<Option<Choice>, String> {
    match shared.store.setting(CLOUD_KEY) {
        Ok(v) => Choice::from_setting(v.as_deref()).map_err(|_| {
            log::error!("own-key providers: {CLOUD_KEY} cannot be read; no provider is used");
            "couldn't read the chosen provider".to_owned()
        }),
        Err(e) => {
            log::error!("own-key providers: {CLOUD_KEY} could not be read ({e}); none is used");
            Err("couldn't read the chosen provider".to_owned())
        }
    }
}

/// **Worker.** Builds the stored choice, if any, as the provider features may use (at launch,
/// before anything can call a model). A choice that cannot be read or built is none, and logged.
pub(crate) fn load(shared: &Shared) {
    let llm = match stored(shared) {
        Ok(Some(choice)) => match build(shared, &choice) {
            Ok(llm) => Some(llm),
            Err(e) => {
                log::error!("own-key providers: the chosen provider cannot be built ({e})");
                None
            }
        },
        Ok(None) | Err(_) => None,
    };
    shared.llms.set_cloud(llm);
}

fn build(shared: &Shared, choice: &Choice) -> Result<Arc<ByokLlm>, ink_llm::EndpointError> {
    let s = shared.cloud.services();
    ByokLlm::new(
        choice.config(),
        s.keys,
        s.transport,
        shared.local_only.clone(),
    )
    .map(Arc::new)
}

/// Why the key store refused, in words for a screen. Carries nothing from the key.
fn key_store_error(e: KeyStoreError) -> &'static str {
    match e {
        KeyStoreError::NotFound => "no key is stored",
        KeyStoreError::Denied => "the key store refused",
        KeyStoreError::Unsupported => "this system has no key store Inkwell can use",
        KeyStoreError::Failed => "the key store failed",
    }
}

fn parse_provider(name: &str, id: &str) -> Result<Provider, String> {
    Provider::from_id(id).ok_or_else(|| {
        format!("{name}: \"provider\" is openai, groq, anthropic, openrouter or custom")
    })
}

/// Whether `text` holds a character that has no place in a model id or an address: a control
/// character, which would also end up in a request's header or body.
fn has_control(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// **Queries thread.** Runs a command: its answer, or `None` for `llm.test` (answered by the test
/// thread), or why it failed (for `command.failed`, naming what, never the key).
pub fn answer(
    shared: &Shared,
    query: CloudQuery,
    reference: Option<&str>,
) -> Result<Option<Value>, String> {
    match query {
        CloudQuery::Providers => Ok(Some(providers(shared, reference))),
        CloudQuery::SaveKey { provider: id, key } => {
            let provider = parse_provider("llm.key.save", &id)?;
            let key = key.expose().trim();
            if key.is_empty() {
                return Err("llm.key.save: the key is empty".into());
            }
            if key.chars().count() > MAX_KEY_CHARS {
                return Err(format!(
                    "llm.key.save: the key is longer than {MAX_KEY_CHARS} characters"
                ));
            }
            // Keys are printable ASCII: anything else is a paste gone wrong, and a space or a
            // line break would end up in a request header.
            if !key.bytes().all(|b| b.is_ascii_graphic()) {
                return Err(
                    "llm.key.save: the key holds a space or a character no API key has".into(),
                );
            }
            shared
                .cloud
                .services()
                .keys
                .save_key(provider.id(), key)
                .map_err(|e| {
                    log::warn!(
                        "llm.key.save: {}'s key was not saved ({e:?})",
                        provider.id()
                    );
                    format!("couldn't save the key: {}", key_store_error(e))
                })?;
            Ok(Some(providers(shared, reference)))
        }
        CloudQuery::DeleteKey { provider: id } => {
            let provider = parse_provider("llm.key.delete", &id)?;
            shared
                .cloud
                .services()
                .keys
                .delete_key(provider.id())
                .map_err(|e| {
                    log::warn!(
                        "llm.key.delete: {}'s key was not deleted ({e:?})",
                        provider.id()
                    );
                    format!("couldn't delete the key: {}", key_store_error(e))
                })?;
            Ok(Some(providers(shared, reference)))
        }
        CloudQuery::Choose {
            provider,
            model,
            base_url,
            local_only,
        } => {
            choose(shared, &provider, model, base_url, local_only)?;
            Ok(Some(providers(shared, reference)))
        }
        CloudQuery::Test => {
            test_soon(shared, reference.map(str::to_owned))?;
            Ok(None)
        }
    }
}

/// `llm.choose`: writes the choice and local-only mode together, or neither; then the features'
/// states, since where they would send may have changed.
fn choose(
    shared: &Shared,
    id: &str,
    model: Option<String>,
    base_url: Option<String>,
    local_only: Option<String>,
) -> Result<(), String> {
    const NAME: &str = "llm.choose";
    let (setting, llm) = if id == NONE {
        if model.is_some() || base_url.is_some() || local_only.is_some() {
            return Err(format!(
                "{NAME}: \"none\" takes no model, base_url or local_only"
            ));
        }
        (NONE.to_owned(), None)
    } else {
        let provider = parse_provider(NAME, id)?;
        let tidy = |field: &str, text: Option<String>, max: usize| -> Result<_, String> {
            let text = text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
            if let Some(t) = &text {
                if t.chars().count() > max {
                    return Err(format!(
                        "{NAME}: \"{field}\" is longer than {max} characters"
                    ));
                }
                if has_control(t) {
                    return Err(format!("{NAME}: \"{field}\" holds a control character"));
                }
            }
            Ok(text)
        };
        let choice = Choice {
            provider,
            model: tidy("model", model, MAX_MODEL_CHARS)?,
            base_url: tidy("base_url", base_url, MAX_URL_CHARS)?,
        };
        let llm = build(shared, &choice).map_err(|e| format!("{NAME}: {e}"))?;
        let remote = matches!(llm.info().endpoint, Endpoint::Remote(_));
        match (remote, local_only.as_deref()) {
            (true, Some("off")) | (false, None) => {}
            (true, _) => {
                return Err(format!(
                    "{NAME}: a provider that is not on this machine is chosen with \
                     \"local_only\":\"off\", since choosing it turns local-only mode off"
                ));
            }
            (false, Some(_)) => {
                return Err(format!(
                    "{NAME}: a provider on this machine keeps local-only mode on: leave out \
                     \"local_only\""
                ));
            }
        }
        (choice.to_setting(), Some(llm))
    };
    let remote = llm
        .as_ref()
        .is_some_and(|l| matches!(l.info().endpoint, Endpoint::Remote(_)));
    let local = if remote { "off" } else { "on" };
    shared
        .store
        .set_settings(&[(CLOUD_KEY, &setting), (LOCAL_ONLY_KEY, local)])
        .map_err(|e| {
            log::error!("{NAME}: the choice could not be saved: {e}");
            "couldn't save the choice, so the provider and local-only mode stay as they were"
                .to_owned()
        })?;
    // Whatever changes, it never passes through a state more open than the one before or after.
    if remote {
        shared.llms.set_cloud(llm);
        shared.local_only.set(false);
    } else {
        shared.local_only.set(true);
        shared.llms.set_cloud(llm);
    }
    shared.events.emit(crate::queries::setting_value(
        LOCAL_ONLY_KEY,
        Some(local.into()),
    ));
    for feature in Feature::ALL {
        if crate::consent::switch(feature).is_some() {
            shared
                .events
                .emit(crate::consent::state(shared, feature, None));
        }
    }
    Ok(())
}

/// `llm.providers`: every provider (whether its key is stored, asked without reading it) and the
/// choice. What cannot be read is said, never read as absent.
pub fn providers(shared: &Shared, reference: Option<&str>) -> Value {
    let keys = shared.cloud.services().keys;
    let mut errors: Vec<&str> = Vec::new();
    let mut has = |p: Provider| match keys.has_key(p.id()) {
        Ok(b) => b,
        Err(e) => {
            log::warn!(
                "llm.providers: whether {} has a key is unknown ({e:?})",
                p.id()
            );
            if !errors.contains(&"the stored keys") {
                errors.push("the stored keys");
            }
            false
        }
    };
    let list: Vec<Value> = Provider::ALL
        .into_iter()
        .map(|p| {
            json!({
                "id": p.id(),
                "default_model": p.default_model(),
                "endpoint": p.default_base_url(),
                "custom_url": p == Provider::Custom,
                "needs_key": p.needs_key(),
                "has_key": has(p),
            })
        })
        .collect();
    let choice = match stored(shared) {
        Ok(c) => c,
        Err(_) => {
            errors.push("the chosen provider");
            None
        }
    };
    let local_only = shared.local_only.is_on();
    let llm = shared.llms.cloud();
    let (info, ready) = match (&choice, &llm) {
        (Some(choice), Some(llm)) => {
            let info = llm.info();
            let keyed = !choice.provider.needs_key()
                || list
                    .iter()
                    .find(|p| p["id"] == choice.provider.id())
                    .is_some_and(|p| p["has_key"] == true);
            let allowed = info.endpoint.is_local() || !local_only;
            (Some(info), keyed && allowed)
        }
        _ => (None, false),
    };
    event(
        "llm.providers",
        &[
            ("providers", Some(Value::Array(list))),
            ("chosen", info.as_ref().map(|i| i.provider.clone().into())),
            ("model", info.as_ref().map(|i| i.model.clone().into())),
            (
                "base_url",
                info.as_ref()
                    .and(choice.as_ref())
                    .and_then(|c| c.base_url.clone())
                    .map(Into::into),
            ),
            (
                "endpoint",
                info.as_ref().map(|i| i.endpoint.describe().into()),
            ),
            (
                "to",
                info.as_ref().map(|i| {
                    if i.endpoint.is_local() {
                        "on_device"
                    } else {
                        "cloud"
                    }
                    .into()
                }),
            ),
            ("local_only", Some(local_only.into())),
            ("ready", Some(ready.into())),
            (
                "error",
                (!errors.is_empty())
                    .then(|| format!("couldn't read {}", errors.join(" or ")).into()),
            ),
            ("ref", reference.map(Into::into)),
        ],
    )
}

// ------------------------------------------------------------------------------------------------
// The test

/// What `llm.test` sends: a fixed question, never the user's words.
fn probe() -> LlmRequest {
    LlmRequest {
        system: "Reply with the single word OK.".into(),
        user: "Is this connection working?".into(),
        max_tokens: 16,
        temperature: 0.0,
        json_schema: None,
    }
}

enum Msg {
    Test(Option<String>),
    Quit,
}

/// The test thread, `ink-llm-test`: a request can take as long as the provider does, so it runs
/// apart from the screens' thread. One test at a time; another asked meanwhile is refused as busy.
pub struct Tester {
    tx: Sender<Msg>,
    busy: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Tester {
    /// Starts `ink-llm-test` and gives [`Shared`] its mailbox.
    pub fn start(shared: Arc<Shared>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let busy = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, busy) = (shared.clone(), busy.clone());
            thread::Builder::new()
                .name("ink-llm-test".into())
                .spawn(move || run(&shared, &rx, &busy))?
        };
        let _ = shared.cloud.tests.set(Mailbox {
            tx: Mutex::new(tx.clone()),
            busy: busy.clone(),
        });
        Ok(Self { tx, busy, thread })
    }

    /// Ends the thread once the test in flight is done (its request sees the shutdown's cancel
    /// before and after it goes out; one already on the wire runs to its timeout).
    pub fn stop(self) {
        let _ = self.tx.send(Msg::Quit);
        if self.thread.join().is_err() {
            log::error!("the test thread panicked outside its boundary");
        }
        self.busy.store(false, Ordering::Release);
    }
}

/// Queues a test, or refuses it while one runs.
fn test_soon(shared: &Shared, id: Option<String>) -> Result<(), String> {
    let Some(mailbox) = shared.cloud.tests.get() else {
        return Err("the test thread has not started".into());
    };
    if mailbox.busy.swap(true, Ordering::AcqRel) {
        return Err("a test is already running; wait for its answer".into());
    }
    lock(&mailbox.tx).send(Msg::Test(id)).map_err(|_| {
        mailbox.busy.store(false, Ordering::Release);
        "the test thread has stopped".to_owned()
    })
}

fn run(shared: &Shared, rx: &Receiver<Msg>, busy: &AtomicBool) {
    while let Ok(Msg::Test(id)) = rx.recv() {
        if shared.shutdown.is_cancelled() {
            busy.store(false, Ordering::Release);
            continue;
        }
        let ran =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(shared, id.as_deref())));
        // Free before the answer goes out: a shell may ask again as soon as it has it.
        busy.store(false, Ordering::Release);
        let answer = ran.unwrap_or_else(|_| {
            log::error!("llm.test panicked; the next test still runs");
            events::command_failed(
                "llm.test",
                id.as_deref(),
                "a bug in the core stopped this test",
            )
        });
        shared.events.emit(answer);
    }
}

/// **Worker.** Sends [`probe`] to the chosen provider, through its local-only check: the answer
/// that says whether it answered, for the caller to emit.
fn test(shared: &Shared, reference: Option<&str>) -> Value {
    let Some(llm) = shared.llms.cloud() else {
        log::warn!("command llm.test failed: no provider is chosen");
        return events::command_failed("llm.test", reference, "no provider is chosen");
    };
    let info = llm.info();
    let outcome = llm.complete(&probe(), &shared.shutdown);
    let (status, error) = match &outcome {
        Ok(_) => (None, None),
        Err(e) => {
            log::warn!("llm.test: {} did not answer: {e}", info.provider);
            let status = match e {
                LlmError::Http { status } => Some(*status),
                _ => None,
            };
            (status, Some(tested_error(e, llm.key_withheld())))
        }
    };
    event(
        "llm.tested",
        &[
            ("provider", Some(info.provider.into())),
            ("model", Some(info.model.into())),
            ("ok", Some(outcome.is_ok().into())),
            ("status", status.map(Into::into)),
            ("error", error.map(Into::into)),
            ("ref", reference.map(Into::into)),
        ],
    )
}

/// Why a test failed, in words for a screen. `withheld`: the provider's key is kept back from its
/// address ([`ByokLlm::key_withheld`]), so a refusal is not the key's fault.
fn tested_error(e: &LlmError, withheld: bool) -> String {
    match e {
        LlmError::NoKey => "couldn't test it: no key is stored for it".into(),
        LlmError::KeychainDenied => "couldn't read its key: the key store refused".into(),
        LlmError::LocalOnly { .. } => {
            "couldn't test it: local-only mode is on, so nothing was sent".into()
        }
        LlmError::Http { status: 401 | 403 } if withheld => {
            "couldn't get an answer: the server asks for a key, and keys are sent only over \
             https or to this computer"
                .into()
        }
        LlmError::Http { status: 401 | 403 } => {
            "couldn't get an answer: the provider refused the key".into()
        }
        LlmError::Http { status: 404 } => {
            "couldn't get an answer: the provider does not know this model or address".into()
        }
        LlmError::Http { status: 429 } => {
            "couldn't get an answer: the account is over its limits or out of credit".into()
        }
        LlmError::Http { status } => format!("couldn't get an answer: the provider said {status}"),
        LlmError::Network(why) => format!("couldn't reach the provider: {why}"),
        LlmError::Cancelled => "couldn't finish the test: Inkwell is closing".into(),
        other => format!("couldn't get an answer: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_never_shows_in_debug_output() {
        let key = KeyText::new("sk-synthetic-canary".into());
        assert_eq!(format!("{key:?}"), "KeyText(<redacted>)");
        let q = CloudQuery::SaveKey {
            provider: "openai".into(),
            key,
        };
        assert!(!format!("{q:?}").contains("canary"));
    }

    #[test]
    fn the_choice_reads_back_and_anything_else_is_refused() {
        for c in [
            Choice {
                provider: Provider::OpenAi,
                model: None,
                base_url: None,
            },
            Choice {
                provider: Provider::Custom,
                model: Some("example-model".into()),
                base_url: Some("http://127.0.0.1:8080/v1".into()),
            },
        ] {
            assert_eq!(Choice::from_setting(Some(&c.to_setting())), Ok(Some(c)));
        }
        assert_eq!(Choice::from_setting(None), Ok(None));
        assert_eq!(Choice::from_setting(Some(NONE)), Ok(None));
        for bad in [
            "",
            "{}",
            r#"{"provider":"nobody"}"#,
            r#"{"provider":"openai","key":"x"}"#,
            r#"{"provider":"openai","model":3}"#,
        ] {
            assert_eq!(
                Choice::from_setting(Some(bad)),
                Err(UnreadableChoice),
                "{bad}"
            );
        }
        assert!(
            !Choice {
                provider: Provider::OpenAi,
                model: None,
                base_url: None
            }
            .to_setting()
            .contains("key")
        );
    }

    #[test]
    fn a_failed_parse_names_the_field_never_its_value() {
        let v = json!({"cmd": "llm.key.save", "provider": "openai", "key": 7});
        let e = parse("llm.key.save", &v).unwrap().unwrap_err();
        assert_eq!(e, "llm.key.save: needs a string \"key\"");
        let v =
            json!({"cmd": "llm.key.save", "provider": "openai", "key": "sk-canary", "extra": 1});
        let e = parse("llm.key.save", &v).unwrap().unwrap_err();
        assert!(!e.contains("canary"), "{e}");
        assert!(parse("llm.unknown", &v).is_none());
    }

    #[test]
    fn a_failed_test_is_said_in_words_without_the_key() {
        assert_eq!(
            tested_error(&LlmError::Http { status: 401 }, false),
            "couldn't get an answer: the provider refused the key"
        );
        assert_eq!(
            tested_error(&LlmError::Http { status: 403 }, true),
            "couldn't get an answer: the server asks for a key, and keys are sent only over \
             https or to this computer"
        );
        assert_eq!(
            tested_error(&LlmError::Http { status: 500 }, true),
            "couldn't get an answer: the provider said 500"
        );
        assert!(tested_error(&LlmError::NoKey, false).starts_with("couldn't"));
    }
}
