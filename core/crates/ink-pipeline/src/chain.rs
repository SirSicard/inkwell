//! The dictation chain: Inkwell 0.2's canonical pipeline, stages 1–11, on the 1.0 traits.
//!
//! | Stage | Here |
//! |---|---|
//! | 1 capture | the platform's mic source and `ink-audio`'s ring, pumped by the caller |
//! | 2 normalise and resample to 16 kHz mono | [`MicPath`](crate::mic::MicPath) before [`push_audio`](DictationChain::push_audio); the take is recorded by `ink-audio`'s [`TakeRecorder`] with 300 ms of lead and an [adaptive tail](crate::tail) |
//! | 3 gain and VAD | [`gain_stage::level`]: VAD-gated when a VAD is installed, the fallback otherwise |
//! | 4 transcribe | the [`OfflineEngine`] the caller routed to [`Job::DictationFinal`](ink_core::Job) |
//! | 5 voice commands | [`VoiceCommandStore::detect`](crate::voicecommand::VoiceCommandStore::detect) |
//! | 6–8 cleanup, style, dictionary, snippets | [`text::write`], under the resolved [mode](crate::modes) |
//! | 9 polish | `ink-llm`'s polish task, when the mode asks for it and the user [consented](crate::consent) to where the model sends it |
//! | 10 persist | a [`RecordKind::Dictation`] record in the [`Store`] |
//! | 11 output | the platform's [`TextInserter`] |
//!
//! # Around the stages
//!
//! - **Live words.** While the key is held, the take's audio (lead included) goes through a live
//!   AGC to the router's live-partials engine when one is set ([`set_live`](DictationChain::set_live)),
//!   and what it hears reaches the shell as [`DictationEvent::Partial`]: provisional, never saved.
//!   The stream is closed before [`DictationEvent::Stopped`], so no partial follows it.
//! - **Voice edit** ([`edit_hotkey`](DictationChain::edit_hotkey)): select text, hold the edit
//!   key, say what to change. The selection is read once the hold is confirmed (never under Secure
//!   Input), the instruction is transcribed like a dictation, and the language model's rewrite
//!   replaces the selection (it is still selected, so inserting over it replaces it). Edits are always push to talk, share the
//!   dictation's recorder (one take at a time: the other key is ignored while a take is open), are
//!   not saved to the library, and never go through voice commands, cleanup, style or snippets.
//! - **A missed release** (the stuck-key watchdog): a push-to-talk or edit hold longer than
//!   [`DEFAULT_STUCK_AFTER`] (180 s) is stopped there and processed, with
//!   [`Warning::ReleaseMissed`], and its record is marked stuck, so the Stats screen counts no
//!   speed, time saved or best from it. A toggle take is exempt: a long one is deliberate.
//! - **A second press never wipes the take.** A press of the key already held changes nothing in
//!   push to talk (it is a lost release or a repeat), and is the stop in toggle mode; the take in
//!   progress is always kept (Inkwell 0.2 once cleared its buffer on such a press).
//!
//! # Threads
//!
//! **Worker**, every method: one thread owns the chain (see [`worker`](crate::worker)). Audio
//! arrives from the pump already in the canonical format, and hotkey events from the platform's
//! callback thread through a queue. The engine, the store, polish and insertion are called here
//! and may block; polish no longer than its budget ([`POLISH_BUDGET`]). Nothing here sleeps: every
//! wait is for audio, and the only deadline the chain wakes for (a tail whose audio stopped
//! arriving) is checked by [`tick`](DictationChain::tick) when the owning thread wakes for it.
//!
//! # Time
//!
//! Key events and audio carry host time on the platform clock. A key event is placed on the
//! recorder's sample timeline through the host time of the latest audio block, so it may arrive
//! before or after the audio of its moment. The minimum hold is measured on the audio clock too.

use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::{Mutex, PoisonError};

use ink_audio::take::{TAIL, Take};
use ink_audio::{Agc, TakeRecorder, VadConfig};
use ink_core::{
    AsrEvent, CANONICAL_RATE, CancelToken, Channel, Clock, EngineStream, EventSink, FocusReader,
    HotkeyEvent, Llm, LlmError, NewRecord, OfflineEngine, RecordId, RecordKind, Segment, Store,
    StoreError, StreamingEngine, TextInserter, TranscribeOptions,
};

use crate::consent::{Consented, LlmConsent};

use crate::dictionary::Dictionary;
use crate::events::{DictationEvent, Discard, EditFailure, TakeFailure, VoiceDetection, Warning};
use crate::gain_stage::{self, Vad};
use crate::modes::{Mode, ModeStore};
use crate::redact::{Spoken, redact};
use crate::snippets::{SnippetStore, SnippetVars};
use crate::style::Style;
use crate::tail::{TailConfig, TailTracker};
use crate::text;
use crate::transition::{RecordingMode, decide_transition};
use crate::voicecommand::{CommandAction, VoiceCommandStore};

/// The shortest hold that is a dictation. A right-hand modifier bound as the hotkey is also
/// pressed as a modifier, in shortcuts, and those presses are brief: nothing shorter than this is
/// shown or transcribed.
pub const DEFAULT_MIN_HOLD: Duration = Duration::from_millis(200);

/// The shortest take transcribed, measured between press and release (lead and tail excluded).
/// 0.2's value.
pub const DEFAULT_MIN_LIVE: Duration = Duration::from_millis(300);

/// How long dictation polish may run, from the moment it starts, before the take goes out as
/// written. On-device polish was measured at about 0.4 s prewarmed and 0.9–1.4 s cold, so this
/// never cuts a working model short: it only stops a model that hangs from holding up this take
/// and every take queued behind it (the chain has one worker). Other language-model jobs keep
/// their own limits.
pub const POLISH_BUDGET: Duration = Duration::from_secs(10);

/// How long a voice edit's rewrite may run before the selection is left alone. Longer than
/// polish's: an edit rewrites a whole selection, and the user is waiting for exactly that.
pub const EDIT_BUDGET: Duration = Duration::from_secs(20);

