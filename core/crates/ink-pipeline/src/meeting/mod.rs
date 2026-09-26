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
//! | Diarization | [`diarize`]: the far end only; labels kept with at least two substantial clusters |
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

use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use ink_audio::{ChunkError, ChunkStore, VadConfig, WindowError};
use ink_core::store::check_supersede;
use ink_core::{
    AsrEvent, CancelToken, Channel, Clock, Diarizer, EventSink, Llm, LlmError, NewCommitment,
    NewRecord, OfflineEngine, RecordId, RecordKind, Segment, Store, StoreError, StreamingEngine,
};
use ink_llm::tasks::commitments::{RecordContext, harvest};
use ink_llm::tasks::dedup::{apply_merges, dedup};
use ink_llm::tasks::due::RecordTime;
use ink_llm::tasks::summary::{SummaryOptions, summarize};

use self::events::{KeptLive, MeetingEvent, MeetingWarning, Phase};
use self::live::{LiveChannel, Settled};
use self::offline::{Pass, Stop};
use crate::redact::Spoken;
use crate::speech::{Region, RegionConfig, SpeechPass, VadSource};

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
}

/// A live meeting. See the module docs.
pub struct MeetingChain {
    core: Core,
    mic: LiveChannel,
    far: LiveChannel,
    asr: Receiver<(Channel, AsrEvent)>,
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
}

impl Core {
    fn emit(&self, event: MeetingEvent) {
        (self.events)(event);
    }

