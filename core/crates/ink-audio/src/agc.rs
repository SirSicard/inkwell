//! The meeting AGC: a slow gain toward the normaliser's [`TARGET_PEAK`] (architecture rule 11).
//!
//! A meeting has no utterance to measure first, so the gain follows the audio: 16 kHz mono, in
//! place, one 20 ms [`LEVEL_FRAME`] at a time.
//!
//! # What moves the gain
//!
//! Only **speech frames**: a frame whose peak is at least [`NOISE_FLOOR`] and stands
//! [`SPEECH_OVER_NOISE`] above the noise floor tracked over the last [`NOISE_WINDOW_FRAMES`] (the
//! quietest frame peak in that window, so it follows the pauses between syllables and rises to meet
//! a louder room within that window). The level is the normaliser's robust peak taken over the last
//! [`SPEECH_WINDOW_FRAMES`] speech frames: the same measure, the same [`TRANSIENT_FRAMES`] skipped,
//! the same target.
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
//! - **It never clips.** Each frame's gain is capped so its own peak stays at or below full scale.
//!   The one-frame look-ahead ([`Agc::LATENCY`]) is what makes that possible: a frame's gain is
//!   decided after the whole frame has been seen.
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

use crate::gain::{LEVEL_FRAME, MAX_GAIN, NOISE_FLOOR, TARGET_PEAK, TRANSIENT_FRAMES, frame_peak};

/// Level frames per second.
const FRAMES_PER_S: f32 = 50.0;

/// The fastest the gain rises, in dB per second, once it has locked.
pub const RISE_DB_PER_S: f32 = 6.0;

/// The fastest the gain falls, in dB per second (a louder talker must not wait).
pub const FALL_DB_PER_S: f32 = 60.0;

/// How far above the tracked noise floor (10 dB) a frame must peak to count as speech.
pub const SPEECH_OVER_NOISE: f32 = 3.162_277_7;

/// The noise floor is the quietest frame peak over this many frames (1.5 s).
pub const NOISE_WINDOW_FRAMES: usize = 75;

/// The level is measured over this many recent speech frames (3 s of speech).
pub const SPEECH_WINDOW_FRAMES: usize = 150;

/// Speech frames measured before the first lock (0.5 s).
pub const MIN_SPEECH_FRAMES: usize = 25;

/// A speech frame steps the gain only if it peaks within this many dB of the current speech level.
pub const ADAPT_RANGE_DB: f32 = 30.0;

/// The slow AGC for meetings. See the module docs.
pub struct Agc {
    /// The frame being filled with input.
    incoming: Vec<f32>,
    /// The previous frame, being handed out, gained, as `incoming` fills.
    outgoing: Vec<f32>,
    /// Position in both frames.
    pos: usize,
    /// The outgoing frame's gain ramp: from `ramp_from` to `ramp_to` across `ramp_len` samples.
    ramp_from: f32,
    ramp_to: f32,
    ramp_len: usize,
    /// The gain the audio is heading for (the state `gain()` reports).
    gain: f32,
    locked: bool,
    /// Recent frame peaks, for the noise floor.
    noise: Ring,
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
    /// The output is the input delayed by this many samples (one level frame, 20 ms).
    pub const LATENCY: usize = LEVEL_FRAME;

    /// An AGC at unity gain. **Allocates** (about 3 KB).
    pub fn new() -> Self {
        Self {
            incoming: vec![0.0; LEVEL_FRAME],
            outgoing: vec![0.0; LEVEL_FRAME],
            pos: 0,
            ramp_from: 1.0,
            ramp_to: 1.0,
            ramp_len: LEVEL_FRAME,
            gain: 1.0,
            locked: false,
            noise: Ring::new(NOISE_WINDOW_FRAMES),
            speech: Ring::new(SPEECH_WINDOW_FRAMES),
            scratch: vec![0.0; SPEECH_WINDOW_FRAMES],
        }
    }

    /// The gain the AGC is holding or heading for (linear, 1.0 to [`MAX_GAIN`]). A single loud
    /// frame may get less, to stay under full scale.
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
            *s = self.outgoing[self.pos] * self.ramp_at(self.pos);
            self.incoming[self.pos] = input;
            self.pos += 1;
            if self.pos == LEVEL_FRAME {
                self.end_frame(LEVEL_FRAME);
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
        for i in self.pos..LEVEL_FRAME {
            out.push(self.outgoing[i] * self.ramp_at(i));
        }
        let partial = self.pos;
        if partial > 0 {
            self.end_frame(partial);
            for i in 0..partial {
                out.push(self.outgoing[i] * self.ramp_at(i));
            }
        }
        self.incoming.fill(0.0);
        self.outgoing.fill(0.0);
        self.pos = 0;
    }

    fn ramp_at(&self, i: usize) -> f32 {
        if self.ramp_to > self.ramp_from {
            let t = (i + 1) as f32 / self.ramp_len as f32;
            self.ramp_from + (self.ramp_to - self.ramp_from) * t
        } else {
            self.ramp_to
        }
    }

    /// The first `len` samples of `incoming` are a complete frame: measure it, move the gain, set
    /// its ramp, and make it the outgoing frame.
    fn end_frame(&mut self, len: usize) {
        let peak = frame_peak(&self.incoming[..len]);
        self.noise.push(peak);
        let noise_floor = self.noise.min();
        if peak >= NOISE_FLOOR && peak >= noise_floor * SPEECH_OVER_NOISE {
            self.speech.push(peak);
            self.adapt(peak);
        }

        // Never clip: this frame's own peak must stay at or below full scale. Rising gain ramps up
        // to the frame's gain from the last one; falling gain applies at once (a ramp down would
        // start above the cap).
        let frame_gain = if peak > 0.0 {
            self.gain.min(1.0 / peak)
        } else {
            self.gain
        };
        self.ramp_from = self.ramp_to;
        self.ramp_to = frame_gain;
        self.ramp_len = len;
        std::mem::swap(&mut self.incoming, &mut self.outgoing);
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

    fn min(&self) -> f32 {
        self.values().iter().fold(f32::INFINITY, |m, &v| m.min(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_most_recent_values() {
        let mut r = Ring::new(3);
        assert_eq!(r.min(), f32::INFINITY);
        for v in [5.0, 1.0, 4.0, 3.0] {
            r.push(v);
        }
        assert_eq!(r.len(), 3);
        let mut kept = r.values().to_vec();
        kept.sort_by(f32::total_cmp);
        assert_eq!(kept, vec![1.0, 3.0, 4.0]);
        r.push(6.0);
        assert_eq!(r.min(), 3.0, "1.0 has left the window");
    }

    #[test]
    fn the_speech_constant_is_ten_db() {
        assert!((20.0 * SPEECH_OVER_NOISE.log10() - 10.0).abs() < 1e-4);
    }
}
