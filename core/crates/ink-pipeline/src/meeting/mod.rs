//! The meeting chain: two sides live, then the final pass ("blotting").
//!
//! ```text
//! live:  mic ─┐                                      partials ─► events only
//!             ├─► canonical ─► Agc + VAD ─► live engine ─► finals ─► checked ─► revision 1
//!        far ─┘   (chunks on disk: the pump)      (the mic: AEC3's linear output once an echo
//!                                                  path is found; its finals behind the echo gate)
//!
//! end:   chunks ─► echo path? ─► AEC3 ─► speech regions ─► final engine ─► mic segments ─► dedup ─┐
//!        chunks ─► speech regions ─► diarizer (far only, rule 5) ─► final engine ─────────────────┴─► supersede ─► summary ─► commitments
//! ```
//!
//! | Stage | Here |
//! |---|---|
//! | Capture and chunks | [`capture`](crate::capture): the pump writes each side's chunks and hands canonical audio on |
//! | Live, per side | `live`: the AGC with a VAD (the fallback when it fails), the live engine, finals placed in the meeting and saved only over VAD speech |
//! | Final pass, per side | `offline`: the chunks read back, VAD-gated gain, one engine call per speech region, empty regions reported |
//! | Echo | [`echo`]: live, the mic cancelled along an echo path once one is found, "you" finals behind the echo gate; at the end, a path fitted over the whole recording, the linear output transcribed where the full output holds speech, "you" lines that repeat the far end removed (and handed back) |
//! | Diarization | [`diarize`]: the far end only, its speech streamed from disk into the diarizer a window at a time; labels kept with at least two substantial clusters |
//! | Supersede | the final pass replaces the live transcript in one transaction, as revision 2, unless the guard ([`check_supersede_explained`]: live "you" finals the pass judged to be echo do not count) refuses it or a region failed |
//! | Summary, commitments | `ink-llm`: a summary (in overlapping windows when long), commitments from it and from the mic, deduplicated |
//!
//! **Me versus them is stream identity** (architecture rule 5): every segment keeps the side it was
//! captured on, from the live final to the final pass; nothing infers it.
//!
//! # Threads
//!
//! **Worker**, every method: one thread owns a chain. The live engine's events arrive on its own
//! callback thread and are queued; the chain takes them after each block and when it stops.
//! [`EndedMeeting::finalize`] runs for as long as the engines take and checks its cancel token
//! between regions.

pub mod diarize;
pub mod echo;
pub mod events;
mod live;
pub(crate) mod offline;
pub mod timeline;
pub mod watchdog;

pub use live::MAX_PENDING_FINALS;

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};

use ink_audio::{ChunkError, ChunkStore, VadConfig, WindowError};
use ink_core::store::check_supersede_explained;
use ink_core::{
    AsrEvent, CancelToken, Channel, Clock, Diarizer, DoneEvidence, EngineError, EventSink,
    Explained, Llm, LlmError, NewCommitment, NewRecord, OfflineEngine, Record, RecordId,
    RecordKind, Segment, Store, StoreError, StreamingEngine, SupersedeWith,
};
use ink_llm::tasks::commitments::{RecordContext, harvest, looks_done};
use ink_llm::tasks::dedup::dedup;
use ink_llm::tasks::due::RecordTime;
use ink_llm::tasks::summary::{SummaryOptions, has_citable_line, summarize};

use ink_echo::{DedupConfig, EchoError, PathReport, echo_duplicates};

use self::diarize::{rule5, to_meeting};
use self::echo::{EchoEvidence, LiveEcho, MicBlock};
use self::events::{
    EchoPass, EchoPath, EchoSearch, EchoState, FarLine, KeptLive, MeetingEvent, MeetingWarning,
    Phase, RemovedEcho,
};
use self::live::{LiveChannel, Settled};
use self::offline::{EchoReader, Pass, RegionWindows, SideRead, SideReader, Stop};
use self::watchdog::{Routing, SideState, Watch, Watchdog};
use crate::capture::SideSummary;
use crate::consent::{Consented, Feature, LlmConsent, stored};
use crate::redact::Spoken;
use crate::speech::{RegionConfig, SpeechPass, VadSource, little_speech_heard};

/// What the chain calls.
#[derive(Clone)]
pub struct MeetingServices {
    /// The live engine (the router's choice for live partials), when one is installed.
    pub live: Option<Arc<dyn StreamingEngine>>,
    /// The final-pass engine (the router's choice for the meeting final).
    pub offline: Arc<dyn OfflineEngine>,
    /// The far end's diarizer, when one is installed.
    pub diarizer: Option<Arc<dyn Diarizer>>,
    /// The library.
    pub store: Arc<dyn Store>,
    /// The platform clock: the timebase of capture.
    pub clock: Arc<dyn Clock>,
    /// The model for the summary and commitments, when one is set up.
    pub llm: Option<Arc<dyn Llm>>,
}

/// How meetings behave.
#[derive(Clone, Debug, Default)]
pub struct MeetingSettings {
    /// The VAD's thresholds, for the AGC and the final pass.
    pub vad: VadConfig,
    /// How speech regions are formed for the final pass.
    pub regions: RegionConfig,
    /// When the summary is written in windows.
    pub summary: SummaryOptions,
    /// The user's UTC offset in minutes, for resolving spoken deadlines.
    pub utc_offset_minutes: i32,
    /// Words the final-pass engine should favour (the dictionary, names).
    pub context: Option<String>,
}

/// What is known about a meeting when it starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeetingStart {
    /// A title (a calendar event), if known. Otherwise the summary's headline becomes it.
    pub title: Option<String>,
    /// The meeting app.
    pub source_app: Option<String>,
    /// Where its chunks are written, relative to the data directory.
    pub audio_dir: Option<String>,
    /// How the capture is routed, for the silent-channel watchdog ([`watchdog`]).
    pub routing: Routing,
}

/// A live meeting. See the module docs.
pub struct MeetingChain {
    core: Core,
    mic: LiveChannel,
    far: LiveChannel,
    asr: Receiver<(Channel, AsrEvent)>,
    watchdog: Watchdog,
    /// Echo cancellation for the live mic ([`echo`]).
    echo: LiveEcho,
}

/// What a meeting keeps from start to end.
struct Core {
    services: MeetingServices,
    settings: MeetingSettings,
    vad: VadSource,
    events: EventSink<MeetingEvent>,
    record: RecordId,
    started_unix_ms: i64,
    t0_ns: u64,
    /// What the pump reported writing for each side (mic, far).
    written: [Option<SideSummary>; 2],
    /// Live finals each side saved unchecked (mic, far).
    backlogged: [u64; 2],
    /// Whether the mic has been a Bluetooth headset mic for the whole meeting so far: then a
    /// meeting of its zeros can be its user's silence. Cleared for good by the first route that is
    /// not Bluetooth, because zeros from any other mic are no data at all (S2.8: the flag used to
    /// stick once set, so a built-in mic that captured only zeros after a headset disconnected got
    /// the softer warning).
    mic_bluetooth: bool,
    /// Live events that arrived after the meeting stopped.
    late: Arc<Late>,
}