/// How long a push-to-talk (or edit) hold may last before the chain takes its release as missed
/// and stops the take there (Inkwell 0.2's watchdog, which saw holds run for minutes after a lost
/// key-up event).
pub const DEFAULT_STUCK_AFTER: Duration = Duration::from_secs(180);

const NS_PER_SAMPLE: u64 = 1_000_000_000 / CANONICAL_RATE as u64;

fn ns(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

fn samples_to_ms(samples: usize) -> u64 {
    samples as u64 * 1_000 / u64::from(CANONICAL_RATE)
}

fn duration_to_samples(d: Duration) -> usize {
    usize::try_from(d.as_millis() * u128::from(CANONICAL_RATE) / 1_000).unwrap_or(usize::MAX)
}

/// How dictation behaves: the user's settings.
#[derive(Clone, Debug)]
pub struct DictationSettings {
    /// Hold to talk, or toggle.
    pub recording_mode: RecordingMode,
    /// A press shorter than this is not a dictation ([`DEFAULT_MIN_HOLD`]).
    pub min_hold: Duration,
    /// A take shorter than this is not transcribed ([`DEFAULT_MIN_LIVE`]).
    pub min_live: Duration,
    /// Insert one space after each dictation, so consecutive ones do not run together.
    pub append_space: bool,
    /// Modes: style, filler removal and polish per app.
    pub modes: ModeStore,
    /// Corrections, and the engine's context words.
    pub dictionary: Dictionary,
    /// Snippets.
    pub snippets: SnippetStore,
    /// Voice commands.
    pub commands: VoiceCommandStore,
    /// The polish prompt when the mode has none; blank for `ink-llm`'s default.
    pub polish_prompt: String,
    /// The user's UTC offset in minutes, for `{date}` and `{time}` (the shell knows it).
    pub utc_offset_minutes: i32,
    /// The VAD's thresholds.
    pub vad: VadConfig,
    /// The adaptive tail.
    pub tail: TailConfig,
    /// How long polish may run ([`POLISH_BUDGET`]). Not a user preference: tests shorten it.
    pub polish_budget: Duration,
    /// The user's switch for polish ("Polish my words"). Off, nothing is polished; on, the modes
    /// that polish do. A voice command overrides both until the chain restarts.
    pub polish_wish: bool,
    /// Where the user agreed polish may send their words ([`LlmConsent`]). Nothing is polished
    /// without a consent that covers the model a call reaches, whatever the switch, the mode or a
    /// voice command says. `None` (the default) polishes nothing.
    pub polish_consent: Option<LlmConsent>,
    /// Where the user agreed voice edit may send the selection and the instruction. No edit
    /// reaches a model without a consent that covers it. `None` (the default) edits nothing.
    pub edit_consent: Option<LlmConsent>,
    /// How long a voice edit's rewrite may run ([`EDIT_BUDGET`]). Tests shorten it.
    pub edit_budget: Duration,
    /// How long a push-to-talk hold may last before its release is taken as missed
    /// ([`DEFAULT_STUCK_AFTER`]). Tests shorten it.
    pub stuck_after: Duration,
}

impl Default for DictationSettings {
    fn default() -> Self {
        Self {
            recording_mode: RecordingMode::PushToTalk,
            min_hold: DEFAULT_MIN_HOLD,
            min_live: DEFAULT_MIN_LIVE,
            append_space: true,
            modes: ModeStore::default(),
            dictionary: Dictionary::default(),
            snippets: SnippetStore::default(),
            commands: VoiceCommandStore::default(),
            polish_prompt: String::new(),
            utc_offset_minutes: 0,
            vad: VadConfig::default(),
            tail: TailConfig::default(),
            polish_budget: POLISH_BUDGET,
            polish_wish: true,
            polish_consent: None,
            edit_consent: None,
            edit_budget: EDIT_BUDGET,
            stuck_after: DEFAULT_STUCK_AFTER,
        }
    }
}

/// What the chain calls.
#[derive(Clone)]
pub struct Services {
    /// The dictation engine (the router's choice for the dictation job).
    pub engine: Arc<dyn OfflineEngine>,
    /// The library.
    pub store: Arc<dyn Store>,
    /// Text insertion.
    pub inserter: Arc<dyn TextInserter>,
    /// The frontmost app, for mode resolution.
    pub focus: Arc<dyn FocusReader>,
    /// The platform clock: the timebase of key events and audio.
    pub clock: Arc<dyn Clock>,
    /// The polish model, when one is set up. Voice edits rewrite through it too.
    pub llm: Option<Arc<dyn Llm>>,
}

/// Which key an event came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Dictate,
    Edit,
}

/// Where the hotkey is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hold {
    /// No take.
    Idle,
    /// Pressed, not yet held for the minimum. The take is recording (so no lead is lost) but
    /// nothing has been shown.
    Pending { press_ns: u64 },
    /// A take, shown to the user, pressed at `press_ns`.
    Recording { press_ns: u64 },
    /// Released; the take is waiting for its tail.
    Tail { release: u64, deadline_ns: u64 },
}

/// What the open take is for.
#[derive(Clone, Debug)]
enum Intent {
    Dictate,
    /// A voice edit, with the selection once the hold is confirmed. [`Spoken`] so that a debug
    /// print of the take cannot write the user's text.
    Edit {
        selection: Option<Spoken>,
    },
}

/// Bookkeeping for the open take.
#[derive(Clone, Debug)]
struct Open {
    started_unix_ms: i64,
    lost_frames: u64,
    intent: Intent,
}

impl Open {
    fn key(&self) -> Key {
        match self.intent {
            Intent::Dictate => Key::Dictate,
            Intent::Edit { .. } => Key::Edit,
        }
    }
}

