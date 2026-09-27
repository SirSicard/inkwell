//! What of a long recording an offline engine may hear: a meeting channel's final pass, or an
//! imported file.
//!
//! ```text
//! 16 kHz mono ─► Windower (≤ 60 s, cut at a quiet frame) ─► per window: level, VAD, gain ─► RegionBuilder ─► Region
//! ```
//!
//! # Only speech reaches the engine
//!
//! The final-pass engine (Qwen3-ASR) does not only return nothing on silence: given seconds of
//! digital zeros it often writes a plausible sentence. And it returns a segment for every window
//! it is given, so a segment is no evidence of speech. So the engine is never given a window: it is
//! given **speech regions**, the stretches the VAD calls speech, each its own call:
//!
//! - VAD segments closer than [`RegionConfig::max_pause`] (2 s) join one region, pauses kept:
//!   splicing speech together hurts recognition (see `ink_audio::vad`), and a short pause between
//!   sentences is not the silence the engine invents text on.
//! - A region is padded by [`VadConfig::edge_pad`] (250 ms) on each side, never into the region
//!   before it.
//! - A region longer than [`RegionConfig::max_region`] (60 s) is cut at its quietest 20 ms in
//!   the last [`RegionConfig::cut_search`] (3 s), so memory stays bounded and each call stays
//!   within what the engine was measured on.
//!
//! # The gain, per window, from speech
//!
//! Each window is levelled as `ink_audio`'s [`normalise_speech`](ink_audio::normalise_speech)
//! levels a take: a provisional gain from the window's robust peak, the VAD over the window lifted
//! by it, then one gain from the robust peak of the speech frames alone. That function is not
//! called directly because it returns only the range from the first speech to the last, and the
//! regions need every segment: the same public steps are composed here instead. A window without
//! speech gets no gain and gives no region. One gain per window of up to 60 s is the meeting AGC's
//! slow gain, decided with the whole window in view.
//!
//! # Without a VAD
//!
//! With no VAD installed, or after the VAD fails (reported once by [`SpeechPass::vad_error`],
//! polled like the AGC's), windows are levelled by
//! [`ink_audio::normalise_without_vad`] and every window that is neither
//! silence nor stationary becomes speech. That can hand the engine non-speech; the shell shows
//! voice detection as unavailable for as long as it lasts, as it does for dictation.
//!
//! # Audible time: a check on the VAD
//!
//! A pass also counts the time its audio stands above [`AUDIBLE_FLOOR_DBFS`], measured on the
//! audio as it arrived (before any gain, so it does not depend on the VAD either).
//! [`little_speech_heard`] compares it with the speech the VAD found: a VAD that hears under a tenth
//! of what is clearly audible is more likely deaf (a model that is wrong, audio at the wrong rate)
//! than listening to a quiet room. That is a diagnostic only: nothing is sent to an engine or
//! dropped because of it. It cannot see a deaf VAD on quiet audio (a −60 dBFS talker is under the
//! floor), which the gain stage's own tests cover.
//!
//! # Judged on one stream, heard on another (echo)
//!
//! With an echo path, a meeting's mic pass is [`SpeechPass::paired`]: the VAD judges AEC3's
//! **full** output, which says whether anyone on the near end spoke, and the regions carry the
//! **linear** output, which keeps the user's words intact in double talk (see
//! `ink_echo::gate`). The VAD hears the full output lifted by the provisional gain of the mic as
//! captured, echo included, so what the suppressor removed stays as far under speech level as it
//! was taken down, instead of being lifted back up (the full output's own gain, or even the linear
//! output's, would lift a quiet stretch of residual echo to speech level). The gain the engine
//! hears comes from the linear output's speech, and the audible time is counted on the full output
//! (echo the suppressor removed is not "audible audio the VAD missed"). A paired pass also keeps
//! the VAD's verdicts (`HeardSpeech`): the evidence the duplicate-line check needs.
//!
//! # Threads and memory
//!
//! **Worker.** A pass holds about one window and one region: at most two minutes of audio, never
//! the recording (architecture rule 3). A paired pass also keeps one float per 32 ms VAD window
//! (under half a megabyte for an hour).

use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use ink_audio::gain::{
    GainOutcome, LEVEL_FRAME, NOISE_FLOOR, apply_gain, from_dbfs, gain_for, levels,
    provisional_gain, rms, speech_levels,
};
use ink_audio::vad::VAD_WINDOW;
use ink_audio::window::quietest_cut;
use ink_audio::{
    SpeechProbability, VadConfig, WindowConfig, WindowError, Windower, normalise_without_vad,
    speech_segments,
};
use ink_core::{CANONICAL_RATE, EngineError};
use ink_echo::{GateConfig, NearSpeech};

use crate::events::{VadUnavailable, VoiceDetection};
use crate::gain_stage::Vad;

