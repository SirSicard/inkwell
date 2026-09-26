//! The meeting AGC: a slow gain toward the normaliser's [`TARGET_PEAK`] (architecture rule 11).
//!
//! A meeting has no utterance to measure first, so the gain follows the audio: 16 kHz mono, in
//! place, one 20 ms [`LEVEL_FRAME`] at a time.
//!
//! # Two modes, one per situation
//!
//! - **[`Agc::with_vad`], whenever a VAD is installed.** The gain learns only from frames the VAD
//!   marks as speech (the WebRTC AGC2 pattern): its probability at or over the threshold, or over
//!   the lower threshold while speech goes on ([`SpeechSegmenter::counts_as_speech`]). Everything
//!   else, rumble, a fan, knocks, a pause, never moves the gain, whatever its shape. The VAD
//!   cannot hear −75 dBFS speech either, so it listens to a copy of the input lifted by a
//!   provisional gain: [`gain_for`] the robust peak of the last [`RECENT_FRAMES`] (1.5 s), with no
//!   judgement of what they hold. That copy is only for the VAD; the output uses the AGC's own
//!   gain. (Listening to the AGC's output instead would never start: it begins at unity gain, where
//!   a quiet talker is inaudible to the VAD.) A frame's verdict comes from the VAD window holding
//!   its middle sample, a window or so after the frame is measured; frames wait for it.
//! - **[`Agc::without_vad`], only while no VAD is available**, with the app saying that voice
//!   detection is unavailable. It learns from frames that stand [`MIN_DYNAMICS_DB`] over the quiet
//!   frame of the last 1.5 s of the speech band, when that 1.5 s has that much contrast at all
//!   ([`speech_band`](crate::speech_band)). It errs toward lifting, and can lift rumble and other
//!   non-speech.
//!
//! Either way the level is the normaliser's full-band robust peak taken over the last
//! [`SPEECH_WINDOW_FRAMES`] learned frames: the same measure, the same [`TRANSIENT_FRAMES`]
//! skipped, the same target.
//!
//! - **Pauses hold the gain.** Nothing is learned from them, so the first word after a pause comes
//!   out at the level the last word did, not pumped up. A frame more than [`ADAPT_RANGE_DB`] below
//!   the current speech level joins the level window (so a much quieter talker takes over once the
//!   window has turned over) but never steps the gain itself.
//! - **Slow up, fast down.** The gain rises at most [`RISE_DB_PER_S`] and falls at most
//!   [`FALL_DB_PER_S`]. The first lock is the exception: once [`MIN_SPEECH_FRAMES`] have been
//!   learned the gain jumps straight to its target, so a quiet talker is lifted from their first
//!   half second rather than over ten.
//! - **It never clips and never steps.** The gain is applied by a limiter working in 10 ms
//!   [`LIMITER_BLOCK`]s with two blocks of look-ahead ([`Agc::LATENCY`], 20 ms). Each block's
//!   safe gain is the AGC's gain capped so the block's own peak stays at or below full scale. A
//!   block is played with a gain that moves geometrically, across the whole block, from where the
//!   previous block ended to the lower of its own safe gain and the next block's. So a loud block
//!   (a cough) starts at its safe gain, the ramp down to it happens in the 10 ms before it, and no
//!   sample is ever above its own block's cap. Consecutive samples' gains differ by at most one
//!   block's share of the change: 60 dB (the whole range) / 160 = 0.375 dB at the very worst.
//!   The limiter never touches the AGC's gain itself, so a transient too short to move the level
//!   ducks only itself.
//! - **Gain stays between 1 and [`MAX_GAIN`].** Healthy audio is never attenuated, as with the
//!   normaliser.
//!
//! # Threading and allocation
//!
//! **Worker** with a VAD (a model runs every 32 ms of audio); **pump or worker** without. The
//! constructors allocate the blocks and windows; [`process`] allocates nothing itself (a VAD
//! implementation may), and [`flush`] only grows its `out` (reserve [`Agc::LATENCY`] samples
//! first).
//!
//! [`process`]: Agc::process
//! [`flush`]: Agc::flush

use std::collections::VecDeque;

use ink_core::EngineError;

