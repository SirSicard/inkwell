//! Diarization error rate, scored the way pyannote.metrics 4.1 scores it, so these numbers and
//! the ones the diarizer was chosen on are the same measurement:
//!
//! - Overlapped speech is scored: two reference speakers at once count twice.
//! - The collar is given **per side** (±0.25 s for AMI), and removed around every reference
//!   boundary from both reference and hypothesis.
//! - With no evaluation map, the region scored is the extent of reference and hypothesis together.
//! - Hypothesis labels map one-to-one onto reference labels to maximise their total co-occurrence
//!   (the Hungarian mapping; here an exact search over subsets, which gives the same total).
//! - A second RTTM line with the same start and duration replaces the first, as pyannote's
//!   `annotation[segment] = label` does.
//!
//! `tests/der.rs` proves it on toy cases with known answers first.

#![allow(dead_code)] // Each test binary uses a different subset.

use std::collections::HashMap;

/// Durations under this (seconds) are empty, as pyannote's segment precision makes them.
const PRECISION: f64 = 1e-6;

/// One labelled stretch of speech.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub start: f64,
    pub end: f64,
    pub label: String,
}

impl Turn {
    pub fn new(start: f64, end: f64, label: &str) -> Self {
        Self {
            start,
            end,
            label: label.into(),
        }
    }
}

/// The `SPEAKER` lines of an RTTM file, in file order, zero-length ones dropped and exact
/// duplicate segments replaced by the later line.
pub fn parse_rttm(text: &str) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.first() != Some(&"SPEAKER") {
            continue;
        }
        let (onset, duration): (f64, f64) = (
            fields[3].parse().expect("RTTM onset"),
            fields[4].parse().expect("RTTM duration"),
        );
        if duration <= 0.0 {
            continue;
        }
        let turn = Turn::new(onset, onset + duration, fields[7]);
        match turns
            .iter_mut()
            .find(|t| t.start == turn.start && t.end == turn.end)
        {
            Some(same) => same.label = turn.label,
            None => turns.push(turn),
        }
    }
    turns
}

/// Turns as RTTM lines for `recording`.
pub fn to_rttm(recording: &str, turns: &[Turn]) -> String {
    turns
        .iter()
        .map(|t| {
            format!(
                "SPEAKER {recording} 1 {:.3} {:.3} <NA> <NA> {} <NA> <NA>\n",
                t.start,
                t.end - t.start,
                t.label
            )
        })
        .collect()
}

/// A diarization error rate and its parts, in percent of the scored reference speech.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Der {
    pub der: f64,
    pub miss: f64,
    pub false_alarm: f64,
    pub confusion: f64,
    /// Scored reference speech, in seconds (overlap counted per speaker).
    pub total: f64,
}