/// The live words of the take being held: its stream, and the live AGC in front of it (rule 11:
/// no engine hears the raw level).
struct Live {
    stream: Box<dyn EngineStream>,
    agc: Agc,
    scratch: Vec<f32>,
}

impl Live {
    fn push(&mut self, samples: &[f32]) -> Result<(), ink_core::EngineError> {
        self.scratch.clear();
        self.scratch.extend_from_slice(samples);
        self.agc.process(&mut self.scratch);
        self.stream.push(&self.scratch)
    }
}

/// A record being written. Dropped before [`keep`](Self::keep), by an error return or a panic in
/// the store, it deletes the record.
struct HalfSaved<'a> {
    store: &'a dyn Store,
    id: RecordId,
    armed: bool,
}

impl HalfSaved<'_> {
    /// The record is complete: keep it.
    fn keep(mut self) -> RecordId {
        self.armed = false;
        self.id.clone()
    }
}

impl Drop for HalfSaved<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let id = &self.id;
        // During a panic a second panic here would abort the process: the store is asked once,
        // and a failure is logged, never raised.
        let deleted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.store.delete_record(id)
        }));
        match deleted {
            // Logged every time: it should only follow a failed or interrupted save, and a line here
            // is how a record removed in error would ever be noticed.
            Ok(Ok(())) => {
                log::warn!("dictation: a half-saved record was removed after an unfinished save")
            }
            Ok(Err(e)) => log::warn!("dictation: a half-saved record could not be removed: {e}"),
            Err(_) => log::warn!("dictation: removing a half-saved record panicked"),
        }
    }
}

/// The dictation chain. See the module docs.
pub struct DictationChain {
    services: Services,
    settings: DictationSettings,
    vad: Vad,
    events: EventSink<DictationEvent>,
    recorder: TakeRecorder,
    tail: TailTracker,
    /// Host time and recorder position of the latest audio block.
    anchor: Option<(u64, u64)>,
    hold: Hold,
    open: Option<Open>,
    /// A mode chosen by voice command, overriding app matching.
    pinned_mode: Option<String>,
    /// Takes processed to the end without a panic; the worker's panic policy reads it.
    completed_takes: u64,
    /// Polish turned on or off by voice command, overriding the mode.
    polish_override: Option<bool>,
    /// The live-partials engine, when one is set.
    live_engine: Option<Arc<dyn StreamingEngine>>,
    /// The held take's live words.
    live: Option<Live>,
    /// Takes confirmed so far: the next take's number.
    takes_started: u64,
    /// The stuck-key watchdog stopped the open take: its record is marked stuck when saved
    /// ([`Store::mark_stuck`](ink_core::Store::mark_stuck)). Cleared at each press.
    stopped_stuck: bool,
}

fn detection(vad: &Vad) -> VoiceDetection {
    match vad {
        Vad::Installed(_) => VoiceDetection::Available,
        Vad::Unavailable(why) => VoiceDetection::Unavailable(*why),
    }
}

impl DictationChain {
    /// A chain, idle. Announces at once whether voice detection is available.
    pub fn new(
        services: Services,
        settings: DictationSettings,
        vad: Vad,
        events: EventSink<DictationEvent>,
    ) -> Self {
        events(DictationEvent::VoiceDetection(detection(&vad)));
        Self {
            services,
            settings,
            vad,
            events,
            recorder: TakeRecorder::new(),
            tail: TailTracker::default(),
            anchor: None,
            hold: Hold::Idle,
            open: None,
            pinned_mode: None,
            polish_override: None,
            completed_takes: 0,
            live_engine: None,
            live: None,
            takes_started: 0,
            stopped_stuck: false,
        }
    }

    /// Sets the engine that shows live words while the key is held (the router's live-partials
    /// engine), or none. Takes from the next one on use it.
    pub fn set_live(&mut self, engine: Option<Arc<dyn StreamingEngine>>) {
        self.live_engine = engine;
    }

    fn emit(&self, event: DictationEvent) {
        (self.events)(event);
    }

    /// Installs a VAD, or records why there is none. The shell hears about it when that changes.
    pub fn set_vad(&mut self, vad: Vad) {
        let (before, after) = (detection(&self.vad), detection(&vad));
        self.vad = vad;
        if before != after {
            self.emit(DictationEvent::VoiceDetection(after));
        }
    }

    /// Replaces the settings. A take in progress finishes under the new ones.
    pub fn set_settings(&mut self, settings: DictationSettings) {
        self.settings = settings;
    }

    /// Whether a take is open and the key is down (or, in toggle mode, not yet pressed again).
    pub fn is_recording(&self) -> bool {
        matches!(self.hold, Hold::Pending { .. } | Hold::Recording { .. })
    }

    /// Takes processed to the end (inserted, discarded or failed) without a panic.
    pub fn completed_takes(&self) -> u64 {
        self.completed_takes
    }

    /// Puts the chain back to idle after a stage panicked, and tells the shell. The take in
    /// progress is dropped, and the recorder, the tail and the timeline start over, so nothing a
    /// panic interrupted half way is trusted again. Settings, the VAD and a pinned mode are kept.
    /// `recovered` is what the shell is told: whether the owner goes on serving.
    pub fn recover_from_panic(&mut self, recovered: bool) {
        self.recorder = TakeRecorder::new();
        self.tail = TailTracker::default();
        self.anchor = None;
        self.hold = Hold::Idle;
        self.open = None;
        self.live = None;
        self.emit(DictationEvent::WorkerFailed { recovered });
    }

    /// The mode a voice command pinned, if any.
    pub fn pinned_mode(&self) -> Option<&str> {
        self.pinned_mode.as_deref()
    }

    /// When [`tick`](Self::tick) should next run, as host time: set while a take waits for a tail
    /// whose audio may never come, and while a push-to-talk key is held (its release may be lost).
    pub fn deadline_ns(&self) -> Option<u64> {
        match self.hold {
            Hold::Tail { deadline_ns, .. } => Some(deadline_ns),
            Hold::Pending { press_ns } | Hold::Recording { press_ns } if self.watched() => {
                Some(press_ns.saturating_add(ns(self.settings.stuck_after)))
            }
            _ => None,
        }
    }

