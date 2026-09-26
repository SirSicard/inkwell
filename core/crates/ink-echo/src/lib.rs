//! Echo cancellation for the mic channel.
//!
//! On laptop speakers the mic hears the far end, and without this stage the far end's words would
//! be transcribed as the user's. The stages, in the order a meeting's final pass runs them:
//!
//! ```text
//! mic ─┬─► PathFinder ─► echo path? ── none (earbuds, headphones) ─► the mic, untouched
//! far ─┘   (GCC-PHAT consensus)   └─ found ─► EchoCanceller ─┬─► linear ─► "you" words ─┐
//!                                             (drift-cancelling  └─► full ─► VAD ─► EchoGate ◄┘
//!                                              resample + AEC3)                        │ kept
//!                                                                  "them" lines ─► dedup ─► "you"
//! ```
//!
//! - [`path`]: GCC-PHAT over 2 s windows and a consensus line through them: the delay, and the
//!   drift between the two clocks. **AEC runs only when it finds a path**
//!   ([`EchoCanceller::when_found`]): with earbuds there is none, and AEC3's suppressor still
//!   deleted words in S0.3's recordings.
//! - [`canceller`]: the far end resampled onto the mic's clock, then AEC3 with its default
//!   13-block filter ([`aec::FILTER_BLOCKS`]; longer filters cancelled less in a real room). It
//!   yields both outputs: the **linear** one, which the "you" transcript reads, and the **full**
//!   one, which only drives the word gate. In S0.3's real double talk the full output cost 9.9
//!   WER points and the linear output 0.7.
//! - [`gate`]: drops words of the linear transcript where the full output says echo only (the
//!   linear output leaks far-end words when nobody on the near end talks).
//! - [`dedup`]: removes a "you" line that repeats, at the same moment, what the far end said, when
//!   the full output also heard no near-end speech over it; removed lines come back by index and
//!   span so they can be stored and restored.
//! - [`measure`]: ERLE and the other numbers the stage is judged on.
//!
//! # Threads
//!
//! Everything here runs on a **worker**, the meeting chain's own; nothing runs on the realtime
//! thread or the pump. The pump only writes both sides' chunks, and the chain reads them back.
//!
//! | Piece | Work | Allocation |
//! |---|---|---|
//! | [`PathFinder::push`] | three 64k-point FFTs per second of audio | none per window, beyond one small entry kept per window |
//! | [`PathFinder::estimate`] | the consensus fit, at most 256 proposing windows | per call |
//! | [`EchoCanceller::push`] | per 10 ms frame: the resample and AEC3 | none of its own; AEC3 allocates inside every frame |
//! | [`EchoGate`] | per word | one byte per frame and one float per VAD window, kept |
//! | [`dedup`] | per line | per call |
//!
//! AEC3 is why none of this could move to the pump: the `aec3` crate's render path clones a
//! frame into its queue on every call (`tests/no_alloc.rs` counts it).
//!
//! All audio is 16 kHz mono ([`RATE`]), from the common start of both streams. No error or log
//! line here carries audio or text.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod aec;
mod buffer;
pub mod canceller;
pub mod dedup;
pub mod gate;
pub mod gcc;
mod interp;
pub mod measure;
pub mod path;

use std::fmt;

use ink_core::Channel;

pub use canceller::{CancellerConfig, EchoCanceller, EchoFrame, FrameLevels, level_db};
pub use dedup::{
    DedupConfig, DedupReport, Duplicate, NearSpeech, echo_duplicates, remove_echo_duplicates,
};
pub use gate::{EchoGate, GateConfig, Verdict};
pub use path::{Alignment, PathFinder, PathReport};

/// The rate everything here runs at: the core's canonical 16 kHz.
pub const RATE: f64 = ink_core::CANONICAL_RATE as f64;
/// One 10 ms frame at [`RATE`]: AEC3's unit of work and the word gate's unit of time.
pub const FRAME: usize = ink_core::CANONICAL_RATE as usize / 100;

/// What can go wrong. No variant carries audio or text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoError {
    /// One stream ran further ahead of the other than the buffers hold. The push was refused
    /// whole; the caller is feeding the two streams out of step (or one of them stalled).
    Backlog {
        /// The stream that is ahead.
        ahead: Channel,
    },
    /// An alignment or setting outside what any real echo path has.
    BadAlignment,
    /// The canceller was already finished.
    Ended,
    /// A speech probability outside 0–1 (NaN included) reached the gate. It was not recorded.
    BadSpeechProbability,
}

impl fmt::Display for EchoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backlog { ahead } => write!(
                f,
                "the {} stream ran too far ahead of the other",
                match ahead {
                    Channel::Mic => "mic",
                    Channel::Far => "far-end",
                }
            ),
            Self::BadAlignment => write!(f, "the echo path's alignment is out of range"),
            Self::Ended => write!(f, "the echo canceller has already finished"),
            Self::BadSpeechProbability => write!(f, "a speech probability outside 0 to 1"),
        }
    }
}

impl std::error::Error for EchoError {}
