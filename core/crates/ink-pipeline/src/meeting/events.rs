//! What the meeting chain tells the shell.
//!
//! As with dictation, every outcome is an event and nothing that went wrong is only logged. The
//! variants with the meeting's words ([`MeetingEvent::Partial`], [`MeetingEvent::Final`]) hold them
//! as [`Spoken`], which prints as a length; every other variant carries no text, so logging an
//! event cannot leak a transcript (I5).

use ink_core::{Channel, EngineError, LlmError, RecordId, StoreError};
use ink_echo::EchoError;

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
    /// No signal from the far end for a minute or more while the mic was audible: maybe the wrong
    /// app is tapped, or its audio goes elsewhere; maybe only a presentation. A soft warning, once
    /// per quiet stretch ([`watchdog`](crate::meeting::watchdog)).
    FarEndQuietWhileYouSpeak {
        /// How long the far end had been without signal, ms.
        quiet_ms: u64,
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
    /// A side captured audio, and every sample of it is exactly zero: no data at all (a denied
    /// capture that still called back, or input muted to zero). A quiet side is not this: its
    /// samples are small, not zero.
    CapturedOnlyZeros {
        /// Which side.
        channel: Channel,
    },
    /// The mic was a Bluetooth headset mic, and every sample it captured is exactly zero. Such a
    /// mic gates to zeros while its user is silent, so this may be someone who never spoke; it is
    /// also what a headset mic that never worked looks like. Softer than
    /// [`CapturedOnlyZeros`](Self::CapturedOnlyZeros), which says no data was captured at all.
    BluetoothMicOnlyZeros,
    /// A side's recorded audio could not even be listed (its directory is gone, or unreadable).
    /// The final pass stops before writing anything, and returns the error too.
    AudioUnlisted {
        /// Which side.
        channel: Channel,
        /// Why, naming the directory, never audio.
        reason: String,
    },
    /// A side has no recorded audio at all: no chunk was ever written for it (a device that never
    /// delivered, a permission denied, a tap that never started). Silence would still have chunks.
    NothingCaptured {
        /// Which side.
        channel: Channel,
    },
    /// A live "you" final lay where the far end was playing and AEC3's full output heard nobody
    /// on the near end (the echo gate, [`echo`](crate::meeting::echo)), so it was not saved: the
    /// live engine transcribed echo that the linear output still carried.
    EchoOnlyFinal {
        /// Its start, ms into the meeting.
        start_ms: u64,
        /// Its end.
        end_ms: u64,
    },
    /// The VAD that judges AEC3's full output for the live echo gate could not be loaded, or
    /// failed. The gate has no evidence from here, so it keeps every live "you" final, and a
    /// weak cancellation can no longer be noticed ([`EchoState::Degraded`]).
    EchoGateVadFailed(EngineError),
    /// Echo cancellation failed in the final pass: the mic was transcribed as captured instead,
    /// with no echo removed.
    EchoFailed(EchoFailure),
    /// The final pass found no echo path, yet the mic's level followed the far end's while it
    /// played (a path the search could not fit: clocks drifting apart faster than it accepts, a
    /// path that changed mid-meeting). Echo is possible, and the mic was transcribed as captured:
    /// "you" lines may hold the far end's words. Headphones, with nothing leaking, stay quiet.
    EchoPathNotFound {
        /// How long the mic was audible while the far end played, ms.
        heard_ms: u64,
    },
    /// Text the library deleted or replaced (the live transcript this pass superseded, an old
    /// summary or title) could not yet be cleared from the database's files: another process is
    /// reading the database. The change itself is saved. The store keeps trying, and
    /// [`DeletedTextScrubbed`](Self::DeletedTextScrubbed) follows once it succeeds. Sent once per
    /// change, from whichever chain sees it first.
    DeletedTextNotScrubbed,
    /// Deleted text the library could not clear before is now cleared from its files.
    DeletedTextScrubbed,
    /// The host could not leave its crash-recovery marker beside the meeting's chunks: if the
    /// app quits unexpectedly, this meeting is not finished at the next launch (its audio is
    /// still saved as it records). The chain never raises it; the host does, through the
    /// meeting's event sink. The string names what failed, never the meeting's words.
    NotCrashProtected(String),
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
    /// Live finals saved without the speech check, because too many waited on the VAD at once
    /// ([`MeetingWarning::LiveFinalsBacklog`]).
    pub backlogged_finals: u64,
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

/// Why the live echo search (re)started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoSearch {
    /// The meeting started.
    Start,
    /// The capture's routing changed ([`MeetingChain::set_routing`]): a new device is a new
    /// echo path, so the old one is dropped.
    ///
    /// [`MeetingChain::set_routing`]: crate::meeting::MeetingChain::set_routing
    DeviceSwitch,
    /// Cancellation failed ([`EchoState::Failed`]).
    AfterFailure,
}

