//! Dictation, live: the keys, the mic, the dictation worker and the engine's warm-up.
//!
//! ```text
//!  dictation key ─tap─► key sink ──┬─► mailbox ─► ink-dictation: DictationChain ─► insert, store
//!  edit key      ─tap─► edit sink ─┤      ▲                 │
//!                                  │      │ 16 kHz mic      └─ events ─► hub; a start ─► ink-warm
//!                    a press ─wake─┴► ink-voice: opens the mic, pumps it (ring ─► MicPath), bands
//!                                     to the ink while a take is open, lets the mic go when idle
//! ```
//!
//! The shell turns it on with `dictation.enable` (and off with `dictation.disable`); both run on
//! the queries thread, as do the settings it reads (`dictation.key`, `dictation.edit_key`,
//! `dictation.polish`, the modes and the dictionary): a changed setting rebinds the keys and hands
//! the chain its new settings at once. Every outcome is an event: `dictation.ready` names the keys
//! held, `dictation.off` says why dictation is not live.
//!
//! **The mic (pre-roll).** It opens at the first press, not at launch, and then stays open, so each
//! take keeps the 300 ms said before its press (`ink-audio`'s take recorder). After [`MIC_IDLE`]
//! with no take it is let go of, as Inkwell 0.2 did (an open input kept the Mac from idle sleep
//! and showed the microphone indicator all day); the chain is told, so nothing heard before is a
//! later take's lead. The first take after that starts when the device does, without a lead.
//!
//! **Threads.** `ink-voice` exists while dictation is enabled; it blocks on its channel while the
//! mic is closed and wakes every [`PUMP_INTERVAL`] while it is open. The key sinks run on the
//! platform's tap thread and only enqueue. `ink-warm` is the engine warmer's
//! ([`EngineWarmer`]).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_audio::{BandAnalyzer, Bands, capture_ring};
use ink_core::{
    AsrEvent, AudioSource, CaptureControl, Channel, EngineError, EngineInfo, EngineStream,
    EventSink, FocusReader, HotkeyBinding, HotkeyEvent, HotkeySource, Job, Llm, Permission,
    PlatformError, Store, StreamingEngine, TextInserter,
};
use ink_engines::{ExternalEngine, ModelDir, Route};
use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
use ink_pipeline::consent::{Feature, LlmConsent};
use ink_pipeline::dictionary::Dictionary;
use ink_pipeline::events::DictationEvent;
use ink_pipeline::mic::MicPath;
use ink_pipeline::modes::{Mode, ModeStore};
use ink_pipeline::warm::{EngineWarmer, WARM_AFTER_IDLE, WarmHandle};
use serde_json::Value;

use crate::dictation::{Block, DictationInbox, DictationWorker, Input};
use crate::events::{self, event};
use crate::gate::Routed;
use crate::hub::Events;
use crate::llms::PolishModel;
use crate::mailbox::{DEFAULT_AUDIO_CAPACITY, Pushed};
use crate::meeting::PUMP_INTERVAL;
use crate::runtime::Shared;

/// How long the mic stays open after a take before it is let go of: one minute, so the
/// microphone indicator goes out soon after the user stops dictating.
pub const MIC_IDLE: Duration = Duration::from_secs(60);

/// The store setting naming the dictation key.
pub const KEY_SETTING: &str = "dictation.key";
/// The store setting naming the voice-edit key, or `off`.
pub const EDIT_KEY_SETTING: &str = "dictation.edit_key";
/// The store setting holding the "Polish my words" switch (`on` or `off`). Only `consent.allow`
/// turns it on ([`crate::consent`]).
pub const POLISH_SETTING: &str = "dictation.polish";
/// The settings a running dictation reads: a change to one reaches it at once.
pub const DICTATION_SETTINGS: &[&str] = &[
    KEY_SETTING,
    EDIT_KEY_SETTING,
    POLISH_SETTING,
    // The consents (the core's own settings: no shell writes them through setting.set).
    Feature::Polish.setting_key(),
    Feature::Edit.setting_key(),
];
/// The dictation key until the user picks another: Fn on the Mac.
#[cfg(not(windows))]
pub const DEFAULT_KEY: &str = "fn";
/// The dictation key until the user picks another: right Ctrl on Windows, where Fn never reaches
/// the OS (ink-platform-win's `DEFAULT_BINDING`).
#[cfg(windows)]
pub const DEFAULT_KEY: &str = ink_platform_win::hotkey::DEFAULT_BINDING;
/// The named keys: the modifiers held on their own that either OS knows by these tokens. One list
/// for both platforms: the Mac holds `fn`, `right_option` and `right_command`, Windows
/// `right_alt` and `right_win`. The settings take any of them on either OS, and a platform that
/// cannot hold one refuses it when dictation binds it (`dictation.off` with `key_refused`), so a
/// key stored on the other OS is said, never quietly swapped. Any other key the platform can
/// watch (a chord, a function key) is taken too: [`crate::hotkey`] has the rule.
pub const KEYS: &[&str] = &[
    "fn",
    "right_option",
    "right_command",
    "right_control",
    "right_shift",
    "right_alt",
    "right_win",
];

