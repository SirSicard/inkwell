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
//! # Threads and memory
//!
//! **Worker.** A pass holds about one window and one region: at most two minutes of audio, never
//! the recording (architecture rule 3).

use std::ops::Range;
use std::sync::Arc;

use ink_audio::gain::{
    GainOutcome, NOISE_FLOOR, apply_gain, gain_for, levels, provisional_gain, speech_levels,
};
use ink_audio::window::quietest_cut;
use ink_audio::{
    SpeechProbability, VadConfig, WindowConfig, WindowError, Windower, normalise_without_vad,
    speech_segments,
};
use ink_core::{CANONICAL_RATE, EngineError};

use crate::events::{VadUnavailable, VoiceDetection};
use crate::gain_stage::Vad;

/// Samples per millisecond at the canonical rate.
const SAMPLES_PER_MS: u64 = CANONICAL_RATE as u64 / 1_000;

/// A sample position on a 16 kHz timeline, in ms.
pub fn samples_to_ms(samples: u64) -> u64 {
    samples / SAMPLES_PER_MS
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
        })
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
        self.drain(out);
        Ok(())
    }

    /// Ends the recording: the rest comes out.
    pub fn finish(&mut self, out: &mut Vec<Region>) {
        self.windower.finish();
        self.drain(out);
        self.regions.finish(out);
    }

    fn drain(&mut self, out: &mut Vec<Region>) {
        while let Some((window, samples)) = self.windower.next_window() {
            let speech = judge(&mut self.vad, &mut self.vad_error, &self.cfg, samples);
            let (gain, spans) = match speech {
                Judged::Speech { gain, spans } => (gain, spans),
                Judged::Nothing => (1.0, Vec::new()),
            };
            let spans: Vec<Range<u64>> = spans
                .into_iter()
                .map(|r| window.start + r.start as u64..window.start + r.end as u64)
                .collect();
            self.regions.push(window.start, samples, gain, &spans, out);
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

/// Judges one window: with the VAD while it works, else with the fallback.
fn judge(
    vad: &mut Vad,
    vad_error: &mut Option<EngineError>,
    cfg: &VadConfig,
    samples: &[f32],
) -> Judged {
    let before = levels(samples);
    if before.robust_peak < NOISE_FLOOR {
        // Digital silence, or near enough: there is nothing to hear, with or without a VAD.
        return Judged::Nothing;
    }
    if let Vad::Installed(source) = vad {
        // The steps of `normalise_speech`, keeping every segment (module docs).
        let mut heard = samples.to_vec();
        apply_gain(&mut heard, provisional_gain(&before));
        match speech_segments(&heard, source.as_mut(), cfg) {
            Ok(segments) => {
                let Some(speech) = speech_levels(samples, &segments) else {
                    return Judged::Nothing;
                };
                let spans = segments
                    .iter()
                    .map(|s| {
                        let r = s.samples();
                        r.start.min(samples.len())..r.end.min(samples.len())
                    })
                    .filter(|r| !r.is_empty())
                    .collect();
                return Judged::Speech {
                    gain: gain_for(speech.robust_peak),
                    spans,
                };
            }
            Err(error) => {
                // Never guessed around: the pass switches to the fallback from this window on,
                // and the caller reports the error.
                *vad = Vad::Unavailable(VadUnavailable::Failed);
                *vad_error = Some(error);
            }
        }
    }
    let mut copy = samples.to_vec();
    let report = normalise_without_vad(&mut copy);
    match report.outcome {
        // Room tone and hum: the fallback's own verdict is "not speech", and this engine invents
        // text on non-speech, so it is not sent.
        GainOutcome::Stationary | GainOutcome::Silence => Judged::Nothing,
        _ => Judged::Speech {
            gain: report.gain(),
            spans: vec![Range {
                start: 0,
                end: samples.len(),
            }],
        },
    }
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
    fn the_gain_is_applied_to_the_region_audio() {
        let mut b = RegionBuilder::new(cfg(), 0);
        let mut out = Vec::new();
        b.push(0, &[0.01; 300], 10.0, &[0..300], &mut out);
        b.finish(&mut out);
        assert!(out[0].audio.iter().all(|&s| (s - 0.1).abs() < 1e-6));
    }
}