/// Why echo cancellation stopped. Nothing here carries audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoFailure {
    /// One side ran more than 10 s ahead of the other (a mic that stopped delivering while the
    /// far end played, or the reverse).
    Backlog {
        /// The side that ran ahead.
        ahead: Channel,
    },
    /// The path found is outside what the canceller follows (more than 10 s of delay, or 1 % of
    /// drift): no real echo path is.
    BadAlignment,
    /// The canceller gave no answer in time (seconds beyond what its audio needs): it is left
    /// behind, and the mic goes on as captured.
    Stalled,
    /// The canceller stopped for a reason inside the pipeline (its thread ended, or it was used
    /// after it finished): a bug, reported rather than hidden.
    Internal,
}

impl From<EchoError> for EchoFailure {
    fn from(error: EchoError) -> Self {
        match error {
            EchoError::Backlog { ahead } => Self::Backlog { ahead },
            EchoError::BadAlignment => Self::BadAlignment,
            EchoError::Ended | EchoError::BadSpeechProbability => Self::Internal,
        }
    }
}

/// Whether the mic is protected from the far end's echo while the meeting is live: sent when it
/// changes ([`echo`](crate::meeting::echo)). The final pass cancels the whole recording again on
/// its own ([`EchoPass`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EchoState {
    /// No echo path yet: the mic reaches the live transcript as captured, **unprotected**. From
    /// the start, and after each device switch or failure, the search needs at least 10 s of
    /// far-end audio. With earbuds or headphones there is no path to find, and this stays.
    Searching {
        /// Where in the meeting the search began, ms.
        since_ms: u64,
        /// Why it began.
        why: EchoSearch,
    },
    /// An echo path was found: from `from_ms` the live "you" transcript hears AEC3's linear
    /// output, behind the echo gate. Sent again when a later estimate moves the path by more than
    /// 2 ms (a first fit over under 10 s of audio has no drift), cancellation carrying on along
    /// the better one, and when [`Degraded`](Self::Degraded) recovers.
    Cancelling {
        /// Where cancellation (along this path) began, ms into the meeting.
        from_ms: u64,
        /// How long the mic went unprotected before it, since the search began, ms (0 when
        /// cancellation was already running).
        unprotected_ms: u64,
        /// Where the windows first supported this path, ms into the meeting: the search runs a
        /// few times a minute, so it can find a path some seconds after that.
        stable_from_ms: Option<u64>,
        /// How late the mic hears the far end, ms.
        delay_ms: f64,
        /// How fast the mic's clock runs against the far end's, ppm.
        drift_ppm: f64,
    },
    /// Cancelling, but removing far less echo than it should: the linear stage, which the live
    /// "you" transcript hears, takes under 4 dB off the far end's frames over the last 20 s of
    /// far-end audio (converged, its frames reach 11–13 dB). Echo from audio the tap does not
    /// carry, or a path gone wrong. Cancellation goes on; `Cancelling` is sent again when it
    /// recovers (6 dB).
    Degraded {
        /// The measured linear-stage ERLE, dB.
        erle_db: f32,
    },
    /// Cancellation failed and stopped. The mic reaches the live transcript as captured again,
    /// and the search restarts ([`EchoSearch::AfterFailure`]).
    Failed(EchoFailure),
    /// Sent once at the end of the live phase when the search was still running and a last look
    /// at its windows found a path: the mic went unprotected for the whole search. The final
    /// pass cancels it.
    FoundAtEnd {
        /// How long the search ran without cancelling, ms.
        unprotected_ms: u64,
        /// Where the windows first supported the path, ms into the meeting.
        stable_from_ms: Option<u64>,
    },
}