/// The platform's pieces dictation needs. The Mac's are made by [`VoicePlatform::mac`], Windows'
/// by [`VoicePlatform::win`]; tests pass mocks. Two hotkey sources, one per key: a source holds one binding.
#[derive(Clone)]
pub struct VoicePlatform {
    /// Opens the mic.
    pub capture: Arc<dyn CaptureControl>,
    /// The dictation key.
    pub keys: Arc<dyn HotkeySource>,
    /// The voice-edit key.
    pub edit_keys: Arc<dyn HotkeySource>,
    /// Text insertion.
    pub inserter: Arc<dyn TextInserter>,
    /// The frontmost app and the selection.
    pub focus: Arc<dyn FocusReader>,
}

impl VoicePlatform {
    /// The Mac's: capture, two event taps, insertion and focus, on the platform clock.
    #[cfg(target_os = "macos")]
    pub fn mac() -> Result<Self, String> {
        let clock = ink_platform_mac::MacClock::new().map_err(|e| e.to_string())?;
        Ok(Self {
            capture: Arc::new(ink_platform_mac::MacCapture::new(clock)),
            keys: Arc::new(ink_platform_mac::MacHotkeySource::new(clock)),
            edit_keys: Arc::new(ink_platform_mac::MacHotkeySource::new(clock)),
            inserter: Arc::new(ink_platform_mac::MacTextInserter::new()),
            focus: Arc::new(ink_platform_mac::MacFocusReader::new()),
        })
    }

    /// Windows': WASAPI capture, two low-level keyboard hooks, insertion and focus, on the
    /// performance counter (the timebase the core's clock uses there too). Nothing is opened or
    /// hooked until dictation starts and binds its keys.
    #[cfg(windows)]
    pub fn win() -> Result<Self, String> {
        let clock = ink_platform_win::WinClock::new().map_err(|e| e.to_string())?;
        Ok(Self {
            capture: Arc::new(ink_platform_win::WinCapture::new(clock)),
            keys: Arc::new(ink_platform_win::WinHotkeySource::new(clock)),
            edit_keys: Arc::new(ink_platform_win::WinHotkeySource::new(clock)),
            inserter: Arc::new(ink_platform_win::WinTextInserter::new()),
            focus: Arc::new(ink_platform_win::WinFocusReader::new()),
        })
    }
}

/// Where dictation is: the platform it may use (set once by the C ABI), and the running parts.
#[derive(Default)]
pub struct VoiceSlot {
    platform: Option<VoicePlatform>,
    running: Option<Voice>,
}

impl VoiceSlot {
    /// Sets the platform (before any `dictation.enable`).
    pub fn set_platform(&mut self, platform: VoicePlatform) {
        self.platform = Some(platform);
    }
}

fn lock(m: &Mutex<VoiceSlot>) -> MutexGuard<'_, VoiceSlot> {
    // Taken by the queries thread and shutdown, and briefly by the command thread after an
    // install ([`vad_installed`]); every step leaves the slot consistent.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why dictation is off, as `dictation.off` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffReason {
    /// The shell turned it off.
    Disabled,
    /// The key cannot be held without Accessibility.
    NeedsAccessibility,
    /// The platform refused the key.
    KeyRefused,
    /// This build has no platform for dictation.
    Unsupported,
    /// The dictation worker stopped after repeated failures.
    WorkerStopped,
    /// Something else failed (a thread did not start).
    Failed,
}

impl OffReason {
    fn name(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NeedsAccessibility => "needs_accessibility",
            Self::KeyRefused => "key_refused",
            Self::Unsupported => "unsupported",
            Self::WorkerStopped => "worker_stopped",
            Self::Failed => "failed",
        }
    }
}

/// `dictation.off`.
pub fn off_event(reason: OffReason, message: Option<&str>, reference: Option<&str>) -> Value {
    event(
        "dictation.off",
        &[
            ("reason", Some(reason.name().into())),
            ("message", message.map(Into::into)),
            ("ref", reference.map(Into::into)),
        ],
    )
}

/// What `dictation.ready` reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    /// The dictation key held.
    pub key: String,
    /// The edit key held, if any.
    pub edit_key: Option<String>,
    /// Why the edit key set could not be held.
    pub edit_key_error: Option<String>,
    /// Settings that could not be read (dictation runs with defaults for them), by name.
    pub unreadable: Vec<&'static str>,
}

fn ready_event(ready: &Ready, reference: Option<&str>) -> Value {
    event(
        "dictation.ready",
        &[
            ("key", Some(ready.key.as_str().into())),
            ("edit_key", ready.edit_key.as_deref().map(Into::into)),
            (
                "edit_key_error",
                ready.edit_key_error.as_deref().map(Into::into),
            ),
            (
                "settings_error",
                (!ready.unreadable.is_empty())
                    .then(|| format!("couldn't read {}", ready.unreadable.join(", ")).into()),
            ),
            ("ref", reference.map(Into::into)),
        ],
    )
}

