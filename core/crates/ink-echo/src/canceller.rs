//! Cancelling along a found path: the far end resampled onto the mic's clock, AEC3, and both of
//! its outputs lined up with the mic.
//!
//! ```text
//! far ─► drift-cancelling resample ─► reference ─┐   (leads the echo by the margin)
//!                                                 ├─► AEC3 ─┬─► linear: what the "you" transcript reads
//! mic ────────────────────────────────────────────┘         └─► full:   drives the echo-only word gate
//! ```
//!
//! The far end is the one resampled, not the mic: the user's voice reaches AEC3 untouched, and
//! every output sample sits on the mic's own timeline, so the "you" transcript's times need no
//! mapping. (S0.3's measurement tool resampled the mic onto the far end's clock instead;
//! the two are the same linear model, and the replay of the real takes checks the numbers.)

use ink_core::Channel;

use crate::aec::{Aec3, FULL_LATENCY, LINEAR_LATENCY};
use crate::buffer::Sliding;
use crate::interp::{Interpolator, REACH};
use crate::path::Alignment;
use crate::{EchoError, FRAME, RATE};

/// How the canceller is set up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CancellerConfig {
    /// How far the reference leads the echo after alignment, ms. AEC3 needs its reference at or
    /// ahead of the echo; S0.3 used 10 ms.
    pub margin_ms: f64,
    /// How far either stream may run ahead of the other, seconds. Past it, a push is refused.
    pub max_skew_s: f64,
}

impl Default for CancellerConfig {
    fn default() -> Self {
        Self {
            margin_ms: 10.0,
            max_skew_s: 10.0,
        }
    }
}

/// One 10 ms frame out of the canceller, on the mic's timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct EchoFrame {
    /// Frame number: it covers mic samples `index·160 .. index·160 + len`.
    pub index: u64,
    /// Samples that are real audio; only the last frame of a stream has fewer than 160.
    pub len: usize,
    /// The mic, as captured.
    pub mic: [f32; FRAME],
    /// The far end on the mic's clock, leading its echo by the margin: AEC3's reference.
    pub reference: [f32; FRAME],
    /// AEC3's linear output: the echo estimate subtracted, nothing suppressed.
    pub linear: [f32; FRAME],
    /// AEC3's full output, after its residual echo suppressor.
    pub full: [f32; FRAME],
}

impl EchoFrame {
    /// Each signal's level over the frame's real samples.
    pub fn levels(&self) -> FrameLevels {
        let n = self.len.max(1);
        FrameLevels {
            mic_db: level_db(&self.mic[..n]),
            reference_db: level_db(&self.reference[..n]),
            linear_db: level_db(&self.linear[..n]),
            full_db: level_db(&self.full[..n]),
        }
    }
}

/// A frame's levels, dBFS (mean square; −200 for digital silence).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameLevels {
    /// The mic.
    pub mic_db: f32,
    /// The reference.
    pub reference_db: f32,
    /// The linear output.
    pub linear_db: f32,
    /// The full output.
    pub full_db: f32,
}

/// Mean-square level in dBFS, floored at −200 dB.
pub fn level_db(x: &[f32]) -> f32 {
    let e = x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    (10.0 * (e + 1e-20).log10()) as f32
}

/// AEC3 along a known echo path. Feed both 16 kHz streams (from their common start) in any
/// chunking; every frame that becomes ready is handed to the callback, in order.
///
/// **Worker.** AEC3 allocates inside its render path on every frame (its render queue clones a
/// frame), so this never runs on the realtime thread, and not on the pump either: the meeting
/// chain runs it on its worker, where it runs its engines. The resample, the alignment and the
/// frame bookkeeping around AEC3 allocate nothing after construction.
pub struct EchoCanceller {
    interp: Interpolator,
    aec: Aec3,
    alignment: Alignment,
    margin: f64,
    far: Sliding,
    mic: Sliding,
    /// The next mic frame to process.
    next: u64,
    /// The last processed frame's inputs, waiting for AEC3's latency to pass.
    held: Option<EchoFrame>,
    /// The last two frames of each output, older first: enough to line up any latency up to a
    /// frame.
    linear_hist: [[f32; FRAME]; 2],
    full_hist: [[f32; FRAME]; 2],
    mic_frame: [f32; FRAME],
    ref_frame: [f32; FRAME],
    ended: bool,
}