/// Samples per millisecond at the canonical rate.
const SAMPLES_PER_MS: u64 = CANONICAL_RATE as u64 / 1_000;

/// A sample position on a 16 kHz timeline, in ms.
pub fn samples_to_ms(samples: u64) -> u64 {
    samples / SAMPLES_PER_MS
}

/// A 20 ms frame whose RMS is above this is "audible": 10 dB over the room floor of a real
/// reading on a built-in mic (−50.6 dBFS, the gate's echo measurements), the same margin those
/// measurements used to call a frame talking. On the AMI meeting fixture, 8.3 s of the mic's 30 s
/// and 14.6 s of the far end's are above it, less than the speech a VAD finds there, so an
/// ordinary meeting does not trip the check.
pub const AUDIBLE_FLOOR_DBFS: f32 = -40.0;

/// Audible time under which [`little_speech_heard`] says nothing: too little to judge a VAD on.
pub const MIN_AUDIBLE_TO_JUDGE_MS: u64 = 30_000;

/// Whether the VAD found suspiciously little speech: at least [`MIN_AUDIBLE_TO_JUDGE_MS`] of
/// audible audio, and speech under a tenth of it. A diagnostic, never a decision.
pub fn little_speech_heard(audible_ms: u64, speech_ms: u64) -> bool {
    audible_ms >= MIN_AUDIBLE_TO_JUDGE_MS && speech_ms.saturating_mul(10) < audible_ms
}

/// Makes a fresh VAD instance. Each use (a live AGC per channel, a final pass, an import) needs its
/// own, because the model is recurrent.
pub type VadFactory =
    Arc<dyn Fn() -> Result<Box<dyn SpeechProbability>, EngineError> + Send + Sync>;

/// Where the chains get voice detection.
#[derive(Clone)]
pub enum VadSource {
    /// A VAD is installed.
    Installed(VadFactory),
    /// None is: everything is levelled by the fallbacks, and the shell says so.
    Unavailable(VadUnavailable),
}

impl std::fmt::Debug for VadSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installed(_) => f.write_str("VadSource::Installed"),
            Self::Unavailable(why) => write!(f, "VadSource::Unavailable({why:?})"),
        }
    }
}

impl VadSource {
    /// **Worker.** A VAD for one use, or why there is none. A factory that fails is reported as
    /// the error, with the fallback in place.
    pub fn open(&self) -> (Vad, Option<EngineError>) {
        match self {
            Self::Installed(make) => match make() {
                Ok(vad) => (Vad::Installed(vad), None),
                Err(error) => (Vad::Unavailable(VadUnavailable::LoadFailed), Some(error)),
            },
            Self::Unavailable(why) => (Vad::Unavailable(*why), None),
        }
    }
}

/// How regions are formed, in samples at 16 kHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionConfig {
    /// VAD segments at most this far apart join one region (32 000 = 2 s).
    pub max_pause: usize,
    /// The longest region handed to the engine (960 000 = 60 s).
    pub max_region: usize,
    /// How far back from `max_region` a long region may be cut (48 000 = 3 s).
    pub cut_search: usize,
    /// The level windows (60 s, cut at the quietest frame in their last 3 s, no overlap: regions
    /// carry across window ends, so no audio is heard twice).
    pub window: WindowConfig,
}

impl Default for RegionConfig {
    fn default() -> Self {
        Self {
            max_pause: 2 * 16_000,
            max_region: 60 * 16_000,
            cut_search: 3 * 16_000,
            window: WindowConfig {
                max_len: 60 * 16_000,
                overlap: 0,
                search: 3 * 16_000,
            },
        }
    }
}

/// A stretch of speech for the engine: levelled, padded, at most [`RegionConfig::max_region`].
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// Its first sample on the pass's timeline.
    pub start: u64,
    /// The levelled audio, 16 kHz mono.
    pub audio: Vec<f32>,
}

impl Region {
    /// One past its last sample.
    pub fn end(&self) -> u64 {
        self.start + self.audio.len() as u64
    }

    /// Where it starts, in ms.
    pub fn start_ms(&self) -> u64 {
        samples_to_ms(self.start)
    }

    /// Where it ends, in ms.
    pub fn end_ms(&self) -> u64 {
        samples_to_ms(self.end())
    }
}

/// Cuts a recording into levelled speech regions. See the module docs.
pub struct SpeechPass {
    windower: Windower,
    vad: Vad,
    cfg: VadConfig,
    /// The VAD's error, once it failed; from then on the fallback levels every window.
    vad_error: Option<EngineError>,
    regions: RegionBuilder,
    /// Samples in frames above [`AUDIBLE_FLOOR_DBFS`].
    audible: u64,
    /// Samples handed out as regions.
    speech: u64,
    /// A paired pass: the stream the regions carry, and the one the VAD's lift is set by, from
    /// sample `heard_start` on (the windower holds the judged one).
    heard: Option<(Vec<f32>, Vec<f32>)>,
    heard_start: u64,
    /// A paired pass: every probability its VAD gives, taken after each window.
    probabilities: Option<Arc<Mutex<Vec<f32>>>>,
    evidence: HeardSpeech,
}

