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
//! | 9 polish | `ink-llm`'s polish task, when the mode asks for it |
//! | 10 persist | a [`RecordKind::Dictation`] record in the [`Store`] |
//! | 11 output | the platform's [`TextInserter`] |
//!
//! # Threads
//!
//! **Worker**, every method: one thread owns the chain (see [`worker`](crate::worker)). Audio
//! arrives from the pump already in the canonical format, and hotkey events from the platform's
//! callback thread through a queue. The engine, the store, polish and insertion are called here
//! and may block. Nothing here sleeps: every wait is for audio, and the only deadline (a tail
//! whose audio stopped arriving) is checked by [`tick`](DictationChain::tick) when the owning
//! thread wakes for it.
//!
//! # Time
//!
//! Key events and audio carry host time on the platform clock. A key event is placed on the
//! recorder's sample timeline through the host time of the latest audio block, so it may arrive
//! before or after the audio of its moment. The minimum hold is measured on the audio clock too.

use std::sync::Arc;
use std::time::Duration;

use ink_audio::take::{TAIL, Take};
use ink_audio::{TakeRecorder, VadConfig};
use ink_core::{
    CANONICAL_RATE, CancelToken, Channel, Clock, EventSink, FocusReader, HotkeyEvent, Llm,
    NewRecord, OfflineEngine, RecordId, RecordKind, Segment, Store, StoreError, TextInserter,
    TranscribeOptions,
};

use crate::dictionary::Dictionary;
use crate::events::{DictationEvent, Discard, TakeFailure, VoiceDetection, Warning};
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
    /// The polish model, when one is set up.
    pub llm: Option<Arc<dyn Llm>>,
}

/// Where the hotkey is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hold {
    /// No take.
    Idle,
    /// Pressed, not yet held for the minimum. The take is recording (so no lead is lost) but
    /// nothing has been shown.
    Pending { press_ns: u64 },
    /// A take, shown to the user.
    Recording,
    /// Released; the take is waiting for its tail.
    Tail { release: u64, deadline_ns: u64 },
}

