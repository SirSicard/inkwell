//! The core at run time: what `ink_init` starts and `ink_shutdown` stops, usable from Rust
//! without the C ABI (the tests drive it directly).
//!
//! | Thread | Runs |
//! |---|---|
//! | `ink-events` | every event to the shell ([`hub`](crate::hub)) |
//! | `ink-commands` | commands, one at a time, in order: warming, model updates, starting a meeting, unregistering |
//! | `ink-meeting`, `ink-pump` | a meeting ([`meeting`](crate::meeting)) |
//! | `ink-dictation` | the dictation chain ([`dictation`](crate::dictation)) |
//!
//! **Shutdown** ([`Core::shutdown`]) goes in an order that leaves nothing loaded behind it:
//! cancel what waits (shell engines, installs, the final pass), stop and join every thread that
//! can hold an engine, let go of the shell's engines (their `release` runs), unload every model
//! and drop residency, and only then stop the event thread. ggml's Metal backend aborts the
//! process at exit if a model is still loaded, so "unloaded before `ink_shutdown` returns" is a
//! requirement, not tidiness.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use ink_audio::BandsWriter;
use ink_core::{
    CancelToken, Clock, EngineError, FocusReader, Job, Llm, OfflineEngine, Store, TextInserter,
};
use ink_engines::{EngineRow, Loader, ModelDir, Os, Registry, Residency, Route, Router, Unloaded};
use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
use ink_pipeline::gain_stage::Vad;
use ink_pipeline::update::{ModelInstaller, update_model};
use serde_json::Value;

use crate::dictation::{DictationInbox, DictationWorker};
use crate::events::{self, event};
use crate::external::Registration;
use crate::gate::{ModelGate, Routed, refused_event};
use crate::hub::{EventOut, Events, Hub};
use crate::llms::{PolishModel, ShellLlms};
use crate::mailbox::DEFAULT_AUDIO_CAPACITY;
use crate::meeting::{CaptureSide, MeetingRun, Replay};

/// The model type residency holds: any offline engine an adapter loads.
pub type Model = Box<dyn OfflineEngine>;

/// `ink_init`'s configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The library, recordings and models live here.
    pub data_dir: PathBuf,
    /// Where models are installed.
    pub models_dir: PathBuf,
    /// The most verbose level logged.
    pub log_level: log::LevelFilter,
    /// Whether log lines go to stderr.
    pub log_stderr: bool,
}

impl Config {
    /// Reads the JSON `ink_init` receives (see `inkwell.h`).
    pub fn parse(json: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| format!("config: {e}"))?;
        let obj = object(&v, "config")?;
        only_fields(
            obj,
            &["data_dir", "models_dir", "log_level", "log_stderr"],
            "config",
        )?;
        let path = |k: &str| -> Result<Option<PathBuf>, String> {
            match obj.get(k) {
                None => Ok(None),
                Some(Value::String(s)) if PathBuf::from(s).is_absolute() => Ok(Some(s.into())),
                Some(_) => Err(format!("config: \"{k}\" must be an absolute path")),
            }
        };
        let data_dir = path("data_dir")?.ok_or("config: \"data_dir\" is required")?;
        let models_dir = path("models_dir")?.unwrap_or_else(|| data_dir.join("models"));
        let log_level = match obj.get("log_level") {
            None => log::LevelFilter::Info,
            Some(l) => l
                .as_str()
                .and_then(crate::logging::parse_level)
                .ok_or("config: \"log_level\" must be off, error, warn, info, debug or trace")?,
        };
        let log_stderr = match obj.get("log_stderr") {
            None => true,
            Some(b) => b
                .as_bool()
                .ok_or("config: \"log_stderr\" must be true or false")?,
        };
        Ok(Self {
            data_dir,
            models_dir,
            log_level,
            log_stderr,
        })
    }
}