/// A VAD whose every probability is also recorded.
struct Recording {
    inner: Box<dyn SpeechProbability>,
    heard: Arc<Mutex<Vec<f32>>>,
}

impl SpeechProbability for Recording {
    fn reset(&mut self) {
        self.inner.reset();
    }

    fn probability(&mut self, window: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
        let p = self.inner.probability(window)?;
        self.heard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(p);
        Ok(p)
    }
}

impl SpeechPass {
    /// A pass over a recording that starts at sample 0. **Allocates.**
    pub fn new(vad: Vad, cfg: VadConfig, regions: RegionConfig) -> Result<Self, WindowError> {
        Ok(Self {
            windower: Windower::new(regions.window)?,
            vad,
            cfg,
            vad_error: None,
            regions: RegionBuilder::new(regions, cfg.edge_pad),
            audible: 0,
            speech: 0,
            heard: None,
            heard_start: 0,
            probabilities: None,
            evidence: HeardSpeech::default(),
        })
    }

    /// A pass that judges one stream and cuts its regions from another of the same length and
    /// timeline ([`push_paired`](Self::push_paired)), keeping the VAD's verdicts as evidence
    /// (`HeardSpeech`). See the module docs. **Allocates.**
    pub fn paired(vad: Vad, cfg: VadConfig, regions: RegionConfig) -> Result<Self, WindowError> {
        let probabilities = Arc::new(Mutex::new(Vec::new()));
        let vad = match vad {
            Vad::Installed(inner) => Vad::Installed(Box::new(Recording {
                inner,
                heard: probabilities.clone(),
            })),
            other => other,
        };
        let mut pass = Self::new(vad, cfg, regions)?;
        pass.heard = Some((Vec::new(), Vec::new()));
        pass.probabilities = Some(probabilities);
        Ok(pass)
    }

    /// Appends one stretch of a paired pass's streams: `judged` goes to the VAD, lifted by the
    /// provisional gain of `level`, and `heard` into the regions. All three must be the same
    /// length. Regions that closed are appended to `out`.
    ///
    /// # Panics
    ///
    /// On a pass made by [`new`](Self::new), or streams of different lengths: both are bugs in
    /// the caller, not conditions of the audio.
    pub fn push_paired(
        &mut self,
        judged: &[f32],
        heard: &[f32],
        level: &[f32],
        out: &mut Vec<Region>,
    ) -> Result<(), WindowError> {
        assert!(
            judged.len() == heard.len() && heard.len() == level.len(),
            "paired streams move together"
        );
        let (h, l) = self
            .heard
            .as_mut()
            .expect("push_paired needs a paired pass");
        h.extend_from_slice(heard);
        l.extend_from_slice(level);
        self.push(judged, out)
    }

    /// The VAD's verdicts so far (a paired pass keeps them; any other has none).
    #[cfg(test)]
    pub(crate) fn evidence(&self) -> &HeardSpeech {
        &self.evidence
    }

    /// The VAD's verdicts, taking them.
    pub(crate) fn take_evidence(&mut self) -> HeardSpeech {
        std::mem::take(&mut self.evidence)
    }

    /// Time the audio so far stood above [`AUDIBLE_FLOOR_DBFS`], ms.
    pub fn audible_ms(&self) -> u64 {
        samples_to_ms(self.audible)
    }

    /// Time handed out as speech regions so far, ms.
    pub fn speech_ms(&self) -> u64 {
        samples_to_ms(self.speech)
    }

    /// Whether windows are being judged by a VAD.
    pub fn detection(&self) -> VoiceDetection {
        match &self.vad {
            Vad::Installed(_) => VoiceDetection::Available,
            Vad::Unavailable(why) => VoiceDetection::Unavailable(*why),
        }
    }

    /// The error the VAD failed with, if it did. The pass has switched to the fallback; the caller
    /// reports it.
    pub fn vad_error(&self) -> Option<&EngineError> {
        self.vad_error.as_ref()
    }

    /// Appends audio; regions that closed are appended to `out`.
    pub fn push(&mut self, audio: &[f32], out: &mut Vec<Region>) -> Result<(), WindowError> {
        self.windower.push(audio)?;
        let from = out.len();
        self.drain(out);
        self.count(&out[from..]);
        Ok(())
    }

    /// Ends the recording: the rest comes out.
    pub fn finish(&mut self, out: &mut Vec<Region>) {
        self.windower.finish();
        let from = out.len();
        self.drain(out);
        self.regions.finish(out);
        self.count(&out[from..]);
    }