    /// Whether the open take stops on its key's release, so a lost release would leave it
    /// running: push to talk, and every edit. A toggle take is stopped by a press.
    fn watched(&self) -> bool {
        self.settings.recording_mode == RecordingMode::PushToTalk
            || self.open.as_ref().is_some_and(|o| o.key() == Key::Edit)
    }

    /// The next mic audio: 16 kHz mono from [`MicPath`](crate::mic::MicPath), the host time of its
    /// first sample, and the device frames the capture ring dropped before it.
    pub fn push_audio(&mut self, samples: &[f32], host_time_ns: u64, dropped_frames: u64) {
        self.anchor = Some((host_time_ns, self.recorder.position()));
        if let Some(open) = &mut self.open {
            open.lost_frames += dropped_frames;
            self.tail.observe(samples);
        }
        if matches!(self.hold, Hold::Recording { .. }) {
            self.feed_live(samples);
        }
        if let Some(take) = self.recorder.push(samples) {
            self.finish_take(take, false);
            return;
        }
        let heard_until = host_time_ns.saturating_add(samples.len() as u64 * NS_PER_SAMPLE);
        match self.hold {
            Hold::Pending { press_ns }
                if heard_until >= press_ns.saturating_add(ns(self.settings.min_hold)) =>
            {
                self.confirm();
            }
            Hold::Tail { release, .. } => self.end_tail_if_quiet(release),
            _ => {}
        }
    }

    /// A dictation hotkey event, from the platform's callback through the owning thread's queue.
    pub fn hotkey(&mut self, event: HotkeyEvent) {
        self.key_event(Key::Dictate, event);
    }

    /// A voice-edit hotkey event. See the module docs.
    pub fn edit_hotkey(&mut self, event: HotkeyEvent) {
        self.key_event(Key::Edit, event);
    }

    /// The key whose take is open, if any.
    fn open_key(&self) -> Option<Key> {
        self.open.as_ref().map(Open::key)
    }

    fn key_event(&mut self, key: Key, event: HotkeyEvent) {
        let edit = key == Key::Edit;
        match event {
            HotkeyEvent::Pressed { at_ns } => {
                // A new press ends a tail still in progress: the next take starts now.
                if matches!(self.hold, Hold::Tail { .. })
                    && let Some(take) = self.recorder.finish()
                {
                    self.finish_take(take, false);
                }
                if self.is_recording() && self.open_key() != Some(key) {
                    // The other key while a take is open: one take at a time. Its release is
                    // ignored the same way.
                    log::info!("dictation: a {key:?} press during another take was ignored");
                    return;
                }
                // Guarded on state, never on the event: a press of the key already held (a lost
                // release, a repeat) keeps the take in push to talk, and stops it in toggle mode.
                let t = decide_transition(
                    true,
                    self.is_recording(),
                    self.settings.recording_mode,
                    edit,
                );
                if t.start {
                    self.start(at_ns, key);
                } else if t.stop {
                    self.stop(at_ns);
                }
            }
            HotkeyEvent::Released { at_ns } => {
                if self.open_key() != Some(key) || !self.is_recording() {
                    return;
                }
                if let Hold::Pending { press_ns } = self.hold {
                    if at_ns.saturating_sub(press_ns) < ns(self.settings.min_hold) {
                        self.abandon();
                        self.emit(DictationEvent::ShortPressIgnored);
                        return;
                    }
                    self.confirm();
                    if !self.is_recording() {
                        // An edit with nothing selected ended at its confirmation.
                        return;
                    }
                }
                let t = decide_transition(
                    false,
                    self.is_recording(),
                    self.settings.recording_mode,
                    edit,
                );
                if t.stop {
                    self.stop(at_ns);
                }
            }
            HotkeyEvent::Cancelled => {
                if self.open_key() == Some(key) {
                    self.end_hold(false);
                }
            }
            HotkeyEvent::Lost => {
                if self.open_key() == Some(key) {
                    self.end_hold(true);
                }
                self.emit(if edit {
                    DictationEvent::EditHotkeyLost
                } else {
                    DictationEvent::HotkeyLost
                });
            }
        }
    }

    /// Ends a take whose tail stopped arriving, once its deadline has passed. Call it when the
    /// owning thread wakes at [`deadline_ns`](Self::deadline_ns); early calls do nothing.
    ///
    /// It also stops a push-to-talk take whose key has been held for
    /// [`stuck_after`](DictationSettings::stuck_after): the release was most likely lost, and the
    /// take is processed with what was said, never discarded.
    pub fn tick(&mut self) {
        let now = self.services.clock.now_ns();
        match self.hold {
            Hold::Tail { deadline_ns, .. } if now >= deadline_ns => {
                if let Some(take) = self.recorder.finish() {
                    self.finish_take(take, true);
                }
            }
            Hold::Pending { press_ns } | Hold::Recording { press_ns }
                if self.watched()
                    && now >= press_ns.saturating_add(ns(self.settings.stuck_after)) =>
            {
                log::warn!(
                    "dictation: a key held {} s with no release; the take is stopped and processed",
                    self.settings.stuck_after.as_secs()
                );
                self.emit(DictationEvent::Warning(Warning::ReleaseMissed));
                self.stopped_stuck = true;
                if matches!(self.hold, Hold::Pending { .. }) {
                    self.confirm();
                    if !self.is_recording() {
                        return;
                    }
                }
                self.stop(now);
            }
            _ => {}
        }
    }

    /// The mic was let go of while idle and will be opened again at the next press: nothing heard
    /// so far may lead a later take, and the timeline restarts with the next audio. A take open
    /// now (a press that raced the close) keeps what it has and goes on when the audio returns.
    pub fn mic_closed(&mut self) {
        self.recorder.claim_heard();
        self.anchor = None;
    }