impl EchoCanceller {
    /// A canceller for `path`.
    ///
    /// # Errors
    ///
    /// [`EchoError::BadAlignment`] for a delay beyond ±10 s, a drift beyond ±1 %, a margin
    /// outside 0–100 ms, or a skew outside 1–60 s (or any of them not finite). A path from
    /// [`PathFinder`](crate::PathFinder) is always within them.
    pub fn new(path: Alignment, config: CancellerConfig) -> Result<Self, EchoError> {
        let sane = path.delay.is_finite()
            && path.delay.abs() <= 10.0 * RATE
            && path.drift.is_finite()
            && path.drift.abs() <= 0.01
            && (0.0..=100.0).contains(&config.margin_ms)
            && (1.0..=60.0).contains(&config.max_skew_s);
        if !sane {
            return Err(EchoError::BadAlignment);
        }
        let skew = (config.max_skew_s * RATE) as usize;
        // The far end's history must also hold the path's delay and the kernel's reach behind
        // the frame being built.
        let lookback = (path.delay.abs() + config.margin_ms / 1000.0 * RATE) as usize + 2 * REACH;
        let cap = skew + lookback + 2 * FRAME;
        Ok(Self {
            interp: Interpolator::new(),
            aec: Aec3::new(),
            alignment: path,
            margin: config.margin_ms / 1000.0 * RATE,
            far: Sliding::with_capacity(cap),
            mic: Sliding::with_capacity(cap),
            next: 0,
            held: None,
            linear_hist: [[0.0; FRAME]; 2],
            full_hist: [[0.0; FRAME]; 2],
            mic_frame: [0.0; FRAME],
            ref_frame: [0.0; FRAME],
            ended: false,
        })
    }