/// Live events after stop: the engine's sinks count them once the queue is closed.
#[derive(Debug, Default)]
struct Late {
    closed: AtomicBool,
    count: AtomicU64,
}

impl Core {
    fn emit(&self, event: MeetingEvent) {
        (self.events)(event);
    }

    fn warn(&self, warning: MeetingWarning) {
        self.emit(MeetingEvent::Warning(warning));
    }

    /// A read of the record for the final pass: a failure is reported, and the pass goes on
    /// without the value.
    fn read<T>(&self, what: Result<T, StoreError>) -> Option<T> {
        match what {
            Ok(value) => Some(value),
            Err(error) => {
                log::warn!("meeting final pass: a read of the record failed: {error}");
                self.warn(MeetingWarning::StoreFailed(error));
                None
            }
        }
    }
}

impl MeetingChain {
    /// **Worker.** Creates the meeting's record (revision 1), stamps its start on the platform
    /// clock, and opens a live stream per side. Capture should start right after: audio stamped
    /// before this moment is left out of the meeting.
    pub fn start(
        services: MeetingServices,
        settings: MeetingSettings,
        vad: VadSource,
        events: EventSink<MeetingEvent>,
        start: MeetingStart,
    ) -> Result<Self, StoreError> {
        let started_unix_ms = services.clock.unix_ms();
        let t0_ns = services.clock.now_ns();
        let record = services.store.create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: start.title,
            started_at_unix_ms: started_unix_ms,
            source_app: start.source_app,
            audio_dir: start.audio_dir,
        })?;
        events(MeetingEvent::Started {
            record: record.clone(),
        });
        let core = Core {
            services,
            settings,
            vad,
            events,
            record,
            started_unix_ms,
            t0_ns,
            written: [None, None],
            backlogged: [0, 0],
            mic_bluetooth: start.routing.mic == ink_core::Transport::Bluetooth,
            late: Arc::default(),
        };
        let (tx, asr) = mpsc::channel();
        let open = |channel: Channel| {
            let (vad, error) = core.vad.open();
            if let Some(error) = error {
                core.warn(MeetingWarning::VadFailed {
                    channel,
                    phase: Phase::Live,
                    error,
                });
            }
            let stream = core.services.live.as_ref().and_then(|engine| {
                let tx = tx.clone();
                let late = core.late.clone();
                // Callback thread: it only enqueues (or counts). The queue is read until the chain
                // stops; an event after that breaks the stream's contract, and is counted, since
                // nothing can act on it (MeetingWarning::LiveEventsAfterStop).
                let sink: EventSink<AsrEvent> = Arc::new(move |event| {
                    if late.closed.load(Ordering::Acquire) || tx.send((channel, event)).is_err() {
                        late.count.fetch_add(1, Ordering::Relaxed);
                    }
                });
                match engine.open_stream(channel, sink) {
                    Ok(stream) => Some(stream),
                    Err(error) => {
                        core.warn(MeetingWarning::LiveEngineFailed { channel, error });
                        None
                    }
                }
            });
            LiveChannel::new(
                channel,
                vad,
                core.settings.vad,
                stream,
                t0_ns,
                &*core.events,
            )
        };
        let mic = open(Channel::Mic);
        let far = open(Channel::Far);
        // Unprotected from here until the search finds an echo path (if there is one to find).
        core.emit(MeetingEvent::Echo(EchoState::Searching {
            since_ms: 0,
            why: EchoSearch::Start,
        }));
        let echo = LiveEcho::new(t0_ns, core.vad.clone());
        Ok(Self {
            core,
            mic,
            far,
            asr,
            watchdog: Watchdog::new(start.routing, t0_ns),
            echo,
        })
    }

    /// The meeting's record.
    pub fn record(&self) -> &RecordId {
        &self.core.record
    }

    /// The host time the meeting started at: sample 0 of its timeline.
    pub fn start_ns(&self) -> u64 {
        self.core.t0_ns
    }

    fn side(&mut self, channel: Channel) -> &mut LiveChannel {
        match channel {
            Channel::Mic => &mut self.mic,
            Channel::Far => &mut self.far,
        }
    }

    /// The next block of one side: 16 kHz mono from [`MicPath`](crate::mic::MicPath), the host
    /// time of its first sample, and the device frames the capture ring dropped before it.
    pub fn push_audio(
        &mut self,
        channel: Channel,
        samples: &[f32],
        host_time_ns: u64,
        dropped_frames: u64,
    ) {
        if dropped_frames > 0 {
            self.core.warn(MeetingWarning::AudioLost {
                channel,
                frames: dropped_frames,
            });
        }
        let events = self.core.events.clone();
        // The mic reaches its live channel through the echo stage: as captured, or cancelled.
        if channel == Channel::Far {
            self.far.push(samples, host_time_ns, &*events);
        }
        let mut mic = Vec::new();
        self.echo.push(
            channel,
            samples,
            host_time_ns,
            dropped_frames > 0,
            &mut mic,
            &*events,
        );
        self.push_mic(mic);
        self.collect(false);
        let now = self.core.services.clock.now_ns();
        for watch in self.watchdog.observe(channel, samples, now) {
            self.watched(watch);
        }
    }

    /// Judges both sides with no audio arriving: the silent-channel watchdog. The owning thread
    /// calls it when it wakes at [`deadline_ns`](Self::deadline_ns) (the chain has no timer; S1.7
    /// wires this into the worker's wait, as the dictation worker does with its tail).
    pub fn tick(&mut self) {
        let now = self.core.services.clock.now_ns();
        for watch in self.watchdog.check(now) {
            self.watched(watch);
        }
    }

    /// When [`tick`](Self::tick) should next run, as host time, if no audio arrives before.
    pub fn deadline_ns(&self) -> Option<u64> {
        self.watchdog.deadline_ns()
    }

    /// The capture's routing changed (a device switched, the headset-mic setting). A new device
    /// is a new echo path: live cancellation stops and the search starts again
    /// ([`EchoSearch::DeviceSwitch`]).
    pub fn set_routing(&mut self, routing: Routing) {
        self.core.mic_bluetooth &= routing.mic == ink_core::Transport::Bluetooth;
        self.watchdog.set_routing(routing);
        let events = self.core.events.clone();
        let mut mic = Vec::new();
        self.echo.device_switched(&mut mic, &*events);
        self.push_mic(mic);
        self.collect(false);
    }

    /// Mic audio from the echo stage into the mic's live channel.
    fn push_mic(&mut self, blocks: Vec<MicBlock>) {
        let events = self.core.events.clone();
        for block in blocks {
            self.mic.push(&block.samples, block.host_ns, &*events);
        }
    }

    fn watched(&self, watch: Watch) {
        match watch {
            Watch::State { channel, state } => {
                if state != SideState::Ok {
                    log::warn!("meeting: the {channel:?} side is {state:?}");
                }
                self.core.emit(MeetingEvent::SideState { channel, state });
            }
            Watch::FarQuietWhileYouSpeak { quiet_ms } => {
                log::info!("meeting: no far-end audio for {quiet_ms} ms while the mic is audible");
                self.core
                    .warn(MeetingWarning::FarEndQuietWhileYouSpeak { quiet_ms });
            }
        }
    }

    /// What the pump wrote for one side ([`SideCapture::finish`](crate::capture::SideCapture::finish)).
    /// It travels with that side's final pass as [`ChannelPass::chunks_written`](events::ChannelPass).
    pub fn capture_ended(&mut self, summary: SideSummary) {
        self.core.written[usize::from(summary.channel == Channel::Far)] = Some(summary);
    }

    /// Something the pump could not do for one side ([`SideCapture`](crate::capture::SideCapture)).
    pub fn capture_issue(&mut self, channel: Channel, issue: crate::capture::CaptureIssue) {
        log::warn!("meeting: {issue}");
        self.core.warn(MeetingWarning::Capture { channel, issue });
    }

    /// Takes the live engine's queued events, and saves the finals the VAD has judged.
    fn collect(&mut self, finishing: bool) {
        let events = self.core.events.clone();
        while let Ok((channel, event)) = self.asr.try_recv() {
            match event {
                AsrEvent::Partial { text } => self.core.emit(MeetingEvent::Partial {
                    channel,
                    text: Spoken::new(text),
                }),
                AsrEvent::Final(text) => {
                    if let Some(overflow) = self.side(channel).final_heard(text, &*events) {
                        self.save(overflow);
                    }
                }
                AsrEvent::Stalled { .. } => self
                    .core
                    .warn(MeetingWarning::LiveEngineStalled { channel }),
            }
        }
        for channel in [Channel::Mic, Channel::Far] {
            for settled in self.side(channel).settle(finishing) {
                self.save(settled);
            }
        }
        self.save_released();
    }

    /// Saves the "you" finals the echo gate has judged, and reports those it judged echo.
    fn save_released(&mut self) {
        for (segment, keep) in self.echo.released() {
            if keep {
                self.save_final(segment);
            } else {
                let (start_ms, end_ms) = (segment.start_ms, segment.end_ms);
                log::info!(
                    "meeting: a live you final at {start_ms} ms was echo of the far end; not saved"
                );
                self.core
                    .warn(MeetingWarning::EchoOnlyFinal { start_ms, end_ms });
            }
        }
    }

    fn save(&mut self, settled: Settled) {
        match settled {
            // A "you" final waits for the echo gate.
            Settled::Keep(segment) if segment.channel == Channel::Mic => self.echo.hold(segment),
            Settled::Keep(segment) => self.save_final(segment),
            Settled::NoSpeech {
                channel,
                start_ms,
                end_ms,
            } => {
                log::info!(
                    "meeting: a {channel:?} live final at {start_ms} ms lay outside VAD speech; not saved"
                );
                self.core.warn(MeetingWarning::FinalWithoutSpeech {
                    channel,
                    start_ms,
                    end_ms,
                });
            }
        }
    }

    fn save_final(&self, segment: Segment) {
        let store = &self.core.services.store;
        if let Err(error) = store.append_segments(&self.core.record, std::slice::from_ref(&segment))
        {
            log::warn!("meeting: a live final could not be saved: {error}");
            self.core.warn(MeetingWarning::StoreFailed(error));
        }
        self.core.emit(MeetingEvent::Final {
            channel: segment.channel,
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            text: Spoken::new(segment.text),
        });
    }

    /// **Worker.** Ends the live phase: flushes each side into its live engine, saves the trailing
    /// finals, and marks the record ended. Call it once capture has stopped and the pump has handed
    /// on the last audio.
    ///
    /// The end is never recorded before the start: the wall clock can be set back during a
    /// meeting, and then the start is used ([`MeetingWarning::ClockWentBack`]).
    pub fn stop(mut self) -> EndedMeeting {
        let events = self.core.events.clone();
        let mut mic = Vec::new();
        self.echo.flush(&mut mic);
        self.push_mic(mic);
        self.mic.finish(&*events);
        self.far.finish(&*events);
        self.collect(true);
        self.echo.close(&*events);
        self.save_released();
        // From here, a live event is late: every stream has finished. What raced in before the
        // close is counted too.
        self.core.backlogged = [self.mic.backlogged(), self.far.backlogged()];
        self.core.late.closed.store(true, Ordering::Release);
        while self.asr.try_recv().is_ok() {
            self.core.late.count.fetch_add(1, Ordering::Relaxed);
        }
        let core = self.core;
        let now = core.services.clock.unix_ms();
        if now < core.started_unix_ms {
            log::warn!("meeting: the wall clock went back during the meeting; end set to start");
            core.warn(MeetingWarning::ClockWentBack);
        }
        let record_ended = match core
            .services
            .store
            .finish_record(&core.record, now.max(core.started_unix_ms))
        {
            Ok(()) => true,
            Err(error) => {
                log::warn!("meeting: the record could not be marked ended: {error}");
                core.warn(MeetingWarning::StoreFailed(error));
                false
            }
        };
        core.emit(MeetingEvent::Stopped);
        EndedMeeting { core, record_ended }
    }
}