/// **Queries thread.** `dictation.enable`: starts dictation, or, when it runs, reads its settings
/// again and binds its keys again (after Accessibility was granted, say). Answers
/// `dictation.ready` or `dictation.off`.
pub fn enable(
    shared: &Arc<Shared>,
    models: &ModelDir,
    utc_offset_minutes: Option<i32>,
    reference: Option<&str>,
) {
    let mut slot = lock(&shared.voice);
    let Some(platform) = slot.platform.clone() else {
        shared.events.emit(off_event(
            OffReason::Unsupported,
            Some("this build has no keys or microphone for dictation on this platform"),
            reference,
        ));
        return;
    };
    if let Some(voice) = slot.running.as_mut() {
        if let Some(offset) = utc_offset_minutes {
            voice.utc_offset_minutes = offset;
        }
        match voice.rebind(shared, true) {
            Ok(ready) => {
                shared.events.emit(ready_event(&ready, reference));
                return;
            }
            Err(Rebind::Off(reason, message)) => {
                if let Some(voice) = slot.running.take() {
                    voice.stop();
                }
                shared
                    .events
                    .emit(off_event(reason, Some(&message), reference));
                return;
            }
            // The worker has stopped: start again from nothing.
            Err(Rebind::WorkerGone) => {
                if let Some(voice) = slot.running.take() {
                    voice.stop();
                }
            }
        }
    }
    match Voice::start(shared, models, platform, utc_offset_minutes.unwrap_or(0)) {
        Ok((voice, ready)) => {
            slot.running = Some(voice);
            shared.events.emit(ready_event(&ready, reference));
        }
        Err((reason, message)) => {
            shared
                .events
                .emit(off_event(reason, Some(&message), reference));
        }
    }
}

/// **Queries thread.** `dictation.disable`: stops dictation (the keys are let go of, the mic
/// closed). Answers `dictation.off` with reason `disabled`.
pub fn disable(shared: &Shared, reference: Option<&str>) {
    let running = lock(&shared.voice).running.take();
    if let Some(voice) = running {
        voice.stop();
    }
    shared
        .events
        .emit(off_event(OffReason::Disabled, None, reference));
}

/// **Queries thread.** A dictation setting changed: a running dictation takes it now (keys
/// rebound only when they changed), and says so with `dictation.ready` (or `dictation.off`).
pub fn settings_changed(shared: &Shared) {
    let mut slot = lock(&shared.voice);
    let Some(voice) = slot.running.as_mut() else {
        return;
    };
    match voice.rebind(shared, false) {
        Ok(ready) => shared.events.emit(ready_event(&ready, None)),
        Err(Rebind::Off(reason, message)) => {
            if let Some(voice) = slot.running.take() {
                voice.stop();
            }
            shared.events.emit(off_event(reason, Some(&message), None));
        }
        Err(Rebind::WorkerGone) => {
            if let Some(voice) = slot.running.take() {
                voice.stop();
            }
            shared.events.emit(off_event(
                OffReason::WorkerStopped,
                Some("dictation stopped after repeated failures; turn it on again"),
                None,
            ));
        }
    }
}

/// **Command thread**, when a model install ends. The voice detector just installed reaches a
/// running dictation now, not at its next start: its VAD is resolved again ([`crate::vad`]) and
/// queued to the chain, which tells the shell when that changes (`dictation.voice_detection`).
/// The slot's lock is let go of before the model is read.
pub fn vad_installed(shared: &Shared) {
    let Some(inbox) = lock(&shared.voice)
        .running
        .as_ref()
        .map(|voice| voice.inbox.clone())
    else {
        return;
    };
    let vad = crate::vad::installed(shared, &shared.models);
    if inbox.send(Input::SetVad(vad)).is_err() {
        log::info!(
            "voice detection was installed after dictation stopped; its next start loads it"
        );
    }
}

/// **Shutdown.** Stops dictation, joining every thread that can hold an engine.
pub fn shutdown(shared: &Shared) {
    let running = lock(&shared.voice).running.take();
    if let Some(voice) = running {
        voice.stop();
    }
}

/// The modes dictation writes in with no modes stored: the built-in default, polished whenever
/// the user's switch is on (so "Polish my words" alone decides on a fresh install). The switch
/// turns on only with the user's consent, and polish runs only where that consent covers
/// ([`LlmConsent`]).
pub fn default_modes() -> ModeStore {
    ModeStore {
        default_id: Mode::builtin_default().id,
        modes: vec![Mode {
            polish_enabled: true,
            ..Mode::builtin_default()
        }],
    }
}

/// The user's modes, as `modes.list` shows them ([`crate::modes::load`]): the stored document,
/// else the imported one, else [`default_modes`]. A document that cannot be read is an error,
/// never quietly the default.
pub fn load_modes(store: &dyn Store) -> Result<ModeStore, String> {
    crate::modes::load(store).map(|loaded| loaded.modes)
}

