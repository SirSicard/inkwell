//! `FileReplaySource`: the one replay harness (architecture rule 7).
//!
//! It plays a WAV fixture into any [`AudioSink`] the way a capture device does: fixed-size blocks,
//! from its own thread, each stamped with a host time. The stamps are synthetic and exact (the
//! clock's time at [`start`](AudioSource::start) plus the frames delivered so far at the file's
//! rate), so the same fixture and the same start time give the same blocks with the same stamps,
//! on any machine and any OS, however the threads are scheduled.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ink_core::{
    AudioBlock, AudioSink, AudioSource, Channel, Clock, PlatformError, SourceStats, StreamFormat,
};

use crate::rate::frames_to_ns;
use crate::realtime::{RealtimeGuard, unguarded};

/// How fast a replay delivers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pacing {
    /// As fast as the sink takes the blocks. Host times are still those of real-time capture.
    #[default]
    Unpaced,
    /// One block per block duration of wall time, like a device. The waiting happens between
    /// pushes, never inside one.
    RealTime,
}

/// A capture source that plays a WAV file (or samples in memory).
///
/// The whole file is loaded when it is opened: replay is for fixtures and bench clips, and a
/// capture session never holds more than seconds in memory (rule 3).
pub struct FileReplaySource {
    channel: Channel,
    format: StreamFormat,
    true_rate: u32,
    samples: Arc<[f32]>,
    block_frames: usize,
    clock: Arc<dyn Clock>,
    pacing: Pacing,
    guard: RealtimeGuard,
    running: Option<Running>,
}

struct Running {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<u64>,
}

impl FileReplaySource {
    /// Loads a WAV file (PCM of 8 to 32 bits, or 32-bit float) as the `channel` stream. Host times
    /// start at `clock.now_ns()` when the replay starts.
    ///
    /// **Worker.**
    pub fn open(
        path: impl AsRef<Path>,
        channel: Channel,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, PlatformError> {
        let path = path.as_ref();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fail = |e: hound::Error| PlatformError::Failed(format!("replay fixture {name}: {e}"));
        let mut reader = hound::WavReader::open(path).map_err(fail)?;
        let spec = reader.spec();
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<Result<_, _>>()
                .map_err(fail)?,
            hound::SampleFormat::Int => {
                if !(1..=32).contains(&spec.bits_per_sample) {
                    return Err(PlatformError::Failed(format!(
                        "replay fixture {name}: {}-bit samples",
                        spec.bits_per_sample
                    )));
                }
                // Full scale is 2^(bits-1): a power of two, so 16-bit samples convert exactly.
                let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 * scale))
                    .collect::<Result<_, _>>()
                    .map_err(fail)?
            }
        };
        let format = StreamFormat {
            sample_rate: spec.sample_rate,
            channels: spec.channels,
        };
        Self::from_samples(samples, format, channel, clock)
    }

    /// A replay of interleaved samples already in memory, for synthetic signals.
    pub fn from_samples(
        samples: impl Into<Arc<[f32]>>,
        format: StreamFormat,
        channel: Channel,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, PlatformError> {
        let samples = samples.into();
        if format.sample_rate == 0 || format.channels == 0 {
            return Err(PlatformError::Failed(format!(
                "replay: invalid format ({} Hz, {} channels)",
                format.sample_rate, format.channels
            )));
        }
        if samples.len() % usize::from(format.channels) != 0 {
            return Err(PlatformError::Failed(
                "replay: the samples are not a whole number of frames".into(),
            ));
        }
        Ok(Self {
            channel,
            format,
            true_rate: format.sample_rate,
            samples,
            block_frames: (format.sample_rate / 50).max(1) as usize,
            clock,
            pacing: Pacing::default(),
            guard: unguarded(),
            running: None,
        })
    }

    /// Frames per block. The default is 20 ms at the file's rate, the block size an earlier
    /// implementation measured from Core Audio (320 frames at 16 kHz). The last block may be
    /// shorter: the replay never invents audio to pad it.
    pub fn with_block_frames(mut self, frames: usize) -> Self {
        self.block_frames = frames.max(1);
        self
    }

    /// Sets the pacing. The default is [`Pacing::Unpaced`].
    pub fn with_pacing(mut self, pacing: Pacing) -> Self {
        self.pacing = pacing;
        self
    }

    /// Runs each block's delivery through `guard` on the replay thread, as a device runs its
    /// callback; tests pass a no-alloc guard (I4).
    pub fn with_realtime_guard(mut self, guard: RealtimeGuard) -> Self {
        self.guard = guard;
        self
    }

    /// Declares `hz` in every block while host time keeps advancing at the file's real rate: a
    /// device that mislabels its rate, for testing the sample-rate check.
    pub fn declaring_rate(mut self, hz: u32) -> Self {
        self.format.sample_rate = hz;
        self
    }

    /// Frames in the file.
    pub fn total_frames(&self) -> u64 {
        (self.samples.len() / usize::from(self.format.channels)) as u64
    }

    /// **Worker.** Waits until the whole file has been delivered, then stops as
    /// [`stop`](AudioSource::stop) does. On a source that is not running it returns zeroed stats.
    pub fn wait(&mut self) -> Result<SourceStats, PlatformError> {
        self.join(false)
    }

    fn join(&mut self, stop: bool) -> Result<SourceStats, PlatformError> {
        let Some(running) = self.running.take() else {
            return Ok(SourceStats::default());
        };
        if stop {
            running.stop.store(true, Ordering::Release);
        }
        // Joining is the guarantee `stop` gives: the thread finishes the push in progress, drops
        // the sink, and only then exits.
        let frames = running.thread.join().map_err(|_| {
            PlatformError::Failed("replay: the sink panicked on the replay thread".into())
        })?;
        Ok(SourceStats {
            frames,
            discontinuities: 0,
        })
    }
}