/// The echo path the final pass fitted over the whole recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EchoPath {
    /// How late the mic hears the far end at the start, ms.
    pub delay_ms: f64,
    /// How fast the mic's clock runs against the far end's, ppm.
    pub drift_ppm: f64,
    /// Windows on the path.
    pub inliers: usize,
    /// Where the windows first supported it, ms into the meeting: live cancellation could not
    /// have begun earlier.
    pub stable_from_ms: Option<u64>,
}

/// What echo cancellation did in the final pass.
///
/// The final pass fits its own path over the whole recording (never the live one), cancels the
/// mic along it from the start, and transcribes AEC3's linear output inside the stretches where
/// the VAD hears speech in the full output. Then "you" lines that repeat the far end over audio
/// where the full output heard nobody are removed ([`RemovedEcho`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EchoPass {
    /// The path, when the recording has one.
    pub path: Option<EchoPath>,
    /// Windows analysed.
    pub windows: usize,
    /// Windows loud and clear enough to vote.
    pub candidates: usize,
    /// Whether the mic was cancelled (false: no path, or cancellation failed:
    /// [`MeetingWarning::EchoFailed`]).
    pub cancelled: bool,
    /// Echo return loss enhancement over the first 10 s of far-end audio, dB: how far AEC3's
    /// full output sits under the mic in the frames the best fifth of them reach (a monitor that
    /// needs no knowledge of when the near end talks; the gate's figure is measured against the
    /// truth, by the echo fixture). `None` under a second of far-end audio.
    pub erle_first_db: Option<f32>,
    /// Echo return loss enhancement over far-end audio after that, dB.
    pub erle_db: Option<f32>,
    /// The same for the linear output alone (what the "you" transcript hears), dB.
    pub linear_erle_db: Option<f32>,
    /// "You" lines removed as echo of the far end.
    pub removed: usize,
    /// Lines whose words repeated the far end's but were kept: the full output heard the near end
    /// over them (a read-back).
    pub kept_near_speech: usize,
    /// Lines whose words repeated the far end's but were kept for want of acoustic evidence.
    pub kept_no_evidence: usize,
    /// Live "you" finals this pass judged to be echo (the echo gate's rule, or dedup's, on its
    /// own evidence), with far-end speech under them by the far end's own VAD: mostly those from
    /// before the live search found the path. The supersede guard does not count them, so a
    /// transcript that loses them is still saved.
    pub live_echo_finals: usize,
}

/// A far-end line a removed "you" line matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FarLine {
    /// Its start, ms into the meeting.
    pub start_ms: u64,
    /// Its end.
    pub end_ms: u64,
}

/// A "you" line the final pass removed as echo of the far end, whole, so it can be put back
/// (the store keeps it with the record: `Store::removed`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovedEcho {
    /// Its start, ms into the meeting.
    pub start_ms: u64,
    /// Its end.
    pub end_ms: u64,
    /// The words.
    pub text: Spoken,
    /// The far-end lines whose words it matched, in time order.
    pub far: Vec<FarLine>,
    /// Its words, normalised.
    pub words: usize,
    /// How many of them matched the far end, in order.
    pub matched: usize,
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
    /// What a side is delivering, as the silent-channel watchdog judges it
    /// ([`watchdog`](crate::meeting::watchdog)). Sent when it changes: a side that stopped, or
    /// gives only digital zeros, within the watchdog's limit; and back to `Ok` when it recovers.
    SideState {
        /// Which side.
        channel: Channel,
        /// Its state.
        state: crate::meeting::watchdog::SideState,
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
    /// Whether the live mic is protected from echo, when it changes.
    Echo(EchoState),
    /// What echo cancellation did in the final pass. Sent before the supersede.
    EchoPass(EchoPass),
    /// "You" lines the final pass removed as echo, with their words (as [`Spoken`]). Sent only
    /// when there are some, before the supersede; the supersede keeps them with the record, in its
    /// own transaction and in the same order (by start time), so they can be put back.
    RemovedAsEcho(Vec<RemovedEcho>),
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
    /// The user said in this meeting that work was already done which earlier meetings' open
    /// commitments promise: each such commitment now carries a "looks done" suggestion for the
    /// user to confirm or dismiss. Sent only when there is at least one.
    LooksDone {
        /// Commitments given a suggestion.
        suggested: usize,
    },
    /// Everything is done. Sent by every final pass that got as far as the supersede.
    Finished {
        /// The transcript's revision now; `None` only when the live one was kept and the record
        /// could not be read.
        revision: Option<u32>,
    },
}
