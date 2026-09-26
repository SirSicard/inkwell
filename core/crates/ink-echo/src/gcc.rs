//! GCC-PHAT delay per window, and the consensus line through the windows (delay and drift).
//!
//! Sign convention, everywhere in this crate: **a lag above zero means the mic lags the far end**
//! (`mic[n + lag]` holds the echo of `far[n]`), the causal direction an acoustic echo has.

use std::sync::Arc;

use num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::RATE;

/// How the windows are cut and judged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GccParams {
    /// Window length, seconds.
    pub win_s: f64,
    /// Hop between window starts, seconds.
    pub hop_s: f64,
    /// Lags searched, ± seconds.
    pub max_lag_s: f64,
    /// The band the phase transform weighs, Hz.
    pub band_lo_hz: f64,
    /// See `band_lo_hz`.
    pub band_hi_hz: f64,
    /// A window is a candidate when its peak stands this many times above the median |r|.
    pub min_pnr: f64,
    /// Windows whose far end is quieter than this (dBFS RMS) are skipped: nothing to hear.
    pub min_far_db: f64,
}

impl Default for GccParams {
    /// S0.3's settings: 2 s windows every second, ±500 ms, 150 Hz to 7 kHz, PNR 8,
    /// far end above −60 dBFS.
    fn default() -> Self {
        Self {
            win_s: 2.0,
            hop_s: 1.0,
            max_lag_s: 0.5,
            band_lo_hz: 150.0,
            band_hi_hz: 7_000.0,
            min_pnr: 8.0,
            min_far_db: -60.0,
        }
    }
}

/// One window's estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowEstimate {
    /// The window's centre, seconds from the start of both streams.
    pub center_s: f64,
    /// The lag at the correlation peak, 16 kHz samples, sub-sample (parabolic).
    pub lag: f64,
    /// The PHAT correlation peak: 1.0 means identical up to the delay.
    pub peak: f64,
    /// Peak over the median |r| across the searched lags.
    pub pnr: f64,
    /// The far end's level in the window, dBFS RMS.
    pub far_db: f64,
    /// The mic's level in the window, dBFS RMS.
    pub mic_db: f64,
    /// Whether the window is a candidate: loud enough and `pnr ≥ min_pnr`.
    pub candidate: bool,
}

pub(crate) fn rms_db(x: &[f32]) -> f64 {
    let e = x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64;
    10.0 * (e + 1e-20).log10()
}

/// The per-window estimator. It plans its FFTs and allocates its buffers once; [`analyze`]
/// allocates nothing.
///
/// [`analyze`]: GccPhat::analyze
pub struct GccPhat {
    params: GccParams,
    win: usize,
    max_lag: usize,
    n: usize,
    lo_bin: usize,
    hi_bin: usize,
    fwd: Arc<dyn Fft<f64>>,
    inv: Arc<dyn Fft<f64>>,
    hann: Vec<f64>,
    x: Vec<Complex<f64>>,
    y: Vec<Complex<f64>>,
    g: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    mags: Vec<f64>,
}