/// A meeting whose live phase is over, waiting for its final pass.
pub struct EndedMeeting {
    core: Core,
    /// Whether the record was marked ended ([`record_ended`](Self::record_ended)).
    record_ended: bool,
}

/// A meeting whose live phase ended without [`MeetingChain::stop`]: the app was killed or crashed
/// while it recorded. What the final pass needs is read back from the record and from beside its
/// chunks by the caller, who has already run [`ChunkStore::recover`] over them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interrupted {
    /// The meeting's record: still revision 1, with the live finals saved before the crash.
    pub record: RecordId,
    /// When it started, Unix ms (the record's start).
    pub started_unix_ms: i64,
    /// The host time of its timeline's start, as written beside its chunks.
    pub t0_ns: u64,
    /// When it ended, Unix ms: the end of its last recorded audio, as the caller works it out.
    /// Never recorded before the start.
    pub ended_unix_ms: i64,
}

/// What a meeting's final pass came to.
#[derive(Clone, Debug, PartialEq)]
pub struct MeetingOutcome {
    /// The transcript's revision now: 2 after a first supersede, 1 when the live one was kept.
    /// `None` only when the live one was kept and the record could not be read
    /// ([`MeetingWarning::StoreFailed`] says so).
    pub revision: Option<u32>,
    /// Whether the final pass replaced the live transcript.
    pub superseded: bool,
    /// The mic side's pass.
    pub mic: events::ChannelPass,
    /// The far side's pass.
    pub far: events::ChannelPass,
    /// What diarization did, when a diarizer is installed.
    pub diarization: Option<events::Diarization>,
    /// What echo cancellation did.
    pub echo: EchoPass,
    /// "You" lines removed as echo of the far end, whole (also sent as
    /// [`MeetingEvent::RemovedAsEcho`]). The supersede keeps them with the record in its own
    /// transaction ([`Store::supersede_with`]), so they can be put back.
    pub removed_as_echo: Vec<RemovedEcho>,
}

