//! The sample-rate check: does a stream deliver as many frames as the host time it took?
//!
//! An earlier implementation once captured a microphone at 48 kHz into a file that declared
//! 16 kHz. Every byte was accounted for, so a size check passed; only the duration gave it away (the
//! file claimed about three times the meeting's length), and it was noticed only after the
//! meeting. Here the pump compares, block by block, the frames delivered with the host time that
//! passed, and reports [`RateVerdict::Mismatch`] within about a second. Nothing is resampled to
//! hide it: the chunks keep the samples and the rate the device declared.

use std::time::Duration;

/// How far the measured rate may stray from the declared one before it is a mismatch.
///
/// The smallest real mismatch is 44.1 kHz against 48 kHz (8.8 %). Device clocks drift from the
/// host clock by well under 0.1 %. 5 % sits between the two with margin on both sides.
pub const RATE_TOLERANCE: f64 = 0.05;

/// Contiguous host time needed before a verdict. Below this, timestamp jitter could dominate.
pub const RATE_MIN_SPAN: Duration = Duration::from_secs(1);

/// A jump in device time larger than this, beyond where the previous block ended, is a gap: audio
/// the device never delivered (a process tap delivers no callbacks while nothing plays).
///
/// It is absolute, not a multiple of the block length, because a wrong declared rate also makes
/// every block look late or early by up to a few block lengths. Real blocks last at most about
/// 100 ms, so even a 6× mislabelled rate stays well under 250 ms per block, while a silent tap
/// jumps by seconds. A glitch shorter than this is absorbed into the chunk; the next chunk
/// re-anchors on a device timestamp.
///
/// It assumes device blocks of at most about 100 ms. A backend that batches callbacks into longer
/// blocks needs it revisited, or a mislabelled rate would start to look like gaps.
pub const GAP_THRESHOLD: Duration = Duration::from_millis(250);

/// Whether a stream's rate agrees with the host clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateVerdict {
    /// Less than [`RATE_MIN_SPAN`] of contiguous host time seen so far.
    Pending,
    /// The measured rate is within [`RATE_TOLERANCE`] of the declared one.
    Consistent {
        /// Frames per second of host time, rounded.
        measured_hz: u32,
    },
    /// The stream delivers audio at a different rate than it declares. The recording is not
    /// resampled; the UI reports it and the chunks keep the declared rate and the raw samples.
    Mismatch {
        /// The rate in the stream's format.
        declared_hz: u32,
        /// Frames per second of host time, rounded.
        measured_hz: u32,
    },
}

impl RateVerdict {
    /// Whether this is a [`Mismatch`](Self::Mismatch).
    pub fn is_mismatch(&self) -> bool {
        matches!(self, Self::Mismatch { .. })
    }
}

/// How a block relates to the one before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Continuity {
    /// The first block of the stream.
    First,
    /// Continues the previous block.
    Continues,
    /// Does not continue it: audio was lost in between (a ring overrun or a jump in device time),
    /// or device time went backwards.
    Gap,
}

/// Measures one stream's rate against its host timestamps.
///
/// Rate is measured over contiguous runs only. A run's span runs from its first block's timestamp
/// to its last block's, and holds the frames of every block but the last, so no step needs the
/// declared rate, which is the thing under test.
#[derive(Clone, Debug)]
pub struct RateCheck {
    declared_hz: u32,
    run: Option<Run>,
    closed_frames: u64,
    closed_ns: u64,
}

#[derive(Clone, Copy, Debug)]
struct Run {
    start_ns: u64,
    last_ns: u64,
    last_frames: u64,
    frames_before_last: u64,
}

impl RateCheck {
    /// A check for a stream that declares `declared_hz`.
    pub fn new(declared_hz: u32) -> Self {
        Self {
            declared_hz,
            run: None,
            closed_frames: 0,
            closed_ns: 0,
        }
    }

    /// The rate the stream declares.
    pub fn declared_hz(&self) -> u32 {
        self.declared_hz
    }

