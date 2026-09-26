//! Audio plumbing between capture and the engines.
//!
//! - S1.2a: the SPSC capture ring (the realtime side of `ink_core::audio::AudioSink`), the chunk
//!   store (raw PCM on disk, torn-chunk recovery) and `FileReplaySource`.
//! - S1.2b: streaming resampler, downmix, the gain stage and meeting AGC, Silero VAD, and `Bands`.
//!
//! Stub until S1.2a.