/// What the core is built from. [`Parts::production`] for the app; tests swap pieces.
pub struct Parts {
    /// The library.
    pub store: Arc<dyn Store>,
    /// The platform clock.
    pub clock: Arc<dyn Clock>,
    /// The registry.
    pub registry: Registry,
    /// Where models are installed.
    pub models: ModelDir,
    /// Loads registry models (the engine adapters this build has).
    pub loader: Arc<dyn Loader<Model>>,
    /// Installs a model's files (the downloader).
    pub installer: Arc<dyn ModelInstaller>,
    /// Where meetings' recordings go.
    pub data_dir: PathBuf,
}

impl Parts {
    /// The real parts: the SQLite library, the platform clock, the built-in registry, the
    /// downloader over HTTPS, and the adapters this build was compiled with.
    pub fn production(config: &Config) -> Result<Self, String> {
        std::fs::create_dir_all(&config.data_dir)
            .map_err(|e| format!("the data directory could not be created: {e}"))?;
        let store = ink_store::SqliteStore::open(config.data_dir.join("library.sqlite"))
            .map_err(|e| format!("the library: {e}"))?;
        let models = ModelDir::new(&config.models_dir);
        let fetch = ink_engines::HttpFetch::new().map_err(|e| format!("HTTP: {e}"))?;
        Ok(Self {
            store: Arc::new(store),
            clock: platform_clock()?,
            registry: Registry::builtin().map_err(|e| format!("the registry: {e}"))?,
            loader: adapters(&models),
            installer: Arc::new(ink_engines::Downloader::new(
                Arc::new(fetch),
                models.clone(),
            )),
            models,
            data_dir: config.data_dir.clone(),
        })
    }
}

#[cfg(target_os = "macos")]
fn platform_clock() -> Result<Arc<dyn Clock>, String> {
    ink_platform_mac::MacClock::new()
        .map(|c| Arc::new(c) as Arc<dyn Clock>)
        .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "macos"))]
fn platform_clock() -> Result<Arc<dyn Clock>, String> {
    // Until ink-platform-win's clock (S3.1): only replays run here, and their host times come
    // from this same clock, so they share its timebase. Real capture must use the platform's.
    Ok(Arc::new(StdClock(std::time::Instant::now())))
}

#[cfg(not(target_os = "macos"))]
struct StdClock(std::time::Instant);

