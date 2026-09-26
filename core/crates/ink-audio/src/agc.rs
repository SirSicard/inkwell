//! The meeting AGC: a slow gain toward the normaliser's [`TARGET_PEAK`] (architecture rule 11).
//!
//! A meeting has no utterance to measure first, so the gain follows the audio: 16 kHz mono, in
//! place, one 20 ms [`LEVEL_FRAME`] at a time.
//!
//! # What moves the gain
//!
//! Only **level frames**. The AGC applies the normaliser's own stationary guard to the last
//! [`RECENT_FRAMES`] (1.5 s): if the robust peak there stands at least [`MIN_DYNAMICS_DB`] (4 dB)
//! over the 10th-percentile frame, the audio is not stationary, and a frame that itself peaks at
//! least 4 dB over that quiet level (and above [`NOISE_FLOOR`]) counts toward the level. The level
//! is the normaliser's robust peak taken over the last [`SPEECH_WINDOW_FRAMES`] such frames: the
//! same measure, the same [`TRANSIENT_FRAMES`] skipped, the same target.
//!
//! Like the normaliser, this decides level, not speech. Low-crest speech (continuous, breathy, in
//! noise) is lifted: its loud frames stand only 5–9 dB over its quiet ones, which an earlier 10 dB
//! frame test never let through, so it stayed at −75 dBFS. White room tone stays below the line,
//! however long it runs, because the window is fixed: its 1.5 s contrast measured at most 3.2 dB
//! over 30 minutes, and low-passed rumble at four cutoffs did not lift the gain in 2 minutes each.
//! Bursty noise (knocks, a cycling fan) does cross it, as it crosses the normaliser's: the live
//! engine then hears it lifted, and what is speech is the VAD's and the engine's call.
//!
//! - **Pauses hold the gain.** Silence, room tone and hum are not speech frames: nothing is learned
//!   from them and the gain stays where the last speech left it. The first word after a pause
//!   comes out at the level the last word did, not pumped up. For up to one noise window after
//!   speech whose gaps were digital silence (a far end that gates its output), room tone can pass
//!   the speech test; it still never moves the gain, because a frame more than [`ADAPT_RANGE_DB`]
//!   below the current speech level only joins the level window (so a much quieter talker takes
//!   over once the window has turned over) and never steps the gain itself.
//! - **It stops rising at the noise floor.** Audio that does not stand out from its own noise
//!   floor never raises the gain, so a quiet room is never lifted toward the target.
//! - **Slow up, fast down.** The gain rises at most [`RISE_DB_PER_S`] and falls at most
//!   [`FALL_DB_PER_S`]. The first lock is the exception: once [`MIN_SPEECH_FRAMES`] of speech have
//!   been measured the gain jumps straight to its target, so a quiet talker is lifted from their
//!   first half second rather than over ten.
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
//! **Pump or worker.** [`Agc::new`] allocates its frame buffers and windows; [`process`] never
//! allocates, and [`flush`] only grows its `out` (reserve [`Agc::LATENCY`] samples first).
//!
//! [`process`]: Agc::process
//! [`flush`]: Agc::flush

use crate::gain::{
    LEVEL_FRAME, MAX_GAIN, MIN_DYNAMICS, NOISE_FLOOR, QUIET_PERCENTILE, TARGET_PEAK,
    TRANSIENT_FRAMES, frame_peak,
};

#[cfg(doc)]
use crate::gain::MIN_DYNAMICS_DB;

/// The limiter's block (10 ms).
pub const LIMITER_BLOCK: usize = LEVEL_FRAME / 2;

/// Level frames per second.
const FRAMES_PER_S: f32 = 50.0;

/// The fastest the gain rises, in dB per second, once it has locked.
pub const RISE_DB_PER_S: f32 = 6.0;

/// The fastest the gain falls, in dB per second (a louder talker must not wait).
pub const FALL_DB_PER_S: f32 = 60.0;

/// The stationary guard looks at this many recent frames (1.5 s).
pub const RECENT_FRAMES: usize = 75;

/// The level is measured over this many recent speech frames (3 s of speech).
pub const SPEECH_WINDOW_FRAMES: usize = 150;

/// Speech frames measured before the first lock (0.5 s).
pub const MIN_SPEECH_FRAMES: usize = 25;

/// A speech frame steps the gain only if it peaks within this many dB of the current speech level.
pub const ADAPT_RANGE_DB: f32 = 30.0;

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
    /// The first block's peak of a 20 ms level frame whose second block has not come yet.
    half_frame: Option<f32>,
    /// The gain the audio is heading for (the state `gain()` reports).
    gain: f32,
    locked: bool,
    /// Recent frame peaks, for the stationary guard.
    recent: Ring,
    /// Scratch for selecting levels out of `recent`.
    recent_scratch: Vec<f32>,
    /// Recent speech frame peaks, for the level.
    speech: Ring,
    /// Scratch for selecting the robust peak out of `speech`.
    scratch: Vec<f32>,
}

impl Default for Agc {
    fn default() -> Self {
        Self::new()
    }
}

impl Agc {
    /// The output is the input delayed by this many samples (two limiter blocks, 20 ms).
    pub const LATENCY: usize = 2 * LIMITER_BLOCK;