/// Why a final pass stopped before replacing the live transcript. Every one of these happens
/// before anything is written: the live transcript stands, and the pass can run again. Once the
/// final pass is saved, nothing stops it from finishing (see [`EndedMeeting::finalize`]).
#[derive(Debug)]
#[non_exhaustive]
pub enum FinalizeError {
    /// The cancel token was set.
    Cancelled,
    /// The recorded audio could not be listed.
    Chunks(ChunkError),
    /// The region sizes in the settings cannot work.
    Regions(WindowError),
    /// Echo cancellation refused the audio outside the mic pass, which falls back on its own
    /// ([`MeetingWarning::EchoFailed`]).
    Echo(EchoError),
}

impl fmt::Display for FinalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("meeting final pass cancelled"),
            Self::Chunks(e) => write!(f, "meeting final pass: {e}"),
            Self::Regions(e) => write!(f, "meeting final pass: {e}"),
            Self::Echo(e) => write!(f, "meeting final pass: {e}"),
        }
    }
}

impl std::error::Error for FinalizeError {}

impl From<Stop> for FinalizeError {
    fn from(stop: Stop) -> Self {
        match stop {
            Stop::Cancelled => Self::Cancelled,
            Stop::Chunks(e) => Self::Chunks(e),
            Stop::Window(e) => Self::Regions(e),
            Stop::Echo(e) => Self::Echo(e),
        }
    }
}

impl EndedMeeting {
    /// **Worker.** An interrupted meeting ([`Interrupted`]), ready for its final pass: the record is
    /// marked ended now, as [`MeetingChain::stop`] would have. What the live phase knew and a crash
    /// lost is taken at its most cautious: how many chunks the pump wrote is unknown (the pass
    /// counts what is on disk), and the mic is not taken for a Bluetooth headset, so a meeting of
    /// its zeros is reported as no data, never as a silent listener.
    pub fn interrupted(
        services: MeetingServices,
        settings: MeetingSettings,
        vad: VadSource,
        events: EventSink<MeetingEvent>,
        meeting: Interrupted,
    ) -> Self {
        let core = Core {
            services,
            settings,
            vad,
            events,
            record: meeting.record,
            started_unix_ms: meeting.started_unix_ms,
            t0_ns: meeting.t0_ns,
            written: [None, None],
            backlogged: [0, 0],
            mic_bluetooth: false,
            late: Arc::default(),
        };
        let ended = meeting.ended_unix_ms.max(meeting.started_unix_ms);
        let record_ended = match core.services.store.finish_record(&core.record, ended) {
            Ok(()) => true,
            Err(error) => {
                log::warn!("meeting recovery: the record could not be marked ended: {error}");
                core.warn(MeetingWarning::StoreFailed(error));
                false
            }
        };
        Self { core, record_ended }
    }

    /// The meeting's record.
    pub fn record(&self) -> &RecordId {
        &self.core.record
    }

    /// Whether the record was marked ended when the live phase ended (or, for an interrupted
    /// meeting, when it was taken up). `false` when the store refused: the record still reads as
    /// live, so whoever keeps a crash-recovery marker for it keeps it, and the next launch ends
    /// it (a record without an end is also never swept).
    pub fn record_ended(&self) -> bool {
        self.record_ended
    }

