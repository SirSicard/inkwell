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
//! After the ring, everything runs on the pump or a worker, never the realtime thread. The DSP
//! between the device's format and an engine:
//!
//! ```text
//! device format ─► Downmix ─► StreamResampler ─► 16 kHz mono ─┬─► TakeRecorder (dictation)
//!  (interleaved)   (mic: primary,  (built once per device)     ├─► Agc (a live meeting)
//!                   far: average)                               ├─► BandAnalyzer ─► BandsWriter ═► BandsReader (shell)
//!                                                               └─► Windower (import, final pass)
//!
//! a take or a window ─► normalise ─► vad::trim_ends ─┬─► Some(range) ─► engine
//!                                                     └─► None ─► discarded: no engine sees it
//! ```
//!
//! **The gain stages decide level, not speech.** [`normalise`] and [`Agc`] leave a stationary
//! room alone (room tone, hum, rumble; see [`speech_band`]) and lift anything else quiet, speech
//! or not: a cycling fan, a cough, sometimes knocks. So
//! the VAD's verdict is binding: a take or window in which [`trim_ends`] finds no speech is
//! discarded before any engine sees it, never passed on whole (a recogniser handed lifted noise
//! may invent words).
//!
//! - [`downmix`]: one channel from many, chosen per stream: the mic's primary channel, the far
//!   end's average.
//! - [`resample`](mod@resample): to 16 kHz, time-aligned, with the tail flushed; streaming and
//!   offline.
//! - [`take`]: a dictation take with 300 ms of lead before the press and tail after the release.
//! - [`gain`]: the per-utterance robust-peak normaliser ahead of every engine (architecture rule
//!   11), and the level measures everything else uses.
//! - [`agc`]: the slow meeting AGC toward the same target, which holds through pauses.
//! - [`speech_band`]: the stationary test both gain stages share, measured on the speech band.
//! - [`vad`]: trims the dead air at the ends of a take, never the pauses inside; the model sits
//!   behind [`SpeechProbability`].
//! - [`window`]: long audio in windows of at most 60 s, cut at the quietest point, overlapping by
//!   2 s.
//! - [`bands`]: three energy bands per FFT hop for the ink, read out through a copy-out reader.
//! - [`synth`]: deterministic synthetic signals for tests and fixtures (real audio never enters
//!   the repository).
//!
//! [`AudioSink`]: ink_core::AudioSink

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod agc;
pub mod bands;
pub mod chunk;
pub mod downmix;
pub mod gain;
pub mod rate;
pub mod realtime;
pub mod replay;
pub mod resample;
pub mod ring;
pub mod speech_band;
pub mod synth;
pub mod take;
pub mod vad;
pub mod window;

pub use agc::Agc;
pub use bands::{BandAnalyzer, Bands, BandsReader, BandsSnapshot, BandsWriter, bands_channel};
pub use chunk::{
    CHUNK_DURATION, ChunkError, ChunkInfo, ChunkList, ChunkStore, ChunkWriter, RecoveryReport,
    Repair, UnreadableChunk, WriterSummary,
};
pub use downmix::Downmix;
pub use gain::{GainOutcome, GainReport, TARGET_PEAK, normalise, robust_peak};
pub use rate::{Continuity, RateCheck, RateVerdict};
pub use realtime::{RealtimeGuard, unguarded};
pub use replay::{FileReplaySource, Pacing};
pub use resample::{ResampleError, StreamResampler, resample};
pub use ring::{
    CaptureConsumer, CaptureProducer, CapturedBlock, DEFAULT_RING_DURATION, Overruns, RingError,
    capture_ring,
};
pub use take::{Take, TakeRecorder};
pub use vad::{SpeechProbability, SpeechSegmenter, VadConfig, trim_ends};
pub use window::{Window, WindowConfig, WindowError, Windower, plan_windows};
