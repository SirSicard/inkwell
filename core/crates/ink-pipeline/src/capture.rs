//! The pump's half of a meeting: each side's capture ring drained into its chunks on disk (disk is
//! the seam, architecture rule 3) and on to the meeting chain as canonical audio.
//!
//! ```text
//! ring ─► CapturedBlock ─┬─► ChunkWriter (device format, raw) ─► chunks: what the final pass reads
//!                        └─► MicPath (16 kHz mono) ─► CanonicalBlock ─► MeetingChain::push_audio
//! ```
//!
//! The chunks are written first: whatever happens to the live chain afterwards, the audio is on
//! disk for the final pass.
//!
//! # The chunk sync stays on the pump
//!
//! [`ChunkWriter::write`] syncs each chunk as it fills (on macOS `F_FULLFSYNC`, checked in the
//! compiled binary), so every tenth second of audio one write blocks the pump for the length of a
//! full sync. That stalls capture only if it outlasts the ring
//! ([`DEFAULT_RING_DURATION`](ink_audio::DEFAULT_RING_DURATION), 2 s). Measured with
//! `tests/fsync_stall.rs` on an M5 Pro's internal disk, ten minutes of 48 kHz stereo in 10 ms blocks:
//! chunk-closing writes took 4.1 ms at the median, 4.8 ms at p99 and 6.1 ms at most (61 closes);
//! other writes 4 µs. The worst is a third of a percent of the ring, so the sync is not moved to a
//! worker. The test fails if a close ever takes a tenth of the ring; rerun it on new hardware.
//!
//! **Pump**, every method: bounded work per call, never waiting on an engine, the store or the
//! chain. Blocks for the chain are handed over by value, so the caller queues them to the chain's
//! thread. That queue, and its bound (what the pump does when the chain falls behind), belong to
//! the C ABI's wiring of the threads (S1.7), not to this module.

use std::fmt;

use ink_audio::{
    CaptureConsumer, CapturedBlock, ChunkStore, ChunkWriter, RateVerdict, WriterSummary,
};
use ink_core::{Channel, StreamFormat};

use crate::mic::MicPath;

/// Nanoseconds per sample at 16 kHz.
const NS_PER_SAMPLE: u64 = 62_500;

/// One block of a side in the canonical format, for [`MeetingChain::push_audio`].
///
/// [`MeetingChain::push_audio`]: crate::meeting::MeetingChain::push_audio
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalBlock {
    /// Which side.
    pub channel: Channel,
    /// 16 kHz mono.
    pub samples: Vec<f32>,
    /// Host time of the first sample.
    pub host_time_ns: u64,
    /// Device frames the ring dropped just before it.
    pub dropped_frames: u64,
}

/// Something the pump could not do. The messages name files and formats, never audio.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CaptureIssue {
    /// A chunk could not be written or opened. The live chain still got the audio; the final pass
    /// will have a gap there.
    ChunkWrite(String),
    /// The device delivers audio at another rate than it declares. The chunks keep its raw
    /// samples; the recording plays at the wrong speed until the device is fixed.
    RateMismatch {
        /// What it declares.
        declared_hz: u32,
        /// What its timing says.
        measured_hz: u32,
    },
    /// A block could not be converted to 16 kHz mono (a format the resampler refuses). It is on
    /// disk, but the live chain did not get it.
    Convert(String),
}

impl fmt::Display for CaptureIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChunkWrite(msg) => write!(f, "capture: {msg}"),
            Self::RateMismatch {
                declared_hz,
                measured_hz,
            } => write!(
                f,
                "capture: the device declares {declared_hz} Hz and delivers {measured_hz} Hz"
            ),
            Self::Convert(msg) => write!(f, "capture: {msg}"),
        }
    }
}

/// What the pump wrote for one side, from its chunk writers ([`WriterSummary`]), for the chain's
/// report ([`MeetingChain::capture_ended`]). It tells a side that never captured anything (no
/// chunks) from one that captured silence.
///
/// [`MeetingChain::capture_ended`]: crate::meeting::MeetingChain::capture_ended
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideSummary {
    /// Which side.
    pub channel: Channel,
    /// Chunk files written.
    pub chunks: u64,
    /// Audio written, ms, each writer's frames at its own rate.
    pub captured_ms: u64,
    /// Places where the stream did not continue (overruns, jumps in device time, failed writes).
    pub gaps: u64,
    /// Device frames the ring reported dropped.
    pub lost_frames: u64,
}

/// One side's ring, chunks and canonical path.
pub struct SideCapture {
    ring: CaptureConsumer,
    out: Outlets,
}

/// Where a side's blocks go: its chunks and its canonical path.
struct Outlets {
    channel: Channel,
    chunks: ChunkStore,
    writer: Option<ChunkWriter>,
    path: Option<MicPath>,
    format: Option<StreamFormat>,
    rate_reported: bool,
    /// Host time of the next canonical sample, for the resampler's tail.
    next_ns: u64,
    /// What each writer did, with the format it wrote.
    summaries: Vec<(StreamFormat, WriterSummary)>,
}

