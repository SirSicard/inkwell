//! Long audio (file import, a meeting's final pass) in windows an engine can take.
//!
//! Windows are at most [`WindowConfig::max_len`] (60 s) of 16 kHz audio. Each boundary is cut at
//! the quietest 20 ms frame in the last [`WindowConfig::search`] (3 s) of the window, so a cut lands
//! in a pause or between words rather than at a blind offset mid-phoneme. Consecutive windows
//! overlap by [`WindowConfig::overlap`] (2 s), so a word near a seam is heard whole at least once.
//! Merging the overlapping transcripts is the pipeline's job, not this module's.
//!
//! The 60 s comes from an earlier implementation, whose 15 s windows produced every duplicated
//! half-word at their seams ("pro prolific"); the models are long-form, so longer windows make
//! seams rare while keeping memory bounded. Here 60 s is a hard ceiling: the search region is the
//! window's last 3 s, not 1.5 s either side of the 60 s mark.
//!
//! [`Windower`] takes the audio in pushes and hands out each window once it can be decided, so a
//! meeting's final pass reads its chunks from disk one at a time and holds about one window in
//! memory, never the session (architecture rule 3). [`plan_windows`] runs it over a buffer
//! already in memory.

use std::fmt;
use std::ops::Range;

use crate::gain::LEVEL_FRAME;

/// Window sizes, in samples at 16 kHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowConfig {
    /// The longest window (960 000 = 60 s).
    pub max_len: usize,
    /// Audio shared by consecutive windows (32 000 = 2 s).
    pub overlap: usize,
    /// How far back from `max_len` a cut may land (48 000 = 3 s).
    pub search: usize,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            max_len: 60 * 16_000,
            overlap: 2 * 16_000,
            search: 3 * 16_000,
        }
    }
}

/// Why windowing refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowError {
    /// The sizes could not make progress (the overlap and search fill the window) or the search
    /// is shorter than one level frame.
    InvalidConfig(String),
    /// Audio pushed after [`Windower::finish`].
    Finished,
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(why) => write!(f, "windowing: invalid sizes: {why}"),
            Self::Finished => f.write_str("windowing: audio pushed after finish"),
        }
    }
}

impl std::error::Error for WindowError {}

/// A window, in samples from the start of the audio. `end` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// First sample.
    pub start: u64,
    /// One past the last sample.
    pub end: u64,
}

impl Window {
    /// Samples in the window.
    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    /// Whether the window holds no samples (never true of a window the windower hands out).
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// The cut point in `samples` within `region`: the middle of the quietest 20 ms frame that fits
/// in it (by energy; ties go to the latest frame, which makes windows as long as allowed). A region
/// shorter than a frame gives its end. The result is clamped to `samples.len()`.
pub fn quietest_cut(samples: &[f32], region: Range<usize>) -> usize {
    let hi = region.end.min(samples.len());
    let lo = region.start.min(hi);
    if hi - lo < LEVEL_FRAME {
        return hi;
    }
    let mut best = lo;
    let mut best_energy = f32::INFINITY;
    let mut i = lo;
    while i + LEVEL_FRAME <= hi {
        let energy: f32 = samples[i..i + LEVEL_FRAME].iter().map(|s| s * s).sum();
        if energy <= best_energy {
            best_energy = energy;
            best = i;
        }
        i += LEVEL_FRAME;
    }
    best + LEVEL_FRAME / 2
}

/// Cuts pushed audio into windows. See the module docs.
///
/// **Worker.** Its buffer grows to about one window plus one push, and is compacted after each
/// window is taken.
pub struct Windower {
    cfg: WindowConfig,
    buf: Vec<f32>,
    /// Position of `buf[0]` in the whole audio.
    buf_start: u64,
    /// Where the next window starts.
    win_start: u64,
    /// Where the last window handed out ended.
    last_end: Option<u64>,
    /// The next window's start, applied on the next call (the last window's samples are borrowed
    /// until then).
    advance_to: Option<u64>,
    finished: bool,
}

impl Windower {
    /// A windower with `cfg`, refusing sizes that could not advance.
    pub fn new(cfg: WindowConfig) -> Result<Self, WindowError> {
        if cfg.search < LEVEL_FRAME {
            return Err(WindowError::InvalidConfig(format!(
                "search {} is shorter than a {LEVEL_FRAME}-sample frame",
                cfg.search
            )));
        }
        if cfg.overlap + cfg.search >= cfg.max_len {
            return Err(WindowError::InvalidConfig(format!(
                "overlap {} plus search {} must be less than max_len {}",
                cfg.overlap, cfg.search, cfg.max_len
            )));
        }
        Ok(Self {
            cfg,
            buf: Vec::new(),
            buf_start: 0,
            win_start: 0,
            last_end: None,
            advance_to: None,
            finished: false,
        })
    }

    /// Appends audio.
    pub fn push(&mut self, audio: &[f32]) -> Result<(), WindowError> {
        if self.finished {
            return Err(WindowError::Finished);
        }
        self.buf.extend_from_slice(audio);
        Ok(())
    }

    /// Marks the end of the audio: the rest comes out as the last window.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Samples held right now.
    pub fn buffered_samples(&self) -> usize {
        self.buf.len()
    }

    /// The next window and its samples, once it can be decided: when more than a whole window of
    /// audio is buffered (so a cut is needed and its whole search region is here), or after
    /// [`finish`](Self::finish). `None` means push more, or, after `finish`, that every window has
    /// been handed out.
    pub fn next_window(&mut self) -> Option<(Window, &[f32])> {
        if let Some(next) = self.advance_to.take() {
            // Cannot underflow: `next` is at or after `buf_start` and within the buffer (a cut
            // minus an overlap that `new` keeps shorter than the window before it).
            let drop = usize::try_from(next - self.buf_start).unwrap_or(self.buf.len());
            self.buf.drain(..drop.min(self.buf.len()));
            self.buf_start = next;
            self.win_start = next;
        }
        let end = self.buf_start + self.buf.len() as u64;
        let limit = self.win_start + self.cfg.max_len as u64;
        if end > limit {
            let offset = (self.win_start - self.buf_start) as usize;
            let region_end = offset + self.cfg.max_len;
            let cut = quietest_cut(&self.buf, region_end - self.cfg.search..region_end);
            let window = Window {
                start: self.win_start,
                end: self.buf_start + cut as u64,
            };
            self.last_end = Some(window.end);
            self.advance_to = Some(window.end - self.cfg.overlap as u64);
            return Some((window, &self.buf[offset..cut]));
        }
        let beyond_last = self.last_end.is_none_or(|e| end > e);
        if self.finished && end > self.win_start && beyond_last {
            let offset = (self.win_start - self.buf_start) as usize;
            let window = Window {
                start: self.win_start,
                end,
            };
            self.last_end = Some(end);
            self.advance_to = Some(end);
            return Some((window, &self.buf[offset..]));
        }
        None
    }
}

/// The windows for a buffer already in memory (a dictation, a short import).
///
/// **Worker.** Copies the buffer once through a [`Windower`].
pub fn plan_windows(audio: &[f32], cfg: WindowConfig) -> Result<Vec<Window>, WindowError> {
    let mut windower = Windower::new(cfg)?;
    windower.push(audio)?;
    windower.finish();
    let mut windows = Vec::new();
    while let Some((window, _)) = windower.next_window() {
        windows.push(window);
    }
    Ok(windows)
}