#[cfg(not(target_os = "macos"))]
impl Clock for StdClock {
    fn now_ns(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn unix_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// The loader for this build's adapters.
fn adapters(models: &ModelDir) -> Arc<dyn Loader<Model>> {
    #[cfg(feature = "engine-llama")]
    {
        Arc::new(Adapters {
            qwen: ink_engines::llama::QwenAsrLoader::new(models.clone()),
        })
    }
    #[cfg(not(feature = "engine-llama"))]
    {
        let _ = models;
        Arc::new(Adapters)
    }
}

#[cfg(feature = "engine-llama")]
struct Adapters {
    qwen: ink_engines::llama::QwenAsrLoader,
}

#[cfg(not(feature = "engine-llama"))]
struct Adapters;

impl Loader<Model> for Adapters {
    fn load(&self, row: &EngineRow) -> Result<Model, EngineError> {
        #[cfg(feature = "engine-llama")]
        if row.runtime == ink_engines::Runtime::LlamaCpp {
            return self.qwen.load(row).map(|m| Box::new(m) as Model);
        }
        Err(EngineError::ModelMissing(format!(
            "{}: this build has no adapter for its runtime",
            row.id
        )))
    }
}

/// What every thread of the core shares.
pub struct Shared {
    /// Events to the shell.
    pub events: Events,
    /// The library.
    pub store: Arc<dyn Store>,
    /// The platform clock.
    pub clock: Arc<dyn Clock>,
    /// Job → engine.
    pub router: Router,
    /// Loaded registry models.
    pub residency: Residency<Model>,
    /// Exclusive holds during updates.
    pub gate: Arc<ModelGate>,
    /// The registry.
    pub registry: Registry,
    /// Installs models.
    pub installer: Arc<dyn ModelInstaller>,
    /// Set at shutdown: whatever waits on a shell engine or an install gives up.
    pub shutdown: CancelToken,
    /// Where meetings' recordings go.
    pub data_dir: PathBuf,
    /// Language models the shell registered (dictation polish).
    pub llms: Arc<ShellLlms>,
    /// Ids of the engines the shell registered, of every kind: one id space.
    externals: Mutex<Vec<String>>,
    /// The ink's bands writer, lent by the C ABI; the pump publishes through it.
    bands: Mutex<Option<BandsWriter>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section here is a swap or a list edit: consistent at each step.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// **Pump.** Publishes the ink's bands, if a writer is lent. The lock is the pump's alone in
    /// practice (lending and shutdown take it once), so it never waits; readers never take it.
    pub fn publish_bands(&self, bands: ink_audio::Bands) {
        if let Some(writer) = lock(&self.bands).as_mut() {
            writer.publish(bands);
        }
    }

    fn take_bands(&self) -> Option<BandsWriter> {
        lock(&self.bands).take()
    }

    fn return_bands(&self, writer: BandsWriter) {
        *lock(&self.bands) = Some(writer);
    }
}

/// A command, read.
#[derive(Clone, Debug, PartialEq)]
enum Command {
    ReplayMeeting(Replay),
    ModelWarm { job: Job },
    ModelUpdate { id: String, next: String },
    EngineUnregister { id: String },
}

struct Envelope {
    name: String,
    id: Option<String>,
    command: Command,
}

/// `v` as a JSON object, or why not.
fn object<'a>(v: &'a Value, what: &str) -> Result<&'a serde_json::Map<String, Value>, String> {
    v.as_object()
        .ok_or_else(|| format!("{what}: not an object"))
}

/// Refuses a field outside `allowed`, as the config and every command do: a misspelt field must
/// not be ignored silently (the shell would think it had asked for something it had not).
fn only_fields(
    obj: &serde_json::Map<String, Value>,
    allowed: &[&str],
    what: &str,
) -> Result<(), String> {
    match obj.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("{what}: unknown field \"{k}\"")),
        None => Ok(()),
    }
}

fn parse_command(json: &str) -> Result<Envelope, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("command: {e}"))?;
    let obj = object(&v, "command")?;
    let name = v
        .get("cmd")
        .and_then(Value::as_str)
        .ok_or("command: needs a string \"cmd\"")?
        .to_owned();
    let id = match v.get("id") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("command: \"id\" must be a string".into()),
    };
    let text = |k: &str| -> Result<String, String> {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{name}: needs a string \"{k}\""))
    };
    let optional = |k: &str| -> Result<Option<String>, String> {
        match v.get(k) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("{name}: \"{k}\" must be a string")),
        }
    };
    let fields: &[&str] = match name.as_str() {
        "replay_meeting" => &["mic", "far", "title", "pacing"],
        "model.warm" => &["job"],
        "model.update" => &["model", "next"],
        "engine.unregister" => &["engine"],
        other => return Err(format!("unknown command \"{other}\"")),
    };
    let allowed: Vec<&str> = ["cmd", "id"].iter().chain(fields).copied().collect();
    only_fields(obj, &allowed, &name)?;
    let command = match name.as_str() {
        "replay_meeting" => Command::ReplayMeeting(Replay {
            mic: text("mic")?.into(),
            far: optional("far")?.map(PathBuf::from),
            title: optional("title")?,
            fast: match optional("pacing")?.as_deref() {
                None | Some("realtime") => false,
                Some("fast") => true,
                Some(_) => return Err("replay_meeting: \"pacing\" is realtime or fast".into()),
            },
        }),
        "model.warm" => Command::ModelWarm {
            job: events::parse_job(&text("job")?).ok_or("model.warm: unknown job")?,
        },
        "model.update" => Command::ModelUpdate {
            id: text("model")?,
            next: text("next")?,
        },
        "engine.unregister" => Command::EngineUnregister {
            id: text("engine")?,
        },
        other => return Err(format!("unknown command \"{other}\"")),
    };
    Ok(Envelope { name, id, command })
}

