//! Stage 3 of a dictation: level the take and find its speech, before any engine sees it
//! (architecture rule 11).
//!
//! Which call runs is decided by whether a VAD is installed, and nothing else:
//!
//! | VAD | Call | A take with no speech | Trim |
//! |---|---|---|---|
//! | installed | [`normalise_speech`]: the gain is learned from speech alone | discarded, no engine sees it | to the speech, pauses kept |
//! | not installed (missing, downloading) | [`normalise_without_vad`], while the shell shows voice detection as unavailable | passed on | none |
//! | installed, but it fails on this take | the same fallback, and the failure is reported | passed on | none |
//!
//! A VAD error is never guessed around and never leaves the take un-levelled: `normalise_speech`
//! returns before touching the take when its VAD fails, so the fallback levels the original.
//!
//! [`normalise_speech`]: ink_audio::normalise_speech
//! [`normalise_without_vad`]: ink_audio::normalise_without_vad

use ink_audio::{
    GainEvidence, GainOutcome, GainReport, SpeechProbability, VadConfig, normalise_speech,
    normalise_without_vad,
};
use ink_core::EngineError;

use crate::events::{Discard, VadUnavailable};

/// The speech-probability source the chain levels with, or why there is none.
pub enum Vad {
    /// Installed: every take is levelled by speech.
    Installed(Box<dyn SpeechProbability>),
    /// Not installed: takes are levelled by the fallback, and the shell says so.
    Unavailable(VadUnavailable),
}

impl std::fmt::Debug for Vad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installed(_) => f.write_str("Vad::Installed"),
            Self::Unavailable(why) => write!(f, "Vad::Unavailable({why:?})"),
        }
    }
}

/// Which call levelled a take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GainPath {
    /// [`normalise_speech`], with the installed VAD.
    Speech,
    /// The fallback, because no VAD is installed.
    NoVad(VadUnavailable),
    /// The fallback, because the VAD failed on this take.
    VadFailed,
}

/// A levelled take.
#[derive(Debug)]
pub struct Levelled {
    /// The audio for the engine: lifted, and trimmed to its speech when a VAD found it.
    pub audio: Vec<f32>,
    /// What the gain stage measured and did (levels and counts only).
    pub report: GainReport,
    /// Which call did it.
    pub path: GainPath,
    /// The VAD's error, when [`path`](Self::path) is [`GainPath::VadFailed`].
    pub vad_error: Option<EngineError>,
}

impl Levelled {
    /// Why the take must not reach an engine, if it must not.
    pub fn discard(&self) -> Option<Discard> {
        match self.report.outcome {
            GainOutcome::NoSpeech => Some(Discard::NoSpeech),
            GainOutcome::Silence => Some(Discard::Silence),
            _ => None,
        }
    }
}

/// **Worker.** Levels `take` (16 kHz mono) with `vad` as the table in the module docs says.
pub fn level(mut take: Vec<f32>, vad: &mut Vad, cfg: &VadConfig) -> Levelled {
    match vad {
        Vad::Installed(source) => match normalise_speech(&mut take, source.as_mut(), cfg) {
            Ok(report) => {
                if let GainEvidence::Vad {
                    speech: Some(keep), ..
                } = &report.evidence
                {
                    let keep = keep.start.min(take.len())..keep.end.min(take.len());
                    take.truncate(keep.end);
                    take.drain(..keep.start);
                }
                Levelled {
                    audio: take,
                    report,
                    path: GainPath::Speech,
                    vad_error: None,
                }
            }
            Err(error) => Levelled {
                report: normalise_without_vad(&mut take),
                audio: take,
                path: GainPath::VadFailed,
                vad_error: Some(error),
            },
        },
        Vad::Unavailable(why) => Levelled {
            report: normalise_without_vad(&mut take),
            audio: take,
            path: GainPath::NoVad(*why),
            vad_error: None,
        },
    }
}