    fn warn(&self, warning: MeetingWarning) {
        self.emit(MeetingEvent::Warning(warning));
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
                // Callback thread: it only enqueues. The queue is read until the chain stops, and the
                // stream is finished by then, with every event delivered (its contract).
                let sink: EventSink<AsrEvent> = Arc::new(move |event| {
                    let _ = tx.send((channel, event));
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
    }

    /// Something the pump could not do for one side ([`SideCapture`](crate::capture::SideCapture)).
    pub fn capture_issue(&mut self, channel: Channel, issue: crate::capture::CaptureIssue) {
        log::warn!("meeting: {issue}");
        self.core.warn(MeetingWarning::Capture { channel, issue });
    }

    /// Takes the live engine's queued events, and saves the finals the VAD has judged.
    fn collect(&mut self, finishing: bool) {
        while let Ok((channel, event)) = self.asr.try_recv() {
            match event {
                AsrEvent::Partial { text } => self.core.emit(MeetingEvent::Partial {
                    channel,
                    text: Spoken::new(text),
                }),
                AsrEvent::Final(text) => self.side(channel).final_heard(text),
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
                    self.core.warn(MeetingWarning::SaveFailed(error));
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
            core.warn(MeetingWarning::SaveFailed(error));
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
    pub revision: u32,
    /// Whether the final pass replaced the live transcript.
    pub superseded: bool,
    /// The mic side's pass.
    pub mic: events::ChannelPass,
    /// The far side's pass.
    pub far: events::ChannelPass,
    /// What diarization did, when a diarizer is installed.
    pub diarization: Option<events::Diarization>,
}

/// Why a final pass stopped. Nothing it had not finished was saved, and the live transcript
/// stands.
#[derive(Debug)]
#[non_exhaustive]
pub enum FinalizeError {
    /// The cancel token was set.
    Cancelled,
    /// The recorded audio could not be listed.
    Chunks(ChunkError),
    /// The region sizes in the settings cannot work.
    Regions(WindowError),
    /// The record could not be read.
    Store(StoreError),
}

impl fmt::Display for FinalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("meeting final pass cancelled"),
            Self::Chunks(e) => write!(f, "meeting final pass: {e}"),
            Self::Regions(e) => write!(f, "meeting final pass: {e}"),
            Self::Store(e) => write!(f, "meeting final pass: {e}"),
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
    /// After an error (a cancellation included) nothing the pass had not finished is saved, and it
    /// can be run again.
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

        // The mic: each region transcribed as it closes.
        let mut mic_report = offline::report(Channel::Mic);
        let mut mic = Vec::new();
        self.side_pass(audio, Channel::Mic, &mut |region| {
            mic.extend(offline::transcribe(
                &ctx,
                Channel::Mic,
                &region.audio,
                region.start,
                &mut mic_report,
            )?);
            Ok(())
        })?;
        core.emit(MeetingEvent::Transcribed(mic_report));

        // The far end: diarized first when a diarizer is installed, so each call has one speaker.
        let mut far_report = offline::report(Channel::Far);
        let (far, diarization) = match &core.services.diarizer {
            Some(diarizer) => {
                let mut regions: Vec<Region> = Vec::new();
                self.side_pass(audio, Channel::Far, &mut |region| {
                    regions.push(region);
                    Ok(())
                })?;
                offline::far_with_speakers(&ctx, diarizer.as_ref(), regions, &mut far_report)?
            }
            None => {
                let mut far = Vec::new();
                self.side_pass(audio, Channel::Far, &mut |region| {
                    far.extend(offline::transcribe(
                        &ctx,
                        Channel::Far,
                        &region.audio,
                        region.start,
                        &mut far_report,
                    )?);
                    Ok(())
                })?;
                (far, None)
            }
        };
        core.emit(MeetingEvent::Transcribed(far_report));
        if let Some(d) = diarization {
            core.emit(MeetingEvent::Diarized(d));
        }

        let mut new: Vec<Segment> = mic.into_iter().chain(far).collect();
        new.sort_by_key(|s| (s.start_ms, s.channel));
        let superseded =
            self.supersede(&new, mic_report.failed_regions + far_report.failed_regions)?;
        let store = &core.services.store;
        let current = store.segments(&core.record).map_err(FinalizeError::Store)?;
        self.wrap_up(&current, cancel)?;
        let revision = store
            .record(&core.record)
            .map_err(FinalizeError::Store)?
            .map_or(1, |r| r.revision);
        core.emit(MeetingEvent::Finished { revision });
        Ok(MeetingOutcome {
            revision,
            superseded,
            mic: mic_report,
            far: far_report,
            diarization,
        })
    }

    /// Runs one side's chunks through a speech pass, and reports its VAD's health.
    fn side_pass(
        &self,
        audio: &ChunkStore,
        channel: Channel,
        on_region: &mut dyn FnMut(Region) -> Result<(), Stop>,
    ) -> Result<(), FinalizeError> {
        let core = &self.core;
        let (vad, error) = core.vad.open();
        if let Some(error) = error {
            core.warn(MeetingWarning::VadFailed {
                channel,
                phase: Phase::Final,
                error,
            });
        }
        let mut pass = SpeechPass::new(vad, core.settings.vad, core.settings.regions)
            .map_err(FinalizeError::Regions)?;
        let skipped = offline::read_side(audio, channel, core.t0_ns, &mut pass, on_region)?;
        if let Some(error) = pass.vad_error().cloned() {
            log::warn!("meeting final pass: the {channel:?} VAD failed; the fallback finished it");
            core.warn(MeetingWarning::VadFailed {
                channel,
                phase: Phase::Final,
                error,
            });
        }
        if skipped > 0 {
            log::warn!("meeting final pass: {skipped} {channel:?} chunk files could not be read");
            core.warn(MeetingWarning::AudioUnreadable {
                channel,
                chunks: skipped,
            });
        }
        Ok(())
    }

    /// Replaces the live transcript with `new`, unless a region failed or the guard refuses.
    fn supersede(&self, new: &[Segment], failed_regions: usize) -> Result<bool, FinalizeError> {
        let core = &self.core;
        let store = &core.services.store;
        if failed_regions > 0 {
            log::warn!(
                "meeting final pass: {failed_regions} regions failed; the live transcript stands"
            );
            core.emit(MeetingEvent::KeptLive(KeptLive::Incomplete {
                failed_regions,
            }));
            return Ok(false);
        }
        let previous = store.segments(&core.record).map_err(FinalizeError::Store)?;
        // The store checks again inside its transaction; checking here first keeps a refusal an
        // outcome of the pass rather than a store failure.
        let refused = match check_supersede(&previous, new) {
            Ok(()) => match store.supersede(&core.record, new) {
                Ok(revision) => {
                    core.emit(MeetingEvent::Superseded { revision });
                    return Ok(true);
                }
                Err(error) => error,
            },
            Err(error) => error,
        };
        log::warn!("meeting final pass not saved over the live transcript: {refused}");
        core.emit(MeetingEvent::KeptLive(KeptLive::Refused(refused)));
        Ok(false)
    }

    /// The summary, then commitments, on the current transcript.
    fn wrap_up(&self, segments: &[Segment], cancel: &CancelToken) -> Result<(), FinalizeError> {
        let core = &self.core;
        if ink_core::store::word_count(segments) == 0 {
            // Nothing was said: there is nothing to summarise, and no call is made.
            return Ok(());
        }
        let Some(llm) = &core.services.llm else {
            core.warn(MeetingWarning::SummaryUnavailable);
            return Ok(());
        };
        let store = &core.services.store;
        let record = store
            .record(&core.record)
            .map_err(FinalizeError::Store)?
            .ok_or(FinalizeError::Store(StoreError::NotFound))?;
        let names = store
            .speaker_names(&core.record)
            .map_err(FinalizeError::Store)?;
        let ctx = RecordContext {
            title: record.title.as_deref(),
            time: RecordTime {
                started_at_unix_ms: core.started_unix_ms,
                utc_offset_minutes: core.settings.utc_offset_minutes,
            },
            speaker_names: &names,
        };
        let cancelled = |e: &LlmError| matches!(e, LlmError::Cancelled);

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
                let saved = store
                    .save_summary(&core.record, &outcome.summary)
                    .and_then(|()| match record.title {
                        Some(_) => Ok(()),
                        None => store.set_title(&core.record, &outcome.draft.headline),
                    });
                match saved {
                    Ok(()) => core.emit(MeetingEvent::Summarized {
                        unverified: outcome.unverified,
                    }),
                    Err(error) => core.warn(MeetingWarning::SaveFailed(error)),
                }
                filed = outcome.actions;
            }
            Err(e) if cancelled(&e) => return Err(FinalizeError::Cancelled),
            Err(error) => {
                log::warn!("meeting summary failed: {error}");
                core.warn(MeetingWarning::SummaryFailed(error));
            }
        }

        match harvest(segments, &ctx, llm.as_ref(), cancel) {
            Ok(h) => filed.extend(h.commitments),
            Err(e) if cancelled(&e) => return Err(FinalizeError::Cancelled),
            Err(error) => {
                log::warn!("meeting commitments failed: {error}");
                core.warn(MeetingWarning::CommitmentsFailed(error));
            }
        }
        if filed.is_empty() {
            core.emit(MeetingEvent::Commitments {
                filed: 0,
                merged: 0,
            });
            return Ok(());
        }
        let merges = match dedup(&filed, llm.as_ref(), cancel) {
            Ok(d) => d.merges,
            Err(e) if cancelled(&e) => return Err(FinalizeError::Cancelled),
            Err(error) => {
                // Filed apart: a promise listed twice beats one lost.
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
            Err(error) => core.warn(MeetingWarning::SaveFailed(error)),
        }
        Ok(())
    }
}