use crate::gain::{
    LEVEL_FRAME, MAX_GAIN, MIN_DYNAMICS, MIN_DYNAMICS_DB, NOISE_FLOOR, TARGET_PEAK,
    TRANSIENT_FRAMES, frame_peak, gain_for,
};
use crate::speech_band::{SpeechBandFilter, contrast_in};
use crate::vad::{self, SpeechProbability, SpeechSegmenter, VAD_WINDOW, VadConfig};

/// The limiter's block (10 ms).
pub const LIMITER_BLOCK: usize = LEVEL_FRAME / 2;

/// Level frames per second.
const FRAMES_PER_S: f32 = 50.0;

/// The fastest the gain rises, in dB per second, once it has locked.
pub const RISE_DB_PER_S: f32 = 6.0;

/// The fastest the gain falls, in dB per second (a louder talker must not wait).
pub const FALL_DB_PER_S: f32 = 60.0;

/// The provisional gain (with a VAD) and the contrast test (without one) look at this many recent
/// frames (1.5 s).
pub const RECENT_FRAMES: usize = 75;

/// The level is measured over this many recent learned frames (3 s of speech).
pub const SPEECH_WINDOW_FRAMES: usize = 150;

/// Frames learned before the first lock (0.5 s).
pub const MIN_SPEECH_FRAMES: usize = 25;

/// A learned frame steps the gain only if it peaks within this many dB of the current level.
pub const ADAPT_RANGE_DB: f32 = 30.0;

/// Recent VAD verdicts kept for frames still waiting on theirs.
const VERDICTS: usize = 8;

/// Frames that can be waiting for a verdict at once (a frame waits at most about one window).
const PENDING: usize = 16;

/// The slow AGC for meetings. See the module docs.
pub struct Agc {
    /// The block being filled with input.
    filling: Vec<f32>,
    /// The block before it: complete, measured, waiting to be played.
    waiting: Vec<f32>,
    waiting_peak: f32,
    /// The block before that, being played (gained) as `filling` fills.
    playing: Vec<f32>,
    /// Position in all three blocks.
    pos: usize,
    /// The playing block's gain ramp: from `ramp_from` to `ramp_to` across `ramp_len` samples,
    /// geometric (`ramp_log` is ln(to / from)).
    ramp_from: f32,
    ramp_to: f32,
    ramp_log: f32,
    ramp_len: usize,
    /// The first block of a 20 ms level frame whose second block has not come yet: its peak, its
    /// speech-band energy and its length.
    half_frame: Option<(f32, f64, usize)>,
    /// Level frames completed in this stream (since the last flush).
    frames: u64,
    /// What decides which frames the level learns from.
    gate: Gate,
    /// The gain and the level it follows.
    level: Level,
}

impl Agc {
    /// The output is the input delayed by this many samples (two limiter blocks, 20 ms).
    pub const LATENCY: usize = 2 * LIMITER_BLOCK;

    /// **The mode to use whenever a VAD is installed.** An AGC at unity gain that learns level only
    /// from what `vad` marks as speech, with `cfg`'s thresholds and hangover. **Allocates.**
    pub fn with_vad(mut vad: Box<dyn SpeechProbability>, cfg: VadConfig) -> Self {
        vad.reset();
        Self::with_gate(Gate::Vad(Box::new(VadGate {
            source: vad,
            cfg,
            segmenter: SpeechSegmenter::new(cfg),
            window: [0.0; VAD_WINDOW],
            pos: 0,
            windows: 0,
            verdicts: [false; VERDICTS],
            pending: VecDeque::with_capacity(PENDING),
            provisional: 1.0,
            recent: Ring::new(RECENT_FRAMES),
            scratch: vec![0.0; RECENT_FRAMES],
            error: None,
        })))
    }

    /// **The fallback, only while no VAD is available**, with the app saying that voice detection
    /// is unavailable. An AGC at unity gain that learns from speech-band contrast; it can lift
    /// non-speech. **Allocates.**
    pub fn without_vad() -> Self {
        Self::with_gate(Gate::Contrast(Box::new(ContrastGate {
            band: SpeechBandFilter::new(),
            band_energy: 0.0,
            recent: Ring::new(RECENT_FRAMES),
            scratch: vec![0.0; RECENT_FRAMES],
        })))
    }