    /// **Worker.** The final pass over the meeting's recorded chunks in `audio`, then the
    /// supersede, the summary and the commitments. See the module docs.
    ///
    /// Until the supersede, an error (a cancellation included) leaves the live transcript as it was,
    /// and the pass can run again. From the supersede on, nothing turns the pass into an error: a
    /// store that fails, a model that fails, even a cancellation, is reported as a warning, the
    /// rest is skipped where it must be, and [`MeetingEvent::Finished`] still comes. What the pass
    /// needs from the record (its title and revision, the speakers' names, the live transcript) is
    /// read before anything is written, for the same reason.
    pub fn finalize(
        &self,
        audio: &ChunkStore,
        cancel: &CancelToken,
    ) -> Result<MeetingOutcome, FinalizeError> {
        let core = &self.core;
        let ctx = Pass {
            engine: core.services.offline.as_ref(),
            context: core.settings.context.as_deref(),
            cancel,
            emit: &*core.events,
        };

        // Echo: a path fitted over the whole recording, from the chunks (never the live search).
        let fit = offline::fit_path(audio, core.t0_ns, cancel).or_else(|stop| match stop {
            Stop::Echo(error) => {
                log::warn!("meeting final pass: the echo path search failed: {error}");
                core.warn(MeetingWarning::EchoFailed(error.into()));
                Ok(None)
            }
            stop => Err(stop),
        })?;
        let mut echo = echo_pass(fit.as_ref().map(|(r, _)| r));
        if let Some((report, following)) = &fit
            && report.path.is_none()
        {
            // No path, so no cancellation: said aloud when the mic followed the far end anyway.
            let (heard_ms, follows) = following.report();
            if heard_ms >= echo::FOLLOW_MIN_HEARD_MS && follows >= echo::FOLLOWS {
                log::warn!(
                    "meeting final pass: no echo path, yet the mic followed the far end (correlation {follows:.2} over {heard_ms} ms); the mic is transcribed as captured"
                );
                core.warn(MeetingWarning::EchoPathNotFound { heard_ms });
            }
        }

        // The mic: each region transcribed as it is read; along the echo path, if there is one.
        let path = fit.and_then(|(r, _)| r.path);
        let mut evidence = None;
        let (mut mic, mic_report) = match path {
            Some(path) => match self.mic_pass_cancelled(audio, path, &ctx) {
                Ok((mic, report, heard, erle)) => {
                    echo.cancelled = true;
                    echo.erle_first_db = erle.first().erle_db();
                    echo.erle_db = erle.after().erle_db();
                    echo.linear_erle_db = erle.linear().erle_db();
                    evidence = Some(heard);
                    (mic, report)
                }
                Err(Stop::Echo(error)) => {
                    log::warn!(
                        "meeting final pass: echo cancellation failed ({error}); the mic is transcribed as captured"
                    );
                    core.warn(MeetingWarning::EchoFailed(error.into()));
                    self.mic_pass(audio, &ctx)?
                }
                Err(stop) => return Err(stop.into()),
            },
            None => self.mic_pass(audio, &ctx)?,
        };

        // The far end. With a diarizer, in two passes over the recorded audio, each holding one
        // region at a time: the diarizer hears the far end's speech streamed from disk, then each
        // region is read again and transcribed, cut where the speaker changes.
        let mut far_report = offline::report(Channel::Far);
        let mut far = Vec::new();
        let mut decided = None;
        let mut first_vad_error = None;
        if let Some(diarizer) = &core.services.diarizer {
            let (mut reader, opened) = self.open_reader(audio, Channel::Far)?;
            let mut windows = RegionWindows::new(&mut reader);
            let turns = diarizer.diarize(&mut windows, cancel);
            if let Some(stop) = windows.failed.take() {
                return Err(stop.into());
            }
            let pieces = std::mem::take(&mut windows.pieces);
            first_vad_error = opened.or_else(|| reader.vad_error().cloned());
            decided = match turns {
                Ok(turns) => Some(rule5(&to_meeting(&turns, &pieces))),
                Err(EngineError::Cancelled) => return Err(FinalizeError::Cancelled),
                Err(error) => {
                    log::warn!("meeting final pass: diarization failed: {error}");
                    core.warn(MeetingWarning::DiarizationFailed(error));
                    None
                }
            };
        }
        let labels = decided.as_ref().and_then(|r| r.turns.as_deref());
        // Where the far end's own VAD heard speech: the regions it gave the engine.
        let mut far_speech = Vec::new();
        let (mut reader, opened) = self.open_reader(audio, Channel::Far)?;
        while let Some(region) = reader.next_region()? {
            far_speech.push((region.start_ms(), region.end_ms()));
            far.extend(offline::transcribe_region(
                &ctx,
                Channel::Far,
                &region,
                labels,
                &mut far_report,
            )?);
        }
        let read = self.close_side(
            Channel::Far,
            opened
                .or(first_vad_error)
                .or_else(|| reader.vad_error().cloned()),
            reader.summary(),
        );
        self.account(&mut far_report, read);
        let diarization = decided.map(|r| events::Diarization {
            clusters: r.clusters,
            substantial: r.substantial.len(),
            labelled: r.turns.is_some(),
            attributed: far.iter().filter(|s| s.speaker.is_some()).count(),
        });
        core.emit(MeetingEvent::Transcribed(far_report));
        if let Some(d) = diarization {
            core.emit(MeetingEvent::Diarized(d));
        }

        // "You" lines that repeat the far end where nobody on the near end spoke: only with the
        // full output's verdicts (without a path there is no acoustic evidence to judge by).
        let removed_as_echo = match &evidence {
            Some(heard) => remove_echo(&mut mic, &far, heard, &mut echo),
            None => Vec::new(),
        };
        if !removed_as_echo.is_empty() {
            log::info!(
                "meeting final pass: {} you lines removed as echo",
                removed_as_echo.len()
            );
        }

        // Read before anything is written: a failure here is reported and the pass goes on
        // without what it could not read.
        let store = &core.services.store;
        let record = core.read(store.record(&core.record)).flatten();
        let names = core
            .read(store.speaker_names(&core.record))
            .unwrap_or_default();
        let previous = core.read(store.segments(&core.record));

        // The live "you" finals this pass judges to be echo (the unprotected ones from before the
        // live search found the path, mostly): the supersede guard does not count them.
        let explained = match (&evidence, &previous) {
            (Some(evidence), Some(previous)) => live_echo(previous, &far, &far_speech, evidence),
            _ => Vec::new(),
        };
        echo.live_echo_finals = explained.len();
        core.emit(MeetingEvent::EchoPass(echo));
        if !removed_as_echo.is_empty() {
            core.emit(MeetingEvent::RemovedAsEcho(removed_as_echo.clone()));
        }

        let mut new: Vec<Segment> = mic.into_iter().chain(far).collect();
        new.sort_by_key(|s| (s.start_ms, s.channel));

        // The lines this pass removed as echo are kept with the record, so they can be put back
        // (an empty list clears what an earlier pass kept), in the supersede's own transaction:
        // the transcript never loses a line whose undo copy did not persist.
        let removed: Vec<Segment> = removed_as_echo
            .iter()
            .map(|r| Segment {
                channel: Channel::Mic,
                start_ms: r.start_ms,
                end_ms: r.end_ms,
                text: r.text.as_str().to_owned(),
                speaker: None,
            })
            .collect();
        let failed = mic_report.failed_regions + far_report.failed_regions;
        let saved = self.supersede(
            &new,
            failed,
            previous.as_deref(),
            SupersedeWith {
                explained: &explained,
                removed: Some(&removed),
            },
        );
        self.report_scrub();
        let revision = saved.or(record.as_ref().map(|r| r.revision));
        // The transcript now, from memory: the pass just saved, or the live one as read.
        let current = if saved.is_some() {
            Some(new.as_slice())
        } else {
            previous.as_deref()
        };
        match current {
            Some(segments) => self.wrap_up(segments, record.as_ref(), &names, cancel),
            None => {
                log::warn!("meeting final pass: no summary; the live transcript could not be read")
            }
        }
        let late = core.late.count.load(Ordering::Relaxed);
        if late > 0 {
            log::warn!("meeting: the live engine sent {late} events after the meeting stopped");
            core.warn(MeetingWarning::LiveEventsAfterStop { count: late });
        }
        // The summary and title replace text too.
        self.report_scrub();
        core.emit(MeetingEvent::Finished { revision });
        Ok(MeetingOutcome {
            revision,
            superseded: saved.is_some(),
            mic: mic_report,
            far: far_report,
            diarization,
            echo,
            removed_as_echo,
        })
    }

    /// The mic as captured: each region transcribed as it is read.
    fn mic_pass(
        &self,
        audio: &ChunkStore,
        ctx: &Pass<'_>,
    ) -> Result<(Vec<Segment>, events::ChannelPass), FinalizeError> {
        let mut report = offline::report(Channel::Mic);
        let mut mic = Vec::new();
        let (mut reader, opened) = self.open_reader(audio, Channel::Mic)?;
        while let Some(region) = reader.next_region()? {
            mic.extend(offline::transcribe(
                ctx,
                Channel::Mic,
                &region.audio,
                region.start,
                &mut report,
            )?);
        }
        let read = self.close_side(
            Channel::Mic,
            opened.or_else(|| reader.vad_error().cloned()),
            reader.summary(),
        );
        self.account(&mut report, read);
        self.core.emit(MeetingEvent::Transcribed(report));
        Ok((mic, report))
    }

    /// The mic cancelled along `path`: regions where the VAD hears speech in AEC3's full output,
    /// the linear output transcribed inside them. Returns the VAD's verdicts too, and the ERLE.
    /// [`Stop::Echo`] when cancellation fails (the caller falls back to [`mic_pass`]).
    ///
    /// [`mic_pass`]: Self::mic_pass
    fn mic_pass_cancelled(
        &self,
        audio: &ChunkStore,
        path: ink_echo::Alignment,
        ctx: &Pass<'_>,
    ) -> Result<
        (
            Vec<Segment>,
            events::ChannelPass,
            echo::EchoEvidence,
            echo::ErleMeter,
        ),
        Stop,
    > {
        let core = &self.core;
        let (vad, opened) = core.vad.open();
        let pass = SpeechPass::paired(vad, core.settings.vad, core.settings.regions)?;
        let mut reader = match EchoReader::open(audio, core.t0_ns, path, pass) {
            Ok(reader) => reader,
            Err(Stop::Chunks(e)) => {
                log::warn!("meeting final pass: the mic side's chunks cannot be listed: {e}");
                core.warn(MeetingWarning::AudioUnlisted {
                    channel: Channel::Mic,
                    reason: e.to_string(),
                });
                return Err(Stop::Chunks(e));
            }
            Err(stop) => return Err(stop),
        };
        let mut report = offline::report(Channel::Mic);
        let mut mic = Vec::new();
        while let Some(region) = reader.next_region()? {
            mic.extend(offline::transcribe(
                ctx,
                Channel::Mic,
                &region.audio,
                region.start,
                &mut report,
            )?);
        }
        let read = self.close_side(
            Channel::Mic,
            opened.or_else(|| reader.vad_error().cloned()),
            reader.summary(),
        );
        self.account(&mut report, read);
        core.emit(MeetingEvent::Transcribed(report));
        let (heard, erle) = reader.into_evidence();
        Ok((mic, report, heard, erle))
    }