/// The dictionary: the one saved in 1.0, else the one imported from 0.2 (an array of
/// `{find, replace}`), else empty.
fn load_dictionary(store: &dyn Store) -> Result<Dictionary, String> {
    if store
        .setting(ink_pipeline::dictionary::SETTING_KEY)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Dictionary::load(store).map_err(|e| e.to_string());
    }
    match store
        .setting(ink_store::import::DICTIONARY_KEY)
        .map_err(|e| e.to_string())?
    {
        None => Ok(Dictionary::default()),
        Some(entries) => {
            let wrapped = format!("{{\"entries\":{entries}}}");
            Dictionary::from_json(&wrapped).map_err(|e| e.to_string())
        }
    }
}

/// Everything dictation reads from the store.
struct Loaded {
    key: String,
    edit_key: Option<String>,
    settings: DictationSettings,
    unreadable: Vec<&'static str>,
}

fn load(store: &dyn Store, utc_offset_minutes: i32) -> Loaded {
    let mut unreadable = Vec::new();
    let mut read = |key: &str, name: &'static str| match store.setting(key) {
        Ok(v) => v,
        Err(e) => {
            log::error!("dictation: {name} could not be read: {e}");
            unreadable.push(name);
            None
        }
    };
    // As stored, never quietly the default: a key the platform cannot hold is refused by name
    // when it binds. The edit key is off (the default) unless one is set: a held key would
    // otherwise read the selection in every app.
    let key = read(KEY_SETTING, "the dictation key")
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_KEY.to_owned());
    let edit_key =
        read(EDIT_KEY_SETTING, "the edit key").filter(|k| k != "off" && !k.trim().is_empty());
    let polish_wish = read(POLISH_SETTING, "the polish switch").as_deref() == Some("on");
    // Unreadable consent is no consent: the feature fails closed, and the shell hears why.
    let mut consent = |feature: Feature, name: &'static str| {
        let stored = match store.setting(feature.setting_key()) {
            Ok(v) => v,
            Err(e) => {
                log::error!("dictation: {name} could not be read: {e}");
                unreadable.push(name);
                return None;
            }
        };
        LlmConsent::from_setting(stored.as_deref()).unwrap_or_else(|e| {
            log::error!("dictation: {e} ({name}); nothing is sent until the user agrees again");
            unreadable.push(name);
            None
        })
    };
    let polish_consent = consent(Feature::Polish, "the polish consent");
    let edit_consent = consent(Feature::Edit, "the voice edit consent");
    let modes = load_modes(store).unwrap_or_else(|e| {
        log::error!("dictation: the modes could not be read: {e}");
        unreadable.push("the modes");
        default_modes()
    });
    let dictionary = load_dictionary(store).unwrap_or_else(|e| {
        log::error!("dictation: the dictionary could not be read: {e}");
        unreadable.push("the dictionary");
        Dictionary::default()
    });
    let mut loaded = Loaded {
        key,
        edit_key,
        settings: DictationSettings {
            modes,
            dictionary,
            polish_wish,
            polish_consent,
            edit_consent,
            utc_offset_minutes,
            ..DictationSettings::default()
        },
        unreadable,
    };
    load_phrases(store, &mut loaded);
    loaded
}

/// The snippets and voice commands, as Settings lists them ([`phrases`](crate::phrases)): the
/// user's own, else the 0.2 import's, else the defaults. One that cannot be read is named in
/// `dictation.ready`, and dictation runs without it (no snippets; commands off).
fn load_phrases(store: &dyn Store, loaded: &mut Loaded) {
    match crate::phrases::load_snippets(store) {
        Ok((snippets, _)) => loaded.settings.snippets = snippets,
        Err(e) => {
            log::error!("dictation: the snippets could not be read: {e}");
            loaded.unreadable.push("the snippets");
        }
    }
    match crate::phrases::load_commands(store) {
        Ok((commands, _)) => loaded.settings.commands = commands,
        Err(e) => {
            log::error!("dictation: the voice commands could not be read: {e}");
            loaded.unreadable.push("the voice commands");
        }
    }
}

/// Takes in progress and the latest dictation activity, for letting the mic go when idle and for
/// the ink's bands.
#[derive(Default)]
struct Activity {
    busy: AtomicBool,
    /// Host time of the latest press or take ending.
    last_ns: AtomicU64,
}

impl Activity {
    fn touch(&self, at_ns: u64) {
        self.last_ns.fetch_max(at_ns, Ordering::AcqRel);
    }
}

/// Messages for `ink-voice`, on a bounded channel ([`ctl_channel`]): a press only tries to add a
/// wake ([`wake`]), so the tap's thread never waits or allocates for it; the rest (the worker gone,
/// a stop) are sent from worker threads and may wait the few ms the mic thread takes to drain.
enum Ctl {
    /// A press: open the mic if it is closed.
    Wake,
    /// The worker stopped for good: let go of the keys.
    WorkerGone,
    /// End the thread.
    Stop,
}

/// How many messages the mic thread's channel holds. One pending wake is as good as many.
const CTL_CAPACITY: usize = 4;

