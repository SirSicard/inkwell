//! Resampling to 16 kHz mono ([`CANONICAL_RATE`]), once, before every later stage.
//!
//! One windowed-sinc resampler (`rubato`'s `SincFixedIn`: 256 taps, Blackman-Harris², cutoff at
//! 95 % of the output's Nyquist) serves both paths:
//!
//! - [`StreamResampler`] is built **once per device stream**, when its format is known, and carries
//!   its filter history, its fractional position and any partial input chunk across pushes. How the
//!   pump slices the audio cannot change the result. A device that changes format gets a new one.
//! - [`resample`] is the same resampler run over a whole buffer and finished.
//!
//! [`finish`](StreamResampler::finish) ends a stream and leaves the resampler ready for the same
//! device's next one, so its tables are built once per device, not once per recording.
//!
//! # Alignment and the tail
//!
//! The kernel needs 128 input samples of lookahead. `rubato` 0.16 holds that lookahead back at the
//! **end** of the stream and does not delay the start: its output sample *m* sits at input position
//! *m·t + (t − 1)* (with *t* = input rate / 16 000), and its `output_delay()` counts output still
//! held back, not a head delay. An earlier implementation trimmed that count from the head, which
//! dropped half a kernel (128 input samples, 2.7 ms at 48 kHz) from the head of every take and
//! moved every event early (`a_click_lands_at_the_same_time_after_resampling`). Here the input is
//! pre-padded with *P* zeros and the first *S* outputs are skipped, with *P − S·t = t − 1* in whole
//! samples, so output *m* sits at input *m·t* (to 1/256 of an input sample).
//!
//! [`finish`](StreamResampler::finish) flushes the lookahead with zeros until exactly
//! [`resampled_len`] samples have come out. Without that flush the last ~2.7 ms (the end of the
//! last word) was lost.
//!
//! # Threading and allocation
//!
//! **Pump or worker.** [`StreamResampler::new`] allocates (the kernel tables and two chunk
//! buffers). [`push`](StreamResampler::push) and [`finish`](StreamResampler::finish) allocate
//! nothing beyond growing `out`; reserve [`max_output_frames`](StreamResampler::max_output_frames)
//! first and they allocate nothing at all.

use std::fmt;

use ink_core::CANONICAL_RATE;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

/// The highest input rate accepted.
pub const MAX_SOURCE_RATE: u32 = 384_000;

/// Kernel length in input samples.
const SINC_LEN: usize = 256;

/// Why resampling could not run.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResampleError {
    /// Zero, or above [`MAX_SOURCE_RATE`].
    InvalidRate(u32),
    /// The resampler library refused (it names why). Not expected with the buffers sized here.
    Resampler(String),
}

impl fmt::Display for ResampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRate(rate) => write!(
                f,
                "resampler: invalid input rate {rate} Hz (1..={MAX_SOURCE_RATE})"
            ),
            Self::Resampler(msg) => write!(f, "resampler: {msg}"),
        }
    }
}

impl std::error::Error for ResampleError {}