/// The shell's platform pieces for dictation (S2.7 passes the Mac ones).
pub struct DictationParts {
    /// Text insertion.
    pub inserter: Arc<dyn TextInserter>,
    /// The frontmost app, for modes.
    pub focus: Arc<dyn FocusReader>,
    /// The polish model, if any.
    pub llm: Option<Arc<dyn Llm>>,
    /// The user's settings.
    pub settings: DictationSettings,
    /// The VAD, or why there is none.
    pub vad: Vad,
}

#[derive(Default)]
struct Runs {
    meeting: Option<MeetingRun>,
    dictation: Option<DictationWorker>,
}

/// What shutdown did, for the caller and the tests.
pub struct Stopped {
    /// Models residency still had loaded, now unloaded.
    pub models_unloaded: usize,
    /// Shell engines let go of.
    pub engines_released: usize,
    /// The bands writer, for the next start.
    pub bands: Option<BandsWriter>,
}

/// The running core. See the module docs.
pub struct Core {
    shared: Arc<Shared>,
    hub: Hub,
    commands: Sender<Envelope>,
    command_thread: JoinHandle<()>,
    runs: Arc<Mutex<Runs>>,
}

impl Core {
    /// Starts the event thread and the command thread, and announces `core.ready`. The ink's
    /// bands are published only once a writer is lent ([`lend_bands`](Self::lend_bands)).
    pub fn start(parts: Parts, out: EventOut) -> io::Result<Self> {
        let hub = Hub::start(out)?;
        let os = Os::current().unwrap_or(Os::MacOs);
        let shared = Arc::new(Shared {
            events: hub.events(),
            router: Router::new(&parts.registry, parts.models, os),
            residency: Residency::new(parts.loader, parts.clock.clone()),
            store: parts.store,
            clock: parts.clock,
            gate: Arc::default(),
            registry: parts.registry,
            installer: parts.installer,
            shutdown: CancelToken::new(),
            data_dir: parts.data_dir,
            llms: Arc::default(),
            externals: Mutex::default(),
            bands: Mutex::new(None),
        });
        let runs = Arc::new(Mutex::new(Runs::default()));
        let (commands, rx) = mpsc::channel::<Envelope>();
        let command_thread = {
            let (shared, runs) = (shared.clone(), runs.clone());
            thread::Builder::new()
                .name("ink-commands".into())
                .spawn(move || {
                    while let Ok(envelope) = rx.recv() {
                        guarded(&shared, &runs, envelope);
                    }
                })?
        };
        shared.events.emit(events::ready());
        Ok(Self {
            shared,
            hub,
            commands,
            command_thread,
            runs,
        })
    }

    /// Lends the bands writer the pump publishes through; [`shutdown`](Self::shutdown) returns
    /// it. Lent after a successful start, so a failed start cannot lose it.
    pub fn lend_bands(&self, writer: BandsWriter) {
        self.shared.return_bands(writer);
    }

    /// Reads a command and queues it. Errors mean nothing was queued.
    pub fn command(&self, json: &str) -> Result<(), String> {
        let envelope = parse_command(json)?;
        self.commands
            .send(envelope)
            .map_err(|_| "the command thread has stopped".to_owned())
    }