/// Merged, sorted intervals.
fn support(mut intervals: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    intervals.retain(|(a, b)| b - a > PRECISION);
    intervals.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in intervals {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn duration(intervals: &[(f64, f64)]) -> f64 {
    intervals.iter().map(|(a, b)| b - a).sum()
}

fn intersect(a: (f64, f64), b: (f64, f64)) -> Option<(f64, f64)> {
    let (lo, hi) = (a.0.max(b.0), a.1.min(b.1));
    (hi - lo > PRECISION).then_some((lo, hi))
}

/// `turns` cut to the evaluation intervals `uem`.
fn crop(turns: &[Turn], uem: &[(f64, f64)]) -> Vec<Turn> {
    let mut out = Vec::new();
    for t in turns {
        for &u in uem {
            if let Some((a, b)) = intersect((t.start, t.end), u) {
                out.push(Turn::new(a, b, &t.label));
            }
        }
    }
    out
}

fn labels(turns: &[Turn]) -> Vec<String> {
    let mut out: Vec<String> = turns.iter().map(|t| t.label.clone()).collect();
    out.sort();
    out.dedup();
    out
}

/// For each hypothesis label, the reference label it maps to under the mapping that maximises
/// total co-occurrence (unmapped labels are absent).
fn optimal_mapping(reference: &[Turn], hypothesis: &[Turn]) -> HashMap<String, String> {
    let (r_labels, h_labels) = (labels(reference), labels(hypothesis));
    let mut co = vec![vec![0.0f64; r_labels.len()]; h_labels.len()];
    for h in hypothesis {
        let i = h_labels.iter().position(|l| *l == h.label).unwrap();
        for r in reference {
            if let Some((a, b)) = intersect((h.start, h.end), (r.start, r.end)) {
                let j = r_labels.iter().position(|l| *l == r.label).unwrap();
                co[i][j] += b - a;
            }
        }
    }
    assert!(
        r_labels.len() <= 20,
        "too many reference speakers for the exact search"
    );
    // best[i][mask]: the most co-occurrence hypothesis labels i.. can add with reference labels
    // in `mask` already taken. Exact, like the Hungarian algorithm.
    let full = 1usize << r_labels.len();
    let n = h_labels.len();
    let mut best = vec![vec![0.0f64; full]; n + 1];
    for i in (0..n).rev() {
        for mask in 0..full {
            let mut b = best[i + 1][mask];
            for j in 0..r_labels.len() {
                if mask & (1 << j) == 0 {
                    b = b.max(co[i][j] + best[i + 1][mask | (1 << j)]);
                }
            }
            best[i][mask] = b;
        }
    }
    let mut mapping = HashMap::new();
    let mut mask = 0usize;
    for i in 0..n {
        if best[i][mask] == best[i + 1][mask] {
            continue;
        }
        let j = (0..r_labels.len())
            .find(|&j| {
                mask & (1 << j) == 0 && co[i][j] + best[i + 1][mask | (1 << j)] == best[i][mask]
            })
            .unwrap();
        if co[i][j] > 0.0 {
            mapping.insert(h_labels[i].clone(), r_labels[j].clone());
        }
        mask |= 1 << j;
    }
    mapping
}

/// DER of `hypothesis` against `reference` with a collar of `collar` seconds on each side of
/// every reference boundary (0.25 for AMI's standard; 0 for none).
pub fn der(reference: &[Turn], hypothesis: &[Turn], collar: f64) -> Der {
    let bounds = reference
        .iter()
        .chain(hypothesis)
        .fold(None, |acc, t| match acc {
            None => Some((t.start, t.end)),
            Some((a, b)) => Some((f64::min(a, t.start), f64::max(b, t.end))),
        });
    let Some(extent) = bounds else {
        return Der {
            der: 0.0,
            miss: 0.0,
            false_alarm: 0.0,
            confusion: 0.0,
            total: 0.0,
        };
    };
    // The scored region: the extent, less the collars around every reference boundary.
    let uem = if collar > 0.0 {
        let collars = support(
            reference
                .iter()
                .flat_map(|t| [t.start, t.end])
                .map(|x| (x - collar, x + collar))
                .collect(),
        );
        let mut gaps = Vec::new();
        let mut at = extent.0;
        for (a, b) in collars {
            if a - at > PRECISION {
                gaps.push((at, a.min(extent.1)));
            }
            at = at.max(b);
        }
        if extent.1 - at > PRECISION {
            gaps.push((at, extent.1));
        }
        gaps.retain(|(a, b)| b - a > PRECISION);
        gaps
    } else {
        vec![extent]
    };
    let reference = crop(reference, &uem);
    let hypothesis = crop(hypothesis, &uem);
    let mapping = optimal_mapping(&reference, &hypothesis);

    let mut cuts: Vec<f64> = reference
        .iter()
        .chain(&hypothesis)
        .flat_map(|t| [t.start, t.end])
        .collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let (mut total, mut miss, mut fa, mut confusion) = (0.0, 0.0, 0.0, 0.0);
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b - a <= PRECISION {
            continue;
        }
        let covers = |t: &&Turn| t.start <= a && t.end >= b;
        let r: Vec<&str> = reference
            .iter()
            .filter(covers)
            .map(|t| t.label.as_str())
            .collect();
        let h: Vec<Option<&str>> = hypothesis
            .iter()
            .filter(covers)
            .map(|t| mapping.get(&t.label).map(String::as_str))
            .collect();
        let d = b - a;
        let (nr, nh) = (r.len(), h.len());
        // Correct: reference labels matched by a mapped hypothesis label, one to one.
        let mut correct = 0;
        let mut unused: Vec<Option<&str>> = h.clone();
        for label in &r {
            if let Some(k) = unused.iter().position(|u| *u == Some(*label)) {
                unused.swap_remove(k);
                correct += 1;
            }
        }
        total += d * nr as f64;
        miss += d * nr.saturating_sub(nh) as f64;
        fa += d * nh.saturating_sub(nr) as f64;
        confusion += d * (nr.min(nh) - correct) as f64;
    }
    let pct = |x: f64| if total > 0.0 { 100.0 * x / total } else { 0.0 };
    Der {
        der: pct(miss + fa + confusion),
        miss: pct(miss),
        false_alarm: pct(fa),
        confusion: pct(confusion),
        total,
    }
}

/// Under the optimal mapping of the uncropped annotations, the share (in percent) of each
/// reference speaker's time their cluster was given; the smallest one. Near zero means that
/// speaker was merged into someone else.
pub fn min_speaker_recall(reference: &[Turn], hypothesis: &[Turn]) -> f64 {
    let mapping = optimal_mapping(reference, hypothesis);
    let timeline = |turns: &[Turn], label: &str| {
        support(
            turns
                .iter()
                .filter(|t| t.label == label)
                .map(|t| (t.start, t.end))
                .collect(),
        )
    };
    labels(reference)
        .iter()
        .map(|speaker| {
            let r = timeline(reference, speaker);
            let got = mapping
                .iter()
                .find(|(_, v)| *v == speaker)
                .map_or(0.0, |(h, _)| {
                    let h = timeline(hypothesis, h);
                    let both: Vec<_> = r
                        .iter()
                        .flat_map(|&a| h.iter().filter_map(move |&b| intersect(a, b)))
                        .collect();
                    duration(&support(both))
                });
            100.0 * got / duration(&r)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Hypothesis clusters, and how many hold at least 2 % of its speech (architecture rule 5).
pub fn clusters(hypothesis: &[Turn]) -> (usize, usize) {
    let each: Vec<f64> = labels(hypothesis)
        .iter()
        .map(|l| {
            duration(&support(
                hypothesis
                    .iter()
                    .filter(|t| t.label == *l)
                    .map(|t| (t.start, t.end))
                    .collect(),
            ))
        })
        .collect();
    let total: f64 = each.iter().sum::<f64>().max(f64::MIN_POSITIVE);
    (
        each.len(),
        each.iter().filter(|&&d| 100.0 * d / total >= 2.0).count(),
    )
}