impl GccPhat {
    /// An estimator for `params`.
    pub fn new(params: GccParams) -> Self {
        // Settings the search cannot mean are clamped, never trusted into an out-of-range index:
        // a window of at least 64 samples, lags inside it, a band of at least one bin.
        let win = ((params.win_s * RATE) as usize).max(64);
        let max_lag = ((params.max_lag_s * RATE) as usize).min(win - 1);
        let n = (2 * win).next_power_of_two();
        let mut planner = FftPlanner::<f64>::new();
        let fwd = planner.plan_fft_forward(n);
        let inv = planner.plan_fft_inverse(n);
        let scratch_len = fwd
            .get_inplace_scratch_len()
            .max(inv.get_inplace_scratch_len());
        let hann = (0..win)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (win as f64 - 1.0)).cos())
            .collect();
        let lo_bin = ((params.band_lo_hz / RATE * n as f64).ceil() as usize).min(n / 2);
        let hi_bin = ((params.band_hi_hz / RATE * n as f64).floor() as usize)
            .min(n / 2)
            .max(lo_bin);
        let zero = Complex::new(0.0, 0.0);
        Self {
            params,
            win,
            max_lag,
            n,
            lo_bin,
            hi_bin,
            fwd,
            inv,
            hann,
            x: vec![zero; n],
            y: vec![zero; n],
            g: vec![zero; n],
            scratch: vec![zero; scratch_len],
            mags: Vec::with_capacity(2 * max_lag + 1),
        }
    }

    /// Window length in samples.
    pub fn window_len(&self) -> usize {
        self.win
    }

    /// Hop between windows in samples.
    pub fn hop_len(&self) -> usize {
        ((self.params.hop_s * RATE) as usize).max(1)
    }

    /// The parameters.
    pub fn params(&self) -> &GccParams {
        &self.params
    }

    /// Estimates one window. `mic` and `far` are the same stretch of both streams, each
    /// [`window_len`](Self::window_len) samples; `center_s` places it on the streams' timeline.
    pub fn analyze(&mut self, mic: &[f32], far: &[f32], center_s: f64) -> WindowEstimate {
        let w = self.win.min(mic.len()).min(far.len());
        let far_db = rms_db(&far[..w]);
        let mic_db = rms_db(&mic[..w]);
        let mut est = WindowEstimate {
            center_s,
            lag: 0.0,
            peak: 0.0,
            pnr: 0.0,
            far_db,
            mic_db,
            candidate: false,
        };
        if far_db < self.params.min_far_db || w < self.win {
            return est;
        }
        let zero = Complex::new(0.0, 0.0);
        self.x.fill(zero);
        self.y.fill(zero);
        self.g.fill(zero);
        for i in 0..w {
            self.x[i].re = f64::from(far[i]) * self.hann[i];
            self.y[i].re = f64::from(mic[i]) * self.hann[i];
        }
        self.fwd
            .process_with_scratch(&mut self.x, &mut self.scratch);
        self.fwd
            .process_with_scratch(&mut self.y, &mut self.scratch);
        // G = Y·X*, PHAT-weighted and band-limited; conjugate-symmetric, so the IFFT is real.
        let n = self.n;
        for k in self.lo_bin..=self.hi_bin {
            let c = self.y[k] * self.x[k].conj();
            let m = c.norm();
            if m > 1e-30 {
                self.g[k] = c / m;
                if k != 0 && k != n / 2 {
                    self.g[n - k] = self.g[k].conj();
                }
            }
        }
        self.inv
            .process_with_scratch(&mut self.g, &mut self.scratch);
        let g = &self.g;
        let r = |lag: isize| -> f64 {
            // Lags stay within ±max_lag (+1 for the parabola) < n/2, so the index is in range.
            let idx = if lag >= 0 {
                lag as usize
            } else {
                (n as isize + lag) as usize
            };
            g[idx].re
        };
        let max_lag = self.max_lag as isize;
        let mut best = (0isize, f64::MIN);
        self.mags.clear();
        for lag in -max_lag..=max_lag {
            let v = r(lag);
            self.mags.push(v.abs());
            if v > best.1 {
                best = (lag, v);
            }
        }
        // The median |r|: the element a full sort would put in the middle.
        let mid = self.mags.len() / 2;
        let (_, median, _) = self.mags.select_nth_unstable_by(mid, f64::total_cmp);
        let median = median.max(1e-30);
        let (k, v) = best;
        let (a, b, c) = (r(k - 1), v, r(k + 1));
        let denom = a - 2.0 * b + c;
        let delta = if denom.abs() > 1e-30 {
            (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        est.lag = k as f64 + delta;
        // An unnormalised IFFT of unit-magnitude bins: a perfect match sums to 2·(bins in band).
        est.peak = v / (2.0 * (self.hi_bin + 1 - self.lo_bin) as f64);
        est.pnr = v / median;
        est.candidate = est.pnr >= self.params.min_pnr;
        est
    }
}

/// The line through the windows: `lag = intercept + slope · t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineFit {
    /// Lag at t = 0, samples.
    pub intercept: f64,
    /// Samples per second.
    pub slope: f64,
    /// RMS distance of the inliers from the line, samples.
    pub residual_rms: f64,
    /// Windows within ±1 ms of the line.
    pub inliers: usize,
    /// Seconds between the first and last inlier.
    pub span_s: f64,
}