/// Output samples for `input_frames` at `source_rate`: the input's duration at 16 kHz, rounded
/// to the nearest sample. Zero for a zero rate.
pub fn resampled_len(input_frames: u64, source_rate: u32) -> u64 {
    if source_rate == 0 {
        return 0;
    }
    let num = u128::from(input_frames) * u128::from(CANONICAL_RATE) * 2 + u128::from(source_rate);
    let len = num / (u128::from(source_rate) * 2);
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Resamples a whole mono buffer at `source_rate` to 16 kHz. The output has exactly
/// [`resampled_len`] samples and is time-aligned with the input.
///
/// **Worker.** Allocates the output and the resampler.
pub fn resample(samples: &[f32], source_rate: u32) -> Result<Vec<f32>, ResampleError> {
    let mut stream = StreamResampler::new(source_rate)?;
    let len = usize::try_from(resampled_len(samples.len() as u64, source_rate))
        .map_err(|_| ResampleError::Resampler("output too long".into()))?;
    let mut out = Vec::with_capacity(len);
    stream.push(samples, &mut out)?;
    stream.finish(&mut out)?;
    Ok(out)
}

/// A resampler for one device stream, mono in, 16 kHz mono out. See the module docs.
pub struct StreamResampler {
    source_rate: u32,
    /// `None` at the canonical rate, which passes through untouched.
    sinc: Option<Box<Sinc>>,
    /// Real input frames pushed (the alignment pad not included).
    frames_in: u64,
    /// Output frames handed out.
    frames_out: u64,
}

struct Sinc {
    engine: SincFixedIn<f32>,
    /// One input chunk; `filled` of it holds audio waiting for the rest.
    input: Vec<f32>,
    filled: usize,
    /// Room for one chunk's output.
    output: Vec<f32>,
    /// Outputs still to drop from the head (the alignment's *S*).
    skip: usize,
    /// The alignment: *S* outputs skipped after *P* zeros of pad, per stream.
    align_skip: usize,
    align_pad: usize,
    /// Output frames the stream may be holding back at any moment.
    held_back: usize,
}

impl StreamResampler {
    /// A resampler from `source_rate` to 16 kHz. **Allocates.**
    pub fn new(source_rate: u32) -> Result<Self, ResampleError> {
        if source_rate == 0 || source_rate > MAX_SOURCE_RATE {
            return Err(ResampleError::InvalidRate(source_rate));
        }
        let sinc = if source_rate == CANONICAL_RATE {
            None
        } else {
            Some(Box::new(Sinc::new(source_rate)?))
        };
        Ok(Self {
            source_rate,
            sinc,
            frames_in: 0,
            frames_out: 0,
        })
    }

    /// The input rate this resampler was built for.
    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }

    /// The most output a push of `input_frames` can append (a bound for reserving `out`).
    pub fn max_output_frames(&self, input_frames: usize) -> usize {
        let due = resampled_len(self.frames_in + input_frames as u64, self.source_rate);
        let due = usize::try_from(due.saturating_sub(self.frames_out)).unwrap_or(usize::MAX);
        due.saturating_add(2)
    }

    /// The most output frames that can be due (by the input's duration) and not yet handed out,
    /// after any push: one input chunk (20 ms) plus the kernel's lookahead, under 23 ms at 48 kHz
    /// and under 40 ms at any rate (`streaming_output_keeps_up_with_its_input`).
    /// Zero at 16 kHz.
    pub fn max_held_back_frames(&self) -> usize {
        self.sinc.as_ref().map_or(0, |s| s.held_back)
    }

    /// Resamples `input` and appends every output sample it completes to `out`.
    ///
    /// **Pump or worker.** Allocation-free when `out` has room for
    /// [`max_output_frames`](Self::max_output_frames)`(input.len())` more samples.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), ResampleError> {
        self.frames_in += input.len() as u64;
        let Some(sinc) = self.sinc.as_deref_mut() else {
            out.extend_from_slice(input);
            self.frames_out += input.len() as u64;
            return Ok(());
        };
        let mut rest = input;
        while !rest.is_empty() {
            let n = (sinc.input.len() - sinc.filled).min(rest.len());
            sinc.input[sinc.filled..sinc.filled + n].copy_from_slice(&rest[..n]);
            sinc.filled += n;
            rest = &rest[n..];
            if sinc.filled == sinc.input.len() {
                self.frames_out += sinc.run_chunk(out, u64::MAX - self.frames_out)?;
            }
        }
        Ok(())
    }

    /// Flushes the kernel's lookahead with zeros and appends the rest of the stream, so the whole
    /// stream has produced exactly [`resampled_len`] samples. The end of the signal is kept.
    ///
    /// The resampler is then ready for a new stream at the same rate, as if just built.
    ///
    /// **Pump or worker.** Allocation-free when `out` has room for
    /// [`max_output_frames`](Self::max_output_frames)`(0)` more samples.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), ResampleError> {
        let Some(sinc) = self.sinc.as_deref_mut() else {
            self.frames_in = 0;
            self.frames_out = 0;
            return Ok(());
        };
        let expected = resampled_len(self.frames_in, self.source_rate);
        // Each flushed chunk produces about a chunk's worth of output, so this ends within a few
        // chunks; the bound turns a resampler that stopped producing into an error, not a hang.
        let mut rounds = 0;
        while self.frames_out < expected {
            if rounds > 4 + sinc.held_back {
                return Err(ResampleError::Resampler(
                    "the flush produced no output".into(),
                ));
            }
            rounds += 1;
            sinc.input[sinc.filled..].fill(0.0);
            sinc.filled = sinc.input.len();
            self.frames_out += sinc.run_chunk(out, expected - self.frames_out)?;
        }
        sinc.rearm()?;
        self.frames_in = 0;
        self.frames_out = 0;
        Ok(())
    }
}

