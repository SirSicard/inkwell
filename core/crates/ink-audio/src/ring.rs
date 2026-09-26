//! The capture ring: one SPSC ring per capture stream, between the realtime callback and the pump.
//!
//! The producer ([`CaptureProducer`]) is the realtime [`AudioSink`]. It copies each block's samples
//! and metadata into memory allocated when the ring was made, and never waits: when the ring is
//! full it drops the block and counts it. The consumer ([`CaptureConsumer`]) runs on the pump and
//! gets the blocks back in order, each with its format, its host time and the audio dropped just
//! before it, so a gap in the recording is always reported where it happened, never silent.
//!
//! Two `rtrb` rings carry a block: its samples, and a small metadata record. The producer reserves
//! the metadata slot first, copies the samples, then commits the metadata, so the consumer never
//! sees a record whose samples are not already in the ring.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ink_core::{AudioBlock, AudioSink, StreamFormat};

/// How much audio a capture ring holds before it overruns. The pump drains every few tens of
/// milliseconds, so two seconds is slack for a stalled pump (a slow disk sync), not a buffer.
pub const DEFAULT_RING_DURATION: Duration = Duration::from_secs(2);

/// The longest ring [`capture_ring`] builds. RAM holds seconds, never a session.
pub const MAX_RING_DURATION: Duration = Duration::from_secs(60);

/// The smallest block the ring is sized for. Metadata slots are sized so that blocks of at least
/// this many frames fill the sample ring before the metadata ring. Real devices deliver hundreds
/// of frames per callback; smaller blocks still work, and only overrun (visibly) sooner.
pub const MIN_BLOCK_FRAMES: usize = 32;

/// Why a ring could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RingError {
    /// A zero sample rate or channel count.
    InvalidFormat(StreamFormat),
    /// Zero, or longer than [`MAX_RING_DURATION`].
    InvalidDuration(Duration),
}

impl std::fmt::Display for RingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFormat(format) => write!(
                f,
                "capture ring: invalid format ({} Hz, {} channels)",
                format.sample_rate, format.channels
            ),
            Self::InvalidDuration(d) => write!(
                f,
                "capture ring: duration {d:?} is outside 0..={MAX_RING_DURATION:?}"
            ),
        }
    }
}

impl std::error::Error for RingError {}

/// Audio the ring dropped because it was full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overruns {
    /// Blocks dropped.
    pub blocks: u64,
    /// Frames in those blocks.
    pub frames: u64,
}

/// One block as the pump receives it.
#[derive(Clone, Copy, Debug)]
pub struct CapturedBlock<'a> {
    /// The block as the device delivered it. The samples borrow the consumer's scratch buffer
    /// until the next [`CaptureConsumer::pop`].
    pub block: AudioBlock<'a>,
    /// Blocks the ring dropped between the previous delivered block and this one.
    pub dropped_blocks_before: u64,
    /// Frames in those blocks.
    pub dropped_frames_before: u64,
}

/// The per-block record that travels beside the samples. `Default` so `rtrb` can reserve a slot
/// for it before the samples are copied.
#[derive(Clone, Copy, Debug, Default)]
struct BlockMeta {
    samples: usize,
    sample_rate: u32,
    channels: u16,
    host_time_ns: u64,
    dropped_blocks_before: u64,
    dropped_frames_before: u64,
}

#[derive(Debug, Default)]
struct Counters {
    blocks: AtomicU64,
    frames: AtomicU64,
}

impl Counters {
    fn load(&self) -> Overruns {
        Overruns {
            blocks: self.blocks.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
        }
    }
}

/// Builds a ring that holds `duration` of audio in `format`, allocating all of it now.
///
/// **Worker.** Call it before the device starts; the producer then goes to the source's
/// [`start`](ink_core::AudioSource::start) and the consumer to the pump.
pub fn capture_ring(
    format: StreamFormat,
    duration: Duration,
) -> Result<(CaptureProducer, CaptureConsumer), RingError> {
    if format.sample_rate == 0 || format.channels == 0 {
        return Err(RingError::InvalidFormat(format));
    }
    if duration.is_zero() || duration > MAX_RING_DURATION {
        return Err(RingError::InvalidDuration(duration));
    }
    // Rounded up, so a ring asked for two seconds holds at least two seconds. Bounded by
    // MAX_RING_DURATION, so this cannot overflow.
    let frames = (u128::from(format.sample_rate) * duration.as_nanos()).div_ceil(1_000_000_000);
    let frames = usize::try_from(frames).map_err(|_| RingError::InvalidDuration(duration))?;
    let samples = frames * usize::from(format.channels);
    let blocks = frames / MIN_BLOCK_FRAMES + 1;

    let (sample_tx, sample_rx) = rtrb::RingBuffer::new(samples);
    let (meta_tx, meta_rx) = rtrb::RingBuffer::new(blocks);
    let counters = Arc::new(Counters::default());
    Ok((
        CaptureProducer {
            samples: sample_tx,
            meta: meta_tx,
            pending_blocks: 0,
            pending_frames: 0,
            counters: counters.clone(),
        },
        CaptureConsumer {
            samples: sample_rx,
            meta: meta_rx,
            scratch: vec![0.0; samples],
            counters,
        },
    ))
}

