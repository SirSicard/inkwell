//! One side of a live meeting.
//!
//! ```text
//! canonical block ─► Agc (with the VAD) ─► live engine stream ─► finals ─► placed, checked ─► revision 1
//!                        └─ VAD verdicts, observed ─┘                         (partials: events only)
//! ```
//!
//! # The AGC and its VAD
//!
//! Whenever a VAD is installed the side's AGC is [`Agc::with_vad`]: it learns level only from what
//! the VAD calls speech. Its VAD is wrapped so every probability it produces is also recorded here
//! ([`Observed`]); the chain runs the same [`SpeechSegmenter`] over them, so it knows, window by
//! window, where the AGC's VAD heard speech, at no extra model cost.
//!
//! The AGC is polled after every block. When its VAD has failed ([`Agc::vad_error`]), the old AGC
//! is flushed into the stream, a [`Agc::without_vad`] takes over, and the side goes on: nothing is
//! lost, and the shell hears that voice detection is unavailable.
//!
//! # Time
//!
//! The AGC's output is its input [`Agc::LATENCY`] (20 ms) late, so an engine that says "at 1.00 s
//! of my stream" means 0.98 s of the side's audio: every final is moved back by the latency before
//! it is placed on the [`Timeline`]. The replacement AGC after a failure starts with another
//! `LATENCY` of silence; that is dropped, so the relation holds for the whole stream.
//!
//! # Finals and speech
//!
//! A live final is saved only when the VAD heard speech somewhere under it. It waits until the VAD
//! has judged its whole span (a window or so), then is saved or reported as
//! [`MeetingWarning::FinalWithoutSpeech`]. Without a VAD, or past the point where it failed, finals
//! cannot be checked and are saved as they come, while the shell shows voice detection as
//! unavailable. The live engine itself hears the whole stream: a streaming recogniser needs
//! continuous audio, and its words are provisional until the final pass, which gives its engine
//! speech alone.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use ink_audio::vad::VAD_WINDOW;
use ink_audio::{Agc, SpeechProbability, SpeechSegmenter, VadConfig};
use ink_core::{Channel, EngineError, EngineStream, Segment, TimedText};

use super::events::{MeetingEvent, MeetingWarning, Phase};
use super::timeline::{Timeline, ns_to_samples};
use crate::events::{VadUnavailable, VoiceDetection};
use crate::gain_stage::Vad;
use crate::speech::samples_to_ms;

const LATENCY: u64 = Agc::LATENCY as u64;
const WINDOW: u64 = VAD_WINDOW as u64;
const SAMPLES_PER_MS: u64 = 16;

/// Audio a final may still refer to with none waiting: a minute. Anchors and verdicts for it are
/// kept; a final about older audio is placed by extrapolation and saved unchecked.
const KEEP_SAMPLES: u64 = 60 * 16_000;

/// Live finals that may wait for the VAD at once, per side. A final waits a window or so; only an
/// engine reporting times the audio has not reached could fill this. Past it, the oldest is saved
/// unchecked (never lost) and [`MeetingWarning::LiveFinalsBacklog`] says so, once per side.
pub const MAX_PENDING_FINALS: usize = 256;
const KEEP_WINDOWS: usize = KEEP_SAMPLES as usize / VAD_WINDOW;

/// A VAD whose every probability is also recorded, for the chain to read after each block.
struct Observed {
    inner: Box<dyn SpeechProbability>,
    heard: Arc<Mutex<Vec<f32>>>,
}

impl SpeechProbability for Observed {
    fn reset(&mut self) {
        self.inner.reset();
    }

    fn probability(&mut self, window: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
        let p = self.inner.probability(window)?;
        // Out of range is the AGC's error to report; it asks nothing more after it.
        if (0.0..=1.0).contains(&p) {
            self.heard
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(p);
        }
        Ok(p)
    }
}

/// Where the AGC's VAD heard speech: one verdict per VAD window of the side's input.
struct Heard {
    probabilities: Arc<Mutex<Vec<f32>>>,
    segmenter: SpeechSegmenter,
    /// `counts_as_speech` per window, from window `base` on.
    verdicts: VecDeque<bool>,
    base: u64,
    /// Windows judged so far.
    windows: u64,
    /// Whether verdicts are still coming (false once the VAD failed).
    live: bool,
}