    /// The mic stream stopped (the device went away, or capture was stopped). A confirmed take is
    /// processed with the audio it got; an unconfirmed one is dropped.
    pub fn stream_ended(&mut self) {
        match self.hold {
            Hold::Idle => {}
            Hold::Pending { .. } => {
                self.abandon();
                self.emit(DictationEvent::Discarded(Discard::Cancelled));
            }
            Hold::Recording { .. } | Hold::Tail { .. } => {
                self.live = None;
                if let Some(take) = self.recorder.finish() {
                    self.finish_take(take, true);
                }
            }
        }
        self.anchor = None;
    }

    /// Where host time `at_ns` falls on the recorder's timeline.
    fn position_at(&self, at_ns: u64) -> u64 {
        let Some((host, position)) = self.anchor else {
            return self.recorder.position();
        };
        let delta =
            (i128::from(at_ns) - i128::from(host)) * i128::from(CANONICAL_RATE) / 1_000_000_000;
        u64::try_from((i128::from(position) + delta).max(0)).unwrap_or(u64::MAX)
    }

    fn start(&mut self, at_ns: u64, key: Key) {
        if !self.recorder.press(self.position_at(at_ns)) {
            // Cannot happen: every path that leaves `Idle` closes the recorder's take first.
            log::error!("dictation: a take was already open at a press; the press was ignored");
            return;
        }
        self.tail.begin(self.recorder.position());
        self.stopped_stuck = false;
        self.open = Some(Open {
            started_unix_ms: self.services.clock.unix_ms(),
            lost_frames: 0,
            intent: match key {
                Key::Dictate => Intent::Dictate,
                Key::Edit => Intent::Edit { selection: None },
            },
        });
        self.hold = Hold::Pending { press_ns: at_ns };
        if self.settings.min_hold.is_zero() {
            self.confirm();
        }
    }

    /// The hold passed the minimum: it is a take. An edit reads the selection here, not at the
    /// press, so a modifier tapped in a shortcut never reaches into the focused app; with nothing
    /// selected the take ends here, unheard.
    fn confirm(&mut self) {
        let press_ns = match self.hold {
            Hold::Pending { press_ns } | Hold::Recording { press_ns } => press_ns,
            _ => self.services.clock.now_ns(),
        };
        let edit = self.open_key() == Some(Key::Edit);
        let (mode, app) = if edit {
            // Never read what the user selected while Secure Input is on: it is a password field
            // or the like, and the selection would go to a language model. Focus that cannot be
            // read cannot be shown safe, so it refuses too.
            match self.services.focus.focus() {
                Ok(focus) if focus.secure_input => {
                    self.abandon();
                    self.emit(DictationEvent::EditFailed(EditFailure::SecureInput));
                    return;
                }
                Ok(_) => {}
                Err(error) => {
                    self.abandon();
                    self.emit(DictationEvent::EditFailed(
                        EditFailure::SelectionUnreadable(error),
                    ));
                    return;
                }
            }
            match self.services.focus.selected_text() {
                Ok(Some(text)) if !text.trim().is_empty() => {
                    if let Some(open) = &mut self.open {
                        open.intent = Intent::Edit {
                            selection: Some(Spoken::new(text)),
                        };
                    }
                }
                Ok(_) => {
                    self.abandon();
                    self.emit(DictationEvent::EditFailed(EditFailure::NoSelection));
                    return;
                }
                Err(error) => {
                    self.abandon();
                    self.emit(DictationEvent::EditFailed(
                        EditFailure::SelectionUnreadable(error),
                    ));
                    return;
                }
            }
            (None, None)
        } else {
            // What the Drop shows: the app in front now and its mode. The text itself is written
            // in the mode of the app that receives it (read again at insertion).
            let app = self.services.focus.focus().ok().and_then(|f| f.app);
            let mode = self
                .settings
                .modes
                .resolve_with_override(
                    app.as_ref().map(|a| a.id.as_str()),
                    self.pinned_mode.as_deref(),
                )
                .name
                .clone();
            (Some(mode), app.map(|a| a.name))
        };
        let take = self.takes_started;
        self.takes_started += 1;
        self.hold = Hold::Recording { press_ns };
        self.emit(DictationEvent::Started {
            take,
            edit,
            mode,
            app,
        });
        if !edit {
            self.open_live(take);
        }
    }

    /// Opens the live stream for take `take` and feeds it the take so far (the lead included), so
    /// the live words describe the audio the final will.
    fn open_live(&mut self, take: u64) {
        let Some(engine) = self.live_engine.clone() else {
            return;
        };
        let events = self.events.clone();
        // Settled words so far; each event shows them and the current hypothesis after them.
        let settled = Arc::new(Mutex::new(String::new()));
        let sink: EventSink<AsrEvent> = Arc::new(move |e| {
            let mut settled = settled.lock().unwrap_or_else(PoisonError::into_inner);
            let shown = match e {
                AsrEvent::Partial { text } => join_words(&settled, &text),
                AsrEvent::Final(t) => {
                    *settled = join_words(&settled, &t.text);
                    settled.clone()
                }
                AsrEvent::Stalled { .. } => return,
            };
            drop(settled);
            events(DictationEvent::Partial {
                take,
                text: Spoken::new(shown),
            });
        });
        match engine.open_stream(Channel::Mic, sink) {
            Ok(stream) => {
                let mut live = Live {
                    stream,
                    agc: Agc::without_vad(),
                    scratch: Vec::new(),
                };
                let so_far = self.recorder.open_audio().unwrap_or(&[]);
                match live.push(so_far) {
                    Ok(()) => self.live = Some(live),
                    Err(e) => log::info!("dictation: live words stopped for this take: {e}"),
                }
            }
            // Usually no live engine installed (Parakeet's models missing): the Drop shows that
            // it is listening, without words. The error names the engine, never any text.
            Err(e) => log::info!("dictation: no live words for this take: {e}"),
        }
    }