    fn with_gate(gate: Gate) -> Self {
        Self {
            filling: vec![0.0; LIMITER_BLOCK],
            waiting: vec![0.0; LIMITER_BLOCK],
            waiting_peak: 0.0,
            playing: vec![0.0; LIMITER_BLOCK],
            pos: 0,
            ramp_from: 1.0,
            ramp_to: 1.0,
            ramp_log: 0.0,
            ramp_len: LIMITER_BLOCK,
            half_frame: None,
            frames: 0,
            gate,
            level: Level {
                gain: 1.0,
                locked: false,
                speech: Ring::new(SPEECH_WINDOW_FRAMES),
                scratch: vec![0.0; SPEECH_WINDOW_FRAMES],
            },
        }
    }

    /// Whether this AGC learns from a VAD ([`with_vad`](Self::with_vad)).
    pub fn uses_vad(&self) -> bool {
        matches!(self.gate, Gate::Vad(_))
    }

    /// The first error the VAD returned, if any (a probability outside `0.0..=1.0` included). From
    /// then on the AGC learns nothing and holds its gain: it never guesses. The pipeline should
    /// say so and choose what to do (a new VAD, or [`without_vad`](Self::without_vad)).
    pub fn vad_error(&self) -> Option<&EngineError> {
        match &self.gate {
            Gate::Vad(g) => g.error.as_ref(),
            Gate::Contrast(_) => None,
        }
    }

    /// The gain the AGC is holding or heading for (linear, 1.0 to [`MAX_GAIN`]). A loud block
    /// is played with less, to stay under full scale, without changing this.
    pub fn gain(&self) -> f32 {
        self.level.gain
    }

    /// Replaces `samples` (16 kHz mono) with the gained audio [`LATENCY`](Self::LATENCY) samples
    /// earlier. The first `LATENCY` samples a new AGC puts out are silence.
    ///
    /// **Worker** (with a VAD) or **pump**. Allocates nothing itself.
    pub fn process(&mut self, samples: &mut [f32]) {
        for s in samples {
            let input = *s;
            *s = self.playing[self.pos] * self.ramp_at(self.pos);
            self.filling[self.pos] = input;
            self.gate.hear(input, &mut self.level);
            self.pos += 1;
            if self.pos == LIMITER_BLOCK {
                self.end_block();
                self.pos = 0;
            }
        }
    }

