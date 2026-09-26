//! The numbers echo cancellation is judged on: frame levels, activity, ERLE and how much of the
//! near end survives. S0.3's definitions, kept here so the bench and the tests share them.
//!
//! Every signal is on one grid (the mic's, at 16 kHz); frames are 10 ms.

use crate::FRAME;

/// Mean-square level of each 10 ms frame, dBFS (−200 for digital silence).
pub fn frame_db(x: &[f32]) -> Vec<f64> {
    x.chunks(FRAME)
        .map(|c| {
            let ms = c.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / c.len() as f64;
            10.0 * (ms + 1e-20).log10()
        })
        .collect()
}

/// The `q` quantile (0–1) of the finite values, nearest rank; −∞ when there are none.
pub fn percentile(v: &[f64], q: f64) -> f64 {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return f64::NEG_INFINITY;
    }
    s.sort_by(f64::total_cmp);
    s[((s.len() - 1) as f64 * q).round() as usize]
}

/// Activity: a frame is active above (its signal's 95th percentile − 30 dB) and above `floor_db`.
pub fn active(levels: &[f64], floor_db: f64) -> Vec<bool> {
    let thr = (percentile(levels, 0.95) - 30.0).max(floor_db);
    levels.iter().map(|l| *l > thr).collect()
}

/// Extends each active frame `before` frames earlier and `after` frames later.
pub fn dilate(a: &[bool], before: usize, after: usize) -> Vec<bool> {
    let mut out = vec![false; a.len()];
    for (i, &on) in a.iter().enumerate() {
        if on {
            let lo = i.saturating_sub(before);
            let hi = (i + after).min(a.len().saturating_sub(1));
            for o in &mut out[lo..=hi] {
                *o = true;
            }
        }
    }
    out
}

/// Energy of `x` over the selected frames.
pub fn energy(x: &[f32], frames: &[bool]) -> f64 {
    frames
        .iter()
        .enumerate()
        .filter(|(_, on)| **on)
        .map(|(f, _)| {
            let lo = (f * FRAME).min(x.len());
            let hi = ((f + 1) * FRAME).min(x.len());
            x[lo..hi].iter().map(|v| f64::from(*v).powi(2)).sum::<f64>()
        })
        .sum()
}

/// `10·log10(num / den)`, or NaN when either is not positive.
pub fn ratio_db(num: f64, den: f64) -> f64 {
    if num <= 0.0 || den <= 0.0 {
        f64::NAN
    } else {
        10.0 * (num / den).log10()
    }
}

/// ERLE over the selected frames: the mic's energy over the output's, dB. Energy-weighted, so
/// loud passages count for more, as in S0.3.
pub fn erle_db(mic: &[f32], out: &[f32], frames: &[bool]) -> f64 {
    ratio_db(energy(mic, frames), energy(out, frames))
}

/// How much of `reference` survives in `y` over the selected frames, whatever else `y` holds:
/// `20·log10(⟨y, ref⟩ / ⟨ref, ref⟩)`. NaN when it does not survive at all.
pub fn projection_gain_db(y: &[f32], reference: &[f32], frames: &[bool]) -> f64 {
    let (mut xy, mut xx) = (0.0, 0.0);
    for (f, on) in frames.iter().enumerate() {
        if !*on {
            continue;
        }
        let hi = ((f + 1) * FRAME).min(y.len()).min(reference.len());
        for i in (f * FRAME).min(hi)..hi {
            xy += f64::from(y[i]) * f64::from(reference[i]);
            xx += f64::from(reference[i]) * f64::from(reference[i]);
        }
    }
    if xx <= 0.0 || xy <= 0.0 {
        f64::NAN
    } else {
        20.0 * (xy / xx).log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_gain_of_half_is_minus_6_db() {
        let r: Vec<f32> = (0..1600).map(|i| ((i as f32) * 0.37).sin()).collect();
        let y: Vec<f32> = r.iter().map(|v| v * 0.5).collect();
        let g = projection_gain_db(&y, &r, &[true; 10]);
        assert!((g - (-6.0206)).abs() < 1e-3, "{g}");
    }

    #[test]
    fn projection_gain_ignores_what_is_uncorrelated() {
        let r: Vec<f32> = (0..16_000).map(|i| ((i as f32) * 0.37).sin()).collect();
        let y: Vec<f32> = r
            .iter()
            .enumerate()
            .map(|(i, v)| v + ((i as f32) * 1.91).sin())
            .collect();
        let g = projection_gain_db(&y, &r, &[true; 100]);
        assert!(g.abs() < 0.05, "{g}");
    }

    #[test]
    fn erle_is_the_energy_ratio_over_the_chosen_frames() {
        let mic = vec![0.1f32; 4 * FRAME];
        let mut out = vec![0.01f32; 4 * FRAME];
        out[3 * FRAME..].fill(1.0);
        assert!((erle_db(&mic, &out, &[true, true, false, false]) - 20.0).abs() < 1e-4);
        assert!(erle_db(&mic, &out, &[false; 4]).is_nan());
    }

    #[test]
    fn activity_is_relative_to_the_loud_frames_and_floored() {
        let levels = [-20.0, -45.0, -55.0, -80.0, -20.0, -20.0];
        assert_eq!(
            active(&levels, -70.0),
            [true, true, false, false, true, true]
        );
        assert_eq!(active(&[-75.0, -90.0], -70.0), [false, false]);
        assert_eq!(
            dilate(&[false, true, false, false], 1, 2),
            [true, true, true, true]
        );
    }
}
