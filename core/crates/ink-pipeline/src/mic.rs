//! Stage 2 for dictation: the mic stream after the capture ring, turned into 16 kHz mono with the
//! host time of every block.
//!
//! ```text
//! ring ─► CapturedBlock (device format) ─► Downmix::Primary ─► StreamResampler ─► 16 kHz mono
//! ```
//!
//! The host time is what lets a key press, stamped on the same clock, be placed on the sample
//! timeline: the dictation chain turns it into a sample position. The resampler is time-aligned
//! (its output sample *j* sits at input time *j / 16 kHz*), so each block's host time maps the
//! whole output exactly, and a dropped block upstream moves nothing downstream of it.

use std::fmt;

use ink_audio::{Downmix, ResampleError, StreamResampler};
use ink_core::{AudioBlock, CANONICAL_RATE, Channel, StreamFormat};

/// One block of the mic in the canonical format.
#[derive(Clone, Copy, Debug)]
pub struct Canonical<'a> {
    /// 16 kHz mono. Borrowed until the next call on the path.
    pub samples: &'a [f32],
    /// Host time of the first sample, ns on the platform clock. When the block produced no
    /// output yet (the resampler is filling), the time the next output sample will have.
    pub host_time_ns: u64,
}

/// Why the mic path failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MicPathError {
    /// The device changed format mid-stream; build a new path for the new format.
    FormatChanged {
        /// What the path was built for.
        expected: StreamFormat,
        /// What arrived.
        got: StreamFormat,
    },
    /// A format with no channels.
    NoChannels,
    /// A zero rate, or a rate or buffer the resampler refuses.
    Resample(ResampleError),
}

impl fmt::Display for MicPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FormatChanged { expected, got } => write!(
                f,
                "mic format changed from {} Hz x{} to {} Hz x{}",
                expected.sample_rate, expected.channels, got.sample_rate, got.channels
            ),
            Self::NoChannels => f.write_str("mic format has no channels"),
            Self::Resample(e) => write!(f, "mic resampler: {e}"),
        }
    }
}

impl std::error::Error for MicPathError {}

impl From<ResampleError> for MicPathError {
    fn from(e: ResampleError) -> Self {
        Self::Resample(e)
    }
}

/// The mic's downmix and resampler, built once per device stream.
///
/// **Pump or worker.** `push` allocates nothing once its buffers have grown to the device's block
/// size.
pub struct MicPath {
    format: StreamFormat,
    resampler: StreamResampler,
    mono: Vec<f32>,
    out: Vec<f32>,
    frames_in: u64,
    frames_out: u64,
}

impl MicPath {
    /// A path for a mic stream in `format`. **Allocates.**
    pub fn new(format: StreamFormat) -> Result<Self, MicPathError> {
        if format.channels == 0 {
            return Err(MicPathError::NoChannels);
        }
        Ok(Self {
            format,
            resampler: StreamResampler::new(format.sample_rate)?,
            mono: Vec::new(),
            out: Vec::new(),
            frames_in: 0,
            frames_out: 0,
        })
    }

    /// Converts one capture block. A block in another format is refused, never guessed at.
    pub fn push(&mut self, block: &AudioBlock<'_>) -> Result<Canonical<'_>, MicPathError> {
        if block.format != self.format {
            return Err(MicPathError::FormatChanged {
                expected: self.format,
                got: block.format,
            });
        }
        self.mono.clear();
        Downmix::for_channel(Channel::Mic).apply(
            block.samples,
            block.format.channels,
            &mut self.mono,
        );
        self.out.clear();
        self.out
            .reserve(self.resampler.max_output_frames(self.mono.len()));
        self.resampler.push(&self.mono, &mut self.out)?;

        // Output sample `frames_out` sits at input frame `frames_out * rate / 16 kHz`; the block's
        // first frame (input frame `frames_in`) sits at `block.host_time_ns`.
        let ns = |frames: u64, rate: u32| i128::from(frames) * 1_000_000_000 / i128::from(rate);
        let host = i128::from(block.host_time_ns) + ns(self.frames_out, CANONICAL_RATE)
            - ns(self.frames_in, self.format.sample_rate);
        self.frames_in += self.mono.len() as u64;
        self.frames_out += self.out.len() as u64;
        Ok(Canonical {
            samples: &self.out,
            host_time_ns: u64::try_from(host.max(0)).unwrap_or(u64::MAX),
        })
    }

    /// Ends the stream: the resampler's last samples. The path is then ready for a new stream in
    /// the same format.
    pub fn finish(&mut self) -> Result<&[f32], MicPathError> {
        self.out.clear();
        self.out.reserve(self.resampler.max_output_frames(0));
        self.resampler.finish(&mut self.out)?;
        self.frames_in = 0;
        self.frames_out = 0;
        Ok(&self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(samples: &[f32], format: StreamFormat, host_time_ns: u64) -> AudioBlock<'_> {
        AudioBlock {
            samples,
            format,
            host_time_ns,
        }
    }

    const STEREO_48K: StreamFormat = StreamFormat {
        sample_rate: 48_000,
        channels: 2,
    };

    #[test]
    fn the_primary_channel_is_kept_and_times_follow_the_blocks() {
        let mut path = MicPath::new(STEREO_48K).unwrap();
        // Channel 0 carries a tone, channel 1 its inverse: an average would be silence.
        let mut interleaved = Vec::new();
        for i in 0..4_800 {
            let s = (i as f32 * 0.05).sin() * 0.5;
            interleaved.extend([s, -s]);
        }
        let mut total = 0;
        let mut energy = 0.0f32;
        for (k, chunk) in interleaved.chunks(960).enumerate() {
            let host = 1_000_000_000 + k as u64 * 10_000_000;
            let out = path.push(&block(chunk, STEREO_48K, host)).unwrap();
            // The first output sample of each block is never later than the block itself.
            assert!(out.host_time_ns <= host, "{} > {host}", out.host_time_ns);
            total += out.samples.len();
            energy += out.samples.iter().map(|s| s * s).sum::<f32>();
        }
        total += path.finish().unwrap().len();
        assert_eq!(total, 1_600);
        assert!(energy > 1.0, "the primary channel came through");
    }

    #[test]
    fn a_format_change_is_refused() {
        let mut path = MicPath::new(STEREO_48K).unwrap();
        let mono = StreamFormat {
            sample_rate: 48_000,
            channels: 1,
        };
        assert!(matches!(
            path.push(&block(&[0.0; 480], mono, 0)),
            Err(MicPathError::FormatChanged { .. })
        ));
        assert!(
            MicPath::new(StreamFormat {
                sample_rate: 48_000,
                channels: 0
            })
            .is_err()
        );
        assert!(
            MicPath::new(StreamFormat {
                sample_rate: 0,
                channels: 1
            })
            .is_err()
        );
    }

    #[test]
    fn at_the_canonical_rate_times_pass_through() {
        let mut path = MicPath::new(StreamFormat::CANONICAL).unwrap();
        let a = path
            .push(&block(&[0.1; 160], StreamFormat::CANONICAL, 5_000))
            .unwrap()
            .host_time_ns;
        assert_eq!(a, 5_000);
        let b = path
            .push(&block(&[0.1; 160], StreamFormat::CANONICAL, 10_005_000))
            .unwrap()
            .host_time_ns;
        assert_eq!(b, 10_005_000);
    }
}
