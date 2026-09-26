//! Types and every trait of the Inkwell core.
//!
//! This crate is the contract the rest of the core and both native shells build on. It holds
//! types and traits only, depends on nothing, and does no I/O. Implementations live elsewhere:
//!
//! | Trait | Implemented in |
//! |---|---|
//! | [`AudioSource`], [`CaptureControl`], [`MeetingDetector`], [`HotkeySource`], [`TextInserter`], [`FocusReader`], [`PermissionProbe`], [`Clock`] | `ink-platform-mac`, `ink-platform-win`; `FileReplaySource` in `ink-audio` |
//! | [`AudioSink`] | `ink-audio` (the capture ring) |
//! | [`Store`] | `ink-store` |
//! | [`OfflineEngine`], [`StreamingEngine`], [`Diarizer`] | `ink-engines`, and engines the Mac shell registers over the C ABI |
//! | [`Llm`] | `ink-llm` |
//!
//! Every method's documentation names the thread it may run on. The terms are defined in
//! [`threading`]; read that module first.
//!
//! With the `mock` feature, [`mock`] provides an in-memory store, mock engines keyed by fixture
//! hash, and a scriptable platform, so every crate can test against these traits without devices,
//! models or a database.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod audio;
pub mod clock;
pub mod engine;
pub mod error;
pub mod llm;
pub mod platform;
pub mod store;
pub mod threading;

#[cfg(feature = "mock")]
pub mod mock;

pub use audio::{
    AudioBlock, AudioSink, AudioSource, CANONICAL_RATE, Channel, SourceStats, StreamFormat,
};
pub use clock::Clock;
pub use engine::{
    AsrEvent, Diarizer, EngineInfo, EngineStream, Job, OfflineEngine, SpeakerId, SpeakerTurn,
    StreamingEngine, TimedText, TranscribeOptions, Transcript,
};
pub use error::{EngineError, LlmError, PlatformError, StoreError};
pub use llm::{Endpoint, Llm, LlmInfo, LlmRequest, LlmResponse};
pub use platform::{
    AppRef, CaptureControl, DeviceId, DeviceInfo, FarEndTarget, FocusInfo, FocusReader,
    HotkeyBinding, HotkeyEvent, HotkeySource, InsertOutcome, MeetingDetector, MeetingSignal,
    Permission, PermissionProbe, PermissionState, Platform, TextInserter, Transport,
};
pub use store::{
    Commitment, CommitmentId, NewCommitment, NewRecord, Record, RecordId, RecordKind, SearchHit,
    Segment, Span, Store, Summary,
};
pub use threading::{CancelToken, EventSink};