    /// Records a block from a live stream and says whether it continues the previous one.
    ///
    /// `lost_before` is audio known to be missing just before this block (a ring overrun). Device
    /// time that jumps beyond [`GAP_THRESHOLD`], or goes backwards, is a gap too.
    pub fn observe(&mut self, host_time_ns: u64, frames: u64, lost_before: bool) -> Continuity {
        let continuity = match self.run {
            None => Continuity::First,
            Some(run) => {
                // Where the previous block ends, by the declared rate. A wrong rate moves this by
                // a few block lengths at most, which the absolute threshold absorbs.
                let expected = run
                    .last_ns
                    .saturating_add(frames_to_ns(run.last_frames, self.declared_hz));
                let jumped = host_time_ns > expected.saturating_add(GAP_THRESHOLD_NS);
                if lost_before || jumped || host_time_ns < run.last_ns {
                    Continuity::Gap
                } else {
                    Continuity::Continues
                }
            }
        };
        self.observe_run(host_time_ns, frames, continuity != Continuity::Continues);
        continuity
    }

    /// Records a span whose continuity the caller already knows, such as a chunk read back from
    /// disk (its header says whether it follows a gap).
    pub fn observe_run(&mut self, host_time_ns: u64, frames: u64, starts_run: bool) {
        match &mut self.run {
            // Time going backwards cannot continue a run, whatever the caller says.
            Some(run) if !starts_run && host_time_ns >= run.last_ns => {
                run.frames_before_last = run.frames_before_last.saturating_add(run.last_frames);
                run.last_ns = host_time_ns;
                run.last_frames = frames;
            }
            _ => {
                self.close_run();
                self.run = Some(Run {
                    start_ns: host_time_ns,
                    last_ns: host_time_ns,
                    last_frames: frames,
                    frames_before_last: 0,
                });
            }
        }
    }

    /// The verdict so far.
    pub fn verdict(&self) -> RateVerdict {
        let (frames, ns) = match self.run {
            Some(run) => (
                self.closed_frames.saturating_add(run.frames_before_last),
                self.closed_ns.saturating_add(run.last_ns - run.start_ns),
            ),
            None => (self.closed_frames, self.closed_ns),
        };
        if ns < RATE_MIN_SPAN_NS || self.declared_hz == 0 {
            return RateVerdict::Pending;
        }
        let measured = frames as f64 * 1e9 / ns as f64;
        let measured_hz = measured.round() as u32;
        if (measured / f64::from(self.declared_hz) - 1.0).abs() > RATE_TOLERANCE {
            RateVerdict::Mismatch {
                declared_hz: self.declared_hz,
                measured_hz,
            }
        } else {
            RateVerdict::Consistent { measured_hz }
        }
    }

    fn close_run(&mut self) {
        if let Some(run) = self.run.take() {
            self.closed_frames = self.closed_frames.saturating_add(run.frames_before_last);
            self.closed_ns = self.closed_ns.saturating_add(run.last_ns - run.start_ns);
        }
    }
}

const RATE_MIN_SPAN_NS: u64 = RATE_MIN_SPAN.as_nanos() as u64;
const GAP_THRESHOLD_NS: u64 = GAP_THRESHOLD.as_nanos() as u64;

/// Nanoseconds that `frames` last at `rate`, saturating. A zero rate gives zero.
pub(crate) fn frames_to_ns(frames: u64, rate: u32) -> u64 {
    if rate == 0 {
        return 0;
    }
    let ns = u128::from(frames) * 1_000_000_000 / u128::from(rate);
    u64::try_from(ns).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_to_ns_is_exact_and_saturates() {
        assert_eq!(frames_to_ns(16_000, 16_000), 1_000_000_000);
        assert_eq!(frames_to_ns(1, 48_000), 20_833);
        assert_eq!(frames_to_ns(u64::MAX, 1), u64::MAX);
        assert_eq!(frames_to_ns(5, 0), 0);
    }

    #[test]
    fn a_run_excludes_its_last_block_so_the_declared_rate_is_never_used_to_measure() {
        // Two blocks of 1000 frames, 1 s apart: 1000 frames per second, whatever is declared.
        let mut check = RateCheck::new(16_000);
        check.observe_run(0, 1_000, true);
        check.observe_run(1_000_000_000, 1_000, false);
        assert_eq!(
            check.verdict(),
            RateVerdict::Mismatch {
                declared_hz: 16_000,
                measured_hz: 1_000
            }
        );
    }

    #[test]
    fn time_going_backwards_starts_a_new_run() {
        let mut check = RateCheck::new(16_000);
        assert_eq!(check.observe(5_000_000_000, 320, false), Continuity::First);
        assert_eq!(
            check.observe(5_020_000_000, 320, false),
            Continuity::Continues
        );
        assert_eq!(check.observe(1_000_000_000, 320, false), Continuity::Gap);
        assert_eq!(check.observe(1_020_000_000, 320, true), Continuity::Gap);
    }
}
