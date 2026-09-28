//! What the dictation chain tells the shell.
//!
//! Every outcome of a take is an event: nothing is dropped quietly, and nothing that went wrong is
//! only logged. The variants that carry the user's words, [`DictationEvent::Inserted`] and
//! [`DictationEvent::Partial`], hold them as [`Spoken`], which prints as a length, so logging an
//! event cannot leak a transcript (I5). Every other variant carries no text of theirs: errors from
//! engines, the store, the platform and language models name what failed, never what was said
//! (`ink-core`'s error contract). A voice edit's selection never leaves the chain.

use ink_core::{EngineError, InsertOutcome, LlmError, PlatformError, RecordId, StoreError};

use crate::consent::LlmConsent;
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
    /// It failed while running (a meeting's AGC or final pass), and the fallback took over for
    /// the rest of that stream or pass.
    Failed,
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
    /// The VAD found speech, but too little of it to set a level (no stretch as long as
    /// `ink_audio::gain::MIN_LEVEL_SEGMENT_WINDOWS`, 192 ms): a quick one-word answer, or a knock
    /// the VAD took for speech. No engine saw the take. Tell the user it was too short and to try
    /// again: something was heard, unlike [`NoSpeech`](Self::NoSpeech), and the key was held long
    /// enough, unlike [`TooShort`](Self::TooShort).
    SpeechTooShort,
    /// The engine returned no words.
    NothingHeard,
    /// The engine returned words, and the mode's cleanup and the dictionary removed them all (a
    /// take of only fillers). Nothing was sent to polish, saved or inserted.
    NothingLeft,
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
    /// Polish is on, but the user has not agreed to send dictations where the model goes now
    /// (never agreed, or the model changed to another destination since): the text went out
    /// unpolished and nothing was sent. Holds the consent it would need.
    PolishNotAllowed(LlmConsent),
    /// Polish gave no answer within its budget ([`POLISH_BUDGET`](crate::chain::POLISH_BUDGET));
    /// the text went out unpolished. Apart from [`PolishFailed`](Self::PolishFailed) with
    /// `Cancelled`, which is a model stopped for its own reasons (the core shutting down), so a
    /// shell can tell a polish that keeps timing out from an ordinary cancel.
    PolishTimedOut,
    /// A style command named no mode.
    NoModeForStyle,
    /// The dictation was inserted (or insertion was attempted) but not saved to the library.
    SaveFailed(StoreError),
    /// Text the library deleted or replaced could not yet be cleared from the database's files:
    /// another process is reading the database. The change itself is saved; the store keeps
    /// trying, and [`DeletedTextScrubbed`](Self::DeletedTextScrubbed) follows once it succeeds.
    /// Sent once per change, from whichever chain sees it first.
    DeletedTextNotScrubbed,
    /// Deleted text the library could not clear before is now cleared from its files.
    DeletedTextScrubbed,
    /// A push-to-talk key (or the edit key) was held for longer than anyone dictates
    /// ([`DEFAULT_STUCK_AFTER`](crate::chain::DEFAULT_STUCK_AFTER), 180 s): its release was most
    /// likely lost. The take was stopped there and processed, never discarded: the user did speak.
    ReleaseMissed,
}

/// Why a voice edit ended without replacing the selection. None of these touched the user's text.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditFailure {
    /// Nothing was selected in the focused app when the key was held.
    NoSelection,
    /// Secure Input was on (a password field, a terminal's secure entry): the selection was not
    /// read, so nothing of it could reach a language model.
    SecureInput,
    /// The selection could not be read (Accessibility is off, or the app does not expose it).
    SelectionUnreadable(PlatformError),
    /// The instruction could not be transcribed.
    Transcription(EngineError),
    /// No language model is set up to rewrite the selection.
    NoModel,
    /// The language model gave no answer within the edit's budget
    /// ([`EDIT_BUDGET`](crate::chain::EDIT_BUDGET)).
    TimedOut,
    /// The language model failed, or answered with nothing.
    Model(LlmError),
    /// The rewrite could not be inserted.
    Insert(PlatformError),
    /// The user has not agreed to send the selection where the model goes now (never agreed, or
    /// the model changed destination since): nothing was sent. Holds the consent it would need.
    NotAllowed(LlmConsent),
}

/// An event from the dictation chain, in order for one chain.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DictationEvent {
    /// Whether takes are levelled with voice detection. Sent when the chain starts and whenever it
    /// changes.
    VoiceDetection(VoiceDetection),
    /// A hold passed the minimum and is now a take: the shell shows it is listening.
    Started {
        /// This take's number, counted from 0 for the chain: its [`Partial`](Self::Partial)s
        /// carry it, so a late one is never shown under the next take.
        take: u64,
        /// Whether the take is a voice edit (the edit key) rather than a dictation.
        edit: bool,
        /// The mode a dictation is expected to write in, by name, from the app in front when it
        /// started (the text is written in the mode of the app that receives it). `None` for an
        /// edit.
        mode: Option<String>,
        /// That app's name, when the OS reported one. `None` for an edit.
        app: Option<String>,
    },
    /// What the live engine hears so far, while the key is held: settled words, then the current
    /// hypothesis. Each replaces the previous one. Ephemeral: never saved, never logged
    /// (architecture rule 4). None arrives for a take after its [`Stopped`](Self::Stopped).
    Partial {
        /// The take it belongs to ([`Started`](Self::Started)'s `take`).
        take: u64,
        /// The words so far.
        text: Spoken,
    },
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
    /// The OS removed the edit key. Nothing more arrives from it until it is started again.
    EditHotkeyLost,
    /// A voice edit replaced the selection with its rewrite.
    Edited {
        /// How the rewrite went in.
        outcome: InsertOutcome,
    },
    /// A voice edit ended without touching the selection.
    EditFailed(EditFailure),
    /// A stage panicked (an engine, the store, polish, the inserter, or the chain itself). The take
    /// in progress is lost. With `recovered`, the chain is idle again and the next take works;
    /// without it, the worker has stopped after repeated panics and dictation is off until the
    /// shell starts a new one.
    WorkerFailed {
        /// Whether the worker is still serving.
        recovered: bool,
    },
}