impl Heard {
    fn drain(&mut self) {
        let fresh = std::mem::take(
            &mut *self
                .probabilities
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for p in fresh {
            self.segmenter.push(p);
            self.verdicts.push_back(self.segmenter.counts_as_speech(p));
            self.windows += 1;
        }
    }

    /// Whether the VAD heard speech in input samples `from..to`: `None` while it has not judged
    /// all of them yet and still can.
    fn speech_in(&self, from: u64, to: u64) -> Option<bool> {
        let first = from / WINDOW;
        let last = to.max(from + 1).div_ceil(WINDOW) - 1;
        // `verdicts` holds windows `base..windows`.
        let judged_to = self.windows;
        let heard = (first.max(self.base)..=last)
            .take_while(|&w| w < judged_to)
            .any(|w| self.verdicts[(w - self.base) as usize]);
        if heard {
            return Some(true);
        }
        if first < self.base {
            // Its start is older than the verdicts kept: it cannot be checked.
            return Some(true);
        }
        if last < judged_to {
            return Some(false);
        }
        // Part of it is not judged yet: wait while verdicts still come, else it cannot be checked.
        if self.live { None } else { Some(true) }
    }

    /// Forgets verdicts before window `window`, keeping at least the last [`KEEP_WINDOWS`].
    fn forget_before(&mut self, window: u64) {
        let keep_from = window.min(self.windows.saturating_sub(KEEP_WINDOWS as u64));
        while self.base < keep_from && !self.verdicts.is_empty() {
            self.verdicts.pop_front();
            self.base += 1;
        }
    }
}

/// A live final waiting to be checked, in the side's input samples and meeting ms.
struct Pending {
    from: u64,
    to: u64,
    start_ms: u64,
    end_ms: u64,
    text: String,
}

/// What became of a final.
pub(crate) enum Settled {
    /// Save it.
    Keep(Segment),
    /// The VAD heard no speech under it.
    NoSpeech {
        /// Which side.
        channel: Channel,
        /// Start, ms into the meeting.
        start_ms: u64,
        /// End.
        end_ms: u64,
    },
}

/// One side of a live meeting. See the module docs. **Worker**, every method.
pub(crate) struct LiveChannel {
    channel: Channel,
    agc: Agc,
    heard: Option<Heard>,
    stream: Option<Box<dyn EngineStream>>,
    timeline: Timeline,
    /// Host time of the meeting's start.
    t0_ns: u64,
    /// Input samples so far.
    input: u64,
    /// Leading output samples of a replacement AGC still to drop.
    skip: usize,
    pending: VecDeque<Pending>,
    /// Whether the pending queue has overflowed (reported once).
    backlogged: bool,
    scratch: Vec<f32>,
}

impl LiveChannel {
    /// A side whose meeting started at host time `t0_ns`, levelled with `vad` when there is one,
    /// feeding `stream` when a live engine is installed. Reports the side's voice detection.
    pub(crate) fn new(
        channel: Channel,
        vad: Vad,
        cfg: VadConfig,
        stream: Option<Box<dyn EngineStream>>,
        t0_ns: u64,
        emit: &dyn Fn(MeetingEvent),
    ) -> Self {
        let (agc, heard, state) = match vad {
            Vad::Installed(inner) => {
                let probabilities = Arc::new(Mutex::new(Vec::new()));
                let observed = Observed {
                    inner,
                    heard: probabilities.clone(),
                };
                let heard = Heard {
                    probabilities,
                    segmenter: SpeechSegmenter::new(cfg),
                    verdicts: VecDeque::new(),
                    base: 0,
                    windows: 0,
                    live: true,
                };
                (
                    Agc::with_vad(Box::new(observed), cfg),
                    Some(heard),
                    VoiceDetection::Available,
                )
            }
            Vad::Unavailable(why) => (Agc::without_vad(), None, VoiceDetection::Unavailable(why)),
        };
        emit(MeetingEvent::VoiceDetection { channel, state });
        Self {
            channel,
            agc,
            heard,
            stream,
            timeline: Timeline::default(),
            t0_ns,
            input: 0,
            skip: 0,
            pending: VecDeque::new(),
            backlogged: false,
            scratch: Vec::new(),
        }
    }