    /// Registers a shell engine: offline and streaming engines with the router, language models
    /// with [`Shared::llms`]. Ids are unique across every kind. Returns its id.
    pub fn register(&self, engine: impl Into<Registration>) -> Result<String, String> {
        let engine = engine.into();
        let id = engine.id().to_owned();
        let refuse = |engine: Registration, why: String| {
            // Nothing is kept, and the header promises release is not called for a refusal.
            engine.disarm();
            Err(why)
        };
        // Held across the check and the insert, so two registrations of one id cannot both pass.
        let mut externals = lock(&self.shared.externals);
        if externals.contains(&id) {
            return refuse(engine, format!("engine id {id} is already registered"));
        }
        let (kind, scores) = match engine {
            Registration::Offline(e) => {
                let e = Arc::new(e);
                let scores = e.scores().to_vec();
                if let Err(err) = self.shared.router.register_offline(e.clone(), &scores) {
                    e.disarm();
                    return Err(err.to_string());
                }
                ("offline", scores)
            }
            Registration::Streaming(e) => {
                let e = Arc::new(e);
                let scores = e.scores().to_vec();
                if let Err(err) = self.shared.router.register_streaming(e.clone(), &scores) {
                    e.disarm();
                    return Err(err.to_string());
                }
                ("streaming", scores)
            }
            Registration::Llm(e) => {
                if self.shared.registry.get(&id).is_some() {
                    return refuse(
                        Registration::Llm(e),
                        format!("engine id {id} is a registry model's"),
                    );
                }
                if !self.shared.llms.insert(e) {
                    return Err(format!("engine id {id} is already registered"));
                }
                ("llm", Vec::new())
            }
        };
        externals.push(id.clone());
        drop(externals);
        self.shared.events.emit(event(
            "engine.registered",
            &[
                ("id", Some(id.as_str().into())),
                ("kind", Some(kind.into())),
                (
                    "jobs",
                    Some(Value::Array(
                        scores
                            .iter()
                            .map(|s| serde_json::json!({"job": events::job(s.job), "wer": s.wer}))
                            .collect(),
                    )),
                ),
            ],
        ));
        Ok(id)
    }

    /// **Worker.** Starts a meeting captured from `capture` (one source per side), unless one is
    /// running. It ends when every source has delivered everything (dropped the sink it was
    /// given), then runs its final pass; its events say how it went.
    pub fn start_meeting(
        &self,
        capture: Vec<CaptureSide>,
        title: Option<String>,
    ) -> Result<(), String> {
        start_meeting(&self.shared, &self.runs, capture, title)
    }

    /// The shared state, for tests and the C ABI.
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Starts the dictation worker with the shell's platform pieces, replacing one already
    /// running. Its engine is whatever the router picks for the dictation job at each take. With
    /// no polish model in `parts`, polish goes to a registered language model, whichever is
    /// registered when a take is polished ([`PolishModel`]).
    pub fn start_dictation(&self, parts: DictationParts) -> io::Result<DictationInbox> {
        let s = &self.shared;
        let events = s.events.clone();
        let sink: ink_core::EventSink<ink_pipeline::events::DictationEvent> =
            Arc::new(move |e| events.emit(events::dictation(&e)));
        let chain = DictationChain::new(
            Services {
                engine: Arc::new(Routed::new(s.clone(), Job::DictationFinal)),
                store: s.store.clone(),
                inserter: parts.inserter,
                focus: parts.focus,
                clock: s.clock.clone(),
                llm: parts
                    .llm
                    .or_else(|| Some(Arc::new(PolishModel::new(s.llms.clone())) as Arc<dyn Llm>)),
            },
            parts.settings,
            parts.vad,
            sink,
        );
        let worker = DictationWorker::spawn(
            chain,
            s.clock.clone(),
            s.events.clone(),
            DEFAULT_AUDIO_CAPACITY,
        )?;
        let inbox = worker.inbox();
        let previous = lock(&self.runs).dictation.replace(worker);
        if let Some(previous) = previous {
            stop_dictation(previous);
        }
        Ok(inbox)
    }