    fn count(&mut self, regions: &[Region]) {
        self.speech += regions.iter().map(|r| r.audio.len() as u64).sum::<u64>();
    }

    fn drain(&mut self, out: &mut Vec<Region>) {
        let floor = from_dbfs(AUDIBLE_FLOOR_DBFS);
        while let Some((window, samples)) = self.windower.next_window() {
            let len = samples.len();
            // The windower keeps windows in order and without gaps, and a paired pass's other
            // stream was appended with the same samples, so this range is always there.
            let (heard, level): (&[f32], &[f32]) = match &self.heard {
                Some((h, l)) => {
                    let from = (window.start - self.heard_start) as usize;
                    (&h[from..from + len], &l[from..from + len])
                }
                None => (samples, samples),
            };
            self.audible += samples
                .chunks(LEVEL_FRAME)
                .filter(|frame| rms(frame) > floor)
                .map(|frame| frame.len() as u64)
                .sum::<u64>();
            if let Some(recorded) = &self.probabilities {
                recorded
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clear();
            }
            let (speech, verdicts) = judge(
                &mut self.vad,
                &mut self.vad_error,
                &self.cfg,
                Streams {
                    judged: samples,
                    heard,
                    level,
                    paired: self.heard.is_some(),
                },
            );
            if let Some(recorded) = &self.probabilities {
                let probabilities = match verdicts {
                    Verdicts::Silence => Some(vec![0.0; len.div_ceil(VAD_WINDOW)]),
                    Verdicts::Vad => Some(std::mem::take(
                        &mut *recorded.lock().unwrap_or_else(PoisonError::into_inner),
                    )),
                    Verdicts::None => None,
                };
                self.evidence.add(window.start, len as u64, probabilities);
            }
            let (gain, spans) = match speech {
                Judged::Speech { gain, spans } => (gain, spans),
                Judged::Nothing => (1.0, Vec::new()),
            };
            let spans: Vec<Range<u64>> = spans
                .into_iter()
                .map(|r| window.start + r.start as u64..window.start + r.end as u64)
                .collect();
            self.regions.push(window.start, heard, gain, &spans, out);
            if let Some((h, l)) = &mut self.heard {
                h.drain(..len);
                l.drain(..len);
                self.heard_start += len as u64;
            }
        }
    }
}

/// Where a window's verdicts came from, for the evidence.
enum Verdicts {
    /// Digital silence (or near it): no speech, without asking the VAD.
    Silence,
    /// The VAD judged every window of it.
    Vad,
    /// The fallback judged it (no VAD, or the VAD failed): no verdicts to keep.
    None,
}

/// Where a paired pass's VAD heard speech: its probability for every 32 ms window, on the pass's
/// timeline. The acoustic evidence of the duplicate-line check ([`NearSpeech`]): whether the near
/// end spoke over a line, with the echo gate's pad and threshold ([`GateConfig`]).
///
/// Windows the fallback judged (no VAD, or after it failed) keep no verdicts: a line over them has
/// no evidence either way. Silence judged by its level alone counts as heard, with no speech.
#[derive(Debug, Default)]
pub(crate) struct HeardSpeech {
    /// Judged stretches in order: (first sample, samples, one probability per VAD window from the
    /// first sample on, or `None` for a stretch without verdicts).
    spans: Vec<(u64, u64, Option<Vec<f32>>)>,
    /// Where judging has reached: the end of the last stretch.
    end: u64,
}

impl HeardSpeech {
    pub(crate) fn add(&mut self, start: u64, len: u64, probabilities: Option<Vec<f32>>) {
        self.spans.push((start, len, probabilities));
        self.end = self.end.max(start + len);
    }

    /// Samples judged so far: the evidence is complete before this point.
    #[cfg(test)]
    pub(crate) fn judged_until(&self) -> u64 {
        self.end
    }
}

impl NearSpeech for HeardSpeech {
    /// Over `start_ms..end_ms`, padded by the gate's 250 ms each side and cut at the end of the
    /// audio (nothing was said after it): `Some(true)` when a VAD window there reached the gate's
    /// threshold, `Some(false)` when every part of it was judged and none did, `None` otherwise.
    fn near_speech(&self, start_ms: u64, end_ms: u64) -> Option<bool> {
        let gate = GateConfig::default();
        let end_ms = end_ms.max(start_ms + 1);
        let lo = start_ms.saturating_sub(gate.pad_ms) * SAMPLES_PER_MS;
        let hi = ((end_ms + gate.pad_ms) * SAMPLES_PER_MS).min(self.end);
        if lo >= hi {
            // Wholly after the audio judged so far.
            return None;
        }
        // The first stretch that ends after `lo`.
        let first = self.spans.partition_point(|(s, n, _)| s + n <= lo);
        let mut covered = lo;
        let mut missing = false;
        for (start, len, verdicts) in &self.spans[first..] {
            if *start >= hi {
                break;
            }
            if *start > covered {
                missing = true;
            }
            match verdicts {
                Some(p) => {
                    let heard = p.iter().enumerate().any(|(k, &p)| {
                        let w0 = start + (k * VAD_WINDOW) as u64;
                        let w1 = (w0 + VAD_WINDOW as u64).min(start + len);
                        w0 < hi && w1 > lo && p >= gate.speech_threshold
                    });
                    if heard {
                        return Some(true);
                    }
                }
                None => missing = true,
            }
            covered = covered.max(start + len);
        }
        if covered < hi || missing {
            None
        } else {
            Some(false)
        }
    }
}

