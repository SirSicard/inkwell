//! Dictation takes: the audio between a key press and its release, with 300 ms either side.
//!
//! People start speaking as they press the key and are still finishing the last word as they let
//! go. So a take begins [`LEAD_SAMPLES`] (300 ms) before the press, from a short history of what the
//! mic already heard, and ends [`TAIL_SAMPLES`] (300 ms) after the release. An earlier
//! implementation's transcript history had "Cl." for a name whose first phoneme fell before the
//! press; the lead is what fixed it.
//!
//! What [`TakeRecorder`] keeps from that implementation:
//!
//! - **Back-to-back takes never share audio.** A take claims everything up to its end, and the next
//!   take's lead starts no earlier, so a quick second press never re-transcribes (and pastes) the
//!   end of the first.
//! - **The live length excludes the lead and tail** ([`Take::live`]). A take always holds 600 ms of
//!   padding, so a too-short check on its total length would let a stray tap transcribe the room.
//! - **A press older than the history gets what is left, never garbage.** The history holds
//!   [`HISTORY_SAMPLES`] (2 s: the lead plus slack for a late key event).
//!
//! And what changes: the tail is waited for **in audio, not in time**. The take completes in the
//! [`push`](TakeRecorder::push) that delivers the 300th millisecond after the release, not after a
//! fixed sleep, so it is exactly as late as the audio is.
//!
//! Positions are absolute sample indices in the 16 kHz stream (the first sample pushed is 0). The
//! pipeline maps a key event's host time onto them through the chunk timestamps; an event may
//! arrive before or after the audio of its moment has been pushed, and both work.
//!
//! # Threading and allocation
//!
//! **Pump.** [`TakeRecorder::new`] allocates the history. [`press`](TakeRecorder::press)
//! allocates the take's buffer, sized for a minute (a longer dictation grows it). Pushes allocate
//! nothing otherwise.

use std::time::Duration;

/// The lead kept before the press.
pub const LEAD: Duration = Duration::from_millis(300);

/// The tail kept after the release.
pub const TAIL: Duration = Duration::from_millis(300);

/// [`LEAD`] in samples at 16 kHz.
pub const LEAD_SAMPLES: usize = 4_800;

/// [`TAIL`] in samples at 16 kHz.
pub const TAIL_SAMPLES: usize = 4_800;

/// How much recent audio the recorder keeps for a lead: 2 s at 16 kHz.
pub const HISTORY_SAMPLES: usize = 32_000;

/// Samples a take reserves when it opens: one minute.
const TAKE_RESERVE: usize = 60 * 16_000;

/// A finished take. Positions are absolute; `start <= press <= release <= end`.
#[derive(Clone, Debug, PartialEq)]
pub struct Take {
    /// The audio, `start..end`.
    pub samples: Vec<f32>,
    /// Where the take's audio begins (the lead's start).
    pub start: u64,
    /// The key press.
    pub press: u64,
    /// The key release.
    pub release: u64,
    /// Where the take's audio ends (the tail's end).
    pub end: u64,
}

impl Take {
    /// Samples before the press.
    pub fn lead(&self) -> usize {
        (self.press - self.start) as usize
    }

    /// Samples between press and release: the length to judge a too-short take by.
    pub fn live(&self) -> usize {
        (self.release - self.press) as usize
    }

    /// Samples after the release.
    pub fn tail(&self) -> usize {
        (self.end - self.release) as usize
    }
}

/// Records dictation takes from the 16 kHz mic stream. See the module docs.
pub struct TakeRecorder {
    /// Ring of recent audio: position `p` lives at `p % HISTORY_SAMPLES`.
    history: Vec<f32>,
    /// Samples pushed so far.
    written: u64,
    /// Audio before this belongs to an earlier take (or a cancelled one).
    claimed: u64,
    open: Option<Open>,
}

struct Open {
    samples: Vec<f32>,
    start: u64,
    press: u64,
    release: Option<u64>,
}