impl Sinc {
    fn new(source_rate: u32) -> Result<Self, ResampleError> {
        let ratio = f64::from(CANONICAL_RATE) / f64::from(source_rate);
        // 20 ms of input per chunk: the push-to-output latency is at most this plus the lookahead.
        let chunk = (source_rate as usize / 50).max(32);
        let parameters = SincInterpolationParameters {
            sinc_len: SINC_LEN,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };
        let engine = SincFixedIn::<f32>::new(ratio, 1.0, parameters, chunk, 1)
            .map_err(|e| ResampleError::Resampler(e.to_string()))?;
        let output = vec![0.0; engine.output_frames_max()];

        // Alignment (module docs): output m of the raw resampler sits at input m·t + (t − 1).
        // Pre-pad P zeros and skip S outputs with P − S·t = t − 1, all whole numbers: with
        // g = gcd(rate, 16000), S + 1 = 16000/g and P = rate/g − 1. For 48 kHz that is P = 2,
        // S = 0; for 44.1 kHz P = 440, S = 159 (10 ms of pad).
        let g = gcd(source_rate, CANONICAL_RATE);
        let skip = (CANONICAL_RATE / g - 1) as usize;
        let pad = (source_rate / g - 1) as usize;

        // What can be due but not out. The resampler emits an output once its input is followed by
        // half a kernel plus 2 + ⌈t⌉ samples (its loop bound), and a partial chunk waits whole. The
        // pad and skip cancel (that is the alignment), and rounding adds up to 2.
        let lookahead = SINC_LEN / 2 + 2 + (1.0 / ratio).ceil() as usize;
        let held_back = ((chunk + lookahead) as f64 * ratio).ceil() as usize + 2;

        let mut sinc = Self {
            engine,
            input: vec![0.0; chunk],
            filled: 0,
            output,
            skip,
            align_skip: skip,
            align_pad: pad,
            held_back,
        };
        sinc.prime()?;
        Ok(sinc)
    }

    /// Clears all state for a new stream and primes it. Allocation-free.
    fn rearm(&mut self) -> Result<(), ResampleError> {
        self.engine.reset();
        self.filled = 0;
        self.skip = self.align_skip;
        self.prime()
    }

    /// Feeds the alignment pad. It is longer than a chunk only at rates far from 16 kHz's
    /// multiples, where whole chunks of it are run through here. Everything they produce must
    /// fall inside the skip (the pad is shorter than the skip's worth of input); anything more
    /// means the alignment arithmetic is wrong, and is an error rather than audio.
    fn prime(&mut self) -> Result<(), ResampleError> {
        let mut pad_left = self.align_pad;
        while pad_left > 0 {
            let n = (self.input.len() - self.filled).min(pad_left);
            self.input[self.filled..self.filled + n].fill(0.0);
            self.filled += n;
            pad_left -= n;
            if self.filled == self.input.len() {
                let produced = self.process_chunk()?;
                if produced > self.skip {
                    return Err(ResampleError::Resampler(
                        "alignment pad produced output".into(),
                    ));
                }
                self.skip -= produced;
            }
        }
        Ok(())
    }

    /// Resamples the full input chunk into `output`; returns how many samples it produced.
    fn process_chunk(&mut self) -> Result<usize, ResampleError> {
        let (_, produced) = self
            .engine
            .process_into_buffer(&[&self.input[..]], &mut [&mut self.output[..]], None)
            .map_err(|e| ResampleError::Resampler(e.to_string()))?;
        self.filled = 0;
        Ok(produced)
    }

    /// Resamples the full input chunk, drops what the alignment skips, and appends at most `limit`
    /// samples to `out`. Returns how many were appended.
    fn run_chunk(&mut self, out: &mut Vec<f32>, limit: u64) -> Result<u64, ResampleError> {
        let produced = self.process_chunk()?;
        let produced = &self.output[..produced];
        let skipped = self.skip.min(produced.len());
        self.skip -= skipped;
        let kept = &produced[skipped..];
        let n = kept.len().min(usize::try_from(limit).unwrap_or(usize::MAX));
        out.extend_from_slice(&kept[..n]);
        Ok(n as u64)
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampled_len_rounds_to_the_nearest_sample() {
        assert_eq!(resampled_len(48_000, 48_000), 16_000);
        assert_eq!(resampled_len(1, 48_000), 0); // 0.33 rounds down
        assert_eq!(resampled_len(2, 48_000), 1); // 0.67 rounds up
        assert_eq!(resampled_len(441, 44_100), 160);
        assert_eq!(resampled_len(5, 8_000), 10);
        assert_eq!(resampled_len(5, 0), 0);
    }

    #[test]
    fn the_alignment_pad_and_skip_satisfy_their_equation() {
        for rate in [
            8_000u32, 11_025, 22_050, 24_000, 32_000, 44_100, 48_000, 96_000,
        ] {
            let g = gcd(rate, CANONICAL_RATE);
            let (skip, pad) = (u64::from(CANONICAL_RATE / g - 1), u64::from(rate / g - 1));
            // P − S·t = t − 1, multiplied through by 16000: 16000·P − S·rate = rate − 16000.
            assert_eq!(
                16_000 * pad as i64 - (skip * u64::from(rate)) as i64,
                i64::from(rate) - 16_000,
                "{rate} Hz"
            );
        }
    }
}
