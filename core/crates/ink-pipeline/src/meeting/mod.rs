//! The meeting chain: two sides live, then the final pass ("blotting").
//!
//! ```text
//! live:  mic ─┐                                      partials ─► events only
//!             ├─► canonical ─► Agc + VAD ─► live engine ─► finals ─► checked ─► revision 1
//!        far ─┘   (chunks on disk: the pump)
//!
//! end:   chunks ─► speech regions ─► final engine ─► mic segments ────────────────┐
//!        chunks ─► speech regions ─► diarizer (far only, rule 5) ─► final engine ─┴─► supersede ─► summary ─► commitments
//! ```
//!
//! | Stage | Here |
//! |---|---|
//! | Capture and chunks | [`capture`](crate::capture): the pump writes each side's chunks and hands canonical audio on |
//! | Live, per side | `live`: the AGC with a VAD (the fallback when it fails), the live engine, finals placed in the meeting and saved only over VAD speech |
//! | Final pass, per side | `offline`: the chunks read back, VAD-gated gain, one engine call per speech region, empty regions reported |
//! | Diarization | [`diarize`]: the far end only, its speech streamed from disk into the diarizer a window at a time; labels kept with at least two substantial clusters |
//! | Supersede | the final pass replaces the live transcript in one transaction, as revision 2, unless the guard ([`check_supersede`]) refuses it or a region failed |
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
use ink_core::store::check_supersede;
use ink_core::{
    AsrEvent, CancelToken, Channel, Clock, Diarizer, EngineError, EventSink, Llm, LlmError,
    NewCommitment, NewRecord, OfflineEngine, Record, RecordId, RecordKind, Segment, Store,
    StoreError, StreamingEngine,
};
use ink_llm::tasks::commitments::{RecordContext, harvest};
use ink_llm::tasks::dedup::{apply_merges, dedup};
use ink_llm::tasks::due::RecordTime;
use ink_llm::tasks::summary::{SummaryOptions, summarize};

use self::diarize::{rule5, to_meeting};
use self::events::{KeptLive, MeetingEvent, MeetingWarning, Phase};
use self::live::{LiveChannel, Settled};
use self::offline::{Pass, RegionWindows, SideRead, SideReader, Stop};
use self::watchdog::{Routing, SideState, Watch, Watchdog};
use crate::capture::SideSummary;
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
        Ok(Self {
            core,
            mic,
            far,
            asr,
            watchdog: Watchdog::new(start.routing, t0_ns),
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
        self.side(channel).push(samples, host_time_ns, &*events);
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

    /// The capture's routing changed (a device switched, the headset-mic setting).
    pub fn set_routing(&mut self, routing: Routing) {
        self.watchdog.set_routing(routing);
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
    }

    fn save(&self, settled: Settled) {
        match settled {
            Settled::Keep(segment) => {
                let store = &self.core.services.store;
                if let Err(error) =
                    store.append_segments(&self.core.record, std::slice::from_ref(&segment))
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

    /// **Worker.** Ends the live phase: flushes each side into its live engine, saves the trailing
    /// finals, and marks the record ended. Call it once capture has stopped and the pump has handed
    /// on the last audio.
    ///
    /// The end is never recorded before the start: the wall clock can be set back during a
    /// meeting, and then the start is used ([`MeetingWarning::ClockWentBack`]).
    pub fn stop(mut self) -> EndedMeeting {
        let events = self.core.events.clone();
        self.mic.finish(&*events);
        self.far.finish(&*events);
        self.collect(true);
        // From here, a live event is late: every stream has finished. What raced in before the
        // close is counted too.
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
        if let Err(error) = core
            .services
            .store
            .finish_record(&core.record, now.max(core.started_unix_ms))
        {
            log::warn!("meeting: the record could not be marked ended: {error}");
            core.warn(MeetingWarning::StoreFailed(error));
        }
        core.emit(MeetingEvent::Stopped);
        EndedMeeting { core }
    }
}

/// A meeting whose live phase is over, waiting for its final pass.
pub struct EndedMeeting {
    core: Core,
}

/// What a meeting's final pass came to.
#[derive(Clone, Debug, PartialEq, Eq)]
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
}

impl fmt::Display for FinalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("meeting final pass cancelled"),
            Self::Chunks(e) => write!(f, "meeting final pass: {e}"),
            Self::Regions(e) => write!(f, "meeting final pass: {e}"),
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
        }
    }
}