impl Default for TakeRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl TakeRecorder {
    /// A recorder at position 0. **Allocates** the history.
    pub fn new() -> Self {
        Self {
            history: vec![0.0; HISTORY_SAMPLES],
            written: 0,
            claimed: 0,
            open: None,
        }
    }

    /// Samples pushed so far: the position of the next sample.
    pub fn position(&self) -> u64 {
        self.written
    }

    /// Opens a take for a key press at `at`. Returns `false` (and changes nothing) when a take is
    /// already open, as on key repeat.
    ///
    /// The lead starts 300 ms before the press, but never before audio an earlier take claimed,
    /// nor before the oldest audio the history holds (the press itself moves up to that point if
    /// it is older still). **Allocates** the take's buffer.
    pub fn press(&mut self, at: u64) -> bool {
        if self.open.is_some() {
            return false;
        }
        let oldest = self.written.saturating_sub(HISTORY_SAMPLES as u64);
        let floor = self.claimed.max(oldest);
        let press = at.max(floor);
        let start = press.saturating_sub(LEAD_SAMPLES as u64).max(floor);
        let mut samples = Vec::with_capacity(TAKE_RESERVE);
        for p in start..self.written {
            samples.push(self.history[(p % HISTORY_SAMPLES as u64) as usize]);
        }
        self.open = Some(Open {
            samples,
            start,
            press,
            release: None,
        });
        true
    }

    /// The key came up at `at` (never before the press). Returns the take if its tail has already
    /// been pushed, otherwise it completes in a later [`push`](Self::push). With no take open, or
    /// after a release already, it does nothing.
    pub fn release(&mut self, at: u64) -> Option<Take> {
        let open = self.open.as_mut()?;
        if open.release.is_some() {
            return None;
        }
        open.release = Some(at.max(open.press));
        self.complete_if_heard()
    }

    /// Takes the next audio. Returns the open take if this audio completed its tail.
    pub fn push(&mut self, audio: &[f32]) -> Option<Take> {
        let from = self.written;
        for (i, &s) in audio.iter().enumerate() {
            self.history[((from + i as u64) % HISTORY_SAMPLES as u64) as usize] = s;
        }
        self.written += audio.len() as u64;
        if let Some(open) = &mut self.open {
            // The take already holds everything from its start up to `from` that exists, so the
            // next position it needs is at or after `from`, or its start if that is later.
            let wanted = open.start + open.samples.len() as u64;
            let until = open.release.map_or(u64::MAX, |r| r + TAIL_SAMPLES as u64);
            let lo = wanted.max(from);
            let hi = until.min(self.written);
            if lo < hi {
                open.samples
                    .extend_from_slice(&audio[(lo - from) as usize..(hi - from) as usize]);
            }
        }
        self.complete_if_heard()
    }

    /// Discards the open take. Its audio is not reused as the next take's lead.
    pub fn cancel(&mut self) {
        if self.open.take().is_some() {
            self.claimed = self.claimed.max(self.written);
        }
    }

    /// The stream stopped: the open take, with whatever of it arrived (the key may still be down,
    /// or the tail cut short).
    pub fn finish(&mut self) -> Option<Take> {
        let open = self.open.as_ref()?;
        let end = open
            .release
            .map_or(self.written, |r| {
                (r + TAIL_SAMPLES as u64).min(self.written)
            })
            .max(open.start);
        let press = open.press.min(end);
        let release = open.release.unwrap_or(end).clamp(press, end);
        self.close(press, release, end)
    }

    fn complete_if_heard(&mut self) -> Option<Take> {
        let open = self.open.as_ref()?;
        let release = open.release?;
        let end = release + TAIL_SAMPLES as u64;
        if self.written < end {
            return None;
        }
        let press = open.press;
        self.close(press, release, end)
    }

    fn close(&mut self, press: u64, release: u64, end: u64) -> Option<Take> {
        let mut open = self.open.take()?;
        open.samples.truncate((end - open.start) as usize);
        self.claimed = self.claimed.max(end);
        Some(Take {
            samples: open.samples,
            start: open.start,
            press,
            release,
            end,
        })
    }
}
