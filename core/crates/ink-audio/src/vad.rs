//! Voice activity: trim the dead air at the ends of a take, never the pauses inside it.
//!
//! # Trim, don't gate
//!
//! An earlier implementation butt-joined every detected speech segment and threw the pauses away.
//! That hurt twice: pauses longer than the hangover were cut out of the middle of sentences,
//! splicing unrelated phonemes together, and the spliced audio left the long-audio windowing (see
//! [`window`](crate::window)) no quiet place to cut, so its seams landed mid-word. Engines handle
//! pauses fine. [`trim_ends`] keeps everything from the start of the first speech segment to the
//! end of the last, pauses included, plus [`VadConfig::edge_pad`] on both sides (Silero's onset
//! lags a soft attack, and the last word's decay matters).
//!
//! # One notion of speech
//!
//! The VAD is also what the gain stages learn level from (see [`gain`](crate::gain) and
//! [`agc`](crate::agc)): only audio it calls speech moves a gain. They use the same thresholds,
//! hysteresis and hangover as the trim, through [`speech_segments`] and
//! [`SpeechSegmenter::counts_as_speech`]; there is no second definition.
//!
//! # The split
//!
//! The logic here (thresholds, hysteresis, hangover, the minimum segment, the trim) is pure Rust
//! and runs against any [`SpeechProbability`]. The model behind it, Silero VAD, is bound where
//! the other ONNX engines are, in `ink-engines`; tests use scripted sources.
//!
//! **For that binding:**
//!
//! - sherpa-onnx's C API (1.13) exposes finished segments and a speech-detected flag, not the
//!   per-window probability this trait asks for. Either run the Silero model so its probability
//!   is available, or return the flag as 1.0 or 0.0 per window with sherpa's own minimum
//!   durations set to zero (so they are not applied twice). The second loses the hysteresis
//!   between the two thresholds; the first keeps it.
//! - **The VAD hears the provisional copy, never the raw take.** A VAD cannot hear −75 dBFS speech
//!   either, so the gain stages hand it audio already lifted by a provisional gain
//!   ([`normalise_speech`](crate::gain::normalise_speech) and the AGC do this). Feed Silero what
//!   they feed it; do not run it on the raw capture and pass the verdict in.
//! - Check Silero's verdicts on clicks and keyboard noise, lifted as the provisional copy lifts
//!   them, with the 64 ms minimum segment ([`VadConfig::min_speech_windows`]). The tests here use
//!   scripted probabilities and cannot tell whether the real model scores a click as speech.

use std::ops::Range;

use ink_core::EngineError;

/// Samples per VAD window: 32 ms at 16 kHz, Silero's window.
pub const VAD_WINDOW: usize = 512;

/// A per-window speech probability, Silero's contract.
///
/// **Worker.** One instance serves one buffer or stream at a time; `&mut self` because the model
/// is recurrent (Silero carries state and 64 samples of context from window to window).
///
/// The contract an implementation keeps:
/// - Input is 16 kHz mono, after the gain stage, in consecutive windows of exactly
///   [`VAD_WINDOW`] samples. The last window of a buffer arrives zero-padded.
/// - [`reset`](Self::reset) comes before each new buffer or stream and clears all state.
/// - [`probability`](Self::probability) returns the probability that the window holds speech,
///   in `0.0..=1.0`. Anything else (NaN included) is reported by the caller as an engine failure,
///   never guessed at.
/// - Errors are [`EngineError`]s: `ModelMissing` when the weights are not installed, `Failed`
///   otherwise. The caller decides what a dictation does without VAD; the VAD never pretends.
pub trait SpeechProbability: Send {
    /// Clears the recurrent state for a new buffer or stream.
    fn reset(&mut self);

    /// The probability that `window` holds speech.
    fn probability(&mut self, window: &[f32; VAD_WINDOW]) -> Result<f32, EngineError>;
}