    /// Appends the last [`LATENCY`](Self::LATENCY) samples still held by the look-ahead, gained,
    /// so everything pushed has come out, and ends the stream: the VAD scores its last, partial
    /// window and is reset. The gain and the level history are kept: the AGC can go on after a
    /// flush, as a new stream (its next output starts after another `LATENCY` of silence).
    ///
    /// **Worker** (with a VAD) or **pump**. Allocation-free when `out` has room for `LATENCY`
    /// more samples.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        for i in self.pos..LIMITER_BLOCK {
            out.push(self.playing[i] * self.ramp_at(i));
        }
        // The waiting block plays next, ramping toward the partial block after it (or, with no
        // partial block, toward nothing: its own safe gain).
        let partial = self.pos;
        let partial_peak = frame_peak(&self.filling[..partial]);
        if partial > 0 {
            let energy = self.gate.take_band_energy();
            self.measure_block(partial_peak, energy, partial);
        }
        let next_safe = if partial > 0 {
            self.safe(partial_peak)
        } else {
            f32::INFINITY
        };
        self.set_ramp(self.safe(self.waiting_peak).min(next_safe), LIMITER_BLOCK);
        for i in 0..LIMITER_BLOCK {
            out.push(self.waiting[i] * self.ramp_at(i));
        }
        // The partial block holds the gain the waiting block ended on. That gain is at or below the
        // partial block's own safe gain (the waiting block's ramp took it into account), and
        // holding it keeps the step bound: a ramp squeezed into a block of a few samples would
        // not.
        if partial > 0 {
            self.set_ramp(self.ramp_to, partial);
            for i in 0..partial {
                out.push(self.filling[i] * self.ramp_at(i));
            }
        }
        // A level frame left half measured is measured as it is.
        if let Some((peak, energy, len)) = self.half_frame.take() {
            self.end_frame(peak, band_rms(energy, len));
        }
        self.gate.end_stream(&mut self.level);
        self.filling.fill(0.0);
        self.waiting.fill(0.0);
        self.playing.fill(0.0);
        self.waiting_peak = 0.0;
        self.pos = 0;
        self.frames = 0;
    }

    /// The gain for sample `i` of the block being played: geometric from `ramp_from` to
    /// `ramp_to`, clamped between them so rounding can never lift it above either.
    fn ramp_at(&self, i: usize) -> f32 {
        if self.ramp_log == 0.0 {
            return self.ramp_to;
        }
        let t = (i + 1) as f32 / self.ramp_len as f32;
        let g = self.ramp_from * (self.ramp_log * t).exp();
        g.clamp(
            self.ramp_from.min(self.ramp_to),
            self.ramp_from.max(self.ramp_to),
        )
    }

    /// The next block ramps from where the last one ended to `to`, across `len` samples.
    fn set_ramp(&mut self, to: f32, len: usize) {
        self.ramp_from = self.ramp_to;
        self.ramp_to = to;
        self.ramp_log = (to / self.ramp_from).ln();
        self.ramp_len = len.max(1);
    }

    /// The highest gain a block peaking at `peak` can take without going past full scale.
    fn safe(&self, peak: f32) -> f32 {
        if peak > 0.0 {
            // Never zero, so a ramp's logarithm stays finite even for an absurd input.
            self.level.gain.min(1.0 / peak).max(f32::MIN_POSITIVE)
        } else {
            self.level.gain
        }
    }

    /// `filling` is a complete block: measure it, then play the waiting block with a ramp that
    /// ends at or below both its own safe gain and this block's, and rotate the three buffers.
    fn end_block(&mut self) {
        let peak = frame_peak(&self.filling);
        let energy = self.gate.take_band_energy();
        self.measure_block(peak, energy, LIMITER_BLOCK);
        let to = self.safe(self.waiting_peak).min(self.safe(peak));
        self.set_ramp(to, LIMITER_BLOCK);
        // playing <- waiting <- filling <- (the old playing, reused)
        std::mem::swap(&mut self.playing, &mut self.waiting);
        std::mem::swap(&mut self.waiting, &mut self.filling);
        self.waiting_peak = peak;
    }

    /// Two limiter blocks make one 20 ms level frame: its full-band peak and its speech-band RMS.
    fn measure_block(&mut self, peak: f32, energy: f64, len: usize) {
        match self.half_frame.take() {
            None => self.half_frame = Some((peak, energy, len)),
            Some((first_peak, first_energy, first_len)) => self.end_frame(
                first_peak.max(peak),
                band_rms(first_energy + energy, first_len + len),
            ),
        }
    }

    /// A complete level frame: the gate decides whether the level learns from it.
    fn end_frame(&mut self, peak: f32, band: f32) {
        let index = self.frames;
        self.frames += 1;
        self.gate.frame(peak, band, index, &mut self.level);
    }
}

/// The gain and the level it follows.
struct Level {
    gain: f32,
    locked: bool,
    /// Recent learned frames' peaks.
    speech: Ring,
    /// Scratch for selecting the robust peak out of `speech`.
    scratch: Vec<f32>,
}

impl Level {
    /// Learns from a frame peaking at `peak` (above the silence floor).
    fn learn(&mut self, peak: f32) {
        if peak < NOISE_FLOOR {
            return;
        }
        self.speech.push(peak);
        self.adapt(peak);
    }

    /// Moves the gain toward the robust peak of the learned frames, after one peaking at `peak`
    /// joined them.
    fn adapt(&mut self, peak: f32) {
        let count = self.speech.len();
        if count < MIN_SPEECH_FRAMES {
            return;
        }
        let window = &mut self.scratch[..count];
        window.copy_from_slice(self.speech.values());
        // Skip the same fraction of loudest frames a full window skips (TRANSIENT_FRAMES of 150),
        // so the first lock on 25 frames measures what the settled AGC will, not a lower
        // percentile. Always at least one, so a lone click never sets the level.
        let k = (TRANSIENT_FRAMES * count)
            .div_ceil(SPEECH_WINDOW_FRAMES)
            .min(count - 1);
        // Loudest first; the k-th is the robust peak. Allocation-free, in place.
        let (_, robust, _) = window.select_nth_unstable_by(k, |a, b| b.total_cmp(a));
        let robust = *robust;
        let desired = (TARGET_PEAK / robust).clamp(1.0, MAX_GAIN);
        if !self.locked {
            self.gain = desired;
            self.locked = true;
            return;
        }
        if peak < robust * 10f32.powf(-ADAPT_RANGE_DB / 20.0) {
            return;
        }
        let rise = 10f32.powf(RISE_DB_PER_S / 20.0 / FRAMES_PER_S);
        let fall = 10f32.powf(-FALL_DB_PER_S / 20.0 / FRAMES_PER_S);
        self.gain = if desired > self.gain {
            desired.min(self.gain * rise)
        } else {
            desired.max(self.gain * fall)
        };
    }
}