    fn feed_live(&mut self, samples: &[f32]) {
        if let Some(live) = &mut self.live
            && let Err(e) = live.push(samples)
        {
            // A stuck live engine costs the live words, never the take.
            log::info!("dictation: live words stopped for this take: {e}");
            self.live = None;
        }
    }

    /// Drops the open take unseen.
    fn abandon(&mut self) {
        self.live = None;
        self.recorder.cancel();
        self.open = None;
        self.hold = Hold::Idle;
    }

    /// The key came up (or, in toggle mode, went down again) at `at_ns`.
    fn stop(&mut self, at_ns: u64) {
        // Closed first: no live word of this take reaches the shell after `Stopped`.
        self.live = None;
        self.emit(DictationEvent::Stopped);
        let release = self.position_at(at_ns);
        self.tail.release(&self.settings.tail);
        if let Some(take) = self.recorder.release(release) {
            self.finish_take(take, false);
            return;
        }
        let deadline_ns = at_ns
            .saturating_add(ns(TAIL))
            .saturating_add(ns(self.settings.tail.grace));
        self.hold = Hold::Tail {
            release,
            deadline_ns,
        };
        self.end_tail_if_quiet(release);
    }

    fn end_tail_if_quiet(&mut self, release: u64) {
        if self.tail.speech_ended(release, &self.settings.tail)
            && let Some(take) = self.recorder.finish()
        {
            self.finish_take(take, false);
        }
    }

    /// The hotkey stopped reporting: events were missed (`lost` false: a disabled event tap) or
    /// the hotkey is gone (`lost` true: a revoked permission). A take the user was shown is
    /// processed, not discarded: they did speak. A toggle take goes on after missed events (its
    /// stop is the next press, which can still come), but not after the hotkey is gone.
    fn end_hold(&mut self, lost: bool) {
        match self.hold {
            Hold::Pending { .. } => {
                self.abandon();
                self.emit(DictationEvent::Discarded(Discard::Cancelled));
            }
            Hold::Recording { .. } if lost || self.watched() => {
                self.stop(self.services.clock.now_ns());
            }
            Hold::Idle | Hold::Recording { .. } | Hold::Tail { .. } => {}
        }
    }

    fn finish_take(&mut self, take: Take, cut_short: bool) {
        self.hold = Hold::Idle;
        self.live = None;
        let open = self.open.take();
        if cut_short {
            self.emit(DictationEvent::Warning(Warning::TailCutShort));
        }
        let lost = open.as_ref().map_or(0, |o| o.lost_frames);
        if lost > 0 {
            self.emit(DictationEvent::Warning(Warning::AudioLost { frames: lost }));
        }
        let started = open
            .as_ref()
            .map_or_else(|| self.services.clock.unix_ms(), |o| o.started_unix_ms);
        match open.map(|o| o.intent) {
            Some(Intent::Edit { selection }) => self.process_edit(take, selection),
            _ => self.process(take, started),
        }
        // Not reached when a stage panics: that is what the count is for.
        self.completed_takes += 1;
    }

    /// Stages 3 and 4 for one take: the level, the discard rules, the engine. `None` when the take
    /// ended there (each ending already reported).
    fn transcribe(&mut self, take: Take) -> Option<Result<String, ink_core::EngineError>> {
        let live_ms = samples_to_ms(take.live());
        if take.live() < duration_to_samples(self.settings.min_live) {
            self.emit(DictationEvent::Discarded(Discard::TooShort { live_ms }));
            return None;
        }
        let levelled = gain_stage::level(take.samples, &mut self.vad, &self.settings.vad);
        log::info!(
            "dictation take: {live_ms} ms held, {:?} via {:?}, {} samples to the engine",
            levelled.report.outcome,
            levelled.path,
            levelled.audio.len()
        );
        if let Some(error) = levelled.vad_error.clone() {
            self.emit(DictationEvent::Warning(Warning::VadFailed(error)));
        }
        if let Some(discard) = levelled.discard() {
            self.emit(DictationEvent::Discarded(discard));
            return None;
        }
        let options = TranscribeOptions {
            channel: Channel::Mic,
            context: self.settings.dictionary.hotwords(),
            cancel: CancelToken::new(),
        };
        Some(
            self.services
                .engine
                .transcribe(&levelled.audio, &options)
                .map(|t| t.text()),
        )
    }

    /// A voice edit: the instruction, then the model's rewrite over the selection. Nothing is
    /// saved: the text belongs to another app.
    fn process_edit(&mut self, take: Take, selection: Option<Spoken>) {
        let Some(selection) = selection else {
            // Cannot happen: an edit becomes a take only once its selection was read.
            log::error!("dictation: an edit reached processing without its selection");
            self.emit(DictationEvent::EditFailed(EditFailure::NoSelection));
            return;
        };
        let raw = match self.transcribe(take) {
            None => return,
            Some(Ok(text)) => text,
            Some(Err(error)) => {
                self.emit(DictationEvent::EditFailed(EditFailure::Transcription(
                    error,
                )));
                return;
            }
        };
        if raw.trim().is_empty() {
            self.emit(DictationEvent::Discarded(Discard::NothingHeard));
            return;
        }
        // Names the engine mishears are corrected here too: an instruction naming the wrong thing
        // would be applied as the wrong instruction.
        let instruction = self.settings.dictionary.apply(&raw);
        let Some(llm) = self.services.llm.clone() else {
            self.emit(DictationEvent::EditFailed(EditFailure::NoModel));
            return;
        };
        // Consent: the selection and the instruction go only where the user agreed voice edit
        // may send them, checked on the model the call reaches (as polish is). Without a consent
        // every model is refused.
        let consented = Consented {
            inner: llm.as_ref(),
            consent: self.settings.edit_consent.as_ref(),
        };
        let budget = self.settings.edit_budget;
        let deadline = Instant::now().checked_add(budget);
        let token = deadline.map_or_else(CancelToken::new, CancelToken::with_deadline);
        let rewritten = match ink_llm::tasks::voice_edit::apply_edit(
            &consented,
            selection.as_str(),
            &instruction,
            &token,
        ) {
            Ok(text) => text,
            Err(LlmError::NotAllowed { refused }) => {
                log::warn!(
                    "dictation: voice edit's model is not where the user agreed to send text; the selection was left alone"
                );
                self.emit(DictationEvent::EditFailed(EditFailure::NotAllowed(
                    LlmConsent::for_model(&refused),
                )));
                return;
            }
            Err(LlmError::Cancelled) if deadline.is_some_and(|d| Instant::now() >= d) => {
                log::warn!("dictation: the edit gave no answer within its {budget:?} budget");
                self.emit(DictationEvent::EditFailed(EditFailure::TimedOut));
                return;
            }
            Err(error) => {
                self.emit(DictationEvent::EditFailed(EditFailure::Model(error)));
                return;
            }
        };
        // No trailing space: an edit replaces the selection exactly.
        match self.services.inserter.insert(&rewritten) {
            Ok(outcome) => {
                log::info!(
                    "dictation edit: {} -> {} ({outcome:?})",
                    redact(selection.as_str()),
                    redact(&rewritten)
                );
                self.emit(DictationEvent::Edited { outcome });
            }
            Err(error) => self.emit(DictationEvent::EditFailed(EditFailure::Insert(error))),
        }
    }