/// What a window holds.
enum Judged {
    /// Speech in `spans` (window-relative samples); level everything by `gain`.
    Speech { gain: f32, spans: Vec<Range<usize>> },
    /// Nothing for the engine.
    Nothing,
}

/// One window of a pass's streams: `judged` is what the VAD hears, lifted by `level`'s
/// provisional gain, and `heard` what the regions carry (all one slice, unless `paired`).
struct Streams<'a> {
    judged: &'a [f32],
    heard: &'a [f32],
    level: &'a [f32],
    paired: bool,
}

/// Judges one window: with the VAD while it works, else with the fallback. The silence check and
/// the speech gain come from what the regions carry; the lift the VAD hears through, from `level`.
fn judge(
    vad: &mut Vad,
    vad_error: &mut Option<EngineError>,
    cfg: &VadConfig,
    streams: Streams<'_>,
) -> (Judged, Verdicts) {
    let Streams {
        judged,
        heard,
        level,
        paired,
    } = streams;
    let before = levels(heard);
    if before.robust_peak < NOISE_FLOOR {
        // Digital silence, or near enough: there is nothing to hear, with or without a VAD.
        return (Judged::Nothing, Verdicts::Silence);
    }
    if let Vad::Installed(source) = vad {
        // The steps of `normalise_speech`, keeping every segment (module docs).
        let mut lifted = judged.to_vec();
        let lift = if paired { levels(level) } else { before };
        apply_gain(&mut lifted, provisional_gain(&lift));
        match speech_segments(&lifted, source.as_mut(), cfg) {
            Ok(segments) => {
                let Some(speech) = speech_levels(heard, &segments) else {
                    return (Judged::Nothing, Verdicts::Vad);
                };
                let spans = segments
                    .iter()
                    .map(|s| {
                        let r = s.samples();
                        r.start.min(heard.len())..r.end.min(heard.len())
                    })
                    .filter(|r| !r.is_empty())
                    .collect();
                return (
                    Judged::Speech {
                        gain: gain_for(speech.robust_peak),
                        spans,
                    },
                    Verdicts::Vad,
                );
            }
            Err(error) => {
                // Never guessed around: the pass switches to the fallback from this window on,
                // and the caller reports the error.
                *vad = Vad::Unavailable(VadUnavailable::Failed);
                *vad_error = Some(error);
            }
        }
    }
    let mut copy = judged.to_vec();
    let report = normalise_without_vad(&mut copy);
    let judged = match report.outcome {
        // Room tone and hum: the fallback's own verdict is "not speech", and this engine invents
        // text on non-speech, so it is not sent.
        GainOutcome::Stationary | GainOutcome::Silence => Judged::Nothing,
        _ => Judged::Speech {
            gain: if paired {
                normalise_without_vad(&mut heard.to_vec()).gain()
            } else {
                report.gain()
            },
            spans: vec![Range {
                start: 0,
                end: heard.len(),
            }],
        },
    };
    (judged, Verdicts::None)
}

/// An open region: from `start` (before padding) to where its speech last ended.
#[derive(Clone, Copy, Debug)]
struct Open {
    start: u64,
    speech_end: u64,
}

/// Joins speech spans into regions and cuts their audio out of the levelled stream.
///
/// It keeps the levelled audio from the open region's start, or, with none open, the last
/// `pad` samples (the next region's lead).
struct RegionBuilder {
    cfg: RegionConfig,
    pad: u64,
    buf: Vec<f32>,
    buf_start: u64,
    open: Option<Open>,
    /// Where the last region ended: the next one never starts before it.
    last_end: u64,
}

impl RegionBuilder {
    fn new(cfg: RegionConfig, pad: usize) -> Self {
        Self {
            cfg,
            pad: pad as u64,
            buf: Vec::new(),
            buf_start: 0,
            open: None,
            last_end: 0,
        }
    }

    fn buf_end(&self) -> u64 {
        self.buf_start + self.buf.len() as u64
    }