    /// Stops everything, in the order the module docs give, and returns once the event thread
    /// has delivered `core.stopped`. Nothing calls the shell after this returns.
    pub fn shutdown(self) -> Stopped {
        let Self {
            shared,
            hub,
            commands,
            command_thread,
            runs,
        } = self;
        shared.shutdown.cancel();
        drop(commands);
        if command_thread.join().is_err() {
            log::error!("the command thread panicked");
        }
        let (meeting, dictation) = {
            let mut runs = lock(&runs);
            (runs.meeting.take(), runs.dictation.take())
        };
        if let Some(meeting) = meeting {
            meeting.stop();
        }
        if let Some(dictation) = dictation {
            stop_dictation(dictation);
        }
        let ids = std::mem::take(&mut *lock(&shared.externals));
        // Every stream is closed by now (the meeting's chain, which held them, is joined), so
        // letting go here runs each engine's release.
        let engines_released = ids
            .iter()
            .filter(|id| shared.router.unregister(id) || shared.llms.remove(id))
            .count();
        // Every thread that could hold a model has been joined, so no lease is left and these
        // unloads cannot be refused. Done explicitly rather than trusting the drop below: that
        // needs every reference to `shared` gone, and one leaked clone would keep a model loaded.
        let _ = shared.residency.set_warm(None);
        let mut models_unloaded = 0;
        for id in shared.residency.resident() {
            match shared.residency.unload(&id) {
                Ok(Unloaded::WasLoaded) => models_unloaded += 1,
                Ok(Unloaded::NotLoaded) => {}
                Err(e) => log::error!("shutdown: model {id} could not be unloaded: {e}"),
            }
        }
        let bands = shared.take_bands();
        let events = shared.events.clone();
        match Arc::try_unwrap(shared) {
            // Router (and with it the shell's engines), residency and the store drop here.
            Ok(shared) => drop(shared),
            Err(shared) => log::error!(
                "shutdown: {} references to the core's state remain; its models are already \
                 unloaded",
                Arc::strong_count(&shared) - 1
            ),
        }
        events.emit(event("core.stopped", &[]));
        hub.stop();
        Stopped {
            models_unloaded,
            engines_released,
            bands,
        }
    }
}

fn stop_dictation(worker: DictationWorker) {
    // Joined here, so the chain, and the engine handle in it, drop on this thread before the
    // caller goes on.
    if worker.stop().is_err() {
        log::error!("the dictation worker panicked outside its boundary");
    }
}

/// Starts a meeting on `capture`, unless one is running; collects one that has finished.
fn start_meeting(
    shared: &Arc<Shared>,
    runs: &Mutex<Runs>,
    capture: Vec<CaptureSide>,
    title: Option<String>,
) -> Result<(), String> {
    let mut runs = lock(runs);
    if runs.meeting.as_ref().is_some_and(|m| !m.is_finished()) {
        return Err("a meeting is already running".into());
    }
    if let Some(done) = runs.meeting.take() {
        done.join();
    }
    runs.meeting = Some(MeetingRun::start(shared, capture, title)?);
    Ok(())
}

/// Runs one command behind a panic boundary, so a bug in one costs that command, not the thread
/// every later command needs. What a command holds is released on the way out: gate holds and
/// uses, residency's load and unload markers and the runs lock are guards, and the locks here
/// ignore poison.
fn guarded(shared: &Arc<Shared>, runs: &Mutex<Runs>, envelope: Envelope) {
    let (name, id) = (envelope.name.clone(), envelope.id.clone());
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_command(shared, runs, envelope);
    }));
    if ran.is_err() {
        // The payload is not logged: it could hold what was said (I5).
        log::error!("command {name} panicked; the next command still runs");
        shared.events.emit(events::command_failed(
            &name,
            id.as_deref(),
            "a bug in the core stopped this command; nothing it held is kept",
        ));
    }
}

