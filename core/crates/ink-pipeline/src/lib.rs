//! The chains: dictation (S1.5a), meetings and file import (S1.5b), and the silent-channel
//! watchdog (S1.5c).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod civil;
pub mod cleanup;
pub mod dictionary;
pub mod export;
pub mod mic;
pub mod modes;
pub mod redact;
pub mod snippets;
pub mod style;
pub mod transition;
pub mod voicecommand;