    /// Takes the side's next canonical block, whose first sample was captured at `host_time_ns`.
    pub(crate) fn push(&mut self, samples: &[f32], host_time_ns: u64, emit: &dyn Fn(MeetingEvent)) {
        let at = ns_to_samples(i128::from(host_time_ns) - i128::from(self.t0_ns));
        self.timeline.observe(self.input, at);
        self.input += samples.len() as u64;
        let mut block = std::mem::take(&mut self.scratch);
        block.clear();
        block.extend_from_slice(samples);
        self.agc.process(&mut block);
        if let Some(heard) = &mut self.heard {
            heard.drain();
        }
        self.feed(&block, emit);
        self.scratch = block;
        self.check_vad(emit);
    }

    /// Switches to the fallback if the AGC's VAD failed.
    fn check_vad(&mut self, emit: &dyn Fn(MeetingEvent)) {
        let Some(error) = self.agc.vad_error().cloned() else {
            return;
        };
        let mut tail = Vec::with_capacity(Agc::LATENCY);
        self.agc.flush(&mut tail);
        if let Some(heard) = &mut self.heard {
            heard.drain();
            heard.live = false;
        }
        self.feed(&tail, emit);
        self.agc = Agc::without_vad();
        self.skip = Agc::LATENCY;
        log::warn!(
            "meeting: the {:?} VAD failed; the AGC goes on without it",
            self.channel
        );
        emit(MeetingEvent::Warning(MeetingWarning::VadFailed {
            channel: self.channel,
            phase: Phase::Live,
            error,
        }));
        emit(MeetingEvent::VoiceDetection {
            channel: self.channel,
            state: VoiceDetection::Unavailable(VadUnavailable::Failed),
        });
    }

    /// Hands AGC output to the live engine, less a replacement AGC's leading silence.
    fn feed(&mut self, out: &[f32], emit: &dyn Fn(MeetingEvent)) {
        let drop = self.skip.min(out.len());
        self.skip -= drop;
        let out = &out[drop..];
        if out.is_empty() {
            return;
        }
        if let Some(stream) = &mut self.stream
            && let Err(error) = stream.push(out)
        {
            self.stream = None;
            log::warn!("meeting: the {:?} live engine failed", self.channel);
            emit(MeetingEvent::Warning(MeetingWarning::LiveEngineFailed {
                channel: self.channel,
                error,
            }));
        }
    }

    /// A final from the live engine, positioned in its stream. With [`MAX_PENDING_FINALS`] already
    /// waiting, the oldest is returned to be saved unchecked.
    pub(crate) fn final_heard(
        &mut self,
        text: TimedText,
        emit: &dyn Fn(MeetingEvent),
    ) -> Option<Settled> {
        if text.text.trim().is_empty() {
            return None;
        }
        let overflow = if self.pending.len() >= MAX_PENDING_FINALS {
            if !self.backlogged {
                self.backlogged = true;
                log::warn!(
                    "meeting: {MAX_PENDING_FINALS} {:?} live finals wait on the VAD; the oldest are saved unchecked",
                    self.channel
                );
                emit(MeetingEvent::Warning(MeetingWarning::LiveFinalsBacklog {
                    channel: self.channel,
                }));
            }
            self.pending.pop_front().map(|p| self.keep(p))
        } else {
            None
        };
        // Stream time is AGC output; the audio it came from is `LATENCY` earlier.
        let from = (text.start_ms * SAMPLES_PER_MS).saturating_sub(LATENCY);
        let to = (text.end_ms.max(text.start_ms) * SAMPLES_PER_MS).saturating_sub(LATENCY);
        // Placed where it was said, never adjusted for the order finals arrive in: the store keeps
        // a record's segments in time order.
        let place = |index: u64| samples_to_ms(self.timeline.position(index).max(0) as u64);
        let start_ms = place(from);
        let end_ms = place(to).max(start_ms);
        self.pending.push_back(Pending {
            from,
            to,
            start_ms,
            end_ms,
            text: text.text,
        });
        overflow
    }