/// What decides which frames the level learns from.
enum Gate {
    Vad(Box<VadGate>),
    Contrast(Box<ContrastGate>),
}

impl Gate {
    fn hear(&mut self, x: f32, level: &mut Level) {
        match self {
            Self::Vad(g) => g.hear(x, level),
            Self::Contrast(g) => g.hear(x),
        }
    }

    /// The speech-band energy of the block just filled (only the fallback measures it).
    fn take_band_energy(&mut self) -> f64 {
        match self {
            Self::Vad(_) => 0.0,
            Self::Contrast(g) => std::mem::take(&mut g.band_energy),
        }
    }

    fn frame(&mut self, peak: f32, band: f32, index: u64, level: &mut Level) {
        match self {
            Self::Vad(g) => g.frame(peak, index, level),
            Self::Contrast(g) => g.frame(peak, band, level),
        }
    }

    fn end_stream(&mut self, level: &mut Level) {
        match self {
            Self::Vad(g) => g.end_stream(level),
            Self::Contrast(g) => {
                g.band.reset();
                g.band_energy = 0.0;
            }
        }
    }
}

/// With a VAD: the level learns from the frames it marks as speech.
struct VadGate {
    source: Box<dyn SpeechProbability>,
    cfg: VadConfig,
    segmenter: SpeechSegmenter,
    /// The window being filled with the lifted copy of the input.
    window: [f32; VAD_WINDOW],
    pos: usize,
    /// Windows scored in this stream.
    windows: u64,
    /// The last [`VERDICTS`] windows' verdicts, by window index modulo the length.
    verdicts: [bool; VERDICTS],
    /// Frames waiting for their window's verdict: (peak, window index).
    pending: VecDeque<(f32, u64)>,
    /// The gain the VAD hears the input through.
    provisional: f32,
    /// Recent frames' full-band peaks, for the provisional gain.
    recent: Ring,
    scratch: Vec<f32>,
    error: Option<EngineError>,
}

impl VadGate {
    fn hear(&mut self, x: f32, level: &mut Level) {
        self.window[self.pos] = (x * self.provisional).clamp(-1.0, 1.0);
        self.pos += 1;
        if self.pos == VAD_WINDOW {
            self.score();
            self.resolve(level);
        }
    }

    /// Scores the filled window and records its verdict. After an error the VAD is not asked
    /// again; every later window is not speech.
    fn score(&mut self) {
        let speech = self.error.is_none()
            && match self.source.probability(&self.window).and_then(vad::checked) {
                Ok(p) => {
                    self.segmenter.push(p);
                    self.segmenter.counts_as_speech(p)
                }
                Err(e) => {
                    self.error = Some(e);
                    false
                }
            };
        self.verdicts[(self.windows % VERDICTS as u64) as usize] = speech;
        self.windows += 1;
        self.pos = 0;
    }

    /// Frame `index` peaking at `peak` is complete: update the provisional gain, and queue the
    /// frame for the verdict of the window holding its middle sample.
    fn frame(&mut self, peak: f32, index: u64, level: &mut Level) {
        self.recent.push(peak);
        let n = self.recent.len();
        let recent = &mut self.scratch[..n];
        recent.copy_from_slice(self.recent.values());
        let robust = *recent
            .select_nth_unstable_by(TRANSIENT_FRAMES.min(n - 1), |a, b| b.total_cmp(a))
            .1;
        self.provisional = gain_for(robust);
        let middle = index * LEVEL_FRAME as u64 + LEVEL_FRAME as u64 / 2;
        if self.pending.len() == PENDING {
            // Cannot happen: a frame waits at most about one window. Were it to, the oldest
            // frame is not learned from, which only ever holds the gain.
            self.pending.pop_front();
        }
        self.pending.push_back((peak, middle / VAD_WINDOW as u64));
        self.resolve(level);
    }