/// The realtime side of a capture ring.
pub struct CaptureProducer {
    samples: rtrb::Producer<f32>,
    meta: rtrb::Producer<BlockMeta>,
    pending_blocks: u64,
    pending_frames: u64,
    counters: Arc<Counters>,
}

impl CaptureProducer {
    /// Everything dropped so far. **Any thread.**
    pub fn overruns(&self) -> Overruns {
        self.counters.load()
    }
}

impl AudioSink for CaptureProducer {
    /// **Realtime.** Copies the block into the ring, or drops and counts it when the ring is full.
    /// Never allocates, locks, blocks or logs.
    fn push(&mut self, block: &AudioBlock<'_>) {
        let samples = block.samples;
        if samples.is_empty() {
            return;
        }
        // Reserve the metadata slot first, copy the samples, then commit the metadata: the
        // consumer can only ever see a record whose samples are already in the ring, and a block
        // that does not fit leaves nothing behind.
        let Ok(mut slot) = self.meta.write_chunk(1) else {
            self.drop_block(block);
            return;
        };
        if self.samples.push_entire_slice(samples).is_err() {
            drop(slot); // uncommitted: the reservation is released
            self.drop_block(block);
            return;
        }
        let meta = BlockMeta {
            samples: samples.len(),
            sample_rate: block.format.sample_rate,
            channels: block.format.channels,
            host_time_ns: block.host_time_ns,
            dropped_blocks_before: self.pending_blocks,
            dropped_frames_before: self.pending_frames,
        };
        let (first, second) = slot.as_mut_slices();
        if let Some(record) = first.first_mut().or(second.first_mut()) {
            *record = meta;
        }
        slot.commit_all();
        self.pending_blocks = 0;
        self.pending_frames = 0;
    }
}

impl CaptureProducer {
    /// The ring is full: count the block, and carry the count to the next block that gets in.
    fn drop_block(&mut self, block: &AudioBlock<'_>) {
        let frames = block.frames() as u64;
        self.pending_blocks += 1;
        self.pending_frames += frames;
        self.counters.blocks.fetch_add(1, Ordering::Relaxed);
        self.counters.frames.fetch_add(frames, Ordering::Relaxed);
    }
}

/// The pump side of a capture ring.
pub struct CaptureConsumer {
    samples: rtrb::Consumer<f32>,
    meta: rtrb::Consumer<BlockMeta>,
    scratch: Vec<f32>,
    counters: Arc<Counters>,
}

impl CaptureConsumer {
    /// The next block, in the order the device delivered them, or `None` when the ring is empty.
    ///
    /// **Pump.** Allocation-free: the samples are copied into scratch space allocated with the
    /// ring.
    pub fn pop(&mut self) -> Option<CapturedBlock<'_>> {
        let meta = self.meta.pop().ok()?;
        // Cannot fail: the producer commits a block's samples (a release store) before its record,
        // and popping the record (an acquire load) makes them visible here. The block fitted in the
        // sample ring, so it also fits in the scratch buffer, which is the same size.
        let scratch = &mut self.scratch[..meta.samples];
        self.samples
            .pop_entire_slice(scratch)
            .expect("a block's samples are committed before its record");
        Some(CapturedBlock {
            block: AudioBlock {
                samples: scratch,
                format: StreamFormat {
                    sample_rate: meta.sample_rate,
                    channels: meta.channels,
                },
                host_time_ns: meta.host_time_ns,
            },
            dropped_blocks_before: meta.dropped_blocks_before,
            dropped_frames_before: meta.dropped_frames_before,
        })
    }

    /// Everything the producer dropped so far, including drops after the last block popped (which
    /// no later block can report).
    pub fn overruns(&self) -> Overruns {
        self.counters.load()
    }

    /// Whether the producer is gone (the source stopped and dropped its sink). Blocks already in
    /// the ring can still be popped.
    pub fn is_abandoned(&self) -> bool {
        self.meta.is_abandoned()
    }
}
