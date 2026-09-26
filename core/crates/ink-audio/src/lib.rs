//! Audio plumbing between capture and the engines.
//!
//! The capture path, one channel at a time:
//!
//! ```text
//! device ──push──► CaptureProducer ══ring══► CaptureConsumer ──► ChunkWriter ──► chunks on disk
//!  (realtime thread: copy, never wait)        (pump thread: drain, write, check the rate)
//! ```
//!
//! - [`ring`]: the SPSC capture ring. Its producer is the realtime [`AudioSink`]; overruns are
//!   counted and reported to the consumer at the block where the audio went missing.
//! - [`chunk`]: raw PCM chunks on disk, one sequence per channel of a record, with torn-chunk
//!   recovery. [`CHUNK_DURATION`] is the unit of "lose nothing".
//! - [`rate`]: the sample-rate check, which compares the audio a stream delivered with the host
//!   time it took and reports a mismatch instead of resampling it away.
//! - [`replay`]: [`FileReplaySource`], the replay harness that drives the same path from WAV
//!   fixtures, deterministically, on every OS.
//! - [`realtime`]: the guard seam that lets tests prove every realtime push allocation-free (I4).
//!
//! Resampling, downmix, the gain stage, VAD and bands arrive in their own modules later.
//!
//! [`AudioSink`]: ink_core::AudioSink

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod chunk;
pub mod rate;
pub mod realtime;
pub mod replay;
pub mod ring;

pub use chunk::{
    CHUNK_DURATION, ChunkError, ChunkInfo, ChunkStore, ChunkWriter, RecoveryReport, Repair,
    WriterSummary,
};
pub use rate::{Continuity, RateCheck, RateVerdict};
pub use realtime::{RealtimeGuard, unguarded};
pub use replay::{FileReplaySource, Pacing};
pub use ring::{
    CaptureConsumer, CaptureProducer, CapturedBlock, DEFAULT_RING_DURATION, Overruns, RingError,
    capture_ring,
};
