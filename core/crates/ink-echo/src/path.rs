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
use crate::gcc::{
    GccParams, GccPhat, MIN_INLIER_SHARE, MIN_INLIERS, WindowEstimate, consensus_fit,
};
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
    /// Stream seconds (from the common start) at which the windows so far first supported this
    /// path: the end of the window that gave it its sixth inlier while at least a quarter of the
    /// candidates so far lay on it. Cancellation along this path could not have started
    /// earlier, so the mic ran unprotected until then. `None` without a path.
    pub stable_from_s: Option<f64>,
}

impl fmt::Display for PathReport {
    /// Numbers only: safe for logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(p) = self.path else {
            return write!(
                f,
                "no echo path ({} candidates, {} windows)",
                self.candidates, self.windows
            );
        };
        write!(
            f,
            "echo path {:+.2} ms, {:+.2} ppm ({} of {} candidates, {} windows, residual {:.3} ms",
            p.delay_ms(),
            p.drift_ppm(),
            self.inliers,
            self.candidates,
            self.windows,
            self.residual_rms_ms
        )?;
        if let Some(t) = self.stable_from_s {
            write!(f, ", stable from {t:.1} s")?;
        }
        write!(f, ")")
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
        let (fit, inliers) = consensus_fit(&self.windows);
        let win_s = self.gcc.window_len() as f64 / RATE;
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
                stable_from_s: stable_since(&self.windows, &inliers, win_s),
            },
            None => PathReport {
                path: None,
                windows: self.windows.len(),
                candidates,
                inliers: 0,
                residual_rms_ms: f64::NAN,
                span_s: 0.0,
                stable_from_s: None,
            },
        }
    }
}

/// Walks the windows in time order and returns the end of the first one at which the line's
/// inliers so far meet the consensus's own bar (see [`consensus_fit`]) against the candidates so
/// far. `inliers` flags the line's windows.
fn stable_since(windows: &[WindowEstimate], inliers: &[bool], win_s: f64) -> Option<f64> {
    let (mut candidates, mut on_line) = (0usize, 0usize);
    for (w, &inlier) in windows.iter().zip(inliers) {
        if w.candidate {
            candidates += 1;
        }
        if inlier {
            on_line += 1;
        }
        if on_line >= MIN_INLIERS && on_line as f64 >= MIN_INLIER_SHARE * candidates as f64 {
            return Some(w.center_s + win_s / 2.0);
        }
    }
    None
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

    fn window(center_s: f64, lag: f64, candidate: bool) -> WindowEstimate {
        WindowEstimate {
            center_s,
            lag,
            peak: 0.5,
            pnr: if candidate { 20.0 } else { 2.0 },
            far_db: -30.0,
            mic_db: -30.0,
            candidate,
        }
    }

    #[test]
    fn the_path_is_stable_from_the_end_of_the_window_that_gave_it_its_sixth_inlier() {
        // Windows every second, 2 s long. The far end is silent until 10 s; then two chance
        // candidates (no inliers yet), then the path's windows. The sixth inlier is centred at
        // 18 s, but 6 of 8 candidates is already above a quarter, so the path stands from the
        // end of that window: 19 s.
        let mut ws: Vec<WindowEstimate> =
            (1..=10).map(|c| window(f64::from(c), 0.0, false)).collect();
        ws.push(window(11.0, -3_000.0, true));
        ws.push(window(12.0, 4_000.0, true));
        for c in 13..=40 {
            ws.push(window(f64::from(c), 736.0, true));
        }
        let (fit, used) = crate::gcc::consensus_fit(&ws);
        assert!(fit.is_some());
        assert_eq!(stable_since(&ws, &used, 2.0), Some(19.0));
    }

    #[test]
    fn a_path_must_also_hold_a_quarter_of_the_candidates_so_far() {
        // 20 chance candidates first: the sixth inlier is not enough until the inliers reach a
        // quarter of all candidates so far (7 of 27, at the window centred at 27 s).
        let mut ws: Vec<WindowEstimate> = (1..=20)
            .map(|c| window(f64::from(c), -7_000.0 + 350.0 * f64::from(c), true))
            .collect();
        for c in 21..=60 {
            ws.push(window(f64::from(c), 736.0, true));
        }
        let (fit, used) = crate::gcc::consensus_fit(&ws);
        assert!(fit.is_some());
        assert_eq!(stable_since(&ws, &used, 2.0), Some(28.0));
    }

    #[test]
    fn no_path_is_never_stable() {
        let p = PathFinder::new();
        let r = p.estimate();
        assert_eq!((r.path, r.stable_from_s), (None, None));
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