/// The segmenter's thresholds and durations, in windows and samples at 16 kHz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VadConfig {
    /// Speech starts at a window at or above this probability (0.5, the earlier default).
    pub threshold: f32,
    /// Once started, speech continues until windows fall below this (0.35: Silero's own
    /// hysteresis of 0.15). Between the two, it simply continues.
    pub neg_threshold: f32,
    /// Segments shorter than this many windows are dropped (2 = 64 ms). This rejects a
    /// single-window spike (a key click) and nothing that could be a word: a 250 ms minimum
    /// would drop a short first word ("so") that a pause separates from the rest.
    pub min_speech_windows: usize,
    /// Windows below `neg_threshold` it takes to end speech (8 = 256 ms). A shorter dip does not
    /// split it. The segment then ends where the silence began, not where the hangover ran out.
    pub hangover_windows: usize,
    /// Samples kept before the first speech and after the last (4000 = 250 ms).
    pub edge_pad: usize,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            neg_threshold: 0.35,
            min_speech_windows: 2,
            hangover_windows: 8,
            edge_pad: 4_000,
        }
    }
}

/// A stretch of speech, in windows: `start` is the first speech window, `end` the first window
/// after it (where the silence began).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    /// First window of the speech.
    pub start: usize,
    /// First window after it.
    pub end: usize,
}

impl Segment {
    /// The segment in samples.
    pub fn samples(&self) -> Range<usize> {
        self.start * VAD_WINDOW..self.end * VAD_WINDOW
    }
}

/// Turns per-window probabilities into speech segments, as they close.
///
/// **Worker or pump.** No allocation. Feed every window's probability in order.
#[derive(Clone, Debug)]
pub struct SpeechSegmenter {
    cfg: VadConfig,
    index: usize,
    state: State,
}

#[derive(Clone, Copy, Debug)]
enum State {
    Silence,
    Speech {
        start: usize,
        /// The first window of the current run below `neg_threshold`, if one has begun.
        silence_since: Option<usize>,
    },
}

impl SpeechSegmenter {
    /// A segmenter at the start of a stream.
    pub fn new(cfg: VadConfig) -> Self {
        Self {
            cfg,
            index: 0,
            state: State::Silence,
        }
    }

    /// Takes the next window's probability. Returns a segment when this window closed one.
    pub fn push(&mut self, p: f32) -> Option<Segment> {
        let i = self.index;
        self.index += 1;
        match &mut self.state {
            State::Silence => {
                if p >= self.cfg.threshold {
                    self.state = State::Speech {
                        start: i,
                        silence_since: None,
                    };
                }
                None
            }
            State::Speech {
                start,
                silence_since,
            } => {
                if p >= self.cfg.threshold {
                    *silence_since = None;
                    None
                } else if p < self.cfg.neg_threshold {
                    let since = *silence_since.get_or_insert(i);
                    if i + 1 - since >= self.cfg.hangover_windows {
                        let start = *start;
                        self.state = State::Silence;
                        self.close(start, since)
                    } else {
                        None
                    }
                } else {
                    // Between the thresholds: speech goes on, and a silence already begun keeps
                    // counting (Silero's own rule).
                    None
                }
            }
        }
    }

    /// Ends the stream: an open segment closes where its silence began, or at the last window.
    pub fn finish(&mut self) -> Option<Segment> {
        let state = std::mem::replace(&mut self.state, State::Silence);
        match state {
            State::Silence => None,
            State::Speech {
                start,
                silence_since,
            } => self.close(start, silence_since.unwrap_or(self.index)),
        }
    }

    fn close(&self, start: usize, end: usize) -> Option<Segment> {
        (end - start >= self.cfg.min_speech_windows).then_some(Segment { start, end })
    }

    /// Whether the window whose probability `p` was just pushed counts as speech, live: speech has
    /// started and `p` is at or above the lower threshold. A window in the hangover (below the
    /// lower threshold, speech not yet ended) does not count; nor does anything before speech
    /// starts. The minimum segment length, which only a closed segment can show, is not applied:
    /// this is for decisions that cannot wait for the segment to close (the AGC's).
    pub fn counts_as_speech(&self, p: f32) -> bool {
        matches!(self.state, State::Speech { .. }) && p >= self.cfg.neg_threshold
    }
}