/// A line needs this many inliers before it counts as an echo path (S0.3's rule).
pub const MIN_INLIERS: usize = 6;
/// ... and this share of the candidate windows. S0.3's recordings ran 90 s, where six
/// inliers are already most of the candidates (67 of 72 with echo); a session of an hour has
/// thousands of candidates, among which six can line up by chance.
pub const MIN_INLIER_SHARE: f64 = 0.25;
/// At most this many candidates propose lines, spread evenly over the session, so the fit stays
/// quick for an hour of windows. Every candidate still votes. S0.3's recordings had fewer.
pub const MAX_PROPOSERS: usize = 256;

/// The consensus fit of `lag = a + b·t` over the candidate windows.
///
/// PNR alone cannot tell echo from chance: on S0.3's synthetic no-echo mix, unrelated speech
/// reached PNR 19 while real double-talk windows sat near 9. What separates them is agreement:
/// echo windows fall on one line (residual ~0.01 ms), chance peaks scatter by hundreds of ms. So
/// every pair of proposing windows at least 5 s apart (and every single one, at zero slope)
/// proposes a line with |drift| ≤ 500 ppm; the line with the most candidates within ±1 ms wins
/// (ties go to the larger PNR sum); least squares refines it on its inliers, three times. Under
/// a 10 s inlier span the drift cannot be estimated and the fit is delay only (the median lag).
///
/// Returns the fit and which windows are its inliers, or `None` when it falls short of
/// [`MIN_INLIERS`] or [`MIN_INLIER_SHARE`].
pub fn consensus_fit(windows: &[WindowEstimate]) -> (Option<LineFit>, Vec<bool>) {
    let tol = 1.0e-3 * RATE;
    let max_slope = 500e-6 * RATE;
    let cand: Vec<usize> = (0..windows.len())
        .filter(|&i| windows[i].candidate)
        .collect();
    let mut used = vec![false; windows.len()];
    if cand.is_empty() {
        return (None, used);
    }
    let stride = cand.len().div_ceil(MAX_PROPOSERS);
    let proposers: Vec<usize> = cand.iter().copied().step_by(stride).collect();
    let inliers = |a: f64, b: f64| -> Vec<usize> {
        cand.iter()
            .copied()
            .filter(|&i| (windows[i].lag - (a + b * windows[i].center_s)).abs() <= tol)
            .collect()
    };
    let score = |a: f64, b: f64| -> (usize, f64) {
        cand.iter()
            .filter(|&&i| (windows[i].lag - (a + b * windows[i].center_s)).abs() <= tol)
            .fold((0, 0.0), |(n, w), &i| (n + 1, w + windows[i].pnr))
    };
    // (inliers, pnr sum, a, b)
    let mut best: (usize, f64, f64, f64) = (0, 0.0, 0.0, 0.0);
    let mut consider = |a: f64, b: f64| {
        let (n, w) = score(a, b);
        if n > best.0 || (n == best.0 && w > best.1) {
            best = (n, w, a, b);
        }
    };
    for &i in &proposers {
        consider(windows[i].lag, 0.0);
    }
    for (x, &i) in proposers.iter().enumerate() {
        for &j in &proposers[x + 1..] {
            let dt = windows[j].center_s - windows[i].center_s;
            if dt.abs() < 5.0 {
                continue;
            }
            let b = (windows[j].lag - windows[i].lag) / dt;
            if b.abs() > max_slope {
                continue;
            }
            consider(windows[i].lag - b * windows[i].center_s, b);
        }
    }
    let (mut a, mut b) = (best.2, best.3);
    let mut keep = inliers(a, b);
    for _ in 0..3 {
        let span = span_of(windows, &keep);
        if keep.len() >= MIN_INLIERS && span >= 10.0 {
            let n = keep.len() as f64;
            let mt = keep.iter().map(|&i| windows[i].center_s).sum::<f64>() / n;
            let ml = keep.iter().map(|&i| windows[i].lag).sum::<f64>() / n;
            let sxx: f64 = keep
                .iter()
                .map(|&i| (windows[i].center_s - mt).powi(2))
                .sum();
            let sxy: f64 = keep
                .iter()
                .map(|&i| (windows[i].center_s - mt) * (windows[i].lag - ml))
                .sum();
            b = sxy / sxx;
            a = ml - b * mt;
        } else if !keep.is_empty() {
            let mut lags: Vec<f64> = keep.iter().map(|&i| windows[i].lag).collect();
            lags.sort_by(f64::total_cmp);
            a = lags[lags.len() / 2];
            b = 0.0;
        }
        keep = inliers(a, b);
    }
    for &i in &keep {
        used[i] = true;
    }
    let share = keep.len() as f64 / cand.len() as f64;
    if keep.len() < MIN_INLIERS || share < MIN_INLIER_SHARE {
        return (None, used);
    }
    let rms = (keep
        .iter()
        .map(|&i| (windows[i].lag - (a + b * windows[i].center_s)).powi(2))
        .sum::<f64>()
        / keep.len() as f64)
        .sqrt();
    let fit = LineFit {
        intercept: a,
        slope: b,
        residual_rms: rms,
        inliers: keep.len(),
        span_s: span_of(windows, &keep),
    };
    (Some(fit), used)
}