    /// Tells the shell when text the library deleted or replaced could not yet be cleared from its
    /// files, and when it has been: each change once ([`Store::scrub_change`]).
    fn report_scrub(&self) {
        match self.core.services.store.scrub_change() {
            Some(true) => {
                log::warn!(
                    "meeting final pass: deleted text is not yet cleared from the library's files"
                );
                self.core.warn(MeetingWarning::DeletedTextNotScrubbed);
            }
            Some(false) => self.core.warn(MeetingWarning::DeletedTextScrubbed),
            None => {}
        }
    }

    /// What a side captured, into its report: the pump's count, what is on disk, and a warning
    /// when there is nothing at all.
    fn account(&self, report: &mut events::ChannelPass, read: SideRead) {
        let channel = report.channel;
        report.chunks_written =
            self.core.written[usize::from(channel == Channel::Far)].map(|w| w.chunks);
        report.chunks = read.chunks;
        report.captured_ms = read.captured_ms;
        report.backlogged_finals = self.core.backlogged[usize::from(channel == Channel::Far)];
        if read.only_zeros {
            log::warn!("meeting final pass: every sample of the {channel:?} side is zero");
            // A Bluetooth headset mic gates to zeros while its user is silent, so a meeting of its
            // zeros may be a listener who never spoke: the softer warning, not "a denied capture".
            // Not suppressed: it is also what a headset mic that never worked looks like, and a
            // whole meeting of it is worth a word (the live watchdog, judging 10 s at a time,
            // rightly says nothing).
            if channel == Channel::Mic && self.core.mic_bluetooth {
                self.core.warn(MeetingWarning::BluetoothMicOnlyZeros);
            } else {
                self.core
                    .warn(MeetingWarning::CapturedOnlyZeros { channel });
            }
        }
        report.audible_ms = read.audible_ms;
        report.speech_ms = read.speech_ms;
        if little_speech_heard(read.audible_ms, read.speech_ms) {
            log::warn!(
                "meeting final pass: the {channel:?} side was audible for {} ms and the VAD found {} ms of speech",
                read.audible_ms,
                read.speech_ms
            );
            self.core.warn(MeetingWarning::LittleSpeechHeard {
                channel,
                audible_ms: read.audible_ms,
                speech_ms: read.speech_ms,
            });
        }
        if read.chunks == 0 {
            log::warn!("meeting final pass: the {channel:?} side has no recorded audio at all");
            self.core.warn(MeetingWarning::NothingCaptured { channel });
        }
    }