impl Drop for FileReplaySource {
    fn drop(&mut self) {
        // Never leave a thread pushing into a sink nobody will stop. A sink panic has nobody to
        // be reported to here; `stop` and `wait` report it.
        let _ = self.join(true);
    }
}

/// Everything the replay thread owns.
struct Delivery {
    samples: Arc<[f32]>,
    format: StreamFormat,
    true_rate: u32,
    block_frames: u64,
    start_ns: u64,
    pacing: Pacing,
    guard: RealtimeGuard,
    stop: Arc<AtomicBool>,
}

impl Delivery {
    /// The replay thread: delivers blocks until the file ends or `stop` is set, then drops the
    /// sink. Returns the frames delivered.
    fn run(self, mut sink: Box<dyn AudioSink>) -> u64 {
        let channels = usize::from(self.format.channels);
        let total = (self.samples.len() / channels) as u64;
        let started = Instant::now();
        let mut done = 0u64;
        while done < total && !self.stop.load(Ordering::Acquire) {
            if self.pacing == Pacing::RealTime {
                let due = started + Duration::from_nanos(frames_to_ns(done, self.true_rate));
                let now = Instant::now();
                if due > now {
                    std::thread::sleep(due - now);
                }
            }
            // One device callback: everything from here to the end of the push runs under the
            // guard, so a test's no-alloc guard covers the source's own work too.
            let mut deliver = || {
                let n = (total - done).min(self.block_frames);
                let from = done as usize * channels;
                let to = (done + n) as usize * channels;
                sink.push(&AudioBlock {
                    samples: &self.samples[from..to],
                    format: self.format,
                    host_time_ns: self
                        .start_ns
                        .saturating_add(frames_to_ns(done, self.true_rate)),
                });
                done += n;
            };
            (self.guard)(&mut deliver);
        }
        drop(sink);
        done
    }
}

impl AudioSource for FileReplaySource {
    fn channel(&self) -> Channel {
        self.channel
    }

    fn format(&self) -> StreamFormat {
        self.format
    }

    fn start(&mut self, sink: Box<dyn AudioSink>) -> Result<(), PlatformError> {
        if self.running.is_some() {
            return Err(PlatformError::Failed("replay already started".into()));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let delivery = Delivery {
            samples: self.samples.clone(),
            format: self.format,
            true_rate: self.true_rate,
            block_frames: self.block_frames as u64,
            start_ns: self.clock.now_ns(),
            pacing: self.pacing,
            guard: self.guard.clone(),
            stop: stop.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("ink-replay".into())
            .spawn(move || delivery.run(sink))
            .map_err(|e| PlatformError::Failed(format!("replay: cannot start its thread: {e}")))?;
        self.running = Some(Running { stop, thread });
        Ok(())
    }

    fn stop(&mut self) -> Result<SourceStats, PlatformError> {
        self.join(true)
    }
}