    fn keep(&self, p: Pending) -> Settled {
        Settled::Keep(Segment {
            channel: self.channel,
            start_ms: p.start_ms,
            end_ms: p.end_ms,
            text: p.text,
            speaker: None,
        })
    }

    /// Finals whose span the VAD has judged, in order. `finishing` settles every one.
    pub(crate) fn settle(&mut self, finishing: bool) -> Vec<Settled> {
        let mut out = Vec::new();
        while let Some(p) = self.pending.front() {
            let speech = match &self.heard {
                None => Some(true),
                Some(heard) => heard.speech_in(p.from, p.to),
            };
            let speech = match (speech, finishing) {
                (Some(s), _) => s,
                // Everything is judged once the AGC has been flushed; what is still open cannot
                // be checked.
                (None, true) => true,
                (None, false) => break,
            };
            let Some(p) = self.pending.pop_front() else {
                break;
            };
            out.push(if speech {
                self.keep(p)
            } else {
                Settled::NoSpeech {
                    channel: self.channel,
                    start_ms: p.start_ms,
                    end_ms: p.end_ms,
                }
            });
        }
        // A final can arrive seconds after its audio, so a minute of anchors is kept even with
        // none waiting (the verdicts keep the same).
        let oldest = self
            .pending
            .front()
            .map_or(self.input, |p| p.from)
            .min(self.input.saturating_sub(KEEP_SAMPLES));
        self.timeline.forget_before(oldest);
        if let Some(heard) = &mut self.heard {
            heard.forget_before(oldest / WINDOW);
        }
        out
    }

    /// Ends the side's live phase: flushes the AGC into the stream (its VAD judges the last partial
    /// window) and closes the stream, whose trailing finals the chain then collects.
    pub(crate) fn finish(&mut self, emit: &dyn Fn(MeetingEvent)) {
        let mut tail = Vec::with_capacity(Agc::LATENCY);
        self.agc.flush(&mut tail);
        if let Some(heard) = &mut self.heard {
            heard.drain();
            heard.live = false;
        }
        self.feed(&tail, emit);
        if let Some(stream) = self.stream.take()
            && let Err(error) = stream.finish()
        {
            log::warn!(
                "meeting: the {:?} live engine failed to finish",
                self.channel
            );
            emit(MeetingEvent::Warning(MeetingWarning::LiveEngineFailed {
                channel: self.channel,
                error,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(verdicts: &[bool], live: bool) -> Heard {
        Heard {
            probabilities: Arc::default(),
            segmenter: SpeechSegmenter::new(VadConfig::default()),
            verdicts: verdicts.iter().copied().collect(),
            base: 0,
            windows: verdicts.len() as u64,
            live,
        }
    }

    #[test]
    fn speech_under_a_span_is_found_by_window() {
        let h = heard(&[false, false, true, false], true);
        assert_eq!(h.speech_in(0, 1_024), Some(false), "windows 0 and 1");
        assert_eq!(h.speech_in(1_000, 1_100), Some(true), "window 1 to 2");
        assert_eq!(h.speech_in(1_600, 2_048), Some(false), "window 3");
        assert_eq!(
            h.speech_in(1_600, 2_100),
            None,
            "window 4 is not judged yet"
        );
        let dead = heard(&[false, false], false);
        assert_eq!(dead.speech_in(900, 1_500), Some(true), "unjudgeable: kept");
    }

    #[test]
    fn forgotten_verdicts_cannot_refuse_a_final() {
        let mut h = heard(&vec![false; KEEP_WINDOWS + 10], true);
        h.forget_before(u64::MAX);
        assert_eq!(h.base, 10);
        assert_eq!(h.speech_in(0, 512), Some(true));
        assert_eq!(h.speech_in(20 * 512, 21 * 512), Some(false));
    }
}