    /// Stages 3–11 for one take.
    fn process(&mut self, take: Take, started_unix_ms: i64) {
        let live_ms = samples_to_ms(take.live());
        // Stages 3 and 4.
        let raw = match self.transcribe(take) {
            None => return,
            Some(Ok(text)) => text,
            Some(Err(error)) => {
                self.emit(DictationEvent::Failed(TakeFailure::Transcription(error)));
                return;
            }
        };
        if raw.trim().is_empty() {
            self.emit(DictationEvent::Discarded(Discard::NothingHeard));
            return;
        }

        // Stage 5.
        if let Some(command) = self.settings.commands.detect(&raw) {
            let action = command.action.clone();
            if self.apply_command(&action) {
                self.emit(DictationEvent::Command(action));
            }
            return;
        }

        // Stages 6–8, under the mode for the app about to receive the text.
        let app = match self.services.focus.focus() {
            Ok(focus) => focus.app.map(|a| a.id),
            Err(error) => {
                self.emit(DictationEvent::Warning(Warning::FocusUnreadable(error)));
                None
            }
        };
        let mode = self
            .settings
            .modes
            .resolve_with_override(app.as_deref(), self.pinned_mode.as_deref());
        let vars = SnippetVars::at(
            self.services.clock.unix_ms(),
            self.settings.utc_offset_minutes,
        );
        let written = text::write(
            &raw,
            mode,
            &self.settings.dictionary,
            &self.settings.snippets,
            &vars,
        );
        if written.trim().is_empty() {
            self.emit(DictationEvent::Discarded(Discard::NothingLeft));
            return;
        }

        // Stage 9. Never empties the text: see `polish`.
        let written = self.polish(written, mode);

        // Stage 10.
        let saved = self.save(&written, started_unix_ms, live_ms, app);
        // A half-saved record the save removed is deleted text too: each change of the library's
        // scrub reaches the shell once.
        match self.services.store.scrub_change() {
            Some(true) => self.emit(DictationEvent::Warning(Warning::DeletedTextNotScrubbed)),
            Some(false) => self.emit(DictationEvent::Warning(Warning::DeletedTextScrubbed)),
            None => {}
        }
        let record = match saved {
            Ok(id) => Some(id),
            Err(error) => {
                self.emit(DictationEvent::Warning(Warning::SaveFailed(error)));
                None
            }
        };

        // Stage 11. The space is added here only: the record and the event hold what was said.
        let insert = if self.settings.append_space {
            // i5-allow: the dictation itself, on its way into the focused app
            format!("{written} ")
        } else {
            written.clone()
        };
        match self.services.inserter.insert(&insert) {
            Ok(outcome) => {
                log::info!("dictation inserted: {} ({outcome:?})", redact(&written));
                self.emit(DictationEvent::Inserted {
                    text: Spoken::new(written),
                    outcome,
                    record,
                });
            }
            Err(error) => self.emit(DictationEvent::Failed(TakeFailure::Insert(error))),
        }
    }