    /// An AGC at unity gain. **Allocates** (about 3 KB).
    pub fn new() -> Self {
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
            gain: 1.0,
            locked: false,
            recent: Ring::new(RECENT_FRAMES),
            recent_scratch: vec![0.0; RECENT_FRAMES],
            speech: Ring::new(SPEECH_WINDOW_FRAMES),
            scratch: vec![0.0; SPEECH_WINDOW_FRAMES],
        }
    }

    /// The gain the AGC is holding or heading for (linear, 1.0 to [`MAX_GAIN`]). A loud block
    /// is played with less, to stay under full scale, without changing this.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Replaces `samples` (16 kHz mono) with the gained audio [`LATENCY`](Self::LATENCY) samples
    /// earlier. The first `LATENCY` samples a new AGC puts out are silence.
    ///
    /// **Pump or worker.** Never allocates.
    pub fn process(&mut self, samples: &mut [f32]) {
        for s in samples {
            let input = *s;
            *s = self.playing[self.pos] * self.ramp_at(self.pos);
            self.filling[self.pos] = input;
            self.pos += 1;
            if self.pos == LIMITER_BLOCK {
                self.end_block();
                self.pos = 0;
            }
        }
    }

    /// Appends the last [`LATENCY`](Self::LATENCY) samples still held by the look-ahead, gained,
    /// so everything pushed has come out. The gain and the level history are kept: the AGC can go
    /// on after a flush (its next output starts after another `LATENCY` of silence).
    ///
    /// **Pump or worker.** Allocation-free when `out` has room for `LATENCY` more samples.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        for i in self.pos..LIMITER_BLOCK {
            out.push(self.playing[i] * self.ramp_at(i));
        }
        // The waiting block plays next, ramping toward the partial block after it (or, with no
        // partial block, toward nothing: its own safe gain).
        let partial = self.pos;
        let partial_peak = frame_peak(&self.filling[..partial]);
        if partial > 0 {
            self.measure_block(partial_peak);
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
        if partial > 0 {
            self.set_ramp(next_safe, partial);
            for i in 0..partial {
                out.push(self.filling[i] * self.ramp_at(i));
            }
        }
        // A level frame left half measured is measured as it is.
        if let Some(first) = self.half_frame.take() {
            self.end_frame(first);
        }
        self.filling.fill(0.0);
        self.waiting.fill(0.0);
        self.playing.fill(0.0);
        self.waiting_peak = 0.0;
        self.pos = 0;
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
            self.gain.min(1.0 / peak).max(f32::MIN_POSITIVE)
        } else {
            self.gain
        }
    }

    /// `filling` is a complete block: measure it, then play the waiting block with a ramp that
    /// ends at or below both its own safe gain and this block's, and rotate the three buffers.
    fn end_block(&mut self) {
        let peak = frame_peak(&self.filling);
        self.measure_block(peak);
        let to = self.safe(self.waiting_peak).min(self.safe(peak));
        self.set_ramp(to, LIMITER_BLOCK);
        // playing <- waiting <- filling <- (the old playing, reused)
        std::mem::swap(&mut self.playing, &mut self.waiting);
        std::mem::swap(&mut self.waiting, &mut self.filling);
        self.waiting_peak = peak;
    }

    /// Two limiter blocks make one 20 ms level frame.
    fn measure_block(&mut self, peak: f32) {
        match self.half_frame.take() {
            None => self.half_frame = Some(peak),
            Some(first) => self.end_frame(first.max(peak)),
        }
    }

    /// A complete level frame peaking at `peak`: run the stationary guard over the recent frames
    /// and, for a level frame, update the level and the gain.
    fn end_frame(&mut self, peak: f32) {
        self.recent.push(peak);
        let (robust, quiet) = self.recent_levels();
        let dynamic = robust >= NOISE_FLOOR && robust >= quiet * MIN_DYNAMICS;
        if dynamic && peak >= NOISE_FLOOR && peak >= quiet * MIN_DYNAMICS {
            self.speech.push(peak);
            self.adapt(peak);
        }
    }

    /// The robust peak and the quiet (10th-percentile) frame of the recent frames, measured as
    /// [`levels`](crate::gain::levels) measures a buffer. Allocation-free.
    fn recent_levels(&mut self) -> (f32, f32) {
        let n = self.recent.len();
        let window = &mut self.recent_scratch[..n];
        window.copy_from_slice(self.recent.values());
        let loudest_first = |a: &f32, b: &f32| b.total_cmp(a);
        let robust = *window
            .select_nth_unstable_by(TRANSIENT_FRAMES.min(n - 1), loudest_first)
            .1;
        let quiet = *window
            .select_nth_unstable_by(n - 1 - n * QUIET_PERCENTILE / 100, loudest_first)
            .1;
        (robust, quiet)
    }

    /// Moves the gain toward the robust peak of recent speech, after a speech frame peaking at
    /// `peak` joined the window.
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
    fn recent_levels_measure_as_the_normaliser_does() {
        // Twenty frames peaking 1..=20 thousandths: the same numbers `gain::levels` gives.
        let mut agc = Agc::new();
        for k in 1..=20 {
            agc.recent.push(k as f32 / 1000.0);
        }
        assert_eq!(agc.recent_levels(), (0.012, 0.003));
    }
}