/// The mic thread's channel: bounded, so its buffer is allocated once, here.
fn ctl_channel() -> (SyncSender<Ctl>, Receiver<Ctl>) {
    mpsc::sync_channel(CTL_CAPACITY)
}

/// **Callback thread** (the tap's). Asks the mic thread to open the mic: never waits and never
/// allocates. When the channel is full, a wake is already pending (or a stop, which ends it all),
/// so this one is not needed.
fn wake(ctl: &SyncSender<Ctl>) {
    let _ = ctl.try_send(Ctl::Wake);
}

/// Why a rebind did not leave dictation running.
enum Rebind {
    Off(OffReason, String),
    WorkerGone,
}

/// The live-partials engine as the router picks it at each take (on the Mac, FluidAudio's
/// Parakeet, registered by the shell; on Windows, Parakeet on sherpa-onnx, a registry model).
struct RoutedLive {
    shared: Arc<Shared>,
}

impl StreamingEngine for RoutedLive {
    fn info(&self) -> EngineInfo {
        match self.shared.router.route(Job::LivePartials) {
            Ok(Route::External { engine, .. }) => engine.info(),
            Ok(Route::Model(row)) => row.info(),
            Err(_) => EngineInfo {
                id: "none".into(),
                jobs: vec![Job::LivePartials],
                licence: String::new(),
            },
        }
    }

    fn open_stream(
        &self,
        channel: Channel,
        events: EventSink<AsrEvent>,
    ) -> Result<Box<dyn EngineStream>, EngineError> {
        match self.shared.router.route(Job::LivePartials)? {
            Route::External {
                engine: ExternalEngine::Streaming(engine),
                ..
            } => engine.open_stream(channel, events),
            Route::Model(row) => {
                crate::gate::live_model(&self.shared, &row).open_stream(channel, events)
            }
            other => Err(EngineError::Failed(format!(
                "live partials route to {}, which is not a streaming engine",
                other.id()
            ))),
        }
    }
}

/// Dictation, running. See the module docs.
pub struct Voice {
    platform: VoicePlatform,
    worker: Option<DictationWorker>,
    inbox: DictationInbox,
    warmer: Option<EngineWarmer>,
    ctl: SyncSender<Ctl>,
    controller: Option<JoinHandle<()>>,
    activity: Arc<Activity>,
    key: Option<String>,
    edit_key: Option<String>,
    utc_offset_minutes: i32,
}

impl Voice {
    fn start(
        shared: &Arc<Shared>,
        models: &ModelDir,
        platform: VoicePlatform,
        utc_offset_minutes: i32,
    ) -> Result<(Self, Ready), (OffReason, String)> {
        let failed = |what: &str, e: std::io::Error| (OffReason::Failed, format!("{what}: {e}"));
        let loaded = load(shared.store.as_ref(), utc_offset_minutes);
        let warmer = EngineWarmer::start(
            Arc::new(Routed::new(shared.clone(), Job::DictationFinal)),
            shared.clock.clone(),
            WARM_AFTER_IDLE,
        )
        .map_err(|e| failed("the warm-up thread did not start", e))?;
        let (ctl, rx) = ctl_channel();
        let activity = Arc::new(Activity::default());
        let sink = chain_sink(
            shared.events.clone(),
            shared.clock.clone(),
            warmer.handle(),
            activity.clone(),
            ctl.clone(),
        );
        let mut chain = DictationChain::new(
            Services {
                engine: warmer.engine(),
                store: shared.store.clone(),
                inserter: platform.inserter.clone(),
                focus: platform.focus.clone(),
                clock: shared.clock.clone(),
                // Polish and voice edit both call through it, behind the local-only switch.
                llm: Some(Arc::new(PolishModel::new(
                    shared.llms.clone(),
                    shared.local_only.clone(),
                )) as Arc<dyn Llm>),
            },
            loaded.settings,
            crate::vad::installed(shared, models),
            sink,
        );
        chain.set_live(Some(Arc::new(RoutedLive {
            shared: shared.clone(),
        })));
        // A mode's own language model, found at each take, behind the same local-only switch.
        chain.set_mode_models(Some(crate::llms::mode_models(
            shared.llms.clone(),
            shared.local_only.clone(),
        )));
        let worker = DictationWorker::spawn(
            chain,
            shared.clock.clone(),
            shared.events.clone(),
            DEFAULT_AUDIO_CAPACITY,
        )
        .map_err(|e| failed("the dictation worker did not start", e))?;
        let inbox = worker.inbox();
        let controller = {
            let (shared, inbox, activity) = (shared.clone(), inbox.clone(), activity.clone());
            let platform = platform.clone();
            thread::Builder::new()
                .name("ink-voice".into())
                .spawn(move || controller(&shared, &platform, &inbox, &rx, &activity))
                .map_err(|e| failed("the mic thread did not start", e))?
        };
        let mut voice = Self {
            platform,
            worker: Some(worker),
            inbox,
            warmer: Some(warmer),
            ctl,
            controller: Some(controller),
            activity,
            key: None,
            edit_key: None,
            utc_offset_minutes,
        };
        match voice.bind(&loaded.key, loaded.edit_key.as_deref(), true) {
            Ok(mut ready) => {
                ready.unreadable = loaded.unreadable;
                Ok((voice, ready))
            }
            Err(off) => {
                voice.stop();
                Err(off)
            }
        }
    }