/// The speech segments in `audio` (16 kHz mono), in order, as [`SpeechSegmenter`] closes them.
///
/// **Worker.** Resets `source` first. Allocates the list. An error from the source, or a
/// probability outside `0.0..=1.0`, is returned as an [`EngineError`], never replaced by a guess.
pub fn speech_segments(
    audio: &[f32],
    source: &mut dyn SpeechProbability,
    cfg: &VadConfig,
) -> Result<Vec<Segment>, EngineError> {
    let mut segments = Vec::new();
    scan(audio, source, cfg, |s| segments.push(s))?;
    Ok(segments)
}

/// The range [`trim_ends`] keeps for `segments` of a buffer `len` samples long: from the first
/// segment's start to the last one's end, padded and clamped. `None` for no segments.
pub fn keep_range(segments: &[Segment], len: usize, cfg: &VadConfig) -> Option<Range<usize>> {
    let (first, last) = (segments.first()?, segments.last()?);
    let lo = (first.start * VAD_WINDOW).saturating_sub(cfg.edge_pad);
    let hi = (last.end * VAD_WINDOW)
        .saturating_add(cfg.edge_pad)
        .min(len);
    Some(lo..hi)
}

/// Runs `source` over `audio` window by window and hands each closed segment to `on_segment`.
/// Allocation-free (the last window is padded on the stack).
fn scan(
    audio: &[f32],
    source: &mut dyn SpeechProbability,
    cfg: &VadConfig,
    mut on_segment: impl FnMut(Segment),
) -> Result<(), EngineError> {
    source.reset();
    let mut segmenter = SpeechSegmenter::new(*cfg);
    let mut padded = [0.0f32; VAD_WINDOW];
    for chunk in audio.chunks(VAD_WINDOW) {
        let window: &[f32; VAD_WINDOW] = match <&[f32; VAD_WINDOW]>::try_from(chunk) {
            Ok(whole) => whole,
            Err(_) => {
                padded[..chunk.len()].copy_from_slice(chunk);
                padded[chunk.len()..].fill(0.0);
                &padded
            }
        };
        let p = checked(source.probability(window)?)?;
        if let Some(s) = segmenter.push(p) {
            on_segment(s);
        }
    }
    if let Some(s) = segmenter.finish() {
        on_segment(s);
    }
    Ok(())
}

/// A probability, or the engine failure that one outside `0.0..=1.0` (NaN included) is.
pub(crate) fn checked(p: f32) -> Result<f32, EngineError> {
    if (0.0..=1.0).contains(&p) {
        Ok(p)
    } else {
        Err(EngineError::Failed(format!(
            "VAD returned a speech probability of {p}, outside 0..=1"
        )))
    }
}

/// The range of `audio` (16 kHz mono) to keep: from the first speech to the last, pauses
/// included, padded by [`VadConfig::edge_pad`] and clamped to the buffer. `None` when there is no
/// speech at all.
///
/// `None` is a verdict, not a failure: discard the buffer, never pass it on untrimmed.
///
/// **Worker.** Resets `source` first. No allocation (the last window is padded on the stack). An
/// error from the source, or a probability outside `0.0..=1.0`, is returned as an
/// [`EngineError`], never replaced by a guess.
pub fn trim_ends(
    audio: &[f32],
    source: &mut dyn SpeechProbability,
    cfg: &VadConfig,
) -> Result<Option<Range<usize>>, EngineError> {
    let mut first: Option<Segment> = None;
    let mut last: Option<Segment> = None;
    scan(audio, source, cfg, |s| {
        first.get_or_insert(s);
        last = Some(s);
    })?;
    Ok(match (first, last) {
        (Some(a), Some(b)) => keep_range(&[a, b], audio.len(), cfg),
        _ => None,
    })
}
