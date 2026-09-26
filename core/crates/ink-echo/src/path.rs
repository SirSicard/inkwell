//! Finding the echo path: GCC-PHAT windows over both streams as they arrive, and the consensus
//! line through them.
//!
//! Echo cancellation runs only when this finds a path. With earbuds or headphones the mic never
//! hears the far end, and running AEC3 anyway still deleted words in S0.3's recordings; the
//! consensus check told "echo" from "none" every time there (synthetic no-echo mix: 0 of 89
//! windows; both mics of the earbuds take: 0 of 81).

use std::fmt;

use ink_core::Channel;

use crate::buffer::Sliding;
use crate::gcc::{GccParams, GccPhat, WindowEstimate, consensus_fit};
use crate::{EchoError, RATE};

/// Where the far end lands in the mic: the consensus line.
///
/// The mic hears far-end sample `n` at mic sample `n·(1 + drift) + delay`, both streams counted
/// from their common start at 16 kHz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alignment {
    /// The lag at the start, 16 kHz samples. Above zero the mic hears the far end late (the
    /// causal direction); below zero the far-end stream itself arrived late.
    pub delay: f64,
    /// Clock drift, as a fraction: the lag grows by this many samples per sample.
    pub drift: f64,
}

impl Alignment {
    /// An alignment given in milliseconds and parts per million.
    pub fn from_ms_ppm(delay_ms: f64, drift_ppm: f64) -> Self {
        Self {
            delay: delay_ms / 1000.0 * RATE,
            drift: drift_ppm * 1e-6,
        }
    }

    /// The lag at the start, ms.
    pub fn delay_ms(&self) -> f64 {
        self.delay / RATE * 1000.0
    }

    /// The drift, parts per million.
    pub fn drift_ppm(&self) -> f64 {
        self.drift * 1e6
    }
}

/// What a search found.
#[derive(Clone, Debug, PartialEq)]
pub struct PathReport {
    /// The path, when the consensus found one.
    pub path: Option<Alignment>,
    /// Windows analysed.
    pub windows: usize,
    /// Windows loud enough, with a peak clear enough, to vote.
    pub candidates: usize,
    /// Candidates on the winning line (0 without a path).
    pub inliers: usize,
    /// RMS distance of the inliers from the line, ms (NaN without a path).
    pub residual_rms_ms: f64,
    /// Seconds between the first and last inlier.
    pub span_s: f64,
}

impl fmt::Display for PathReport {
    /// Numbers only: safe for logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.path {
            Some(p) => write!(
                f,
                "echo path {:+.2} ms, {:+.2} ppm ({} of {} candidates, {} windows, residual {:.3} ms)",
                p.delay_ms(),
                p.drift_ppm(),
                self.inliers,
                self.candidates,
                self.windows,
                self.residual_rms_ms
            ),
            None => write!(
                f,
                "no echo path ({} candidates, {} windows)",
                self.candidates, self.windows
            ),
        }
    }
}

/// Runs GCC-PHAT over both streams as they arrive, one window per hop, and fits the line on
/// demand. Both streams are 16 kHz and start at the same instant.
///
/// **Worker.** A window costs three 64k-point FFTs, once a second of audio.
pub struct PathFinder {
    gcc: GccPhat,
    mic: Sliding,
    far: Sliding,
    next_start: u64,
    windows: Vec<WindowEstimate>,
}

/// How far one stream may run ahead of the other before [`PathFinder::push`] refuses it.
const MAX_SKEW_S: f64 = 10.0;

impl PathFinder {
    /// A finder with S0.3's settings.
    pub fn new() -> Self {
        Self::with_params(GccParams::default())
    }

    /// A finder with other window settings.
    pub fn with_params(params: GccParams) -> Self {
        let gcc = GccPhat::new(params);
        let cap = gcc.window_len() + (MAX_SKEW_S * RATE) as usize;
        Self {
            gcc,
            mic: Sliding::with_capacity(cap),
            far: Sliding::with_capacity(cap),
            next_start: 0,
            windows: Vec::new(),
        }
    }

    /// Appends the next stretch of either stream or both (they need not be the same length) and
    /// analyses every window both now cover.
    ///
    /// # Errors
    ///
    /// [`EchoError::Backlog`] when one stream is more than 10 s ahead of the other; nothing is
    /// appended then.
    pub fn push(&mut self, mic: &[f32], far: &[f32]) -> Result<(), EchoError> {
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
        let w = self.gcc.window_len() as u64;
        let hop = self.gcc.hop_len() as u64;
        while self.next_start + w <= self.mic.end().min(self.far.end()) {
            let s = self.next_start;
            let (mb, fb) = (self.mic.base(), self.far.base());
            // Both buffers still hold `s..s + w`: they only discard below `next_start`.
            let m = &self.mic.as_slice()[(s - mb) as usize..(s - mb + w) as usize];
            let f = &self.far.as_slice()[(s - fb) as usize..(s - fb + w) as usize];
            let center = (s as f64 + w as f64 / 2.0) / RATE;
            let est = self.gcc.analyze(m, f, center);
            self.windows.push(est);
            self.next_start += hop;
            self.mic.discard_before(self.next_start);
            self.far.discard_before(self.next_start);
        }
        Ok(())
    }

    /// Every window so far, in time order.
    pub fn windows(&self) -> &[WindowEstimate] {
        &self.windows
    }

    /// The consensus over every window so far.
    pub fn estimate(&self) -> PathReport {
        let candidates = self.windows.iter().filter(|w| w.candidate).count();
        let (fit, _) = consensus_fit(&self.windows);
        match fit {
            Some(fit) => PathReport {
                path: Some(Alignment {
                    delay: fit.intercept,
                    drift: fit.slope / RATE,
                }),
                windows: self.windows.len(),
                candidates,
                inliers: fit.inliers,
                residual_rms_ms: fit.residual_rms / RATE * 1000.0,
                span_s: fit.span_s,
            },
            None => PathReport {
                path: None,
                windows: self.windows.len(),
                candidates,
                inliers: 0,
                residual_rms_ms: f64::NAN,
                span_s: 0.0,
            },
        }
    }
}

impl Default for PathFinder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_far_ahead_of_the_other_is_refused() {
        let mut p = PathFinder::new();
        let second = vec![0.0f32; 16_000];
        for _ in 0..12 {
            p.push(&second, &[]).expect("within the skew");
        }
        assert_eq!(
            p.push(&second, &[]),
            Err(EchoError::Backlog {
                ahead: Channel::Mic
            })
        );
    }

    #[test]
    fn windows_come_once_a_second_whatever_the_chunking() {
        let mut a = PathFinder::new();
        let mut b = PathFinder::new();
        let x: Vec<f32> = (0..16_000 * 6)
            .map(|i| ((i as f32) * 0.37).sin() * 0.1)
            .collect();
        a.push(&x, &x).unwrap();
        for (m, f) in x.chunks(1_000).zip(x.chunks(1_000)) {
            b.push(m, f).unwrap();
        }
        assert_eq!(a.windows().len(), 5);
        assert_eq!(a.windows(), b.windows());
    }
}