    /// Reads the settings again, hands them to the chain, and binds the keys (only those that
    /// changed, unless `force`).
    fn rebind(&mut self, shared: &Shared, force: bool) -> Result<Ready, Rebind> {
        let loaded = load(shared.store.as_ref(), self.utc_offset_minutes);
        self.inbox
            .send(Input::SetSettings(Box::new(loaded.settings)))
            .map_err(|_| Rebind::WorkerGone)?;
        let mut ready = self
            .bind(&loaded.key, loaded.edit_key.as_deref(), force)
            .map_err(|(reason, message)| Rebind::Off(reason, message))?;
        ready.unreadable = loaded.unreadable;
        Ok(ready)
    }

    /// Holds `key` (and `edit_key`), replacing what was held. The dictation key must bind; the edit
    /// key's failure is reported and dictation goes on without it. Both are taken in their one
    /// spelling ([`crate::hotkey::spelling`]), so the edit key is never the dictation key under
    /// another name.
    fn bind(
        &mut self,
        key: &str,
        edit_key: Option<&str>,
        force: bool,
    ) -> Result<Ready, (OffReason, String)> {
        let key = crate::hotkey::spelling(key);
        let key = key.as_str();
        let edit_key = edit_key.map(crate::hotkey::spelling);
        let edit_key = edit_key.as_deref();
        if force || self.key.as_deref() != Some(key) {
            // The edit key taking the dictation key's place is let go of first, so two taps never
            // hold one key.
            if self.edit_key.as_deref() == Some(key) {
                self.platform.edit_keys.stop();
                self.edit_key = None;
            }
            self.key = None;
            let sink = key_sink(
                self.inbox.clone(),
                self.ctl.clone(),
                self.activity.clone(),
                false,
            );
            if let Err(e) = self
                .platform
                .keys
                .start(&HotkeyBinding(key.to_owned()), sink)
            {
                let reason = match e {
                    PlatformError::PermissionDenied(Permission::Accessibility) => {
                        OffReason::NeedsAccessibility
                    }
                    _ => OffReason::KeyRefused,
                };
                log::warn!("dictation: the key {key} could not be held: {e}");
                return Err((reason, refusal_words(&e)));
            }
            self.key = Some(key.to_owned());
        }
        let mut edit_key_error = None;
        let wanted = edit_key.filter(|e| *e != key);
        if edit_key.is_some() && wanted.is_none() {
            edit_key_error = Some("the edit key is the dictation key; pick another".to_owned());
        }
        if force || self.edit_key.as_deref() != wanted {
            self.platform.edit_keys.stop();
            self.edit_key = None;
            if let Some(edit) = wanted {
                let sink = key_sink(
                    self.inbox.clone(),
                    self.ctl.clone(),
                    self.activity.clone(),
                    true,
                );
                match self
                    .platform
                    .edit_keys
                    .start(&HotkeyBinding(edit.to_owned()), sink)
                {
                    Ok(()) => self.edit_key = Some(edit.to_owned()),
                    Err(e) => {
                        log::warn!("dictation: the edit key {edit} could not be held: {e}");
                        edit_key_error = Some(refusal_words(&e));
                    }
                }
            }
        }
        Ok(Ready {
            key: key.to_owned(),
            edit_key: self.edit_key.clone(),
            edit_key_error,
            unreadable: Vec::new(),
        })
    }

    /// Lets go of the keys, closes the mic, and joins every thread, the worker last but one and
    /// the warmer last (the worker's chain holds a handle to it).
    pub fn stop(mut self) {
        self.platform.keys.stop();
        self.platform.edit_keys.stop();
        let _ = self.ctl.send(Ctl::Stop);
        if let Some(controller) = self.controller.take()
            && controller.join().is_err()
        {
            log::error!("the mic thread panicked");
        }
        if let Some(worker) = self.worker.take()
            && worker.stop().is_err()
        {
            log::error!("the dictation worker panicked outside its boundary");
        }
        if let Some(warmer) = self.warmer.take() {
            warmer.stop();
        }
    }
}

/// Why a key could not be held, as the shell shows it: a platform's refusal in its own words
/// (the reason `hotkey.check` gives, without Display's "not supported here:"), anything else as
/// it is.
fn refusal_words(e: &PlatformError) -> String {
    match e {
        PlatformError::Unsupported(why) => (*why).to_owned(),
        other => other.to_string(),
    }
}

