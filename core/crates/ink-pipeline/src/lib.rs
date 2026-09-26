//! The chains: dictation (S1.5a), meetings and file import (S1.5b), and the silent-channel
//! watchdog (S1.5c).
//!
//! # Dictation
//!
//! Inkwell 0.2's canonical pipeline, stages 1–11, on the 1.0 traits. [`chain`] holds the stage
//! table; the stages' parts:
//!
//! | Module | Holds |
//! |---|---|
//! | [`mic`] | Stage 2 for a live mic: downmix and resample, with host times. |
//! | [`transition`] | What a hotkey edge means (hold versus toggle). |
//! | [`tail`] | The adaptive tail after the release. |
//! | [`gain_stage`] | Stage 3: VAD-gated gain, or the fallback, and the discard rule. |
//! | [`voicecommand`] | Stage 5. |
//! | [`cleanup`], [`style`], [`dictionary`], [`snippets`], [`text`] | Stages 6–8, pure. |
//! | [`modes`] | Which style, cleanup and polish apply, per app. |
//! | [`events`] | Everything the chain reports. |
//! | [`worker`] | The thread that owns a chain. |
//! | [`update`] | Replacing a model's files only after it is unloaded. |
//! | [`export`] | Dictations as text, SRT, JSON or CSV. |
//!
//! # Privacy (I5)
//!
//! Transcripts never reach a log or an error. Log lines carry counts, levels and timings, and
//! [`redact`](redact::redact) for text lengths; `tests/privacy_lint.rs` checks every log call in
//! this crate. The one event with the user's words holds them as [`Spoken`](redact::Spoken), which
//! prints as a length.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod chain;
pub mod civil;
pub mod cleanup;
pub mod dictionary;
pub mod events;
pub mod export;
pub mod gain_stage;
pub mod mic;
pub mod modes;
pub mod redact;
pub mod snippets;
pub mod style;
pub mod tail;
pub mod text;
pub mod transition;
pub mod update;
pub mod voicecommand;
pub mod worker;