/// Bookkeeping for the open take.
#[derive(Clone, Copy, Debug)]
struct Open {
    started_unix_ms: i64,
    lost_frames: u64,
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
    /// Polish turned on or off by voice command, overriding the mode.
    polish_override: Option<bool>,
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
        }
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
        matches!(self.hold, Hold::Pending { .. } | Hold::Recording)
    }

    /// The mode a voice command pinned, if any.
    pub fn pinned_mode(&self) -> Option<&str> {
        self.pinned_mode.as_deref()
    }

    /// When [`tick`](Self::tick) should next run, as host time: set while a take waits for a tail
    /// whose audio may never come.
    pub fn deadline_ns(&self) -> Option<u64> {
        match self.hold {
            Hold::Tail { deadline_ns, .. } => Some(deadline_ns),
            _ => None,
        }
    }

    /// The next mic audio: 16 kHz mono from [`MicPath`](crate::mic::MicPath), the host time of its
    /// first sample, and the device frames the capture ring dropped before it.
    pub fn push_audio(&mut self, samples: &[f32], host_time_ns: u64, dropped_frames: u64) {
        self.anchor = Some((host_time_ns, self.recorder.position()));
        if let Some(open) = &mut self.open {
            open.lost_frames += dropped_frames;
            self.tail.observe(samples);
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

    /// A hotkey event, from the platform's callback through the owning thread's queue.
    pub fn hotkey(&mut self, event: HotkeyEvent) {
        match event {
            HotkeyEvent::Pressed { at_ns } => {
                // A new press ends a tail still in progress: the next take starts now.
                if matches!(self.hold, Hold::Tail { .. })
                    && let Some(take) = self.recorder.finish()
                {
                    self.finish_take(take, false);
                }
                let t = decide_transition(
                    true,
                    self.is_recording(),
                    self.settings.recording_mode,
                    false,
                );
                if t.start {
                    self.start(at_ns);
                } else if t.stop {
                    self.stop(at_ns);
                }
            }
            HotkeyEvent::Released { at_ns } => {
                if let Hold::Pending { press_ns } = self.hold {
                    if at_ns.saturating_sub(press_ns) < ns(self.settings.min_hold) {
                        self.abandon();
                        self.emit(DictationEvent::ShortPressIgnored);
                        return;
                    }
                    self.confirm();
                }
                let t = decide_transition(
                    false,
                    self.is_recording(),
                    self.settings.recording_mode,
                    false,
                );
                if t.stop {
                    self.stop(at_ns);
                }
            }
            HotkeyEvent::Cancelled => self.end_hold(false),
            HotkeyEvent::Lost => {
                self.end_hold(true);
                self.emit(DictationEvent::HotkeyLost);
            }
        }
    }

    /// Ends a take whose tail stopped arriving, once its deadline has passed. Call it when the
    /// owning thread wakes at [`deadline_ns`](Self::deadline_ns); early calls do nothing.
    pub fn tick(&mut self) {
        if let Hold::Tail { deadline_ns, .. } = self.hold
            && self.services.clock.now_ns() >= deadline_ns
            && let Some(take) = self.recorder.finish()
        {
            self.finish_take(take, true);
        }
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
            Hold::Recording | Hold::Tail { .. } => {
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

    fn start(&mut self, at_ns: u64) {
        if !self.recorder.press(self.position_at(at_ns)) {
            // Cannot happen: every path that leaves `Idle` closes the recorder's take first.
            log::error!("dictation: a take was already open at a press; the press was ignored");
            return;
        }
        self.tail.begin(self.recorder.position());
        self.open = Some(Open {
            started_unix_ms: self.services.clock.unix_ms(),
            lost_frames: 0,
        });
        self.hold = Hold::Pending { press_ns: at_ns };
        if self.settings.min_hold.is_zero() {
            self.confirm();
        }
    }

    fn confirm(&mut self) {
        self.hold = Hold::Recording;
        self.emit(DictationEvent::Started);
    }

    /// Drops the open take unseen.
    fn abandon(&mut self) {
        self.recorder.cancel();
        self.open = None;
        self.hold = Hold::Idle;
    }

    /// The key came up (or, in toggle mode, went down again) at `at_ns`.
    fn stop(&mut self, at_ns: u64) {
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
            Hold::Recording
                if lost || self.settings.recording_mode == RecordingMode::PushToTalk =>
            {
                self.stop(self.services.clock.now_ns());
            }
            Hold::Idle | Hold::Recording | Hold::Tail { .. } => {}
        }
    }

    fn finish_take(&mut self, take: Take, cut_short: bool) {
        self.hold = Hold::Idle;
        let open = self.open.take();
        if cut_short {
            self.emit(DictationEvent::Warning(Warning::TailCutShort));
        }
        let lost = open.map_or(0, |o| o.lost_frames);
        if lost > 0 {
            self.emit(DictationEvent::Warning(Warning::AudioLost { frames: lost }));
        }
        let started = open.map_or_else(|| self.services.clock.unix_ms(), |o| o.started_unix_ms);
        self.process(take, started);
    }

    /// Stages 3–11 for one take.
    fn process(&mut self, take: Take, started_unix_ms: i64) {
        let live_ms = samples_to_ms(take.live());
        if take.live() < duration_to_samples(self.settings.min_live) {
            self.emit(DictationEvent::Discarded(Discard::TooShort { live_ms }));
            return;
        }

        // Stage 3.
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
            return;
        }

        // Stage 4.
        let options = TranscribeOptions {
            channel: Channel::Mic,
            context: self.settings.dictionary.hotwords(),
            cancel: CancelToken::new(),
        };
        let raw = match self.services.engine.transcribe(&levelled.audio, &options) {
            Ok(transcript) => transcript.text(),
            Err(error) => {
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
            self.apply_command(&action);
            self.emit(DictationEvent::Command(action));
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
            .resolve_with_override(app.as_deref(), self.pinned_mode.as_deref())
            .clone();
        let vars = SnippetVars::at(
            self.services.clock.unix_ms(),
            self.settings.utc_offset_minutes,
        );
        let written = text::write(
            &raw,
            &mode,
            &self.settings.dictionary,
            &self.settings.snippets,
            &vars,
        );

        // Stage 9.
        let written = self.polish(written, &mode);
        if written.trim().is_empty() {
            self.emit(DictationEvent::Discarded(Discard::NothingHeard));
            return;
        }

        // Stage 10.
        let record = match self.save(&written, started_unix_ms, live_ms, app) {
            Ok(id) => Some(id),
            Err(error) => {
                self.emit(DictationEvent::Warning(Warning::SaveFailed(error)));
                None
            }
        };

        // Stage 11. The space is added here only: the record and the event hold what was said.
        let insert = if self.settings.append_space {
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
    /// as written, and says so.
    fn polish(&self, written: String, mode: &Mode) -> String {
        if !self.polish_override.unwrap_or(mode.polish_enabled) {
            return written;
        }
        let Some(llm) = &self.services.llm else {
            self.emit(DictationEvent::Warning(Warning::PolishUnavailable));
            return written;
        };
        let prompt = if mode.polish_prompt.trim().is_empty() {
            &self.settings.polish_prompt
        } else {
            &mode.polish_prompt
        };
        match ink_llm::tasks::polish::polish(llm.as_ref(), prompt, &written, &CancelToken::new()) {
            Ok(polished) => polished,
            Err(error) => {
                self.emit(DictationEvent::Warning(Warning::PolishFailed(error)));
                written
            }
        }
    }

    /// Stage 10: one dictation record with one mic segment. A record left half-written is
    /// deleted, so the library never shows an empty dictation.
    fn save(
        &self,
        written: &str,
        started_unix_ms: i64,
        live_ms: u64,
        app: Option<String>,
    ) -> Result<RecordId, StoreError> {
        let store = &self.services.store;
        let id = store.create_record(NewRecord {
            kind: RecordKind::Dictation,
            title: None,
            started_at_unix_ms: started_unix_ms,
            source_app: app,
            // A dictation keeps no audio.
            audio_dir: None,
        })?;
        let filled = store
            .append_segments(
                &id,
                &[Segment {
                    channel: Channel::Mic,
                    start_ms: 0,
                    end_ms: live_ms,
                    text: written.to_owned(),
                    speaker: None,
                }],
            )
            .and_then(|()| store.finish_record(&id, self.services.clock.unix_ms()));
        if let Err(error) = filled {
            if let Err(cleanup) = store.delete_record(&id) {
                log::warn!("dictation: a half-saved record could not be removed: {cleanup}");
            }
            return Err(error);
        }
        Ok(id)
    }

    /// The chain's part of a voice command: style and polish. The rest is the shell's.
    fn apply_command(&mut self, action: &CommandAction) {
        match action {
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
                let now = self.polish_override.unwrap_or(mode.polish_enabled);
                self.polish_override = Some(!now);
            }
            _ => {}
        }
    }
}