/// The sink for one key: queues the edge for the chain, and on a press wakes the mic.
///
/// **Callback thread** (the event tap's, which macOS disables if it is slow): a short lock is
/// allowed (the mailbox's, never held across a wait), never a wait, and no allocation per event:
/// the wake is a `try_send` on the bounded channel, the activity an atomic, and the mailbox's queue
/// was grown by the audio before and is kept.
fn key_sink(
    inbox: DictationInbox,
    ctl: SyncSender<Ctl>,
    activity: Arc<Activity>,
    edit: bool,
) -> EventSink<HotkeyEvent> {
    Arc::new(move |e| {
        if let HotkeyEvent::Pressed { at_ns } = e {
            activity.touch(at_ns);
            wake(&ctl);
        }
        // After the worker stopped, refused (the keys are let go of then).
        let _ = inbox.send(if edit {
            Input::EditHotkey(e)
        } else {
            Input::Hotkey(e)
        });
    })
}

/// The chain's events: to the shell, and to what follows the takes (the warm-up at a start; the
/// activity the mic's idle time is measured from).
fn chain_sink(
    events: Events,
    clock: Arc<dyn ink_core::Clock>,
    warm: WarmHandle,
    activity: Arc<Activity>,
    ctl: SyncSender<Ctl>,
) -> EventSink<DictationEvent> {
    Arc::new(move |e| {
        match &e {
            DictationEvent::Started { .. } => {
                activity.busy.store(true, Ordering::Release);
                warm.key_down();
            }
            DictationEvent::Inserted { .. }
            | DictationEvent::Discarded(_)
            | DictationEvent::Failed(_)
            | DictationEvent::Command(_)
            | DictationEvent::ShortPressIgnored
            | DictationEvent::Edited { .. }
            | DictationEvent::EditFailed(_)
            | DictationEvent::WorkerFailed { .. } => {
                activity.busy.store(false, Ordering::Release);
                activity.touch(clock.now_ns());
            }
            _ => {}
        }
        if matches!(e, DictationEvent::WorkerFailed { recovered: false }) {
            let _ = ctl.send(Ctl::WorkerGone);
        }
        events.emit(events::dictation(&e));
    })
}

/// `dictation.mic_failed`.
fn mic_failed(message: &str) -> Value {
    event("dictation.mic_failed", &[("message", Some(message.into()))])
}

/// The open mic: its source, its ring, and its path to 16 kHz.
struct OpenMic {
    source: Box<dyn AudioSource>,
    ring: ink_audio::CaptureConsumer,
    path: MicPath,
}

fn open_mic(capture: &dyn CaptureControl) -> Result<OpenMic, String> {
    let mut source = capture.open_mic(None).map_err(|e| e.to_string())?;
    let format = source.format();
    let (producer, ring) =
        capture_ring(format, ink_audio::DEFAULT_RING_DURATION).map_err(|e| e.to_string())?;
    let path = MicPath::new(format).map_err(|e| e.to_string())?;
    source
        .start(Box::new(producer))
        .map_err(|e| e.to_string())?;
    Ok(OpenMic { source, ring, path })
}

/// How a stretch of open mic ended.
enum Closed {
    Idle,
    Failed,
    Stop,
}

/// `ink-voice`: waits for a press, opens the mic, pumps it until it is idle, closes it.
fn controller(
    shared: &Shared,
    platform: &VoicePlatform,
    inbox: &DictationInbox,
    rx: &Receiver<Ctl>,
    activity: &Activity,
) {
    loop {
        match rx.recv() {
            Ok(Ctl::Wake) => {}
            Ok(Ctl::WorkerGone) => {
                worker_gone(shared, platform);
                continue;
            }
            Ok(Ctl::Stop) | Err(_) => return,
        }
        let mut mic = match open_mic(platform.capture.as_ref()) {
            Ok(mic) => mic,
            Err(message) => {
                log::warn!("dictation: the mic could not be opened: {message}");
                // The press waiting for audio is dropped (reported as cancelled). Queued before the
                // failure is said, so a press made in answer to it comes after it and is not the
                // one cancelled.
                let _ = inbox.send(Input::StreamEnded);
                shared.events.emit(mic_failed(&message));
                continue;
            }
        };
        let closed = pump(shared, platform, inbox, rx, activity, &mut mic);
        if let Err(e) = mic.source.stop() {
            // The device may still be held (the microphone indicator stays on); the next press opens
            // a new stream either way. The error names the device, never audio.
            log::warn!("dictation: the mic did not stop cleanly: {e}");
        }
        match closed {
            Closed::Idle | Closed::Stop => {
                let _ = inbox.send(Input::MicClosed);
            }
            // A take in progress ends with what it has; nothing heard leads the next.
            Closed::Failed => {
                let _ = inbox.send(Input::StreamEnded);
                let _ = inbox.send(Input::MicClosed);
            }
        }
        // Idle is a still frame (architecture rule 9).
        shared.publish_bands(Bands::default());
        if matches!(closed, Closed::Stop) {
            return;
        }
    }
}

/// The worker stopped for good: let go of the keys (presses would reach nothing) and say so.
fn worker_gone(shared: &Shared, platform: &VoicePlatform) {
    platform.keys.stop();
    platform.edit_keys.stop();
    shared.events.emit(off_event(
        OffReason::WorkerStopped,
        Some("dictation stopped after repeated failures; turn it on again"),
        None,
    ));
}

