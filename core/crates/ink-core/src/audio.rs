//! Audio types and the capture seam.

use crate::error::PlatformError;

/// The rate of everything downstream of capture: 16 kHz mono f32. Resampling happens once, in
/// `ink-audio`, so no later stage negotiates formats.
pub const CANONICAL_RATE: u32 = 16_000;

/// Which side of the conversation a stream carries.
///
/// Me versus them is stream identity, never a model (architecture rule 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    /// The user's microphone: "you", with no diarization error by construction.
    Mic,
    /// Everything the machine plays: "them". The only channel that is diarized.
    Far,
}

/// The format of a capture stream as the device delivers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFormat {
    /// Frames per second.
    pub sample_rate: u32,
    /// Interleaved samples per frame.
    pub channels: u16,
}

impl StreamFormat {
    /// 16 kHz mono, the format every stage after `ink-audio` sees.
    pub const CANONICAL: Self = Self {
        sample_rate: CANONICAL_RATE,
        channels: 1,
    };
}

/// One capture callback's audio, borrowed from the realtime thread for the length of a
/// [`AudioSink::push`] call.
#[derive(Clone, Copy, Debug)]
pub struct AudioBlock<'a> {
    /// Interleaved samples; the length is a whole number of frames.
    pub samples: &'a [f32],
    /// The stream's format.
    pub format: StreamFormat,
    /// Host time of the first frame, in nanoseconds on the [`Clock`](crate::clock::Clock)
    /// timebase. The mic and the far end are aligned by these stamps, so both must come from the
    /// same clock as `Clock::now_ns`.
    pub host_time_ns: u64,
}

impl AudioBlock<'_> {
    /// Frames in the block. A zero channel count is treated as mono rather than dividing by zero.
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.format.channels.max(1))
    }
}

/// Receives capture blocks.
///
/// **Realtime.** `push` runs on the OS audio callback thread: it must not allocate, lock, block or
/// log (I4). The real implementation is `ink-audio`'s ring producer, which copies the samples and
/// counts overruns instead of failing, so a slow consumer loses audio visibly, never the callback.
pub trait AudioSink: Send {
    /// Copies the block out and returns within the callback's deadline.
    fn push(&mut self, block: &AudioBlock<'_>);
}

/// What a source reports when it stops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceStats {
    /// Frames handed to the sink.
    pub frames: u64,
    /// Gaps in the device timestamps. Each one is audio lost upstream of the sink.
    pub discontinuities: u64,
}

/// A capture stream: a microphone, a system tap, a loopback, or a file replay.
///
/// Owned by one worker at a time (`Send`, not `Sync`). The source never allocates or locks on the
/// realtime thread itself; the sink is the only thing it calls there.
pub trait AudioSource: Send {
    /// Which side of the conversation this stream carries.
    fn channel(&self) -> Channel;

    /// The format `push` will deliver.
    fn format(&self) -> StreamFormat;

    /// **Worker.** Starts delivery. After `Ok`, the sink's `push` runs on the realtime thread
    /// until [`stop`](Self::stop) returns. Starting a started source is an error.
    fn start(&mut self, sink: Box<dyn AudioSink>) -> Result<(), PlatformError>;

    /// **Worker.** Stops delivery. When it returns, no `push` is running or will run, and the sink
    /// has been dropped. Stopping a stopped source returns zeroed stats.
    fn stop(&mut self) -> Result<SourceStats, PlatformError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_count_interleaved_samples() {
        let samples = [0.0f32; 960];
        let stereo = AudioBlock {
            samples: &samples,
            format: StreamFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            host_time_ns: 0,
        };
        assert_eq!(stereo.frames(), 480);
        let broken = AudioBlock {
            format: StreamFormat {
                sample_rate: 48_000,
                channels: 0,
            },
            ..stereo
        };
        assert_eq!(broken.frames(), 960);
    }
}