fn run_command(shared: &Arc<Shared>, runs: &Mutex<Runs>, envelope: Envelope) {
    let Envelope { name, id, command } = envelope;
    let fail = |message: String| {
        log::warn!("command {name} failed: {message}");
        shared
            .events
            .emit(events::command_failed(&name, id.as_deref(), &message));
    };
    match command {
        Command::ReplayMeeting(replay) => {
            let started = replay
                .open(shared)
                .and_then(|capture| start_meeting(shared, runs, capture, replay.title.clone()));
            if let Err(e) = started {
                fail(e);
            }
        }
        Command::ModelWarm { job } => warm(shared, job),
        Command::ModelUpdate { id: current, next } => update(shared, &current, &next, &fail),
        Command::EngineUnregister { id: engine } => {
            let known = {
                let mut ids = lock(&shared.externals);
                let before = ids.len();
                ids.retain(|i| *i != engine);
                ids.len() != before
            };
            if known && (shared.router.unregister(&engine) || shared.llms.remove(&engine)) {
                shared
                    .events
                    .emit(event("engine.unregistered", &[("id", Some(engine.into()))]));
            } else {
                fail(format!("no shell engine is registered as {engine}"));
            }
        }
    }
}

fn warm(shared: &Shared, job: Job) {
    let failed = |id: Option<&str>, message: String| {
        log::warn!("warming the {job:?} model failed: {message}");
        shared.events.emit(event(
            "model.warm_failed",
            &[
                ("job", Some(events::job(job).into())),
                ("id", id.map(Into::into)),
                ("message", Some(message.into())),
            ],
        ));
    };
    match shared.router.route(job) {
        Err(e) => failed(None, e.to_string()),
        Ok(Route::External { id, .. }) => {
            // The shell loaded it; there is nothing for residency to do.
            shared.events.emit(event(
                "model.warmed",
                &[
                    ("id", Some(id.into())),
                    ("job", Some(events::job(job).into())),
                ],
            ));
        }
        Ok(Route::Model(row)) => {
            let entered = shared.gate.enter(&row.id);
            let Ok(_use) = entered else {
                shared.events.emit(refused_event(&row.id, job));
                return;
            };
            match shared.residency.set_warm(Some(&row)) {
                Ok(()) => shared.events.emit(event(
                    "model.warmed",
                    &[
                        ("id", Some(row.id.as_str().into())),
                        ("job", Some(events::job(job).into())),
                    ],
                )),
                Err(e) => failed(Some(&row.id), e.to_string()),
            }
        }
    }
}

fn update(shared: &Shared, current: &str, next: &str, fail: &dyn Fn(String)) {
    let (Some(current_row), Some(next_row)) =
        (shared.registry.get(current), shared.registry.get(next))
    else {
        return fail(format!("{current} and {next} must both be registry models"));
    };
    let ids: Vec<&str> = if current == next {
        vec![current]
    } else {
        vec![current, next]
    };
    // Taken before anything is unloaded, and held until the new model is installed and warm.
    let hold = match shared.gate.hold(&ids) {
        Ok(hold) => hold,
        Err(refused) => return fail(refused.to_string()),
    };
    let ids_event = |ty: &str, extra: &[(&str, Option<Value>)]| {
        let mut fields = vec![("id", Some(current.into())), ("next", Some(next.into()))];
        fields.extend_from_slice(extra);
        event(ty, &fields)
    };
    shared.events.emit(ids_event("model.update_started", &[]));
    let result = update_model(
        &shared.residency,
        shared.installer.as_ref(),
        current_row,
        next_row,
        &shared.shutdown,
    );
    drop(hold);
    let (ok, no_model_warm, message) = match &result {
        Ok(()) => (true, false, None),
        Err(e) => (false, e.no_model_warm(), Some(Value::from(e.to_string()))),
    };
    if let Err(e) = &result {
        log::warn!("model update {current} -> {next} failed: {e}");
    }
    shared.events.emit(ids_event(
        "model.update_finished",
        &[
            ("ok", Some(ok.into())),
            ("no_model_warm", Some(no_model_warm.into())),
            ("message", message),
        ],
    ));
}