    /// Takes the next window (starting where the last one ended), levelled by `gain`, with its
    /// speech `spans` (absolute, in order).
    fn push(
        &mut self,
        start: u64,
        samples: &[f32],
        gain: f32,
        spans: &[Range<u64>],
        out: &mut Vec<Region>,
    ) {
        debug_assert_eq!(
            start,
            self.buf_end(),
            "windows arrive in order, without gaps"
        );
        let from = self.buf.len();
        self.buf.extend_from_slice(samples);
        apply_gain(&mut self.buf[from..], gain);
        let max_pause = self.cfg.max_pause as u64;
        for span in spans {
            match &mut self.open {
                Some(open) if span.start <= open.speech_end + max_pause => {
                    open.speech_end = open.speech_end.max(span.end);
                }
                _ => {
                    self.close(out);
                    self.open = Some(Open {
                        start: span.start,
                        speech_end: span.end,
                    });
                }
            }
        }
        // Speech that could still join the open region would have to start within `max_pause`
        // of its end; once the audio is past that, nothing can.
        if self
            .open
            .is_some_and(|open| self.buf_end() > open.speech_end + max_pause)
        {
            self.close(out);
        }
        self.cut_long(out);
        self.trim();
    }

    /// Closes the open region, if any.
    fn close(&mut self, out: &mut Vec<Region>) {
        let Some(open) = self.open.take() else {
            return;
        };
        let start = self.region_start(open);
        let end = (open.speech_end + self.pad).min(self.buf_end()).max(start);
        self.emit(start, end, out);
    }

    /// Where `open` starts, padded: never before the last region's end or the audio kept.
    fn region_start(&self, open: Open) -> u64 {
        open.start
            .saturating_sub(self.pad)
            .max(self.last_end)
            .max(self.buf_start)
    }

    /// Emits the open region's first `max_region` while more than that is buffered, so a long
    /// monologue never holds more than one region in memory.
    fn cut_long(&mut self, out: &mut Vec<Region>) {
        while let Some(open) = self.open {
            let start = self.region_start(open);
            if self.buf_end() - start <= self.cfg.max_region as u64 {
                return;
            }
            let cut = self.cut_point(start);
            self.push_region(start, cut, out);
            // The rest goes on from the cut, with no lead: it continues the same speech.
            self.open = Some(Open {
                start: cut,
                speech_end: open.speech_end.max(cut),
            });
        }
    }

    /// Emits `start..end`, cut into pieces of at most `max_region`.
    fn emit(&mut self, mut start: u64, end: u64, out: &mut Vec<Region>) {
        while end - start > self.cfg.max_region as u64 {
            let cut = self.cut_point(start);
            self.push_region(start, cut, out);
            start = cut;
        }
        if end > start {
            self.push_region(start, end, out);
        }
    }

    /// The quietest point in the last `cut_search` of the `max_region` after `start`.
    fn cut_point(&self, start: u64) -> u64 {
        let from = (start - self.buf_start) as usize;
        let limit = from + self.cfg.max_region;
        let search = limit.saturating_sub(self.cfg.cut_search).max(from + 1);
        let cut = quietest_cut(&self.buf, search..limit);
        // Always progress, even if the search region were empty.
        self.buf_start + cut.max(from + 1) as u64
    }

    fn push_region(&mut self, start: u64, end: u64, out: &mut Vec<Region>) {
        let from = (start - self.buf_start) as usize;
        let to = (end - self.buf_start) as usize;
        out.push(Region {
            start,
            audio: self.buf[from..to].to_vec(),
        });
        self.last_end = end;
    }

    /// Drops audio no region can need any more.
    fn trim(&mut self) {
        let keep_from = match self.open {
            Some(open) => self.region_start(open),
            None => self
                .buf_end()
                .saturating_sub(self.pad)
                .max(self.last_end)
                .max(self.buf_start),
        };
        let drop = (keep_from - self.buf_start) as usize;
        self.buf.drain(..drop);
        self.buf_start = keep_from;
    }

    fn finish(&mut self, out: &mut Vec<Region>) {
        self.close(out);
        self.buf.clear();
    }
}