impl EndedMeeting {
    /// The meeting's record.
    pub fn record(&self) -> &RecordId {
        &self.core.record
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

        // The mic: each region transcribed as it is read.
        let mut mic_report = offline::report(Channel::Mic);
        let mut mic = Vec::new();
        let (mut reader, opened) = self.open_reader(audio, Channel::Mic)?;
        while let Some(region) = reader.next_region()? {
            mic.extend(offline::transcribe(
                &ctx,
                Channel::Mic,
                &region.audio,
                region.start,
                &mut mic_report,
            )?);
        }
        let read = self.close_reader(&reader, Channel::Mic, opened);
        self.account(&mut mic_report, read);
        core.emit(MeetingEvent::Transcribed(mic_report));

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
        let (mut reader, opened) = self.open_reader(audio, Channel::Far)?;
        while let Some(region) = reader.next_region()? {
            far.extend(offline::transcribe_region(
                &ctx,
                Channel::Far,
                &region,
                labels,
                &mut far_report,
            )?);
        }
        let read = self.close_reader(&reader, Channel::Far, opened.or(first_vad_error));
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

        let mut new: Vec<Segment> = mic.into_iter().chain(far).collect();
        new.sort_by_key(|s| (s.start_ms, s.channel));

        // Read before anything is written: a failure here is reported and the pass goes on
        // without what it could not read.
        let store = &core.services.store;
        let record = core.read(store.record(&core.record)).flatten();
        let names = core
            .read(store.speaker_names(&core.record))
            .unwrap_or_default();
        let previous = core.read(store.segments(&core.record));

        let failed = mic_report.failed_regions + far_report.failed_regions;
        let saved = self.supersede(&new, failed, previous.as_deref());
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
        core.emit(MeetingEvent::Finished { revision });
        Ok(MeetingOutcome {
            revision,
            superseded: saved.is_some(),
            mic: mic_report,
            far: far_report,
            diarization,
        })
    }

    /// What a side captured, into its report: the pump's count, what is on disk, and a warning
    /// when there is nothing at all.
    fn account(&self, report: &mut events::ChannelPass, read: SideRead) {
        let channel = report.channel;
        report.chunks_written =
            self.core.written[usize::from(channel == Channel::Far)].map(|w| w.chunks);
        report.chunks = read.chunks;
        report.captured_ms = read.captured_ms;
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
    /// [`close_reader`](Self::close_reader) to report.
    fn open_reader<'a>(
        &self,
        audio: &'a ChunkStore,
        channel: Channel,
    ) -> Result<(SideReader<'a>, Option<EngineError>), FinalizeError> {
        let core = &self.core;
        let (vad, error) = core.vad.open();
        let pass = SpeechPass::new(vad, core.settings.vad, core.settings.regions)
            .map_err(FinalizeError::Regions)?;
        let reader = SideReader::open(audio, channel, core.t0_ns, pass)?;
        Ok((reader, error))
    }

    /// Reports a finished side's VAD health and unreadable audio, once per side, and returns
    /// what it read. `vad_error` is the VAD factory's error, or another pass's over the same side.
    fn close_reader(
        &self,
        reader: &SideReader<'_>,
        channel: Channel,
        vad_error: Option<EngineError>,
    ) -> SideRead {
        let core = &self.core;
        if let Some(error) = vad_error.or_else(|| reader.vad_error().cloned()) {
            log::warn!("meeting final pass: the {channel:?} VAD failed; the fallback finished it");
            core.warn(MeetingWarning::VadFailed {
                channel,
                phase: Phase::Final,
                error,
            });
        }
        let read = reader.summary();
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
    /// it, the store's own check (inside its transaction) decides.
    fn supersede(
        &self,
        new: &[Segment],
        failed_regions: usize,
        previous: Option<&[Segment]>,
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
        let checked = previous.map_or(Ok(()), |previous| check_supersede(previous, new));
        let refused = match checked.and_then(|()| core.services.store.supersede(&core.record, new))
        {
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
    /// read failed, the title is left alone, since whether it had one is unknown.
    fn wrap_up(
        &self,
        segments: &[Segment],
        record: Option<&Record>,
        names: &[(ink_core::SpeakerId, String)],
        cancel: &CancelToken,
    ) {
        let core = &self.core;
        if ink_core::store::word_count(segments) == 0 {
            // Nothing was said: there is nothing to summarise, and no call is made.
            return;
        }
        let Some(llm) = &core.services.llm else {
            core.warn(MeetingWarning::SummaryUnavailable);
            return;
        };
        let store = &core.services.store;
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
        match summarize(
            segments,
            &ctx,
            &core.settings.summary,
            now,
            llm.as_ref(),
            cancel,
        ) {
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
            Err(error) => {
                log::warn!("meeting summary failed: {error}");
                let cancelled = error == LlmError::Cancelled;
                core.warn(MeetingWarning::SummaryFailed(error));
                if cancelled {
                    return;
                }
            }
        }

        match harvest(segments, &ctx, llm.as_ref(), cancel) {
            Ok(h) => filed.extend(h.commitments),
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
        let merges = match dedup(&filed, llm.as_ref(), cancel) {
            Ok(d) => d.merges,
            Err(error) => {
                // Filed apart: a promise listed twice beats one lost. A cancellation files them
                // apart too: they were found, and the pass is past the point of undoing.
                log::warn!("meeting commitment dedup failed: {error}");
                core.warn(MeetingWarning::CommitmentsFailed(error));
                Vec::new()
            }
        };
        let saved = store
            .add_commitments(&core.record, &filed)
            .and_then(|ids| apply_merges(store.as_ref(), &ids, &merges));
        match saved {
            Ok(()) => core.emit(MeetingEvent::Commitments {
                filed: filed.len(),
                merged: merges.len(),
            }),
            Err(error) => core.warn(MeetingWarning::StoreFailed(error)),
        }
    }
}
