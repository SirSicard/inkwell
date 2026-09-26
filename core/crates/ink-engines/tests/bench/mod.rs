//! Bench helpers for the real-model tests: the word error rate scorer, a WAV reader and the bench
//! directory. Nothing here needs a model; `bench_scorer.rs` proves the scorer on toy cases.
//!
//! The scorer is the one the engine choice was measured with, ported: lower-case, every character
//! that is not a letter, mark or number becomes a space, split on whitespace; word error rate is
//! (substitutions + deletions + insertions) / reference words, and a corpus rate sums edits and
//! reference words over documents (it is not the mean of per-document rates).
//!
//! One difference, stated rather than hidden: the reference keeps Unicode categories L, M and N;
//! Rust's standard library has no category table, so this keeps `char::is_alphanumeric` (Unicode
//! Alphabetic or Numeric). They differ only on combining marks outside Other_Alphabetic and on a
//! few letter-like symbols. English AMI and FLEURS text is ASCII after lower-casing, where the two
//! agree exactly.

#![allow(dead_code)] // Each test binary uses a different subset.

use std::fmt;
use std::path::{Path, PathBuf};

/// The ported normaliser.
pub fn normalise(text: &str) -> Vec<String> {
    text.to_lowercase()
        .chars()
        .map(|c| {
            if c == ' ' || c.is_alphanumeric() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// Edit counts for one reference/hypothesis pair (or a corpus, summed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edits {
    /// Reference words.
    pub reference: usize,
    /// Substitutions.
    pub substitutions: usize,
    /// Deletions (reference words missing from the hypothesis).
    pub deletions: usize,
    /// Insertions (hypothesis words not in the reference).
    pub insertions: usize,
}

impl Edits {
    /// Total edits.
    pub fn total(&self) -> usize {
        self.substitutions + self.deletions + self.insertions
    }

    /// Word error rate in percent. NaN for an empty reference, as in the reference scorer.
    pub fn wer(&self) -> f64 {
        if self.reference == 0 {
            f64::NAN
        } else {
            100.0 * self.total() as f64 / self.reference as f64
        }
    }
}

impl std::ops::Add for Edits {
    type Output = Edits;

    fn add(self, o: Edits) -> Edits {
        Edits {
            reference: self.reference + o.reference,
            substitutions: self.substitutions + o.substitutions,
            deletions: self.deletions + o.deletions,
            insertions: self.insertions + o.insertions,
        }
    }
}

impl fmt::Display for Edits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "WER {:6.2}  ({} edits / {} words; S/D/I {}/{}/{})",
            self.wer(),
            self.total(),
            self.reference,
            self.substitutions,
            self.deletions,
            self.insertions
        )
    }
}

/// Word-level Levenshtein alignment with a backtrace for the S/D/I split. The total is the edit
/// distance; the split between S, D and I can differ from other aligners' on ties, so only the
/// total (and so the rate) is compared across scorers.
pub fn measure<S: AsRef<str>>(reference: &[S], hypothesis: &[S]) -> Edits {
    let (n, m) = (reference.len(), hypothesis.len());
    // cost[i][j]: edits to turn reference[..i] into hypothesis[..j].
    let mut cost = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in cost.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in cost[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let same = reference[i - 1].as_ref() == hypothesis[j - 1].as_ref();
            cost[i][j] = (cost[i - 1][j - 1] + usize::from(!same))
                .min(cost[i - 1][j] + 1)
                .min(cost[i][j - 1] + 1);
        }
    }
    let mut e = Edits {
        reference: n,
        ..Edits::default()
    };
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 {
            let same = reference[i - 1].as_ref() == hypothesis[j - 1].as_ref();
            if cost[i][j] == cost[i - 1][j - 1] + usize::from(!same) {
                e.substitutions += usize::from(!same);
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if i > 0 && cost[i][j] == cost[i - 1][j] + 1 {
            e.deletions += 1;
            i -= 1;
        } else {
            e.insertions += 1;
            j -= 1;
        }
    }
    e
}

/// An independent edit distance (two rows, no backtrace), to cross-check [`measure`].
pub fn edit_distance<S: AsRef<str>>(reference: &[S], hypothesis: &[S]) -> usize {
    let mut prev: Vec<usize> = (0..=hypothesis.len()).collect();
    for (i, r) in reference.iter().enumerate() {
        let mut cur = vec![i + 1; hypothesis.len() + 1];
        for (j, h) in hypothesis.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(r.as_ref() != h.as_ref()))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[hypothesis.len()]
}

/// Scores one document: both sides normalised.
pub fn score(reference: &str, hypothesis: &str) -> Edits {
    measure(&normalise(reference), &normalise(hypothesis))
}

/// `$INK_BENCH_DIR`, or a panic that says how to set it. Real-model tests fail loudly rather than
/// pass without running.
pub fn bench_dir() -> PathBuf {
    match std::env::var_os("INK_BENCH_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => panic!(
            "INK_BENCH_DIR is not set: point it at the bench data (models/, ami-ihm/, ami-ihm.tsv) \
             and run with --ignored"
        ),
    }
}

/// Reads a mono WAV file as f32 samples: 16-bit PCM or 32-bit float, including the extensible
/// header. Returns (sample rate, samples). Anything else is an error, never a silent misread.
pub fn read_wav(path: &Path) -> Result<(u32, Vec<f32>), String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{}: not a RIFF/WAVE file", path.display()));
    }
    let u16_at = |p: usize| u16::from_le_bytes([data[p], data[p + 1]]);
    let u32_at = |p: usize| u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
    let (mut pos, mut fmt, mut payload) = (12, None, None);
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32_at(pos + 4) as usize;
        let body = pos + 8..(pos + 8 + size).min(data.len());
        if id == b"fmt " && body.len() >= 16 {
            let mut tag = u16_at(body.start);
            if tag == 0xFFFE && body.len() >= 26 {
                tag = u16_at(body.start + 24);
            }
            fmt = Some((
                tag,
                u16_at(body.start + 2),
                u32_at(body.start + 4),
                u16_at(body.start + 14),
            ));
        } else if id == b"data" {
            payload = Some(body);
        }
        pos += 8 + size + (size & 1);
    }
    let (tag, channels, rate, bits) =
        fmt.ok_or_else(|| format!("{}: no fmt chunk", path.display()))?;
    let payload = &data[payload.ok_or_else(|| format!("{}: no data chunk", path.display()))?];
    if channels != 1 {
        return Err(format!(
            "{}: {channels} channels, expected mono",
            path.display()
        ));
    }
    let samples = match (tag, bits) {
        (3, 32) => payload
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
        (1, 16) => payload
            .chunks_exact(2)
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
            .collect(),
        _ => {
            return Err(format!(
                "{}: unsupported format tag {tag}, {bits} bits",
                path.display()
            ));
        }
    };
    Ok((rate, samples))
}

/// One row of an AMI TSV: the WAV's file name (column 1) and the reference (column 3, the column
/// the engine choice was scored against).
pub struct AmiRow {
    pub wav: String,
    pub reference: String,
}

pub fn read_ami_tsv(path: &Path) -> Result<Vec<AmiRow>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            match (cols.get(1), cols.get(3)) {
                (Some(wav), Some(reference)) if !wav.is_empty() => Ok(AmiRow {
                    wav: (*wav).to_owned(),
                    reference: (*reference).to_owned(),
                }),
                _ => Err(format!("{}: a row without columns 1 and 3", path.display())),
            }
        })
        .collect()
}
