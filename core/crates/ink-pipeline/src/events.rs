//! What the dictation chain tells the shell.
//!
//! Every outcome of a take is an event: nothing is dropped quietly, and nothing that went wrong is
//! only logged. The one variant that carries the user's words, [`DictationEvent::Inserted`], holds
//! them as [`Spoken`], which prints as a length, so logging an event cannot leak a transcript
//! (I5). Every other variant carries no text at all: errors from engines, the store, the platform
//! and language models name what failed, never what was said (`ink-core`'s error contract).

use ink_core::{EngineError, InsertOutcome, LlmError, PlatformError, RecordId, StoreError};

use crate::redact::Spoken;
use crate::voicecommand::CommandAction;

/// Why no speech-probability source (VAD) is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VadUnavailable {
    /// The model is not installed.
    ModelMissing,
    /// The model is downloading.
    Downloading,
    /// It could not be loaded.
    LoadFailed,
}

/// Whether takes are levelled with voice detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceDetection {
    /// A VAD is installed: the gain is learned from speech alone, and a take without speech is
    /// discarded.
    Available,
    /// No VAD: takes are levelled by the fallback, which can lift non-speech. The shell shows this
    /// state for as long as it lasts.
    Unavailable(VadUnavailable),
}

/// Why a take ended without an insertion. None of these is an error; each is still reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Discard {
    /// The key was held for less than the minimum live length.
    TooShort {
        /// How long it was held, ms.
        live_ms: u64,
    },
    /// The take is digital silence (a muted or dead microphone).
    Silence,
    /// The VAD found no speech. No engine saw the take.
    NoSpeech,
    /// The engine heard nothing, or only fillers.
    NothingHeard,
    /// The hotkey was cancelled or lost with the take open, before it was confirmed.
    Cancelled,
}

/// A take that failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TakeFailure {
    /// The engine failed. Nothing was inserted or saved.
    Transcription(EngineError),
    /// The text could not be inserted. It was saved if [`Warning::SaveFailed`] did not arrive.
    Insert(PlatformError),
}

/// Something went wrong on the way, and the take went on without it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// The VAD failed on this take, so it was levelled by the fallback (as with no VAD) and passed
    /// on whole. The VAD stays installed for the next take.
    VadFailed(EngineError),
    /// The capture ring dropped audio while the take was open: the transcript may miss words.
    AudioLost {
        /// Device frames lost.
        frames: u64,
    },
    /// The audio stopped arriving before the tail did; the take ended at the deadline.
    TailCutShort,
    /// The frontmost app could not be read, so the default mode was used.
    FocusUnreadable(PlatformError),
    /// Polish is on for this mode but no language model is set up; the text went out unpolished.
    PolishUnavailable,
    /// Polish failed; the text went out unpolished.
    PolishFailed(LlmError),
    /// A style command named no mode.
    NoModeForStyle,
    /// The dictation was inserted (or insertion was attempted) but not saved to the library.
    SaveFailed(StoreError),
}

/// An event from the dictation chain, in order for one chain.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DictationEvent {
    /// Whether takes are levelled with voice detection. Sent when the chain starts and whenever it
    /// changes.
    VoiceDetection(VoiceDetection),
    /// A hold passed the minimum and is now a take: the shell shows it is listening.
    Started,
    /// A press shorter than the minimum hold (a modifier used in a shortcut). Nothing was shown
    /// and nothing is transcribed.
    ShortPressIgnored,
    /// The take is closed and being processed.
    Stopped,
    /// The take ended without an insertion.
    Discarded(Discard),
    /// The take was a voice command. The chain has already applied style and polish commands; the
    /// others are the shell's to carry out.
    Command(CommandAction),
    /// The dictation went out.
    Inserted {
        /// The text as inserted (without the trailing space setting's space).
        text: Spoken,
        /// How it went in.
        outcome: InsertOutcome,
        /// Its library record, unless saving failed.
        record: Option<RecordId>,
    },
    /// The take failed.
    Failed(TakeFailure),
    /// Something went wrong and the take went on without it.
    Warning(Warning),
    /// The OS removed the hotkey. Nothing more arrives until it is started again.
    HotkeyLost,
}