    /// A reader over one side's chunks, with a fresh VAD. Returns, too, the error the VAD factory
    /// failed with, if it did (the pass then levels with the fallback), for
    /// [`close_side`](Self::close_side) to report.
    fn open_reader<'a>(
        &self,
        audio: &'a ChunkStore,
        channel: Channel,
    ) -> Result<(SideReader<'a>, Option<EngineError>), FinalizeError> {
        let core = &self.core;
        let (vad, error) = core.vad.open();
        let pass = SpeechPass::new(vad, core.settings.vad, core.settings.regions)
            .map_err(FinalizeError::Regions)?;
        let reader = match SideReader::open(audio, channel, core.t0_ns, pass) {
            Ok(reader) => reader,
            Err(Stop::Chunks(e)) => {
                log::warn!(
                    "meeting final pass: the {channel:?} side's chunks cannot be listed: {e}"
                );
                core.warn(MeetingWarning::AudioUnlisted {
                    channel,
                    reason: e.to_string(),
                });
                return Err(FinalizeError::Chunks(e));
            }
            Err(stop) => return Err(stop.into()),
        };
        Ok((reader, error))
    }

    /// Reports a finished side's VAD health and unreadable audio, once per side, and returns
    /// what it read. `vad_error` is the VAD factory's error, or a pass's over the side.
    fn close_side(
        &self,
        channel: Channel,
        vad_error: Option<EngineError>,
        read: SideRead,
    ) -> SideRead {
        let core = &self.core;
        if let Some(error) = vad_error {
            log::warn!("meeting final pass: the {channel:?} VAD failed; the fallback finished it");
            core.warn(MeetingWarning::VadFailed {
                channel,
                phase: Phase::Final,
                error,
            });
        }
        if read.skipped > 0 {
            log::warn!(
                "meeting final pass: {} {channel:?} chunk files could not be read",
                read.skipped
            );
            core.warn(MeetingWarning::AudioUnreadable {
                channel,
                chunks: read.skipped,
            });
        }
        read
    }

    /// Replaces the live transcript with `new`, unless a region failed or the guard refuses.
    /// Returns the new revision when it did. `previous` is the live transcript, when it could be
    /// read: the guard is checked against it first, so a refusal is an outcome of the pass; without
    /// it, the store's own check (inside its transaction) decides. `with`: the live finals the pass
    /// judged to be echo, which the guard does not count, and its removed lines, saved in the same
    /// transaction.
    fn supersede(
        &self,
        new: &[Segment],
        failed_regions: usize,
        previous: Option<&[Segment]>,
        with: SupersedeWith<'_>,
    ) -> Option<u32> {
        let core = &self.core;
        if failed_regions > 0 {
            log::warn!(
                "meeting final pass: {failed_regions} regions failed; the live transcript stands"
            );
            core.emit(MeetingEvent::KeptLive(KeptLive::Incomplete {
                failed_regions,
            }));
            return None;
        }
        let checked = previous.map_or(Ok(()), |previous| {
            check_supersede_explained(previous, new, with.explained)
        });
        let saved =
            checked.and_then(|()| core.services.store.supersede_with(&core.record, new, with));
        let refused = match saved {
            Ok(revision) => {
                core.emit(MeetingEvent::Superseded { revision });
                return Some(revision);
            }
            Err(error) => error,
        };
        log::warn!("meeting final pass not saved over the live transcript: {refused}");
        core.emit(MeetingEvent::KeptLive(KeptLive::Refused(refused)));
        None
    }

    /// The summary, then commitments, on the current transcript. It runs after the supersede, so
    /// it never fails the pass: each failure (a cancellation included) is a warning, and what
    /// depends on it is skipped. `record` is the record as read before the supersede; when that
    /// read failed, the title is left alone, since whether it had one is unknown. Skipped, with no
    /// call, when no line has the words a citation needs (`has_citable_line`).
    fn wrap_up(
        &self,
        segments: &[Segment],
        record: Option<&Record>,
        names: &[(ink_core::SpeakerId, String)],
        cancel: &CancelToken,
    ) {
        let core = &self.core;
        if !has_citable_line(segments) {
            // Nothing said, or only a stray word or two on each line (a noise heard as "Oh."): no
            // decision or action could cite a line, and a model asked anyway invents a headline.
            // So no call is made for the summary or for commitments (a transcript this thin holds
            // no promise worth a call), and the record keeps no title and no summary. A floor, not
            // a proof: a noise heard as three words still reaches the model.
            return;
        }
        let Some(llm) = &core.services.llm else {
            core.warn(MeetingWarning::SummaryUnavailable);
            return;
        };
        let store = &core.services.store;
        // Consent, read now (not at the start: the user may have changed it since): the
        // transcript goes only where the user agreed, checked on the model each call reaches.
        // Without a consent (or one that cannot be read) every model is refused.
        let consent = stored(store.as_ref(), Feature::Meetings);
        let consented = Consented {
            inner: llm.as_ref(),
            consent: consent.as_ref(),
        };
        let llm = &consented;
        let ctx = RecordContext {
            title: record.and_then(|r| r.title.as_deref()),
            time: RecordTime {
                started_at_unix_ms: core.started_unix_ms,
                utc_offset_minutes: core.settings.utc_offset_minutes,
            },
            speaker_names: names,
        };

        let mut filed: Vec<NewCommitment> = Vec::new();
        let now = core.services.clock.unix_ms();
        match summarize(segments, &ctx, &core.settings.summary, now, llm, cancel) {
            Ok(outcome) => {
                let untitled = record.is_some_and(|r| r.title.is_none());
                let saved = store
                    .save_summary(&core.record, &outcome.summary)
                    .and_then(|()| match untitled {
                        true => store.set_title(&core.record, &outcome.draft.headline),
                        false => Ok(()),
                    });
                match saved {
                    Ok(()) => core.emit(MeetingEvent::Summarized {
                        unverified: outcome.unverified,
                    }),
                    Err(error) => core.warn(MeetingWarning::StoreFailed(error)),
                }
                filed = outcome.actions;
            }
            Err(LlmError::NotAllowed { refused }) => {
                // Nothing was sent. Commitments would send the same transcript to the same
                // model, so they are skipped too.
                log::warn!(
                    "meeting: the model is not where the user agreed to send the transcript; no summary"
                );
                core.warn(MeetingWarning::SummaryNotAllowed(LlmConsent::for_model(
                    &refused,
                )));
                return;
            }
            Err(error) => {
                log::warn!("meeting summary failed: {error}");
                let cancelled = error == LlmError::Cancelled;
                core.warn(MeetingWarning::SummaryFailed(error));
                if cancelled {
                    return;
                }
            }
        }

        // Filing is once per record. A pass that runs again (a second crash during recovery,
        // after the first pass filed) hears the same audio, and its promises are already in Owed,
        // where the user may have settled some: those rows stay as they are, and nothing is
        // harvested, suggested or filed again. Chosen over replacing the rows because a rerun's
        // model may word a promise differently, so matching old rows to new ones (to carry done
        // and not-yet over) would guess; and the first filing is whole, rows and merges, since
        // `add_commitments_merged` is one transaction. The summary above is replaced, which is
        // idempotent.
        match store.commitments(&core.record) {
            Ok(existing) if !existing.is_empty() => {
                log::info!(
                    "meeting: an earlier pass filed {} commitments; kept, none filed again",
                    existing.len()
                );
                core.emit(MeetingEvent::Commitments {
                    filed: 0,
                    merged: 0,
                });
                return;
            }
            Ok(_) => {}
            Err(error) => {
                // Filed anyway, as with a failed dedup: a promise listed twice beats one lost.
                log::warn!("meeting: the record's commitments could not be read: {error}");
                core.warn(MeetingWarning::StoreFailed(error));
            }
        }

        match harvest(segments, &ctx, llm, cancel) {
            Ok(h) => {
                filed.extend(h.commitments);
                self.suggest_done(&h.already_done);
            }
            Err(error) => {
                log::warn!("meeting commitments failed: {error}");
                let cancelled = error == LlmError::Cancelled;
                core.warn(MeetingWarning::CommitmentsFailed(error));
                if cancelled {
                    return;
                }
            }
        }
        if filed.is_empty() {
            core.emit(MeetingEvent::Commitments {
                filed: 0,
                merged: 0,
            });
            return;
        }
        let merges = match dedup(&filed, llm, cancel) {
            Ok(d) => d.merges,
            Err(error) => {
                // Filed apart: a promise listed twice beats one lost. A cancellation files them
                // apart too: they were found, and the pass is past the point of undoing.
                log::warn!("meeting commitment dedup failed: {error}");
                core.warn(MeetingWarning::CommitmentsFailed(error));
                Vec::new()
            }
        };
        // The rows and their merges in one transaction: a crash between them would leave a pair
        // filed apart for good (the once-per-record gate above never files again).
        let pairs: Vec<(usize, usize)> = merges.iter().map(|m| (m.from, m.into)).collect();
        match store.add_commitments_merged(&core.record, &filed, &pairs) {
            Ok(_) => core.emit(MeetingEvent::Commitments {
                filed: filed.len(),
                merged: merges.len(),
            }),
            Err(error) => core.warn(MeetingWarning::StoreFailed(error)),
        }
    }
}

impl EndedMeeting {
    /// "Looks done": marks the open commitments of other meetings that what the user said here
    /// suggests are finished ([`looks_done`]). Before this meeting's own are filed, which are
    /// skipped anyway. A store failure is a warning; the pass goes on.
    fn suggest_done(&self, already_done: &[ink_llm::tasks::commitments::Candidate]) {
        if already_done.is_empty() {
            return;
        }
        let core = &self.core;
        let store = &core.services.store;
        let open = match store.open_commitments(LOOKS_DONE_OPEN_LIMIT) {
            Ok(open) => open,
            Err(error) => {
                log::warn!(
                    "meeting: the open commitments could not be read for looks-done: {error}"
                );
                core.warn(MeetingWarning::StoreFailed(error));
                return;
            }
        };
        let mut suggested = 0;
        for (id, span) in looks_done(already_done, &open, Some(&core.record)) {
            let evidence = DoneEvidence {
                record: core.record.clone(),
                span,
            };
            match store.set_done_evidence(&id, Some(&evidence)) {
                Ok(()) => suggested += 1,
                Err(error) => {
                    log::warn!("meeting: a looks-done suggestion could not be saved: {error}");
                    core.warn(MeetingWarning::StoreFailed(error));
                }
            }
        }
        if suggested > 0 {
            log::info!("meeting: {suggested} earlier commitments look done");
            core.emit(MeetingEvent::LooksDone { suggested });
        }
    }
}

