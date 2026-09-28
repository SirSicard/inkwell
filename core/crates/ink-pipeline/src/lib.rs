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
//! | [`consent`] | Where the user agreed polish may send their words. |
//! | [`events`] | Everything the chain reports. |
//! | [`worker`] | The thread that owns a chain. |
//! | [`update`] | Replacing a model's files only after it is unloaded. |
//! | [`warm`] | Warming the dictation engine when a take starts, after a quiet spell. |
//! | [`export`] | Dictations as text, SRT, JSON or CSV. |
//!
//! # Meetings and file import
//!
//! | Module | Holds |
//! |---|---|
//! | [`capture`] | The pump's half: each side's capture ring into its chunks on disk, and on as canonical audio. |
//! | [`meeting`] | Two sides live (the AGC with a VAD, live finals over VAD speech), then the final pass per side, far-end diarization, the supersede, the summary and commitments. |
//! | [`speech`] | What of a long recording an offline engine may hear: VAD-gated gain per window, and speech regions. |
//! | [`import`] | A file becomes a record, through the same gain and speech regions. |
//!
//! # Privacy (I5)
//!
//! Transcripts never reach a log or an error. Log lines carry counts, levels and timings, and
//! [`redact`](redact::redact) for text lengths; `tests/privacy_lint.rs` checks every log call in
//! this crate. The events with the user's words (a dictation inserted, a meeting's partials and
//! finals) hold them as [`Spoken`](redact::Spoken), which prints as a length.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod capture;
pub mod chain;
pub mod civil;
pub mod cleanup;
pub mod consent;
pub mod dictionary;
pub mod events;
pub mod export;
pub mod gain_stage;
pub mod import;
pub mod meeting;
pub mod mic;
pub mod modes;
pub mod redact;
pub mod snippets;
pub mod speech;
pub mod style;
pub mod tail;
pub mod text;
pub mod transition;
pub mod update;
pub mod voicecommand;
pub mod warm;
pub mod worker;