fn pump(
    shared: &Shared,
    platform: &VoicePlatform,
    inbox: &DictationInbox,
    rx: &Receiver<Ctl>,
    activity: &Activity,
    mic: &mut OpenMic,
) -> Closed {
    let mut analyzer = BandAnalyzer::new();
    let mut publishing = false;
    let idle_ns = u64::try_from(MIC_IDLE.as_nanos()).unwrap_or(u64::MAX);
    loop {
        match rx.recv_timeout(PUMP_INTERVAL) {
            Ok(Ctl::Wake) | Err(RecvTimeoutError::Timeout) => {}
            Ok(Ctl::WorkerGone) => {
                worker_gone(shared, platform);
                return Closed::Idle;
            }
            Ok(Ctl::Stop) | Err(RecvTimeoutError::Disconnected) => return Closed::Stop,
        }
        let busy = activity.busy.load(Ordering::Acquire);
        while let Some(captured) = mic.ring.pop() {
            let dropped = captured.dropped_frames_before;
            let out = match mic.path.push(&captured.block) {
                Ok(out) => out,
                Err(e) => {
                    log::warn!("dictation: the mic stream failed: {e}");
                    shared.events.emit(mic_failed(&e.to_string()));
                    return Closed::Failed;
                }
            };
            if out.samples.is_empty() && dropped == 0 {
                continue;
            }
            // The ink follows the voice only while a take is open; a meeting's pump publishes
            // its own.
            if busy && let Some(b) = analyzer.process(out.samples) {
                shared.publish_bands(b);
            }
            let block = Block {
                samples: out.samples.to_vec(),
                host_time_ns: out.host_time_ns,
                dropped_frames: dropped,
            };
            if let Pushed::Closed = inbox.push_audio(block) {
                // The worker has stopped; its WorkerGone follows.
                break;
            }
        }
        if publishing && !busy {
            shared.publish_bands(Bands::default());
        }
        publishing = busy;
        let last = activity.last_ns.load(Ordering::Acquire);
        if !busy && shared.clock.now_ns().saturating_sub(last) >= idle_ns {
            log::info!(
                "dictation: the mic is let go of after {} s without a take",
                MIC_IDLE.as_secs()
            );
            return Closed::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// Each OS starts on a key it can hold, and the settings accept it as the key and the edit key:
    /// Fn on the Mac, right Ctrl on Windows (Fn never reaches Windows).
    #[test]
    fn the_default_key_is_this_platforms_own() {
        assert!(KEYS.contains(&DEFAULT_KEY), "{DEFAULT_KEY}");
        assert_eq!(
            crate::hotkey::check(DEFAULT_KEY).as_deref(),
            Ok(DEFAULT_KEY)
        );
        let expected = if cfg!(windows) { "right_control" } else { "fn" };
        assert_eq!(DEFAULT_KEY, expected);
    }

    /// A refused key reads as the parser's own words, which the shell shows after "can't be used
    /// here:"; other failures keep their kind.
    #[test]
    fn a_refused_key_is_said_in_the_parsers_words() {
        assert_eq!(
            refusal_words(&PlatformError::Unsupported("Inkwell doesn't know that key")),
            "Inkwell doesn't know that key"
        );
        assert_eq!(
            refusal_words(&PlatformError::Failed("the tap did not start".into())),
            "platform call failed: the tap did not start"
        );
    }

    /// Every key the Windows hook holds on its own can be chosen (the Windows shell offers them).
    #[cfg(windows)]
    #[test]
    fn every_windows_key_can_be_chosen() {
        for key in ink_platform_win::hotkey::KEYS {
            assert!(KEYS.contains(key), "{key}");
            assert_eq!(crate::hotkey::stored_value(key).as_deref(), Ok(*key));
        }
    }

    /// Windows' dictation platform is made without touching a device or installing a hook (those
    /// wait for `dictation.enable`), so it is made at every launch, with or without a desktop.
    #[cfg(windows)]
    #[test]
    fn the_windows_platform_is_made_without_opening_anything() {
        let platform = VoicePlatform::win().expect("the Windows platform");
        // Stopping keys that were never bound is a no-op.
        platform.keys.stop();
        platform.edit_keys.stop();
    }

    /// The tap's thread never waits on the mic thread: a burst of presses with nobody receiving
    /// returns at once, the pending wakes stay bounded, and a stop still gets through once the
    /// mic thread drains them.
    #[test]
    fn a_press_wakes_the_mic_without_ever_waiting() {
        let (tx, rx) = ctl_channel();
        let started = Instant::now();
        for _ in 0..10_000 {
            wake(&tx);
        }
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        let stopper = std::thread::spawn(move || tx.send(Ctl::Stop).is_ok());
        let mut wakes = 0;
        loop {
            match rx.recv() {
                Ok(Ctl::Wake) => wakes += 1,
                Ok(Ctl::Stop) => break,
                Ok(Ctl::WorkerGone) | Err(_) => panic!("unexpected"),
            }
        }
        assert!(stopper.join().unwrap());
        assert!((1..=CTL_CAPACITY).contains(&wakes), "{wakes} wakes pending");
    }
}