    /// Learns from every waiting frame whose window has been scored and called speech.
    fn resolve(&mut self, level: &mut Level) {
        while let Some(&(peak, window)) = self.pending.front() {
            if window >= self.windows {
                break;
            }
            self.pending.pop_front();
            let kept = self.windows - window <= VERDICTS as u64;
            if kept && self.verdicts[(window % VERDICTS as u64) as usize] {
                level.learn(peak);
            }
        }
    }

    /// The stream ends: score the last, partial window (zero-padded), resolve what waits, and
    /// start the VAD afresh for the next stream.
    fn end_stream(&mut self, level: &mut Level) {
        if self.pos > 0 {
            self.window[self.pos..].fill(0.0);
            self.score();
        }
        self.resolve(level);
        self.pending.clear();
        self.segmenter = SpeechSegmenter::new(self.cfg);
        self.source.reset();
        self.windows = 0;
        self.pos = 0;
    }
}

/// Without a VAD: the level learns from frames with speech-band contrast.
struct ContrastGate {
    band: SpeechBandFilter,
    /// The filling block's speech-band energy so far.
    band_energy: f64,
    /// Recent frames' speech-band envelope (RMS).
    recent: Ring,
    scratch: Vec<f32>,
}

impl ContrastGate {
    fn hear(&mut self, x: f32) {
        let band = f64::from(self.band.process(x));
        self.band_energy += band * band;
    }

    fn frame(&mut self, peak: f32, band: f32, level: &mut Level) {
        self.recent.push(band);
        let (contrast_db, quiet) = contrast_in(self.recent.ordered(), &mut self.scratch);
        if contrast_db >= MIN_DYNAMICS_DB && band >= quiet * MIN_DYNAMICS {
            level.learn(peak);
        }
    }
}

/// RMS from a sum of squares over `len` samples.
fn band_rms(energy: f64, len: usize) -> f32 {
    (energy / len.max(1) as f64).sqrt() as f32
}

/// A fixed-capacity ring of recent values, allocated once.
struct Ring {
    values: Vec<f32>,
    next: usize,
    len: usize,
}

impl Ring {
    fn new(capacity: usize) -> Self {
        Self {
            values: vec![0.0; capacity],
            next: 0,
            len: 0,
        }
    }

    fn push(&mut self, v: f32) {
        self.values[self.next] = v;
        self.next = (self.next + 1) % self.values.len();
        self.len = (self.len + 1).min(self.values.len());
    }

    fn len(&self) -> usize {
        self.len
    }

    /// The stored values, in no particular order.
    fn values(&self) -> &[f32] {
        &self.values[..self.len]
    }

    /// The stored values, oldest first.
    fn ordered(&self) -> impl Iterator<Item = f32> + Clone + '_ {
        let (older, newer) = if self.len == self.values.len() {
            (&self.values[self.next..], &self.values[..self.next])
        } else {
            (&self.values[..self.len], &self.values[..0])
        };
        older.iter().chain(newer).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_most_recent_values() {
        let mut r = Ring::new(3);
        assert!(r.values().is_empty());
        for v in [5.0, 1.0, 4.0, 3.0] {
            r.push(v);
        }
        assert_eq!(r.len(), 3);
        let mut kept = r.values().to_vec();
        kept.sort_by(f32::total_cmp);
        assert_eq!(kept, vec![1.0, 3.0, 4.0]);
        r.push(6.0);
        let mut kept = r.values().to_vec();
        kept.sort_by(f32::total_cmp);
        assert_eq!(kept, vec![3.0, 4.0, 6.0], "1.0 has left the window");
    }

    #[test]
    fn the_ring_reads_back_oldest_first() {
        let mut r = Ring::new(3);
        r.push(1.0);
        r.push(2.0);
        assert_eq!(r.ordered().collect::<Vec<_>>(), vec![1.0, 2.0]);
        for v in [3.0, 4.0, 5.0] {
            r.push(v);
        }
        assert_eq!(r.ordered().collect::<Vec<_>>(), vec![3.0, 4.0, 5.0]);
    }
}
