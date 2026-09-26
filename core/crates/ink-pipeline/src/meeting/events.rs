//! What the meeting chain tells the shell.
//!
//! As with dictation, every outcome is an event and nothing that went wrong is only logged. The
//! variants with the meeting's words ([`MeetingEvent::Partial`], [`MeetingEvent::Final`]) hold them
//! as [`Spoken`], which prints as a length; every other variant carries no text, so logging an
//! event cannot leak a transcript (I5).

use ink_core::{Channel, EngineError, LlmError, RecordId, StoreError};

use crate::capture::CaptureIssue;
use crate::events::VoiceDetection;
use crate::redact::Spoken;

/// Which part of a meeting a problem came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// While it was live.
    Live,
    /// The offline pass at the end.
    Final,
}

/// Something went wrong, and the meeting went on without it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MeetingWarning {
    /// The VAD failed. The channel's live AGC (or its final pass) switched to the fallback, which
    /// levels without voice detection, and went on.
    VadFailed {
        /// Which side.
        channel: Channel,
        /// Live or final.
        phase: Phase,
        /// What failed.
        error: EngineError,
    },
    /// The pump could not write a chunk, or convert a block, or the device's rate is wrong.
    Capture {
        /// Which side.
        channel: Channel,
        /// What happened.
        issue: CaptureIssue,
    },
    /// The capture ring dropped audio: the transcript may miss words there.
    AudioLost {
        /// Which side.
        channel: Channel,
        /// Device frames lost.
        frames: u64,
    },
    /// The live engine failed on this channel; its live transcript stops here. The final pass
    /// still runs from the recorded audio.
    LiveEngineFailed {
        /// Which side.
        channel: Channel,
        /// What failed.
        error: EngineError,
    },
    /// The live engine reported times the VAD has not judged, [`MAX_PENDING_FINALS`] of them at
    /// once. The oldest are saved without the speech check from here on. Sent once per side.
    ///
    /// [`MAX_PENDING_FINALS`]: crate::meeting::MAX_PENDING_FINALS
    LiveFinalsBacklog {
        /// Which side.
        channel: Channel,
    },
    /// The live engine sent events after the meeting stopped, breaking its contract (every event
    /// before its stream's `finish` returns). They could not be used. The count is as of the end
    /// of the final pass.
    LiveEventsAfterStop {
        /// Events counted.
        count: u64,
    },
    /// The live engine has fallen behind real time.
    LiveEngineStalled {
        /// Which side.
        channel: Channel,
    },
    /// A live final lay where the VAD heard no speech, so it was not saved: the live engine wrote
    /// words for non-speech.
    FinalWithoutSpeech {
        /// Which side.
        channel: Channel,
        /// Its start, ms into the meeting.
        start_ms: u64,
        /// Its end.
        end_ms: u64,
    },
    /// The VAD heard speech here and the final-pass engine returned no words for it: speech the
    /// engine dropped (or the VAD's mistake). The region is left without text.
    EmptySpeechRegion {
        /// Which side.
        channel: Channel,
        /// Its start, ms into the meeting.
        start_ms: u64,
        /// Its end.
        end_ms: u64,
    },
    /// The final-pass engine failed on a region. Its words are missing from the final pass, so the
    /// final pass is not saved over the live transcript.
    FinalEngineFailed {
        /// Which side.
        channel: Channel,
        /// The region's start, ms into the meeting.
        start_ms: u64,
        /// Its end.
        end_ms: u64,
        /// What failed.
        error: EngineError,
    },
    /// Recorded audio could not be read back for the final pass (a torn chunk recovery could not
    /// repair, a read error, or audio the resampler refused). The final pass has a gap at each;
    /// what follows a gap stays where it was said.
    AudioUnreadable {
        /// Which side.
        channel: Channel,
        /// Chunk files skipped, or cut short.
        chunks: usize,
    },
    /// A side was clearly audible, and the VAD found under a tenth of it as speech
    /// ([`little_speech_heard`](crate::speech::little_speech_heard)): the VAD may be deaf (a wrong
    /// model, audio at the wrong rate). A diagnostic: nothing was sent or dropped because of it.
    LittleSpeechHeard {
        /// Which side.
        channel: Channel,
        /// Time above the audible floor, ms.
        audible_ms: u64,
        /// Time found as speech, ms.
        speech_ms: u64,
    },
    /// A side has no recorded audio at all: no chunk was ever written for it (a device that never
    /// delivered, a permission denied, a tap that never started). Silence would still have chunks.
    NothingCaptured {
        /// Which side.
        channel: Channel,
    },
    /// The diarizer failed: the far end keeps no speaker labels.
    DiarizationFailed(EngineError),
    /// The store failed: a live final, the final pass, the summary or commitments could not be
    /// saved, or the record could not be read back for the final pass. The meeting went on
    /// without it; the events around it say what was skipped.
    StoreFailed(StoreError),
    /// The wall clock reads earlier than the meeting's start (it was set back during the meeting),
    /// so the end was recorded as the start.
    ClockWentBack,
    /// No language model is set up: no summary and no commitments.
    SummaryUnavailable,
    /// The summary failed. Commitments still ran.
    SummaryFailed(LlmError),
    /// Harvesting or deduplicating commitments failed.
    CommitmentsFailed(LlmError),
}

