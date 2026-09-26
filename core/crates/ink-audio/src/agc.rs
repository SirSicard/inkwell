//! The meeting AGC: a slow gain toward the normaliser's [`TARGET_PEAK`] (architecture rule 11).
//!
//! A meeting has no utterance to measure first, so the gain follows the audio: 16 kHz mono, in
//! place, one 20 ms [`LEVEL_FRAME`] at a time.
//!
//! # What moves the gain
//!
//! Only **level frames**. The AGC runs the stationary test of [`speech_band`](crate::speech_band)
//! on the speech-band envelope of the last [`RECENT_FRAMES`] (1.5 s). When those are not
//! stationary, a frame whose own band envelope stands [`MIN_DYNAMICS_DB`] over their quiet frame,
//! and whose full-band peak is above [`NOISE_FLOOR`], counts toward the level. The level is the
//! normaliser's full-band robust peak taken over the last [`SPEECH_WINDOW_FRAMES`] such frames:
//! the same measure, the same [`TRANSIENT_FRAMES`] skipped, the same target.
//!
//! Like the normaliser, this decides level, not speech. What the stationary test asks, and the
//! committed tests that hold it (rumble and white room tone never lift the gain; speech and
//! breathy speech 8 dB over white noise or rumble reach the target), are in
//! [`speech_band`](crate::speech_band). Non-stationary noise that is not speech, such as a
//! cycling fan, can be lifted: the live engine then hears it, and what is speech is the VAD's and
//! the engine's call.
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
    LEVEL_FRAME, MAX_GAIN, MIN_DYNAMICS, NOISE_FLOOR, TARGET_PEAK, TRANSIENT_FRAMES, frame_peak,
};
use crate::speech_band::{SpeechBandFilter, Stationarity};

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

/// The stationary test looks at this many recent frames (1.5 s).
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
    /// The first block of a 20 ms level frame whose second block has not come yet: its peak, its
    /// speech-band energy and its length.
    half_frame: Option<(f32, f64, usize)>,
    /// The speech-band filter, run over the input as it arrives.
    band: SpeechBandFilter,
    /// The filling block's speech-band energy so far.
    band_energy: f64,
    /// The gain the audio is heading for (the state `gain()` reports).
    gain: f32,
    locked: bool,
    /// Recent frames' speech-band envelope (RMS), for the stationary test.
    recent: Ring,
    /// Scratch for ranking `recent`.
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

    /// An AGC at unity gain. **Allocates** its blocks and windows.
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
            band: SpeechBandFilter::new(),
            band_energy: 0.0,
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
            let band = f64::from(self.band.process(input));
            self.band_energy += band * band;
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
            let energy = std::mem::take(&mut self.band_energy);
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
        self.filling.fill(0.0);
        self.waiting.fill(0.0);
        self.playing.fill(0.0);
        self.waiting_peak = 0.0;
        self.pos = 0;
        self.band.reset();
        self.band_energy = 0.0;
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
        let energy = std::mem::take(&mut self.band_energy);
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

    /// A complete level frame: run the stationary test over the recent frames and, for a level
    /// frame, update the level and the gain. Allocation-free.
    fn end_frame(&mut self, peak: f32, band: f32) {
        self.recent.push(band);
        let (recent, quiet) =
            Stationarity::measure(self.recent.ordered(), &mut self.recent_scratch);
        if !recent.is_stationary() && peak >= NOISE_FLOOR && band >= quiet * MIN_DYNAMICS {
            self.speech.push(peak);
            self.adapt(peak);
        }
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