#[cfg(test)]
// Speech spans are lists of ranges, one range long or not.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn cfg() -> RegionConfig {
        RegionConfig {
            max_pause: 100,
            max_region: 1_000,
            cut_search: 200,
            ..RegionConfig::default()
        }
    }

    /// Feeds `len` samples of `level` (with one quiet sample every 50) as one window with `spans`.
    fn window(b: &mut RegionBuilder, len: usize, spans: &[Range<u64>], out: &mut Vec<Region>) {
        let start = b.buf_end();
        let samples: Vec<f32> = (0..len)
            .map(|i| {
                if (start as usize + i).is_multiple_of(50) {
                    0.0
                } else {
                    0.1
                }
            })
            .collect();
        b.push(start, &samples, 1.0, spans, out);
    }

    #[test]
    fn spans_within_the_pause_join_and_are_padded_without_overlap() {
        let mut b = RegionBuilder::new(cfg(), 20);
        let mut out = Vec::new();
        window(&mut b, 600, &[100..200, 250..300, 500..550], &mut out);
        // 100..300 joined (a 50-sample pause); 500 is 200 after 300, so it starts a new region.
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].start, out[0].end()), (80, 320));
        b.finish(&mut out);
        assert_eq!(out.len(), 2);
        assert_eq!((out[1].start, out[1].end()), (480, 570));
    }

    #[test]
    fn a_region_carries_across_windows_and_its_lead_is_kept() {
        let mut b = RegionBuilder::new(cfg(), 20);
        let mut out = Vec::new();
        window(&mut b, 400, &[390..400], &mut out);
        window(&mut b, 100, &[400..450], &mut out);
        assert!(out.is_empty(), "still open: {out:?}");
        // The audio passes 450 + max_pause: nothing can join any more.
        window(&mut b, 400, &[], &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].start, out[0].end()), (370, 470));
        assert_eq!(out[0].audio.len(), 100);
    }

    #[test]
    fn a_long_region_is_cut_at_its_quietest_frame_and_nothing_is_lost() {
        let cfg = RegionConfig {
            max_pause: 100,
            max_region: 4_000,
            cut_search: 1_600,
            ..RegionConfig::default()
        };
        let mut b = RegionBuilder::new(cfg, 0);
        let mut out = Vec::new();
        // Speech throughout, with one quiet stretch at 3000..3320, inside the first cut's search
        // (2400..4000).
        let mut audio = vec![0.1f32; 9_000];
        audio[3_000..3_320].fill(0.0);
        for (k, block) in audio.chunks(1_000).enumerate() {
            let start = (k * 1_000) as u64;
            b.push(
                start,
                block,
                1.0,
                &[start..start + block.len() as u64],
                &mut out,
            );
        }
        b.finish(&mut out);
        // Frames are scanned from 2400 in 320s: 3040..3360 is the quietest; its middle is 3200.
        assert_eq!(out[0].end(), 3_200);
        let mut at = 0;
        for r in &out {
            assert_eq!(r.start, at, "regions are contiguous");
            assert!(r.audio.len() <= 4_000);
            at = r.end();
        }
        assert_eq!(at, 9_000);
    }

    #[test]
    fn a_window_without_speech_keeps_only_the_lead() {
        let mut b = RegionBuilder::new(cfg(), 20);
        let mut out = Vec::new();
        window(&mut b, 5_000, &[], &mut out);
        assert!(out.is_empty());
        assert_eq!(b.buf.len(), 20);
    }

    #[test]
    fn audible_time_counts_frames_above_the_floor_before_any_gain() {
        let mut pass = SpeechPass::new(
            Vad::Unavailable(VadUnavailable::ModelMissing),
            VadConfig::default(),
            RegionConfig::default(),
        )
        .unwrap();
        // One second at −30 dBFS RMS (a square wave), one at −50, one of digital silence.
        let level = |dbfs: f32| from_dbfs(dbfs);
        let mut audio: Vec<f32> = (0..16_000)
            .map(|i| {
                if i % 2 == 0 {
                    level(-30.0)
                } else {
                    -level(-30.0)
                }
            })
            .collect();
        audio.extend((0..16_000).map(|i| {
            if i % 2 == 0 {
                level(-50.0)
            } else {
                -level(-50.0)
            }
        }));
        audio.extend(std::iter::repeat_n(0.0, 16_000));
        let mut out = Vec::new();
        pass.push(&audio, &mut out).unwrap();
        pass.finish(&mut out);
        assert_eq!(pass.audible_ms(), 1_000);
        assert_eq!(
            pass.speech_ms(),
            out.iter().map(|r| r.audio.len() as u64 / 16).sum::<u64>()
        );
    }

    #[test]
    fn little_speech_is_judged_only_on_enough_audible_time() {
        assert!(little_speech_heard(30_000, 2_999));
        assert!(!little_speech_heard(30_000, 3_000));
        assert!(!little_speech_heard(29_999, 0), "too little to judge");
    }

    /// Speech wherever the window it hears is above `0` dBFS RMS.
    struct Loud(f32);

    impl SpeechProbability for Loud {
        fn reset(&mut self) {}
        fn probability(&mut self, w: &[f32; VAD_WINDOW]) -> Result<f32, EngineError> {
            Ok(if to_dbfs(rms(w)) > self.0 { 1.0 } else { 0.0 })
        }
    }

    use ink_audio::gain::to_dbfs;

    fn square(seconds: f64, dbfs: f32) -> Vec<f32> {
        let a = from_dbfs(dbfs);
        (0..(seconds * 16_000.0) as usize)
            .map(|i| if i % 2 == 0 { a } else { -a })
            .collect()
    }

    fn paired(vad_db: f32) -> SpeechPass {
        SpeechPass::paired(
            Vad::Installed(Box::new(Loud(vad_db))),
            VadConfig::default(),
            RegionConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn a_paired_pass_is_judged_on_one_stream_and_carries_the_other() {
        // Judged: speech 2–3 s only. Heard: a steady −30 dBFS throughout (echo the linear output
        // still carries, say).
        let judged = [square(2.0, -90.0), square(1.0, -30.0), square(3.0, -90.0)].concat();
        let heard = square(6.0, -30.0);
        let mut pass = paired(-50.0);
        let mut out = Vec::new();
        pass.push_paired(&judged, &heard, &heard, &mut out).unwrap();
        pass.finish(&mut out);
        assert_eq!(
            out.len(),
            1,
            "{:?}",
            out.iter().map(|r| r.start).collect::<Vec<_>>()
        );
        let r = &out[0];
        // 2–3 s padded by 250 ms, give or take a VAD window.
        assert!((27_500..=28_100).contains(&r.start), "{}", r.start);
        assert!((51_900..=52_600).contains(&r.end()), "{}", r.end());
        // The audio is the heard stream's, levelled: a square wave, never the judged silence.
        let peak = r.audio.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(r.audio.iter().all(|s| (s.abs() - peak).abs() < 1e-6));
        // Audible time is the judged stream's: 1 s.
        assert_eq!(pass.audible_ms(), 1_000);
    }

    #[test]
    fn a_paired_pass_lifts_the_judged_stream_by_the_level_stream_s_gain() {
        // The judged stream is quiet (what a suppressor left), the level stream loud (the mic,
        // echo and all): lifted by its gain the judged one stays quiet, so the VAD hears no
        // speech. The heard stream (the linear output, quieter) does not set the lift.
        let judged = square(3.0, -65.0);
        let heard = square(3.0, -40.0);
        let level = square(3.0, -15.0);
        let mut pass = paired(-50.0);
        let mut out = Vec::new();
        pass.push_paired(&judged, &heard, &level, &mut out).unwrap();
        pass.finish(&mut out);
        assert!(out.is_empty());
        // The same quiet stream judged alone is lifted to speech level and heard.
        let mut alone = SpeechPass::new(
            Vad::Installed(Box::new(Loud(-50.0))),
            VadConfig::default(),
            RegionConfig::default(),
        )
        .unwrap();
        alone.push(&judged, &mut out).unwrap();
        alone.finish(&mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_paired_pass_keeps_the_vad_s_verdicts_as_evidence() {
        let judged = [square(2.0, -90.0), square(1.0, -30.0), square(3.0, -90.0)].concat();
        let heard = square(6.0, -30.0);
        let mut pass = paired(-50.0);
        let mut out = Vec::new();
        pass.push_paired(&judged, &heard, &heard, &mut out).unwrap();
        pass.finish(&mut out);
        let e = pass.evidence();
        assert_eq!(e.judged_until(), 96_000);
        assert_eq!(e.near_speech(2_200, 2_800), Some(true));
        // 250 ms of pad either side: 3.3–3.6 s reaches back to 3.05 s, clear of the speech.
        assert_eq!(e.near_speech(3_300, 3_600), Some(false));
        assert_eq!(e.near_speech(3_100, 3_600), Some(true), "within the pad");
        assert_eq!(e.near_speech(500, 1_000), Some(false));
        // After the audio: nothing was said there; wholly after it: no evidence.
        assert_eq!(e.near_speech(5_900, 6_500), Some(false));
        assert_eq!(e.near_speech(6_300, 6_500), None);
    }

    #[test]
    fn evidence_is_keyed_by_position_and_a_stretch_without_verdicts_is_none() {
        let mut e = HeardSpeech::default();
        // 0–1 s judged, no speech; 1–2 s judged by the fallback; 2–3 s speech at 2.5 s.
        e.add(0, 16_000, Some(vec![0.0; 32]));
        e.add(16_000, 16_000, None);
        let mut p = vec![0.1; 32];
        p[16] = 0.9; // 2.512–2.544 s
        e.add(32_000, 16_000, Some(p));
        assert_eq!(e.near_speech(100, 400), Some(false));
        assert_eq!(e.near_speech(1_200, 1_400), None, "no verdicts there");
        assert_eq!(e.near_speech(600, 800), None, "the pad reaches 1.05 s");
        assert_eq!(
            e.near_speech(2_600, 2_700),
            Some(true),
            "within 250 ms of 2.544 s"
        );
        assert_eq!(e.near_speech(2_850, 3_000), Some(false));
    }

    #[test]
    fn the_gain_is_applied_to_the_region_audio() {
        let mut b = RegionBuilder::new(cfg(), 0);
        let mut out = Vec::new();
        b.push(0, &[0.01; 300], 10.0, &[0..300], &mut out);
        b.finish(&mut out);
        assert!(out[0].audio.iter().all(|&s| (s - 0.1).abs() < 1e-6));
    }
}