fn span_of(windows: &[WindowEstimate], set: &[usize]) -> f64 {
    if set.is_empty() {
        return 0.0;
    }
    let hi = set
        .iter()
        .map(|&i| windows[i].center_s)
        .fold(f64::MIN, f64::max);
    let lo = set
        .iter()
        .map(|&i| windows[i].center_s)
        .fold(f64::MAX, f64::min);
    hi - lo
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u64) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((s >> 33) as f64 / (1u64 << 31) as f64 - 0.5) as f32
            })
            .collect()
    }

    fn windows(mic: &[f32], far: &[f32]) -> Vec<WindowEstimate> {
        let mut g = GccPhat::new(GccParams::default());
        let (w, hop) = (g.window_len(), g.hop_len());
        let mut out = Vec::new();
        let mut start = 0;
        while start + w <= mic.len().min(far.len()) {
            let c = (start as f64 + w as f64 / 2.0) / RATE;
            out.push(g.analyze(&mic[start..start + w], &far[start..start + w], c));
            start += hop;
        }
        out
    }

    /// White noise far end; the mic is the far end delayed by a whole number of samples. The
    /// window estimate lands on it exactly, and the sign says "mic lags".
    #[test]
    fn an_integer_lag_is_recovered_with_the_mic_lags_sign() {
        let far = noise(16_000 * 8, 12_345);
        let d = 123usize;
        let mut mic = vec![0.0f32; far.len()];
        mic[d..].copy_from_slice(&far[..far.len() - d]);
        let ws = windows(&mic, &far);
        assert!(ws.iter().all(|w| w.candidate));
        let (fit, _) = consensus_fit(&ws);
        let fit = fit.expect("a path");
        assert!(
            (fit.intercept - d as f64).abs() < 0.05,
            "intercept {}",
            fit.intercept
        );
        assert!(fit.slope.abs() < 1e-3, "slope {}", fit.slope);
    }

    #[test]
    fn a_mic_that_leads_gives_a_negative_lag() {
        let far = noise(16_000 * 8, 99);
        let d = 400usize;
        let mut mic = vec![0.0f32; far.len()];
        mic[..far.len() - d].copy_from_slice(&far[d..]);
        let ws = windows(&mic, &far);
        let (fit, _) = consensus_fit(&ws);
        let fit = fit.expect("a path");
        assert!((fit.intercept + d as f64).abs() < 0.05, "{}", fit.intercept);
    }

    #[test]
    fn unrelated_streams_give_no_line() {
        let far = noise(16_000 * 60, 1);
        let mic = noise(16_000 * 60, 2);
        let ws = windows(&mic, &far);
        let (fit, used) = consensus_fit(&ws);
        assert!(fit.is_none(), "{fit:?}");
        assert!(used.iter().filter(|u| **u).count() < MIN_INLIERS);
    }

    #[test]
    fn a_silent_far_end_gives_no_candidates() {
        let far = vec![0.0f32; 16_000 * 20];
        let mic = noise(16_000 * 20, 3);
        let ws = windows(&mic, &far);
        assert!(ws.iter().all(|w| !w.candidate));
        assert!(consensus_fit(&ws).0.is_none());
    }

    #[test]
    fn settings_it_cannot_mean_are_clamped_not_trusted() {
        let far = noise(16_000 * 3, 5);
        for params in [
            GccParams {
                win_s: 0.0,
                ..GccParams::default()
            },
            GccParams {
                max_lag_s: 30.0,
                ..GccParams::default()
            },
            GccParams {
                band_lo_hz: 7_000.0,
                band_hi_hz: 100.0,
                ..GccParams::default()
            },
            GccParams {
                win_s: f64::NAN,
                band_lo_hz: f64::NAN,
                ..GccParams::default()
            },
        ] {
            let mut g = GccPhat::new(params);
            let w = g.window_len();
            let est = g.analyze(&far[..w], &far[..w], 1.0);
            assert!(est.lag.is_finite(), "{params:?}");
        }
    }

    fn est(center_s: f64, lag: f64) -> WindowEstimate {
        WindowEstimate {
            center_s,
            lag,
            peak: 0.5,
            pnr: 20.0,
            far_db: -30.0,
            mic_db: -30.0,
            candidate: true,
        }
    }

    #[test]
    fn the_fit_is_the_consensus_line_not_the_loudest_outlier() {
        // 20 windows on lag = 736 + 0.03·t, and 8 chance peaks scattered far off it.
        let mut ws: Vec<WindowEstimate> = (0..20)
            .map(|i| {
                est(
                    1.0 + 3.0 * f64::from(i),
                    736.0 + 0.03 * (1.0 + 3.0 * f64::from(i)),
                )
            })
            .collect();
        for (i, lag) in [
            -4000.0, 2500.0, -300.0, 7000.0, 90.0, -6500.0, 3100.0, 1500.0,
        ]
        .into_iter()
        .enumerate()
        {
            let mut w = est(2.5 + 7.0 * i as f64, lag);
            w.pnr = 60.0;
            ws.push(w);
        }
        let (fit, used) = consensus_fit(&ws);
        let fit = fit.expect("a path");
        assert_eq!(fit.inliers, 20);
        assert!((fit.intercept - 736.0).abs() < 1e-6, "{}", fit.intercept);
        assert!((fit.slope - 0.03).abs() < 1e-9, "{}", fit.slope);
        assert!(used[..20].iter().all(|u| *u) && used[20..].iter().all(|u| !*u));
    }

    #[test]
    fn a_short_span_gives_a_delay_only_fit() {
        let ws: Vec<WindowEstimate> = (0..8)
            .map(|i| est(1.0 + f64::from(i), 500.0 + 0.2 * f64::from(i % 2)))
            .collect();
        let fit = consensus_fit(&ws).0.expect("a path");
        assert_eq!(fit.slope, 0.0);
        assert!((fit.intercept - 500.0).abs() <= 0.2);
    }

    #[test]
    fn six_inliers_among_many_chance_candidates_are_not_a_path() {
        // Six windows agree, but 40 candidates scatter: a quarter must agree before it is an
        // echo path.
        let mut ws: Vec<WindowEstimate> = (0..6).map(|i| est(f64::from(i) * 4.0, 200.0)).collect();
        for i in 0..40 {
            ws.push(est(1.0 + f64::from(i), -7_000.0 + 350.0 * f64::from(i)));
        }
        assert!(consensus_fit(&ws).0.is_none());
        // With only six candidates in all, the same six are a path (S0.3's rule).
        assert!(consensus_fit(&ws[..6]).0.is_some());
    }
}
