//! Speech engines and the diarizer.
//!
//! Every engine takes 16 kHz mono f32 ([`CANONICAL_RATE`](crate::audio::CANONICAL_RATE)) that has
//! already been through the gain stage (architecture rule 11): an engine never sees the raw level.
//! Engines the Mac shell registers over the C ABI implement these same traits behind a vtable
//! (S1.7), so the threading contracts below hold for them too.

use crate::audio::Channel;
use crate::error::EngineError;
use crate::threading::{CancelToken, EventSink};

/// A job the router fills with the best installed engine for this OS (architecture rule 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Job {
    /// The text a dictation inserts.
    DictationFinal,
    /// The offline pass that supersedes a meeting's live transcript.
    MeetingFinal,
    /// Provisional text while someone is speaking. Never persisted.
    LivePartials,
    /// Who spoke when, on the far end only.
    Diarization,
}

/// What an engine is, for the router and the About screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineInfo {
    /// A stable id, the registry key (for example `qwen3-asr-1.7b-q8`).
    pub id: String,
    /// The jobs it can fill.
    pub jobs: Vec<Job>,
    /// The model weights' licence (SPDX where one exists), shown with its attribution.
    pub licence: String,
}

/// Text with a position, in milliseconds from the start of the audio the engine was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedText {
    /// Start, in ms.
    pub start_ms: u64,
    /// End, in ms.
    pub end_ms: u64,
    /// The text.
    pub text: String,
}

/// An offline engine's result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transcript {
    /// Segments in time order.
    pub segments: Vec<TimedText>,
}

impl Transcript {
    /// The segments' text joined by single spaces, with empty segments left out.
    pub fn text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Options for one offline transcription.
#[derive(Clone, Debug)]
pub struct TranscribeOptions {
    /// The channel the audio came from.
    pub channel: Channel,
    /// Words to favour (the user's dictionary, names), for engines that take a context prompt.
    pub context: Option<String>,
    /// Checked between chunks; the engine returns [`EngineError::Cancelled`] when set.
    pub cancel: CancelToken,
}

/// Transcribes a whole buffer: the dictation final, the meeting final pass, and file import.
pub trait OfflineEngine: Send + Sync {
    /// **Any thread except realtime.** What this engine is.
    fn info(&self) -> EngineInfo;

    /// **Worker.** Transcribes `audio` (16 kHz mono, gain applied). It may block for as long as
    /// the work takes and returns [`EngineError::Cancelled`] once `options.cancel` is set.
    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError>;
}

/// What a live stream reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsrEvent {
    /// Provisional text for the live view. Ephemeral: never written to the store (architecture
    /// rule 4). Each partial replaces the previous one.
    Partial {
        /// The current hypothesis.
        text: String,
    },
    /// Settled text, positioned from the start of the stream.
    Final(TimedText),
    /// The engine has fallen behind real time. The UI shows it; audio is not silently dropped.
    Stalled {
        /// Why, for the UI.
        reason: String,
    },
}

/// Produces live partials.
pub trait StreamingEngine: Send + Sync {
    /// **Any thread except realtime.** What this engine is.
    fn info(&self) -> EngineInfo;

    /// **Worker.** Opens a stream for one channel. `events` runs on a callback thread, in order
    /// for this stream, and must not block.
    fn open_stream(
        &self,
        channel: Channel,
        events: EventSink<AsrEvent>,
    ) -> Result<Box<dyn EngineStream>, EngineError>;
}

/// One open live stream.
pub trait EngineStream: Send {
    /// **Worker**, one thread per stream, calls in order. Feeds 16 kHz mono audio (gain applied).
    /// It must return before the audio it was given would have finished playing.
    fn push(&mut self, audio: &[f32]) -> Result<(), EngineError>;

    /// **Worker.** Flushes and closes the stream. Every trailing event has been delivered before it
    /// returns.
    fn finish(self: Box<Self>) -> Result<(), EngineError>;
}

/// A diarized speaker, as the diarizer labels it (for example `"spk0"`). The user can name it
/// later; the label itself never changes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpeakerId(pub String);

/// One speaker's stretch of the far end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeakerTurn {
    /// Who.
    pub speaker: SpeakerId,
    /// Start, in ms from the start of the audio.
    pub start_ms: u64,
    /// End, in ms.
    pub end_ms: u64,
}

/// Labels who spoke when, on the far end only (architecture rule 5). Whether the labels are kept
/// (at least two substantial clusters) is the pipeline's decision, not the diarizer's.
pub trait Diarizer: Send + Sync {
    /// **Any thread except realtime.** What this diarizer is.
    fn info(&self) -> EngineInfo;

    /// **Worker.** Offline diarization for the final pass. Returns [`EngineError::Cancelled`] once
    /// `cancel` is set.
    fn diarize(&self, audio: &[f32], cancel: &CancelToken)
    -> Result<Vec<SpeakerTurn>, EngineError>;

    /// **Worker.** Live labels. Turns arrive on `turns` (a callback thread) and are provisional,
    /// like partials. An offline-only diarizer returns [`EngineError::Unsupported`].
    fn open_stream(
        &self,
        turns: EventSink<SpeakerTurn>,
    ) -> Result<Box<dyn EngineStream>, EngineError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_text_skips_empty_segments() {
        let seg = |text: &str| TimedText {
            start_ms: 0,
            end_ms: 0,
            text: text.into(),
        };
        let t = Transcript {
            segments: vec![seg(" hello "), seg(""), seg("  "), seg("world")],
        };
        assert_eq!(t.text(), "hello world");
    }
}