/// What the final pass did on one side.
///
/// `chunks` and `captured_ms` tell a side that never captured anything (no chunks, nothing
/// captured) from one that was silent (chunks full of silence, no speech).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelPass {
    /// Which side.
    pub channel: Channel,
    /// Chunk files the pump wrote for this side, as its writers counted them
    /// ([`SideSummary`](crate::capture::SideSummary)). `None` when the pump did not report: a
    /// meeting finalized after a restart, or an import.
    pub chunks_written: Option<u64>,
    /// Chunk files the final pass found for this side, readable or not. An import has none.
    pub chunks: usize,
    /// Audio in the readable chunks (an import: in the file), ms. Zero: nothing was captured.
    pub captured_ms: u64,
    /// Of that, time above the audible floor
    /// ([`AUDIBLE_FLOOR_DBFS`](crate::speech::AUDIBLE_FLOOR_DBFS)), measured before any gain:
    /// what [`speech_ms`](Self::speech_ms) is checked against.
    pub audible_ms: u64,
    /// Speech regions the engine was given.
    pub regions: usize,
    /// Of those, how many came back without words.
    pub empty_regions: usize,
    /// Of those, how many failed.
    pub failed_regions: usize,
    /// Words in the result.
    pub word_count: usize,
    /// Time the VAD found as speech, ms: the regions, padding included (silence never reaches the
    /// engine).
    pub speech_ms: u64,
}

/// What diarization did with the far end (architecture rule 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Diarization {
    /// Clusters the diarizer returned.
    pub clusters: usize,
    /// Clusters holding at least 2 % of the far end's speech.
    pub substantial: usize,
    /// Whether the labels were kept: at least two substantial clusters.
    pub labelled: bool,
    /// Far-end segments given a speaker.
    pub attributed: usize,
}

/// Why the final pass did not replace the live transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeptLive {
    /// The supersede guard refused it (an empty result, or a side under half its live words): far
    /// more likely an engine failure than a correction.
    Refused(StoreError),
    /// Some regions failed ([`MeetingWarning::FinalEngineFailed`]): the result is incomplete.
    Incomplete {
        /// Regions that failed.
        failed_regions: usize,
    },
}

/// An event from a meeting, in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MeetingEvent {
    /// The record exists and capture is being transcribed live.
    Started {
        /// The meeting's record.
        record: RecordId,
    },
    /// Whether a side's audio is levelled with voice detection. Sent for each side at the start,
    /// and whenever it changes.
    VoiceDetection {
        /// Which side.
        channel: Channel,
        /// The state.
        state: VoiceDetection,
    },
    /// Provisional live text. Never saved (architecture rule 4); each replaces the last.
    Partial {
        /// Which side.
        channel: Channel,
        /// The hypothesis.
        text: Spoken,
    },
    /// Settled live text, saved as revision 1.
    Final {
        /// Which side.
        channel: Channel,
        /// Start, ms into the meeting.
        start_ms: u64,
        /// End.
        end_ms: u64,
        /// The words.
        text: Spoken,
    },
    /// Something went wrong and the meeting went on.
    Warning(MeetingWarning),
    /// Capture ended; the record is marked ended.
    Stopped,
    /// The final pass finished one side.
    Transcribed(ChannelPass),
    /// Diarization of the far end finished. Not sent when no diarizer is installed.
    Diarized(Diarization),
    /// The final pass replaced the live transcript.
    Superseded {
        /// The new revision.
        revision: u32,
    },
    /// The final pass did not replace the live transcript, which stays as it was.
    KeptLive(KeptLive),
    /// The summary is saved.
    Summarized {
        /// Items dropped because their citation did not check out.
        unverified: usize,
    },
    /// Commitments are saved.
    Commitments {
        /// Filed, the duplicates included.
        filed: usize,
        /// Of those, folded into another ("said twice").
        merged: usize,
    },
    /// Everything is done. Sent by every final pass that got as far as the supersede.
    Finished {
        /// The transcript's revision now; `None` only when the live one was kept and the record
        /// could not be read.
        revision: Option<u32>,
    },
}