/// How many open commitments "looks done" reads to match against: Owed's own page size.
pub const LOOKS_DONE_OPEN_LIMIT: usize = 1_000;

/// The report of a path search, before the mic is cancelled.
fn echo_pass(fit: Option<&PathReport>) -> EchoPass {
    let Some(fit) = fit else {
        return EchoPass::default();
    };
    EchoPass {
        path: fit.path.map(|p| EchoPath {
            delay_ms: p.delay_ms(),
            drift_ppm: p.drift_ppm(),
            inliers: fit.inliers,
            stable_from_ms: fit.stable_from_s.map(|s| (s * 1000.0).round() as u64),
        }),
        windows: fit.windows,
        candidates: fit.candidates,
        ..EchoPass::default()
    }
}

/// The live "you" finals in `previous` this pass judges to be echo: the echo gate's verdict on
/// each over the pass's evidence, or a line that repeats one of the pass's far-end lines where
/// nobody on the near end spoke (dedup's rule, as the pass's own lines are judged).
///
/// Both of those read the same evidence as the pass's own removals, so a fault in it would both
/// remove the user's words and waive the guard that should notice. So a final also needs
/// evidence that does not come from the mic's side at all: speech in the far end's own regions
/// (its VAD, on its own audio) overlapping it, or ending within the room's tail (the echo gate's
/// 300 ms) before it began. Echo is the far end's speech; with none, there was none to hear.
fn live_echo(
    previous: &[Segment],
    far: &[Segment],
    far_speech: &[(u64, u64)],
    evidence: &EchoEvidence,
) -> Vec<Explained> {
    let tail = ink_echo::GateConfig::default().tail_ms;
    let far_spoke = |s: &Segment| {
        far_speech
            .iter()
            .any(|&(a, b)| a < s.end_ms.max(s.start_ms + 1) && b + tail >= s.start_ms)
    };
    let mic: Vec<&Segment> = previous
        .iter()
        .filter(|s| s.channel == Channel::Mic)
        .collect();
    let lines = |segments: &[&Segment]| -> Vec<ink_core::TimedText> {
        segments
            .iter()
            .map(|s| ink_core::TimedText {
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                text: s.text.clone(),
            })
            .collect()
    };
    let far: Vec<&Segment> = far.iter().collect();
    let repeated = echo_duplicates(
        &lines(&mic),
        &lines(&far),
        evidence,
        &DedupConfig::default(),
    )
    .removed;
    mic.iter()
        .enumerate()
        .filter(|(k, s)| {
            (repeated.iter().any(|d| d.you == *k) || evidence.echo_only(s.start_ms, s.end_ms))
                && far_spoke(s)
        })
        .map(|(_, s)| Explained {
            channel: Channel::Mic,
            start_ms: s.start_ms,
            end_ms: s.end_ms,
        })
        .collect()
}

/// Removes from `mic` the lines that repeat `far` where `heard` says nobody on the near end spoke
/// (`ink_echo::dedup`'s rule), counting into `echo`, and returns them whole.
fn remove_echo(
    mic: &mut Vec<Segment>,
    far: &[Segment],
    heard: &EchoEvidence,
    echo: &mut EchoPass,
) -> Vec<RemovedEcho> {
    let lines = |segments: &[Segment]| -> Vec<ink_core::TimedText> {
        segments
            .iter()
            .map(|s| ink_core::TimedText {
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                text: s.text.clone(),
            })
            .collect()
    };
    let report = echo_duplicates(&lines(mic), &lines(far), heard, &DedupConfig::default());
    echo.removed = report.removed.len();
    echo.kept_near_speech = report.kept_near_speech;
    echo.kept_no_evidence = report.kept_no_evidence;
    let mut drop = vec![false; mic.len()];
    let removed = report
        .removed
        .iter()
        .map(|d| {
            drop[d.you] = true;
            RemovedEcho {
                start_ms: d.start_ms,
                end_ms: d.end_ms,
                text: Spoken::new(mic[d.you].text.clone()),
                far: d
                    .far
                    .iter()
                    .map(|&f| FarLine {
                        start_ms: far[f].start_ms,
                        end_ms: far[f].end_ms,
                    })
                    .collect(),
                words: d.words,
                matched: d.matched,
            }
        })
        .collect();
    let mut k = 0;
    mic.retain(|_| {
        k += 1;
        !drop[k - 1]
    });
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech::HeardSpeech;

    fn mic(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment {
            channel: Channel::Mic,
            start_ms,
            end_ms,
            text: text.into(),
            speaker: None,
        }
    }

    /// Evidence for 20 s: the far end's reference playing throughout, and the full output's VAD
    /// hearing nobody on the near end anywhere. The gate calls every line in it echo.
    fn all_echo() -> EchoEvidence {
        let mut heard = HeardSpeech::default();
        heard.add(
            0,
            20 * 16_000,
            Some(vec![0.0; (20 * 16_000usize).div_ceil(512)]),
        );
        EchoEvidence::new(heard, vec![true; 2_000])
    }

    #[test]
    fn a_live_final_is_explained_only_where_the_far_end_s_own_vad_heard_speech() {
        let evidence = all_echo();
        let previous = [
            mic(5_000, 7_000, "echo words from the far end"),
            mic(12_000, 14_000, "yes that works for me"),
            Segment {
                channel: Channel::Far,
                start_ms: 4_800,
                end_ms: 7_400,
                text: "echo words from the far end".into(),
                speaker: None,
            },
        ];
        assert!(evidence.echo_only(5_000, 7_000) && evidence.echo_only(12_000, 14_000));
        // The far end's regions: speech 4.8–7.4 s only. The first final is explained; the second
        // has no far-end speech under it, so the gate's verdict alone does not waive the guard.
        let far_speech = [(4_800, 7_400)];
        let explained = live_echo(&previous, &[], &far_speech, &evidence);
        assert_eq!(
            explained,
            vec![Explained {
                channel: Channel::Mic,
                start_ms: 5_000,
                end_ms: 7_000
            }]
        );
        // With no far-end speech at all, nothing is explained, and a pass that drops the "you"
        // finals is refused.
        let explained = live_echo(&previous, &[], &[], &evidence);
        assert!(explained.is_empty());
        let new = [previous[2].clone()];
        assert!(matches!(
            check_supersede_explained(&previous, &new, &explained),
            Err(StoreError::SuspiciousSupersede {
                channel: Channel::Mic,
                ..
            })
        ));
        // Echo arrives after the far end said it: speech that ended within the room's tail
        // before the final began still counts.
        let explained = live_echo(&previous, &[], &[(3_000, 4_800)], &evidence);
        assert_eq!(explained.len(), 1);
        let explained = live_echo(&previous, &[], &[(3_000, 4_600)], &evidence);
        assert!(explained.is_empty());
    }
}