    /// Stage 9: polish when the mode (or a voice command) asks for it. Any failure keeps the text
    /// as written, and says so. A blank answer is a failure, never an empty dictation (ink-llm's
    /// task refuses one; this checks again rather than rely on it).
    ///
    /// **Consent.** The call goes out only when [`polish_consent`](DictationSettings::polish_consent)
    /// covers the model it reaches, checked by that model at the call ([`Llm::complete_if`]), so a
    /// model that changed destination since the user agreed never receives the text. Without it
    /// the text goes out as written with [`Warning::PolishNotAllowed`], naming the consent needed.
    ///
    /// The call's token is cancelled when the [budget](DictationSettings::polish_budget) runs out.
    /// The budget is a deadline the token carries, so no thread or timer fires it: the model sees
    /// it at its next check of the token (a shell engine's wait checks every 20 ms). A polish that
    /// ran out of its budget is reported as [`Warning::PolishTimedOut`]; one the model stopped
    /// before the budget (a shell engine at shutdown) stays [`Warning::PolishFailed`], so the shell
    /// can tell "polish keeps timing out" from an ordinary cancel.
    ///
    /// The next take's press does not cancel it. The take behind it loses no audio (the owner's
    /// queue holds tens of seconds of it) and at most starts later, while cancelling would cost a
    /// working polish every time someone presses again quickly, which is how push-to-talk is used.
    fn polish(&self, written: String, mode: &Mode) -> String {
        let wanted = self.settings.polish_wish && mode.polish_enabled;
        if !self.polish_override.unwrap_or(wanted) {
            return written;
        }
        let Some(llm) = &self.services.llm else {
            self.emit(DictationEvent::Warning(Warning::PolishUnavailable));
            return written;
        };
        // Without a consent every model is refused at the call (nothing is sent); the model
        // itself says first if there is none at all.
        let consented = Consented {
            inner: llm.as_ref(),
            consent: self.settings.polish_consent.as_ref(),
        };
        let prompt = if mode.polish_prompt.trim().is_empty() {
            &self.settings.polish_prompt
        } else {
            &mode.polish_prompt
        };
        let budget = self.settings.polish_budget;
        let deadline = Instant::now().checked_add(budget);
        let token = deadline.map_or_else(CancelToken::new, CancelToken::with_deadline);
        match ink_llm::tasks::polish::polish(&consented, prompt, &written, &token) {
            Ok(polished) if !polished.trim().is_empty() => polished,
            Ok(_) => {
                self.emit(DictationEvent::Warning(Warning::PolishFailed(
                    LlmError::BadResponse("polish: the answer was blank".into()),
                )));
                written
            }
            Err(LlmError::NotAllowed { refused }) => {
                // The model reached is not where the user agreed: nothing was sent. The consent
                // named is the one that model needs, from the info of the model refused.
                log::warn!(
                    "dictation: polish's model is not where the user agreed to send words; the text goes out as written"
                );
                self.emit(DictationEvent::Warning(Warning::PolishNotAllowed(
                    LlmConsent::for_model(&refused),
                )));
                written
            }
            Err(error) => {
                if error == LlmError::Cancelled && deadline.is_some_and(|d| Instant::now() >= d) {
                    log::warn!(
                        "dictation: polish gave no answer within its {budget:?} budget; the text goes out as written"
                    );
                    self.emit(DictationEvent::Warning(Warning::PolishTimedOut));
                    return written;
                }
                if error == LlmError::Cancelled {
                    // The model stopped for its own reasons (a shell engine when the core stops).
                    log::info!("dictation: polish was cancelled; the text goes out as written");
                }
                self.emit(DictationEvent::Warning(Warning::PolishFailed(error)));
                written
            }
        }
    }

    /// Stage 10: one dictation record with one mic segment, marked stuck when the watchdog stopped
    /// the take. A record left half-written is deleted, so the library never shows an empty
    /// dictation: by a drop guard, so a store that panics half way is cleaned up too.
    fn save(
        &self,
        written: &str,
        started_unix_ms: i64,
        live_ms: u64,
        app: Option<String>,
    ) -> Result<RecordId, StoreError> {
        let store = self.services.store.as_ref();
        let id = store.create_record(NewRecord {
            kind: RecordKind::Dictation,
            title: None,
            started_at_unix_ms: started_unix_ms,
            source_app: app,
            // A dictation keeps no audio.
            audio_dir: None,
        })?;
        let half = HalfSaved {
            store,
            id,
            armed: true,
        };
        store.append_segments(
            &half.id,
            &[Segment {
                channel: Channel::Mic,
                start_ms: 0,
                end_ms: live_ms,
                text: written.to_owned(),
                speaker: None,
            }],
        )?;
        store.finish_record(&half.id, self.services.clock.unix_ms())?;
        let id = half.keep();
        // Only the Stats screen reads the mark: a store that refuses it costs that, never the take.
        if self.stopped_stuck
            && let Err(error) = store.mark_stuck(&id)
        {
            log::warn!("dictation: the take the watchdog stopped could not be marked: {error}");
        }
        Ok(id)
    }

    /// The chain's part of a voice command: style, polish and fixed text
    /// ([`CommandAction::carried_out`]). The rest reach the shell as the event only. `false` when
    /// the command failed and said so (fixed text that could not be inserted), so no
    /// [`DictationEvent::Command`] follows.
    fn apply_command(&mut self, action: &CommandAction) -> bool {
        match action {
            CommandAction::InsertText { text } => {
                // Inserted as a dictation's text is (with its trailing space), and not saved: it is
                // the user's own fixed text, not something said.
                let insert = if self.settings.append_space {
                    // i5-allow: the command's fixed text, on its way into the focused app
                    format!("{text} ")
                } else {
                    text.clone()
                };
                match self.services.inserter.insert(&insert) {
                    Ok(outcome) => log::info!("voice command's text inserted ({outcome:?})"),
                    Err(error) => {
                        self.emit(DictationEvent::Failed(TakeFailure::Insert(error)));
                        return false;
                    }
                }
            }
            CommandAction::ChangeStyle { style } => {
                let modes = &self.settings.modes;
                let target = Style::parse(style)
                    .and_then(|s| modes.first_with_style(s))
                    .or_else(|| modes.find_by_name(style))
                    .map(|m| m.id.clone());
                match target {
                    Some(id) => self.pinned_mode = Some(id),
                    None => self.emit(DictationEvent::Warning(Warning::NoModeForStyle)),
                }
            }
            CommandAction::TogglePolish => {
                let mode = self
                    .settings
                    .modes
                    .resolve_with_override(None, self.pinned_mode.as_deref());
                let now = self
                    .polish_override
                    .unwrap_or(self.settings.polish_wish && mode.polish_enabled);
                self.polish_override = Some(!now);
            }
            // Exhaustive on purpose, as `CommandAction::carried_out` is: the two must agree, or a
            // command would be reported carried out and do nothing.
            CommandAction::Undo
            | CommandAction::SwitchModel { .. }
            | CommandAction::ToggleDictation
            | CommandAction::OpenUrl { .. }
            | CommandAction::OpenApp { .. } => debug_assert!(!action.carried_out()),
        }
        true
    }
}

/// `settled` and `more`, with one space between when both have words.
fn join_words(settled: &str, more: &str) -> String {
    match (settled.trim(), more.trim()) {
        ("", more) => more.to_owned(),
        (settled, "") => settled.to_owned(),
        (settled, more) => format!("{settled} {more}"),
    }
}