impl SideCapture {
    /// A side draining `ring` into `chunks` (the meeting's record directory).
    pub fn new(channel: Channel, ring: CaptureConsumer, chunks: ChunkStore) -> Self {
        Self {
            ring,
            out: Outlets {
                channel,
                chunks,
                writer: None,
                path: None,
                format: None,
                rate_reported: false,
                next_ns: 0,
                summaries: Vec::new(),
            },
        }
    }

    /// Drains what the ring holds. Each block goes to its chunk, then to `on_block` as canonical
    /// audio; whatever fails goes to `on_issue`, and the rest goes on. Returns the blocks drained.
    pub fn drain(
        &mut self,
        on_block: &mut dyn FnMut(CanonicalBlock),
        on_issue: &mut dyn FnMut(CaptureIssue),
    ) -> usize {
        let mut blocks = 0;
        while let Some(captured) = self.ring.pop() {
            blocks += 1;
            self.out.take(captured, on_block, on_issue);
        }
        blocks
    }

    /// Ends the side once capture has stopped: drains the ring, flushes the path, and syncs the
    /// last chunk. Returns what the side's writers did, together (one writer per format the device
    /// used), for [`MeetingChain::capture_ended`](crate::meeting::MeetingChain::capture_ended).
    #[must_use = "hand it to MeetingChain::capture_ended, or the side's report cannot say what was written"]
    pub fn finish(
        mut self,
        on_block: &mut dyn FnMut(CanonicalBlock),
        on_issue: &mut dyn FnMut(CaptureIssue),
    ) -> SideSummary {
        self.drain(on_block, on_issue);
        self.out.close(on_block, on_issue);
        let mut summary = SideSummary {
            channel: self.out.channel,
            chunks: 0,
            captured_ms: 0,
            gaps: 0,
            lost_frames: 0,
        };
        for (format, w) in &self.out.summaries {
            summary.chunks += w.chunks;
            summary.captured_ms += w.frames * 1_000 / u64::from(format.sample_rate.max(1));
            summary.gaps += w.gaps;
            summary.lost_frames += w.lost_frames;
        }
        summary
    }
}

impl Outlets {
    /// One block: to its chunk first, then to the chain.
    fn take(
        &mut self,
        captured: CapturedBlock<'_>,
        on_block: &mut dyn FnMut(CanonicalBlock),
        on_issue: &mut dyn FnMut(CaptureIssue),
    ) {
        let block = captured.block;
        let dropped = captured.dropped_frames_before;
        if self.format != Some(block.format) {
            self.reopen(block.format, on_block, on_issue);
        }
        if let Some(writer) = &mut self.writer {
            if let Err(e) = writer.write(&block, dropped) {
                on_issue(CaptureIssue::ChunkWrite(e.to_string()));
            }
            if let RateVerdict::Mismatch {
                declared_hz,
                measured_hz,
            } = writer.rate()
                && !self.rate_reported
            {
                self.rate_reported = true;
                on_issue(CaptureIssue::RateMismatch {
                    declared_hz,
                    measured_hz,
                });
            }
        }
        let Some(path) = &mut self.path else {
            return;
        };
        match path.push(&block) {
            Ok(canonical) => {
                self.next_ns = canonical
                    .host_time_ns
                    .saturating_add(canonical.samples.len() as u64 * NS_PER_SAMPLE);
                on_block(CanonicalBlock {
                    channel: self.channel,
                    samples: canonical.samples.to_vec(),
                    host_time_ns: canonical.host_time_ns,
                    dropped_frames: dropped,
                });
            }
            Err(e) => on_issue(CaptureIssue::Convert(e.to_string())),
        }
    }

    /// A block arrived in a new format (or the first one did): close the chunk writer and the path
    /// for the old one, and open them for the new one. The sequence of chunks goes on; each chunk's
    /// header states its own format.
    fn reopen(
        &mut self,
        format: StreamFormat,
        on_block: &mut dyn FnMut(CanonicalBlock),
        on_issue: &mut dyn FnMut(CaptureIssue),
    ) {
        self.close(on_block, on_issue);
        self.format = Some(format);
        match self.chunks.writer(self.channel, format) {
            Ok(writer) => self.writer = Some(writer),
            Err(e) => on_issue(CaptureIssue::ChunkWrite(e.to_string())),
        }
        match MicPath::for_channel(self.channel, format) {
            Ok(path) => self.path = Some(path),
            Err(e) => on_issue(CaptureIssue::Convert(e.to_string())),
        }
    }

    /// Ends the current writer and path: the resampler's last samples go to the chain, and the
    /// open chunk is synced.
    fn close(
        &mut self,
        on_block: &mut dyn FnMut(CanonicalBlock),
        on_issue: &mut dyn FnMut(CaptureIssue),
    ) {
        if let Some(mut path) = self.path.take() {
            match path.finish() {
                Ok(tail) if !tail.is_empty() => on_block(CanonicalBlock {
                    channel: self.channel,
                    samples: tail.to_vec(),
                    host_time_ns: self.next_ns,
                    dropped_frames: 0,
                }),
                Ok(_) => {}
                Err(e) => on_issue(CaptureIssue::Convert(e.to_string())),
            }
        }
        if let Some(writer) = self.writer.take() {
            let format = writer.format();
            match writer.finish() {
                Ok(summary) => self.summaries.push((format, summary)),
                Err(e) => on_issue(CaptureIssue::ChunkWrite(e.to_string())),
            }
        }
    }
}