    /// The rule: AEC runs only when the search found an echo path. With none (earbuds,
    /// headphones), this returns `None` and the mic goes to the transcript as captured.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn when_found(
        report: &crate::PathReport,
        config: CancellerConfig,
    ) -> Result<Option<Self>, EchoError> {
        report.path.map(|p| Self::new(p, config)).transpose()
    }

    /// The path this canceller follows.
    pub fn alignment(&self) -> Alignment {
        self.alignment
    }

    fn reference_pos(&self, k: f64) -> f64 {
        reference_pos(self.alignment, self.margin, k)
    }

    /// Appends the next stretch of either stream or both, and hands every frame whose outputs
    /// are complete to `on_frame`, in order. A frame completes once the one after it has been
    /// through AEC3 (its latency is under a frame), so frames trail the input by 10 ms until
    /// [`finish`].
    ///
    /// # Errors
    ///
    /// [`EchoError::Backlog`] when a stream would run further ahead of the other than the
    /// configured skew; nothing is appended then. [`EchoError::Ended`] after [`finish`].
    ///
    /// [`finish`]: EchoCanceller::finish
    pub fn push(
        &mut self,
        mic: &[f32],
        far: &[f32],
        mut on_frame: impl FnMut(&EchoFrame),
    ) -> Result<(), EchoError> {
        if self.ended {
            return Err(EchoError::Ended);
        }
        if mic.len() > self.mic.room() {
            return Err(EchoError::Backlog {
                ahead: Channel::Mic,
            });
        }
        if far.len() > self.far.room() {
            return Err(EchoError::Backlog {
                ahead: Channel::Far,
            });
        }
        // Checked both first, so a refusal leaves neither stream half-appended.
        let mic_full = |_| EchoError::Backlog {
            ahead: Channel::Mic,
        };
        let far_full = |_| EchoError::Backlog {
            ahead: Channel::Far,
        };
        self.mic.push(mic).map_err(mic_full)?;
        self.far.push(far).map_err(far_full)?;
        while self.ready(false) {
            self.step(FRAME, &mut on_frame);
        }
        Ok(())
    }

    /// Ends both streams: the last partial frame is padded with silence, the far end is taken to
    /// be silent after its last sample, and every remaining frame is handed to `on_frame`.
    /// Returns the number of frames the stream produced in all.
    ///
    /// # Errors
    ///
    /// [`EchoError::Ended`] when called twice.
    pub fn finish(&mut self, mut on_frame: impl FnMut(&EchoFrame)) -> Result<u64, EchoError> {
        if self.ended {
            return Err(EchoError::Ended);
        }
        self.ended = true;
        while self.ready(true) {
            let left = self.mic.end() - self.next * FRAME as u64;
            let len = (left as usize).min(FRAME);
            self.step(len, &mut on_frame);
        }
        // One silent frame carries the last real frame through AEC3's latency.
        if self.held.is_some() {
            self.mic_frame = [0.0; FRAME];
            self.ref_frame = [0.0; FRAME];
            self.run_aec_and_emit(None, &mut on_frame);
        }
        Ok(self.next)
    }

    /// Whether the next frame can be built: its mic samples are in, and so is every far sample
    /// its reference reaches (or the streams have ended).
    fn ready(&self, ending: bool) -> bool {
        let start = self.next * FRAME as u64;
        if ending {
            return self.mic.end() > start;
        }
        if self.mic.end() < start + FRAME as u64 {
            return false;
        }
        let last = self.reference_pos((start + FRAME as u64 - 1) as f64);
        self.far.end() as f64 > last.ceil() + REACH as f64
    }

    /// Builds frame `next` (with `len` real mic samples), runs it through AEC3, and emits the
    /// frame before it, whose outputs are now complete.
    fn step(&mut self, len: usize, on_frame: &mut impl FnMut(&EchoFrame)) {
        let start = self.next * FRAME as u64;
        self.mic.copy_out(start, &mut self.mic_frame);
        let base = self.far.base() as f64;
        let far = self.far.as_slice();
        for (i, r) in self.ref_frame.iter_mut().enumerate() {
            let pos = reference_pos(self.alignment, self.margin, (start + i as u64) as f64);
            *r = self.interp.sample_at(far, pos - base);
        }
        let frame = EchoFrame {
            index: self.next,
            len,
            mic: self.mic_frame,
            reference: self.ref_frame,
            linear: [0.0; FRAME],
            full: [0.0; FRAME],
        };
        self.run_aec_and_emit(Some(frame), on_frame);
        self.next += 1;
        // Keep what the next frame's reference can still reach, and nothing older.
        let oldest = self.reference_pos((self.next * FRAME as u64) as f64) - REACH as f64 - 2.0;
        if oldest > 0.0 {
            self.far.discard_before(oldest as u64);
        }
        self.mic.discard_before(self.next * FRAME as u64);
    }

    /// Runs the current frame through AEC3, then emits the held frame with its outputs taken
    /// from the history at each output's latency; `current` becomes the held frame.
    fn run_aec_and_emit(
        &mut self,
        current: Option<EchoFrame>,
        on_frame: &mut impl FnMut(&EchoFrame),
    ) {
        self.linear_hist[0] = self.linear_hist[1];
        self.full_hist[0] = self.full_hist[1];
        let [_, lin_new] = &mut self.linear_hist;
        let [_, full_new] = &mut self.full_hist;
        self.aec
            .process(&self.ref_frame, &self.mic_frame, lin_new, full_new);
        if let Some(mut held) = self.held.take() {
            delayed(&self.linear_hist, LINEAR_LATENCY, &mut held.linear);
            delayed(&self.full_hist, FULL_LATENCY, &mut held.full);
            on_frame(&held);
        }
        self.held = current;
    }
}

/// The far-end position (fractional, in far samples) whose sound is AEC3's reference at mic
/// sample `k`: `(k + margin − delay) / (1 + drift)`. The mic hears far sample `n` at
/// `n·(1 + drift) + delay`, so the reference leads that echo by `margin`.
fn reference_pos(path: Alignment, margin: f64, k: f64) -> f64 {
    (k + margin - path.delay) / (1.0 + path.drift)
}

/// The frame that starts `latency` samples into the two-frame history.
fn delayed(hist: &[[f32; FRAME]; 2], latency: usize, out: &mut [f32; FRAME]) {
    let (older, newer) = (&hist[0], &hist[1]);
    out[..FRAME - latency].copy_from_slice(&older[latency..]);
    out[FRAME - latency..].copy_from_slice(&newer[..latency]);
}