/// A core for unit tests: an in-memory library, `clock`, no models, and events kept in memory.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::{Arc, Mutex};

    use ink_core::{CancelToken, Clock, EngineError};
    use ink_engines::{DownloadError, EngineRow, Loader, ModelDir, Registry};
    use ink_pipeline::update::ModelInstaller;
    use serde_json::Value;

    use super::{Core, Model, Parts};

    struct NoModels;

    impl Loader<Model> for NoModels {
        fn load(&self, row: &EngineRow) -> Result<Model, EngineError> {
            Err(EngineError::ModelMissing(row.id.clone()))
        }
    }

    impl ModelInstaller for NoModels {
        fn install(&self, _: &EngineRow, _: &CancelToken) -> Result<(), DownloadError> {
            Ok(())
        }
    }

    pub(crate) fn core(
        clock: Arc<dyn Clock>,
        data_dir: std::path::PathBuf,
    ) -> (Core, Arc<Mutex<Vec<Value>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let parts = Parts {
            store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
            clock,
            registry: Registry::new(Vec::new()).unwrap(),
            models: ModelDir::new(data_dir.join("models")),
            loader: Arc::new(NoModels),
            installer: Arc::new(NoModels),
            data_dir,
        };
        let core = Core::start(
            parts,
            Box::new(move |e| sink.lock().unwrap().push(serde_json::from_str(e).unwrap())),
        )
        .unwrap();
        (core, events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_needs_an_absolute_data_dir_and_knows_its_fields() {
        let dir = std::env::temp_dir();
        let json = serde_json::json!({"data_dir": dir}).to_string();
        let c = Config::parse(&json).unwrap();
        assert_eq!(c.models_dir, dir.join("models"));
        assert_eq!(c.log_level, log::LevelFilter::Info);
        assert!(Config::parse(r#"{"data_dir":"relative"}"#).is_err());
        assert!(Config::parse("{}").is_err());
        let extra = serde_json::json!({"data_dir": dir, "colour": "blue"}).to_string();
        assert!(Config::parse(&extra).is_err());
    }

    #[test]
    fn commands_parse_or_say_why_not() {
        let e =
            parse_command(r#"{"cmd":"model.update","model":"a","next":"b","id":"c1"}"#).unwrap();
        assert_eq!(e.id.as_deref(), Some("c1"));
        assert_eq!(
            e.command,
            Command::ModelUpdate {
                id: "a".into(),
                next: "b".into()
            }
        );
        let e =
            parse_command(r#"{"cmd":"replay_meeting","mic":"/m.wav","pacing":"fast","id":"7"}"#)
                .unwrap();
        assert_eq!(e.id.as_deref(), Some("7"));
        assert!(matches!(
            e.command,
            Command::ReplayMeeting(Replay { fast: true, .. })
        ));
        for bad in [
            "not json",
            // A stray field is refused, as in the config: a typo must not be silently ignored.
            r#"{"cmd":"model.update","model":"a","next":"b","path":"/tmp/x"}"#,
            r#"{"cmd":"model.warm","job":"dictation_final","jobs":"meeting_final"}"#,
            r#"{"cmd":"replay_meeting","mic":"/m.wav","speed":"fast"}"#,
            r#"{"cmd":"engine.unregister","engine":"e","force":true}"#,
            r#"{"cmd":"launch"}"#,
            r#"{"cmd":"launch","x":1}"#,
            r#"{"cmd":"model.warm","job":"typing"}"#,
            r#"{"cmd":"replay_meeting"}"#,
            r#"{"cmd":"replay_meeting","mic":"/m.wav","pacing":"slow"}"#,
            r#"{"cmd":"engine.unregister","engine":3}"#,
            r#"{"cmd":"model.update","model":"a","next":"b","id":7}"#,
        ] {
            assert!(parse_command(bad).is_err(), "{bad}");
        }
    }
}
