//! Echo cancellation in the meeting chain: `ink-echo` wired to the mic.
//!
//! On laptop speakers the mic hears the far end, and its words would be transcribed as the
//! user's. The rules (architecture, "Echo"):
//!
//! - **AEC runs only when an echo path is found** ([`EchoCanceller::when_found`]'s rule). With
//!   earbuds or headphones there is none, and the mic goes on as captured.
//! - **The "you" transcript reads AEC3's linear output**, and AEC3's full output only says
//!   whether anyone on the near end spoke: the full output cost the gate 9.9 WER points in real
//!   double talk, the linear output 0.7, but the linear output alone leaks far-end words when
//!   nobody talks.
//!
//! # The final pass
//!
//! The final pass fits its own path over the whole recording (`fit_path`, a fresh [`PathFinder`]
//! read from the chunks: it never extends the live one), and with a path reads the mic through
//! the canceller from the start (`EchoReader`). Qwen3-ASR gives no word timings, so
//! the gate works on regions: the VAD judges the full output, and the engine is given the linear
//! output inside the regions where it heard speech ([`SpeechPass::paired`]). The VAD's verdicts
//! are kept (`HeardSpeech`) as the evidence for duplicate-line removal, which then drops "you"
//! lines that repeat the far end over audio where nobody on the near end spoke; the lines removed
//! are handed back whole ([`RemovedEcho`]) so they can be restored.
//!
//! # Live
//!
//! Live, the search runs as the meeting does (`LiveEcho`), and its state is an event
//! ([`EchoState`]):
//!
//! - Both sides are placed on one timeline from the meeting's start: counted, and placed by their
//!   stamps only after a gap, as the chunk writer does. A far end that delivers nothing (a process
//!   tap while nothing plays) is taken as silence once the mic is a second ahead of it.
//! - The search estimates at 10 s of mic audio, then every 20 s: three times a minute, and once
//!   more at the end if it has found nothing ([`EchoState::FoundAtEnd`]). The mic is
//!   **unprotected** until it finds a path ([`EchoState::Searching`], then
//!   [`EchoState::Cancelling`] with the time it went unprotected).
//! - With a path, the live "you" channel hears AEC3's linear output. AEC3 is first adapted on the
//!   last 20 s of both sides (kept for this, never more), so it starts converged. It runs on a
//!   thread of its own: the `aec3` crate's canceller cannot move between threads, and the chain
//!   must be able to.
//! - The search goes on: a first fit over under 10 s of windows has no drift, and when a later
//!   one puts the echo 2 ms from where the canceller has it, cancellation restarts along it.
//! - Live "you" finals pass the echo gate ([`EchoGate`], fed by a VAD over the full output): one
//!   where the far end played and nobody on the near end spoke is not saved
//!   ([`MeetingWarning::EchoOnlyFinal`]).
//! - A linear stage that stops taking echo off is [`EchoState::Degraded`]; a canceller that
//!   fails ([`EchoFailure`]), or a device switch, ends cancellation visibly and starts the search
//!   again.
//!
//! [`EchoGate`]: ink_echo::EchoGate
//! [`MeetingWarning::EchoOnlyFinal`]: super::events::MeetingWarning::EchoOnlyFinal
//! [`EchoState::FoundAtEnd`]: super::events::EchoState::FoundAtEnd
//! [`EchoState::Searching`]: super::events::EchoState::Searching
//! [`EchoState::Cancelling`]: super::events::EchoState::Cancelling
//! [`EchoState::Degraded`]: super::events::EchoState::Degraded
//! [`EchoFailure`]: super::events::EchoFailure
//!
//! [`EchoCanceller::when_found`]: ink_echo::EchoCanceller::when_found
//! [`PathFinder`]: ink_echo::PathFinder
//! [`SpeechPass::paired`]: crate::speech::SpeechPass::paired
//! [`RemovedEcho`]: super::events::RemovedEcho
//! [`EchoState`]: super::events::EchoState

use std::collections::VecDeque;

use ink_audio::SpeechProbability;
use ink_audio::gain::{LEVEL_FRAME, TRANSIENT_FRAMES, gain_for};
use ink_audio::rate::GAP_THRESHOLD;
use ink_audio::vad::VAD_WINDOW;
use ink_core::{Channel, EngineError, Segment};
use ink_echo::{
    Alignment, CancellerConfig, EchoCanceller, EchoError, EchoFrame, EchoGate, FRAME, GateConfig,
    PathFinder, PathReport, level_db,
};

use std::sync::mpsc;
use std::thread::JoinHandle;

use super::events::{EchoFailure, EchoSearch, EchoState, MeetingEvent, MeetingWarning};
use super::timeline::ns_to_samples;
use crate::gain_stage::Vad;
use crate::speech::VadSource;

/// Far-end audio AEC3 is given to converge before its ERLE counts, in 10 ms frames: 10 s, as the
/// gate measured it (11–12 dB in the first 10 s of a real take, 24 dB after).
const CONVERGE_FRAMES: u64 = 1_000;

/// The least far-end audio an ERLE is reported on, in frames: 1 s.
const MIN_ERLE_FRAMES: u64 = 100;

/// A mic frame under this level (dBFS) says nothing about echo: digital silence, a muted mic.
const MIC_FLOOR_DB: f32 = -70.0;

/// The share of far-end frames at or above the ERLE reported: the best fifth.
const ERLE_PERCENTILE: f64 = 0.8;

/// Histogram bins, 0.25 dB each from −20 dB.
const BINS: usize = 400;
const BIN_DB: f32 = 0.25;
const LOW_DB: f32 = -20.0;

/// A frame's echo return loss enhancement, dB, when the far end was playing (the reference above
/// the gate's floor) and the mic heard anything: the mic's level over AEC3's full output's, and
/// over its linear output's.
pub(crate) fn frame_erle_db(frame: &EchoFrame) -> Option<(f32, f32)> {
    let n = frame.len.clamp(1, FRAME);
    let far = level_db(&frame.reference[..n]) > GateConfig::default().far_floor_db;
    let mic = level_db(&frame.mic[..n]);
    (far && mic > MIC_FLOOR_DB).then(|| {
        (
            mic - level_db(&frame.full[..n]),
            mic - level_db(&frame.linear[..n]),
        )
    })
}

/// Per-frame ERLE values, binned, for a percentile in constant memory.
#[derive(Clone, Debug)]
pub(crate) struct Histogram {
    bins: [u32; BINS],
    frames: u64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            bins: [0; BINS],
            frames: 0,
        }
    }
}

impl Histogram {
    fn add(&mut self, db: f32) {
        let bin = ((db - LOW_DB) / BIN_DB)
            .floor()
            .clamp(0.0, (BINS - 1) as f32) as usize;
        self.bins[bin] += 1;
        self.frames += 1;
    }

    /// The ERLE reported: the level [`ERLE_PERCENTILE`] of the frames reach (the lower edge of
    /// its bin), or `None` under a second of frames.
    pub(crate) fn erle_db(&self) -> Option<f32> {
        if self.frames < MIN_ERLE_FRAMES {
            return None;
        }
        let above = ((1.0 - ERLE_PERCENTILE) * self.frames as f64).ceil() as u64;
        let mut seen = 0u64;
        for (bin, &n) in self.bins.iter().enumerate().rev() {
            seen += u64::from(n);
            if seen >= above {
                return Some(LOW_DB + bin as f32 * BIN_DB);
            }
        }
        Some(LOW_DB)
    }
}

/// How well AEC3 is cancelling, measured without knowing when the near end talks.
///
/// The gate's ERLE is the mic's energy over the output's where only the far end plays, and that
/// needs the near end's truth: a VAD cannot stand in for it, because a badly cancelled frame is
/// loud far-end speech, which the VAD calls speech and so leaves out, precisely when the number
/// matters. So this reports the per-frame ERLE that the best fifth of far-end frames reach: near-end
/// speech over the far end only lowers individual frames, and while a fifth of the far end's audio
/// plays alone, those frames set the number. A path that no longer fits lowers every frame. It is
/// a monitor, not the gate's figure (the echo fixture measures that against the truth).
///
/// The first 10 s of far-end audio are kept apart ([`first`](Self::first)): AEC3 converges there.
///
/// The linear stage's own ERLE is kept too ([`linear`](Self::linear)): it is what the "you"
/// transcript hears. AEC3's suppressor ducks the full output whenever the far end plays, even
/// along a path that no longer fits, so only the linear stage shows a path gone wrong.
#[derive(Clone, Debug, Default)]
pub(crate) struct ErleMeter {
    far_frames: u64,
    first: Histogram,
    after: Histogram,
    linear: Histogram,
}

impl ErleMeter {
    /// Takes one canceller frame. Returns its full-output and linear ERLE when it counts, and
    /// whether AEC3 had had its 10 s by then.
    pub(crate) fn observe(&mut self, frame: &EchoFrame) -> Option<(bool, f32, f32)> {
        let (full, linear) = frame_erle_db(frame)?;
        let converged = self.far_frames >= CONVERGE_FRAMES;
        self.far_frames += 1;
        if converged {
            self.after.add(full);
            self.linear.add(linear);
        } else {
            self.first.add(full);
        }
        Some((converged, full, linear))
    }

    /// The linear stage over far-end audio after the first 10 s.
    pub(crate) fn linear(&self) -> &Histogram {
        &self.linear
    }

    /// Over the first 10 s of far-end audio.
    pub(crate) fn first(&self) -> &Histogram {
        &self.first
    }

    /// Over far-end audio after the first 10 s.
    pub(crate) fn after(&self) -> &Histogram {
        &self.after
    }
}

/// The final pass's echo stage over a recording, frame by frame, for measuring it (the echo
/// fixture, a bench): the path fitted over both sides' chunks in `audio` (host time `t0_ns` is
/// sample 0), then, when there is one, AEC3 adapted on the recording's first 20 s and run along
/// it from the start, every frame handed to `on_frame` (numbered from the meeting's start).
/// Returns the search's report, or `None` when a side's chunks cannot be listed.
///
/// **Worker**, for as long as the recording takes; it checks `cancel` between seconds of audio.
pub fn replay(
    audio: &ink_audio::ChunkStore,
    t0_ns: u64,
    cancel: &ink_core::CancelToken,
    mut on_frame: impl FnMut(&EchoFrame),
) -> Result<Option<PathReport>, super::FinalizeError> {
    let Some(report) = super::offline::fit_path(audio, t0_ns, cancel)? else {
        return Ok(None);
    };
    if let Some(path) = report.path {
        let mut cancelled = super::offline::Cancelled::open(audio, t0_ns, path)?;
        while cancelled.step(&mut on_frame)? {
            if cancel.is_cancelled() {
                return Err(super::FinalizeError::Cancelled);
            }
        }
    }
    Ok(Some(report))
}

// ---------------------------------------------------------------------------------------------
// Live

/// Mic audio the live search sees before its first estimate: 10 s (it needs at least that much
/// far-end audio to find a path).
const FIRST_ESTIMATE: u64 = 10 * 16_000;

/// Mic audio between estimates after that: 20 s, so three a minute. An estimate is the consensus
/// fit over every window so far (tens of milliseconds for an hour of them). Estimates go on while
/// cancelling: a first fit over under 10 s of windows has no drift, and a better one moves the
/// path (see [`REFIT`]).
const ESTIMATE_EVERY: u64 = 20 * 16_000;

/// A later estimate that puts the echo more than this far (samples: 2 ms) from where the running
/// canceller has it restarts cancellation along it. At 50 ppm of drift a delay-only first fit is
/// 2 ms out after 40 s; AEC3's filter spans 52 ms.
const REFIT: f64 = 32.0;

/// Audio kept per side to adapt AEC3 on before cancellation starts (samples: 20 s), so it starts
/// converged: the audio just before, which the live channel has already had as captured. Bounded
/// (2.5 MB for both sides), never the session.
const HISTORY: usize = 20 * 16_000;

/// Audio pushed into the search or the canceller at once, per side: a second (both refuse a side
/// more than 10 s ahead of the other).
const PIECE: usize = 16_000;

/// When the far end has delivered nothing for this much of the mic's audio, it is taken to be
/// silent up to where the mic is: a process tap delivers no callbacks while nothing plays, and
/// the mic's cancelled output would otherwise wait for it. 1 s: past any pump hiccup between the
/// two sides' blocks.
const FAR_IDLE: u64 = 16_000;

/// Far-end frames the live ERLE is judged over: the last 20 s of far-end audio, every 5 s.
const ROLLING_FRAMES: usize = 2_000;
const ROLLING_STEP: usize = 500;

/// Under this linear-stage ERLE (dB) cancellation is [`EchoState::Degraded`]; at or over the
/// second it has recovered. Converged, the linear stage's frames reach 11.5 dB on the gate's real
/// take and 13 dB on the synthetic room ([`ErleMeter`]); a path that no longer fits, about 0.
const DEGRADED_DB: f32 = 4.0;
const RECOVERED_DB: f32 = 6.0;

/// Host time of the chunk writer's gap rule.
const GAP_NS: u64 = GAP_THRESHOLD.as_nanos() as u64;
const NS_PER_SAMPLE: u64 = 62_500;

/// One side's audio on the echo stage's timeline: sample 0 is the meeting's start.
///
/// A block continues the last unless the capture says audio was lost, or its stamp jumps more
/// than the chunk writer's gap threshold past where the last one ended (or goes back): then it is
/// placed by its stamp, silence filling the gap. Between gaps samples are counted, not stamped,
/// so the device clock's drift stays in the audio, where the path search measures it. The last
/// [`HISTORY`] of it is kept.
#[derive(Debug, Default)]
struct Lane {
    /// The next sample's position; `None` before the first.
    next: Option<u64>,
    /// Where the last block ended, by its stamp and length.
    expected_ns: Option<u64>,
    last_ns: u64,
    /// The samples before `next`, at most [`HISTORY`].
    history: VecDeque<f32>,
}

/// Where a block goes: silence before it, and samples of its head to leave out.
struct Placed {
    zeros: u64,
    drop: usize,
}

impl Lane {
    fn pos(&self) -> u64 {
        self.next.unwrap_or(0)
    }

    /// Places a block of `len` samples stamped `host_ns`. `realign` (the far end): a block that
    /// lands on audio already placed loses its head, so what follows stays on its stamps; else
    /// (the mic, whose audio is the user's) it goes on from there.
    fn place(&mut self, t0_ns: u64, host_ns: u64, len: usize, lost: bool, realign: bool) -> Placed {
        let at = ns_to_samples(i128::from(host_ns) - i128::from(t0_ns));
        let gap = lost
            || self
                .expected_ns
                .is_none_or(|e| host_ns > e + GAP_NS || host_ns < self.last_ns);
        self.last_ns = host_ns;
        self.expected_ns = Some(host_ns + len as u64 * NS_PER_SAMPLE);
        let next = self.pos();
        let placed = if !gap {
            Placed { zeros: 0, drop: 0 }
        } else if at >= next as i64 {
            Placed {
                zeros: at as u64 - next,
                drop: 0,
            }
        } else if at < 0 && self.next.is_none() {
            // From before the meeting's start.
            Placed {
                zeros: 0,
                drop: (at.unsigned_abs() as usize).min(len),
            }
        } else if realign {
            Placed {
                zeros: 0,
                drop: ((next as i64 - at) as usize).min(len),
            }
        } else {
            Placed { zeros: 0, drop: 0 }
        };
        self.next = Some(next);
        placed
    }

    /// Appends samples at the lane's position.
    fn advance(&mut self, samples: &[f32]) {
        self.next = Some(self.pos() + samples.len() as u64);
        let keep = samples.len().min(HISTORY);
        let over = (self.history.len() + keep).saturating_sub(HISTORY);
        self.history.drain(..over);
        self.history.extend(&samples[samples.len() - keep..]);
    }

    /// Where the history starts.
    fn history_start(&self) -> u64 {
        self.pos() - self.history.len() as u64
    }

    /// The kept samples from `from` on (`from` at or after [`history_start`](Self::history_start)).
    fn since(&self, from: u64) -> Vec<f32> {
        let skip = from.saturating_sub(self.history_start()) as usize;
        self.history.iter().skip(skip).copied().collect()
    }
}

/// Mic audio for the live "you" channel: as captured, or cancelled, stamped on the host clock.
#[derive(Debug)]
pub(crate) struct MicBlock {
    pub samples: Vec<f32>,
    pub host_ns: u64,
}

/// The search for an echo path, from `origin` on the stage's timeline.
struct Search {
    finder: PathFinder,
    origin: u64,
    since_ms: u64,
    /// Mic position of the next estimate.
    next_estimate: u64,
    /// Whether it has found a path (later ones refine it).
    found: bool,
}

impl Search {
    fn new(origin: u64) -> Self {
        Self {
            finder: PathFinder::new(),
            origin,
            since_ms: origin / 16,
            next_estimate: origin + FIRST_ESTIMATE,
            found: false,
        }
    }

    /// Where `path` (fitted from this search's origin) puts the echo of stage position `at`.
    fn lag(&self, path: Alignment, at: u64) -> f64 {
        path.delay + at.saturating_sub(self.origin) as f64 * path.drift
    }
}

/// The full output's voice activity for the echo gate: a VAD over each 32 ms of AEC3's full
/// output, lifted by the provisional gain of the mic as captured (the robust peak of its last
/// 1.5 s, as the AGC lifts its VAD's copy), so what the suppressor removed stays as far under
/// speech level as it was taken down (the final pass does the same: `SpeechPass::paired`).
struct GateTrack {
    vad: Box<dyn SpeechProbability>,
    window: [f32; VAD_WINDOW],
    filled: usize,
    /// Windows judged: window `w` covers the canceller's samples `512·w ..`.
    windows: u64,
    /// The mic's 20 ms peaks, the last 1.5 s.
    peaks: VecDeque<f32>,
    frame_peak: f32,
    frame_fill: usize,
    scratch: Vec<f32>,
    gain: f32,
}

/// Level frames the provisional gain looks back over: 1.5 s.
const RECENT_FRAMES: usize = 75;

impl GateTrack {
    fn new(mut vad: Box<dyn SpeechProbability>) -> Self {
        vad.reset();
        Self {
            vad,
            window: [0.0; VAD_WINDOW],
            filled: 0,
            windows: 0,
            peaks: VecDeque::with_capacity(RECENT_FRAMES),
            frame_peak: 0.0,
            frame_fill: 0,
            scratch: Vec::with_capacity(RECENT_FRAMES),
            gain: 1.0,
        }
    }

    /// Takes one canceller frame; judges each window it completes into `gate`.
    fn push(&mut self, frame: &EchoFrame, gate: &mut EchoGate) -> Result<(), EngineError> {
        let n = frame.len.min(FRAME);
        for (&full, &mic) in frame.full[..n].iter().zip(&frame.mic[..n]) {
            self.frame_peak = self.frame_peak.max(mic.abs());
            self.frame_fill += 1;
            if self.frame_fill == LEVEL_FRAME {
                if self.peaks.len() == RECENT_FRAMES {
                    self.peaks.pop_front();
                }
                self.peaks.push_back(self.frame_peak);
                self.frame_peak = 0.0;
                self.frame_fill = 0;
                self.scratch.clear();
                self.scratch.extend(self.peaks.iter().copied());
                let k = TRANSIENT_FRAMES.min(self.scratch.len() - 1);
                let robust = *self
                    .scratch
                    .select_nth_unstable_by(k, |a, b| b.total_cmp(a))
                    .1;
                self.gain = gain_for(robust);
            }
            self.window[self.filled] = (full * self.gain).clamp(-1.0, 1.0);
            self.filled += 1;
            if self.filled == VAD_WINDOW {
                self.filled = 0;
                let p = self.vad.probability(&self.window)?;
                if !(0.0..=1.0).contains(&p) {
                    return Err(EngineError::Failed(
                        "the VAD gave a probability outside 0 to 1".into(),
                    ));
                }
                // In range, so the gate records it, at its own window index.
                let _ = gate.push_speech(self.windows, p);
                self.windows += 1;
            }
        }
        Ok(())
    }

    /// Canceller samples judged so far.
    fn judged(&self) -> u64 {
        self.windows * VAD_WINDOW as u64
    }
}

/// A request to the canceller's thread.
enum Request {
    Push(Vec<f32>, Vec<f32>),
    Finish,
}

/// AEC3 on a thread of its own. The `aec3` crate's canceller cannot move between threads (its
/// backend is a boxed trait object without `Send`), and the meeting chain must be able to, so the
/// canceller is made, fed and finished on a worker thread it owns. The chain waits for each
/// answer, so the two stay in step, a block at a time.
struct CancellerThread {
    requests: Option<mpsc::Sender<Request>>,
    replies: mpsc::Receiver<Result<Vec<EchoFrame>, EchoError>>,
    thread: Option<JoinHandle<()>>,
}

impl CancellerThread {
    fn spawn(path: Alignment) -> Result<Self, EchoFailure> {
        let (requests, inbox) = mpsc::channel::<Request>();
        let (outbox, replies) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("ink-echo-live".into())
            .spawn(move || {
                let mut canceller = match EchoCanceller::new(path, CancellerConfig::default()) {
                    Ok(c) => c,
                    Err(error) => {
                        let _ = outbox.send(Err(error));
                        return;
                    }
                };
                if outbox.send(Ok(Vec::new())).is_err() {
                    return;
                }
                while let Ok(request) = inbox.recv() {
                    let mut frames = Vec::new();
                    let done = match request {
                        Request::Push(mic, far) => {
                            canceller.push(&mic, &far, |f| frames.push(f.clone()))
                        }
                        Request::Finish => canceller.finish(|f| frames.push(f.clone())).map(drop),
                    };
                    if outbox.send(done.map(|()| frames)).is_err() {
                        return;
                    }
                }
            })
            .map_err(|_| EchoFailure::Internal)?;
        let mut this = Self {
            requests: Some(requests),
            replies,
            thread: Some(thread),
        };
        this.answer()?;
        Ok(this)
    }

    fn answer(&mut self) -> Result<Vec<EchoFrame>, EchoFailure> {
        match self.replies.recv() {
            Ok(reply) => reply.map_err(EchoFailure::from),
            Err(_) => Err(EchoFailure::Internal),
        }
    }

    fn ask(&mut self, request: Request) -> Result<Vec<EchoFrame>, EchoFailure> {
        let sent = self
            .requests
            .as_ref()
            .is_some_and(|r| r.send(request).is_ok());
        if !sent {
            return Err(EchoFailure::Internal);
        }
        self.answer()
    }

    fn push(&mut self, mic: &[f32], far: &[f32]) -> Result<Vec<EchoFrame>, EchoFailure> {
        self.ask(Request::Push(mic.to_vec(), far.to_vec()))
    }

    fn finish(&mut self) -> Result<Vec<EchoFrame>, EchoFailure> {
        self.ask(Request::Finish)
    }
}

impl Drop for CancellerThread {
    fn drop(&mut self) {
        // Closing the requests ends the thread's loop.
        self.requests = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Live cancellation along a path. Its canceller counts from `origin` on the stage's timeline,
/// some seconds before `live_from`: the frames before that adapted AEC3 on audio the live channel
/// already had as captured, and are dropped.
struct Active {
    canceller: CancellerThread,
    /// The path, as the search fitted it (from the search's origin).
    path: Alignment,
    origin: u64,
    live_from: u64,
    gate: EchoGate,
    /// `None` without a VAD, or once it failed: the gate has no evidence from there.
    track: Option<GateTrack>,
    erle: ErleMeter,
    /// The state it was found in, sent again when it recovers.
    found: EchoState,
    /// Its canceller has handed out its last frames.
    flushed: bool,
}

/// The live ERLE over the last [`ROLLING_FRAMES`] far-end frames, judged every
/// [`ROLLING_STEP`]. It outlives a refit along a better fit of the same path; a new search (a
/// device switch, a failure) starts it again.
#[derive(Default)]
struct Monitor {
    rolling: VecDeque<f32>,
    since_judged: usize,
    degraded: bool,
}

/// What a canceller's frames came to: the cancelled mic for the live channel, and the gate's
/// and the monitor's inputs.
#[derive(Default)]
struct Output {
    /// The first frame for the live channel, and the linear output from it.
    first: Option<u64>,
    linear: Vec<f32>,
    /// Linear-stage ERLE of converged far-end frames.
    judged: Vec<f32>,
    vad_failed: Option<EngineError>,
}

impl Active {
    /// Takes the canceller's frames: all to the gate and the monitor; those from `live_from` on
    /// to the live channel.
    fn take(&mut self, frames: &[EchoFrame], out: &mut Output) {
        let live_frame = (self.live_from - self.origin) / FRAME as u64;
        for f in frames {
            if f.index >= live_frame {
                out.first.get_or_insert(f.index);
                out.linear.extend_from_slice(&f.linear[..f.len]);
            }
            self.gate.push_frame(f);
            if let Some(track) = &mut self.track
                && let Err(error) = track.push(f, &mut self.gate)
            {
                self.track = None;
                out.vad_failed = Some(error);
            }
            if let Some((true, _, linear)) = self.erle.observe(f) {
                out.judged.push(linear);
            }
        }
    }
}

/// Echo cancellation while the meeting is live. See the module docs.
///
/// Both sides' audio comes in as captured; the mic goes out again ([`MicBlock`]) for its live
/// channel: as captured while there is no path, cancelled (AEC3's linear output) once there is.
/// Live "you" finals then pass the echo gate ([`hold`](Self::hold)): one where the far end was
/// playing and the full output heard nobody on the near end is not saved.
///
/// **Worker.** Its work is the search's (three 64k-point FFTs per second of audio), the
/// canceller's (on its own thread) and a VAD's. It keeps the search's windows (a few numbers a
/// second), the gate's evidence (a byte per 10 ms and a float per 32 ms) and [`HISTORY`] of each
/// side's audio.
pub(crate) struct LiveEcho {
    t0_ns: u64,
    vad: VadSource,
    mic: Lane,
    far: Lane,
    /// A mic block's position and stamp: the cancelled output is stamped from it.
    anchor: (u64, u64),
    search: Search,
    active: Option<Box<Active>>,
    monitor: Monitor,
    /// Whether the far end is taken to be silent (it delivered nothing for a while).
    far_idle: bool,
    held: VecDeque<Segment>,
    released: Vec<(Segment, bool)>,
    /// Estimates run, for the throttle's test.
    estimates: u32,
}

impl LiveEcho {
    /// The stage for a meeting that started at host time `t0_ns`. The caller sends the first
    /// state ([`EchoState::Searching`] from 0).
    pub(crate) fn new(t0_ns: u64, vad: VadSource) -> Self {
        Self {
            t0_ns,
            vad,
            mic: Lane::default(),
            far: Lane::default(),
            anchor: (0, t0_ns),
            search: Search::new(0),
            active: None,
            monitor: Monitor::default(),
            far_idle: false,
            held: VecDeque::new(),
            released: Vec::new(),
            estimates: 0,
        }
    }

    /// The next block of one side, as [`MeetingChain::push_audio`] has it. Mic audio for the
    /// live channel goes to `out`, in order.
    ///
    /// [`MeetingChain::push_audio`]: super::MeetingChain::push_audio
    pub(crate) fn push(
        &mut self,
        channel: Channel,
        samples: &[f32],
        host_ns: u64,
        lost: bool,
        out: &mut Vec<MicBlock>,
        emit: &dyn Fn(MeetingEvent),
    ) {
        let t0 = self.t0_ns;
        match channel {
            Channel::Mic => {
                let placed = self.mic.place(t0, host_ns, samples.len(), lost, false);
                self.feed_zeros(Channel::Mic, placed.zeros, out, emit);
                let head = placed.drop;
                self.anchor = (self.mic.pos(), host_ns + head as u64 * NS_PER_SAMPLE);
                for piece in samples[head..].chunks(PIECE) {
                    self.feed(Channel::Mic, piece, false, out, emit);
                }
                self.fill_idle_far(out, emit);
            }
            Channel::Far => {
                self.far_idle = false;
                let placed = self.far.place(t0, host_ns, samples.len(), lost, true);
                self.feed_zeros(Channel::Far, placed.zeros, out, emit);
                for piece in samples[placed.drop..].chunks(PIECE) {
                    self.feed(Channel::Far, piece, false, out, emit);
                }
            }
        }
    }

    /// The capture's routing changed: a new device is a new echo path. Cancellation stops, and
    /// the search starts again from here.
    pub(crate) fn device_switched(&mut self, out: &mut Vec<MicBlock>, emit: &dyn Fn(MeetingEvent)) {
        self.restart(EchoSearch::DeviceSwitch, out, emit);
    }

    /// Capture has ended: the canceller's last frames go out. The gate stays, for the finals the
    /// live engine still settles; [`close`](Self::close) ends the stage.
    pub(crate) fn flush(&mut self, out: &mut Vec<MicBlock>) {
        let anchor = self.anchor;
        if let Some(active) = &mut self.active {
            flush(active, anchor, out);
        }
    }

    /// The live phase is over: finals still waiting on the gate are judged with what it has, and
    /// a search that has not found a path looks at its windows one last time.
    pub(crate) fn close(&mut self, emit: &dyn Fn(MeetingEvent)) {
        self.release(true);
        if self.active.is_some() {
            return;
        }
        self.estimates += 1;
        let report = self.search.finder.estimate();
        if report.path.is_some() {
            let unprotected_ms = (self.mic.pos() / 16).saturating_sub(self.search.since_ms);
            log::info!(
                "meeting: at the end, the live echo search found a path: {report}; the mic went uncancelled for {unprotected_ms} ms"
            );
            emit(MeetingEvent::Echo(EchoState::FoundAtEnd {
                unprotected_ms,
                stable_from_ms: stable_from_ms(&report, self.search.origin),
            }));
        }
    }

    /// A live "you" final to save once the echo gate has judged it. Hand them over in order;
    /// [`released`](Self::released) gives them back with the verdicts.
    pub(crate) fn hold(&mut self, segment: Segment) {
        self.held.push_back(segment);
        self.release(false);
    }

    /// Finals judged since the last call, in order, each with whether to keep it.
    pub(crate) fn released(&mut self) -> Vec<(Segment, bool)> {
        std::mem::take(&mut self.released)
    }

    /// Judges held finals as far as the gate's evidence reaches (`all`: every one, with what
    /// there is). One the canceller does not cover is kept: there is nothing to judge it by.
    fn release(&mut self, all: bool) {
        let pad = GateConfig::default().pad_ms;
        while let Some(segment) = self.held.front() {
            let keep = match &self.active {
                Some(a) if segment.start_ms * 16 >= a.live_from => {
                    let origin_ms = a.origin / 16;
                    let (start, end) = (segment.start_ms - origin_ms, segment.end_ms - origin_ms);
                    let judged_ms = a.track.as_ref().map_or(u64::MAX, |t| t.judged() / 16);
                    if !all && end + pad > judged_ms {
                        return;
                    }
                    a.gate.verdict(start, end).keep()
                }
                _ => true,
            };
            let Some(segment) = self.held.pop_front() else {
                return;
            };
            self.released.push((segment, keep));
        }
    }

    /// `n` samples of silence on `channel`, a second at a time.
    fn feed_zeros(
        &mut self,
        channel: Channel,
        n: u64,
        out: &mut Vec<MicBlock>,
        emit: &dyn Fn(MeetingEvent),
    ) {
        static ZEROS: [f32; PIECE] = [0.0; PIECE];
        let mut left = n;
        while left > 0 {
            let k = left.min(PIECE as u64) as usize;
            left -= k as u64;
            self.feed(channel, &ZEROS[..k], true, out, emit);
            if channel == Channel::Mic {
                self.fill_idle_far(out, emit);
            }
        }
    }

    /// While the far end delivers nothing, it is silent up to where the mic is (see
    /// [`FAR_IDLE`]).
    fn fill_idle_far(&mut self, out: &mut Vec<MicBlock>, emit: &dyn Fn(MeetingEvent)) {
        let (m, f) = (self.mic.pos(), self.far.pos());
        if !self.far_idle && m > f + FAR_IDLE {
            self.far_idle = true;
        }
        if self.far_idle && m > f {
            self.feed_zeros(Channel::Far, m - f, out, emit);
        }
    }

    /// One piece of one side, at its lane's position: into the search, and the canceller if one
    /// runs. `zeros`: silence filling a gap, which the live channel does not get (it places blocks
    /// by their stamps).
    fn feed(
        &mut self,
        channel: Channel,
        piece: &[f32],
        zeros: bool,
        out: &mut Vec<MicBlock>,
        emit: &dyn Fn(MeetingEvent),
    ) {
        let is_mic = channel == Channel::Mic;
        let at = match channel {
            Channel::Mic => self.mic.pos(),
            Channel::Far => self.far.pos(),
        };
        match channel {
            Channel::Mic => self.mic.advance(piece),
            Channel::Far => self.far.advance(piece),
        }
        let anchor = self.anchor;

        // The search, from its origin.
        let skip = self
            .search
            .origin
            .saturating_sub(at)
            .min(piece.len() as u64) as usize;
        let part = &piece[skip..];
        let pushed = if is_mic {
            self.search.finder.push(part, &[])
        } else {
            self.search.finder.push(&[], part)
        };
        if let Err(error) = pushed {
            // One side more than 10 s ahead of the other (a mic that stopped delivering while the
            // far end played): the search starts again from here.
            log::info!("meeting: the live echo search restarts: {error}");
            self.search = Search::new(self.mic.pos().max(self.far.pos()));
        }

        // The canceller, from where it starts; the mic before it goes on as captured.
        match &mut self.active {
            None => {
                if is_mic && !zeros {
                    raw(anchor, at, piece, out);
                }
            }
            Some(active) => {
                let before = active.origin.saturating_sub(at).min(piece.len() as u64) as usize;
                if is_mic && before > 0 && !zeros {
                    raw(anchor, at, &piece[..before], out);
                }
                let part = &piece[before..];
                let frames = if is_mic {
                    active.canceller.push(part, &[])
                } else {
                    active.canceller.push(&[], part)
                };
                match frames {
                    Ok(frames) => {
                        let mut taken = Output::default();
                        active.take(&frames, &mut taken);
                        self.deliver(taken, out, emit);
                        self.release(false);
                    }
                    Err(failure) => {
                        if is_mic && !zeros {
                            raw(anchor, at + before as u64, part, out);
                        }
                        self.fail(failure, out, emit);
                    }
                }
            }
        }

        if is_mic && self.mic.pos() >= self.search.next_estimate {
            self.search.next_estimate = self.mic.pos() + ESTIMATE_EVERY;
            self.estimate(out, emit);
        }
    }

    /// Cancelled mic for the live channel, a gate VAD that failed, and the monitor's frames.
    fn deliver(&mut self, taken: Output, out: &mut Vec<MicBlock>, emit: &dyn Fn(MeetingEvent)) {
        let Some(active) = &self.active else {
            return;
        };
        if let Some(index) = taken.first {
            out.push(MicBlock {
                host_ns: host_at(self.anchor, active.origin + index * FRAME as u64),
                samples: taken.linear,
            });
        }
        if let Some(error) = taken.vad_failed {
            log::warn!("meeting: the echo gate's VAD failed; live finals are kept unjudged");
            emit(MeetingEvent::Warning(MeetingWarning::EchoGateVadFailed(
                error,
            )));
        }
        self.monitor(&taken.judged, emit);
    }

    /// The search's estimate: a path starts cancellation, or restarts it when it moves the echo
    /// by more than [`REFIT`].
    fn estimate(&mut self, out: &mut Vec<MicBlock>, emit: &dyn Fn(MeetingEvent)) {
        self.estimates += 1;
        let report = self.search.finder.estimate();
        let Some(path) = report.path else {
            return;
        };
        let now = self.mic.pos();
        if let Some(active) = &self.active {
            let moved = self.search.lag(path, now) - self.search.lag(active.path, now);
            if moved.abs() <= REFIT {
                return;
            }
            log::info!(
                "meeting: the live echo path moved by {:.1} ms; cancellation restarts along it",
                moved / 16.0
            );
            let anchor = self.anchor;
            if let Some(mut old) = self.active.take() {
                flush(&mut old, anchor, out);
                self.active = Some(old);
                self.release(true);
                self.active = None;
            }
        }
        self.start(path, &report, out, emit);
    }

    /// Starts cancellation along `path` for the mic from where it has reached, adapting AEC3 on
    /// the history before that first.
    fn start(
        &mut self,
        path: Alignment,
        report: &PathReport,
        out: &mut Vec<MicBlock>,
        emit: &dyn Fn(MeetingEvent),
    ) {
        let live_from = self.mic.pos();
        // As far back as both sides' history reaches, but not before this search began (before
        // a device switch the audio took another path), on a frame boundary from `live_from`.
        let earliest = self
            .mic
            .history_start()
            .max(self.far.history_start())
            .max(self.search.origin);
        let back = live_from.saturating_sub(earliest);
        let origin = live_from - back / FRAME as u64 * FRAME as u64;
        let rebased = Alignment {
            delay: self.search.lag(path, origin),
            drift: path.drift,
        };
        let canceller = match CancellerThread::spawn(rebased) {
            Ok(c) => c,
            Err(failure) => {
                log::warn!("meeting: the live echo canceller could not start: {failure:?}");
                emit(MeetingEvent::Echo(EchoState::Failed(failure)));
                return;
            }
        };
        let track = match self.vad.open() {
            (Vad::Installed(vad), _) => Some(GateTrack::new(vad)),
            (Vad::Unavailable(_), error) => {
                if let Some(error) = error {
                    log::warn!("meeting: the echo gate's VAD could not be loaded");
                    emit(MeetingEvent::Warning(MeetingWarning::EchoGateVadFailed(
                        error,
                    )));
                }
                None
            }
        };
        let from_ms = live_from / 16;
        // Unprotected until the search's first path; a refined one follows on without a gap.
        let unprotected_ms = if self.search.found {
            0
        } else {
            from_ms.saturating_sub(self.search.since_ms)
        };
        self.search.found = true;
        let found = EchoState::Cancelling {
            from_ms,
            unprotected_ms,
            stable_from_ms: stable_from_ms(report, self.search.origin),
            delay_ms: path.delay_ms(),
            drift_ppm: path.drift_ppm(),
        };
        let mut active = Box::new(Active {
            canceller,
            path,
            origin,
            live_from,
            gate: EchoGate::new(GateConfig::default(), VAD_WINDOW),
            track,
            erle: ErleMeter::default(),
            found,
            flushed: false,
        });
        // Adapt on the history, a second of each side at a time.
        let (mic, far) = (self.mic.since(origin), self.far.since(origin));
        let mut taken = Output::default();
        let mut primed = Ok(());
        for k in 0..mic.len().max(far.len()).div_ceil(PIECE) {
            let piece =
                |x: &[f32]| x[(k * PIECE).min(x.len())..((k + 1) * PIECE).min(x.len())].to_vec();
            match active.canceller.push(&piece(&mic), &piece(&far)) {
                Ok(frames) => active.take(&frames, &mut taken),
                Err(failure) => {
                    primed = Err(failure);
                    break;
                }
            }
        }
        log::info!(
            "meeting: live echo cancellation starts at {from_ms} ms, adapted on {} ms: {report}",
            (live_from - origin) / 16
        );
        emit(MeetingEvent::Echo(found));
        self.active = Some(active);
        match primed {
            Ok(()) => self.deliver(taken, out, emit),
            Err(failure) => self.fail(failure, out, emit),
        }
    }

    /// Judges the live ERLE on each [`ROLLING_STEP`] of converged far-end frames.
    fn monitor(&mut self, judged: &[f32], emit: &dyn Fn(MeetingEvent)) {
        let Some(active) = &self.active else {
            return;
        };
        let found = active.found;
        let a = &mut self.monitor;
        for &db in judged {
            if a.rolling.len() == ROLLING_FRAMES {
                a.rolling.pop_front();
            }
            a.rolling.push_back(db);
            a.since_judged += 1;
            if a.rolling.len() < ROLLING_FRAMES || a.since_judged < ROLLING_STEP {
                continue;
            }
            a.since_judged = 0;
            let mut sorted: Vec<f32> = a.rolling.iter().copied().collect();
            sorted.sort_by(f32::total_cmp);
            let erle_db = sorted[((sorted.len() as f64) * ERLE_PERCENTILE) as usize];
            if !a.degraded && erle_db < DEGRADED_DB {
                a.degraded = true;
                log::warn!("meeting: live echo cancellation is weak: linear ERLE {erle_db:.1} dB");
                emit(MeetingEvent::Echo(EchoState::Degraded { erle_db }));
            } else if a.degraded && erle_db >= RECOVERED_DB {
                a.degraded = false;
                log::info!(
                    "meeting: live echo cancellation recovered: linear ERLE {erle_db:.1} dB"
                );
                emit(MeetingEvent::Echo(found));
            }
        }
    }

    /// Cancellation failed: it stops, visibly, and the search starts again.
    fn fail(&mut self, failure: EchoFailure, out: &mut Vec<MicBlock>, emit: &dyn Fn(MeetingEvent)) {
        log::warn!("meeting: live echo cancellation failed: {failure:?}");
        emit(MeetingEvent::Echo(EchoState::Failed(failure)));
        self.restart(EchoSearch::AfterFailure, out, emit);
    }

    /// Ends cancellation, if it runs (its last frames to the live channel, the finals it covers
    /// judged), and starts a new search where both sides have reached.
    fn restart(&mut self, why: EchoSearch, out: &mut Vec<MicBlock>, emit: &dyn Fn(MeetingEvent)) {
        let anchor = self.anchor;
        if let Some(active) = &mut self.active {
            flush(active, anchor, out);
        }
        self.release(true);
        self.active = None;
        self.monitor = Monitor::default();
        let origin = self.mic.pos().max(self.far.pos());
        self.search = Search::new(origin);
        emit(MeetingEvent::Echo(EchoState::Searching {
            since_ms: origin / 16,
            why,
        }));
    }

    #[cfg(test)]
    fn estimates(&self) -> u32 {
        self.estimates
    }
}

/// Ends a canceller: its last frames to the live channel, and to the gate. Once only.
fn flush(active: &mut Active, anchor: (u64, u64), out: &mut Vec<MicBlock>) {
    if active.flushed {
        return;
    }
    active.flushed = true;
    match active.canceller.finish() {
        Ok(frames) => {
            let mut taken = Output::default();
            active.take(&frames, &mut taken);
            if let Some(index) = taken.first {
                out.push(MicBlock {
                    host_ns: host_at(anchor, active.origin + index * FRAME as u64),
                    samples: taken.linear,
                });
            }
        }
        Err(failure) => {
            log::warn!("meeting: the live echo canceller could not finish: {failure:?}");
        }
    }
}

/// The host time of stage position `at`, from `anchor` (a position and its stamp).
fn host_at(anchor: (u64, u64), at: u64) -> u64 {
    let (pos, host) = anchor;
    let ns = i128::from(host) + (i128::from(at) - i128::from(pos)) * i128::from(NS_PER_SAMPLE);
    u64::try_from(ns.max(0)).unwrap_or(u64::MAX)
}

/// Mic samples as captured, at `at` on the stage's timeline, for the live channel.
fn raw(anchor: (u64, u64), at: u64, samples: &[f32], out: &mut Vec<MicBlock>) {
    if !samples.is_empty() {
        out.push(MicBlock {
            samples: samples.to_vec(),
            host_ns: host_at(anchor, at),
        });
    }
}

/// Where a search's windows first supported its path, ms into the meeting.
fn stable_from_ms(report: &PathReport, origin: u64) -> Option<u64> {
    report
        .stable_from_s
        .map(|s| origin / 16 + (s * 1000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(index: u64, mic: f32, full: f32, far: f32) -> EchoFrame {
        EchoFrame {
            index,
            len: FRAME,
            mic: [mic; FRAME],
            reference: [far; FRAME],
            linear: [0.0; FRAME],
            full: [full; FRAME],
        }
    }

    fn db(x: f32) -> f32 {
        10f32.powf(x / 20.0)
    }

    fn noise(seconds: f64, seed: u64) -> Vec<f32> {
        let mut rng = ink_audio::synth::Lcg::new(seed);
        (0..(seconds * 16_000.0) as usize)
            .map(|_| (0.05 * rng.next_gaussian()) as f32)
            .collect()
    }

    /// Feeds both sides in 100 ms blocks stamped where they fall, from `from_s`.
    fn feed(live: &mut LiveEcho, mic: &[f32], far: Option<&[f32]>, from_s: f64) -> Vec<MicBlock> {
        let mut out = Vec::new();
        let emit = |_: MeetingEvent| {};
        let at = (from_s * 16_000.0) as usize;
        for k in 0..mic.len().div_ceil(1_600) {
            let r = k * 1_600..((k + 1) * 1_600).min(mic.len());
            let host = ((at + r.start) as u64) * NS_PER_SAMPLE;
            live.push(Channel::Mic, &mic[r.clone()], host, false, &mut out, &emit);
            if let Some(far) = far {
                live.push(Channel::Far, &far[r], host, false, &mut out, &emit);
            }
        }
        out
    }

    #[test]
    fn the_search_estimates_three_times_a_minute_and_once_more_at_the_end() {
        let mut live = LiveEcho::new(
            0,
            VadSource::Unavailable(crate::events::VadUnavailable::ModelMissing),
        );
        // Unrelated noise on both sides: no path, so the search runs the whole minute.
        feed(&mut live, &noise(65.0, 1), Some(&noise(65.0, 2)), 0.0);
        assert_eq!(live.estimates(), 3, "at 10, 30 and 50 s");
        live.close(&|_| {});
        assert_eq!(live.estimates(), 4);
    }

    #[test]
    fn the_mic_goes_on_as_captured_while_there_is_no_path() {
        let mut live = LiveEcho::new(
            0,
            VadSource::Unavailable(crate::events::VadUnavailable::ModelMissing),
        );
        let mic = noise(2.0, 3);
        let out = feed(&mut live, &mic, Some(&noise(2.0, 4)), 0.0);
        let heard: Vec<f32> = out.iter().flat_map(|b| b.samples.iter().copied()).collect();
        assert_eq!(heard, mic);
        assert_eq!(out[3].host_ns, 3 * 1_600 * NS_PER_SAMPLE);
    }

    #[test]
    fn an_idle_far_end_is_silence_up_to_the_mic() {
        let mut live = LiveEcho::new(
            0,
            VadSource::Unavailable(crate::events::VadUnavailable::ModelMissing),
        );
        feed(&mut live, &noise(3.0, 5), None, 0.0);
        assert_eq!(live.far.pos(), live.mic.pos());
        // The far end comes back stamped 2.98 s in: its first 20 ms land on the silence already
        // placed and are left out, so the rest stays on its stamps.
        let mut out = Vec::new();
        let far = noise(0.1, 6);
        live.push(
            Channel::Far,
            &far,
            47_680 * NS_PER_SAMPLE,
            false,
            &mut out,
            &|_| {},
        );
        assert_eq!(live.far.pos(), 48_000 + 1_600 - 320);
    }

    #[test]
    fn a_lane_counts_samples_and_places_by_stamp_only_after_a_gap() {
        let mut lane = Lane::default();
        // The first block, 1 s in: silence before it.
        let p = lane.place(0, 16_000 * NS_PER_SAMPLE, 160, false, false);
        assert_eq!((p.zeros, p.drop), (16_000, 0));
        lane.advance(&vec![0.0; 16_000 + 160]);
        // Stamps a few samples off (jitter, drift) are not gaps: counted on.
        let p = lane.place(0, (16_160 + 5) * NS_PER_SAMPLE, 160, false, false);
        assert_eq!((p.zeros, p.drop), (0, 0));
        lane.advance(&[0.0; 160]);
        // A jump past the gap threshold is placed by its stamp.
        let p = lane.place(0, (16_320 + 8_000) * NS_PER_SAMPLE, 160, false, false);
        assert_eq!((p.zeros, p.drop), (8_000, 0));
        lane.advance(&vec![0.0; 8_160]);
        // Audio lost upstream re-places, and a mic block that lands on placed audio goes on.
        let p = lane.place(0, 24_000 * NS_PER_SAMPLE, 160, true, false);
        assert_eq!((p.zeros, p.drop), (0, 0));
        let p = lane.place(0, 24_000 * NS_PER_SAMPLE, 160, true, true);
        assert_eq!((p.zeros, p.drop), (0, 160), "the far end realigns");
        assert_eq!(lane.history.len(), 24_480);
    }

    #[test]
    fn erle_is_the_level_the_best_fifth_of_far_end_frames_reach() {
        let mut m = ErleMeter::default();
        // 10 s at 10 dB (AEC3 converging), then 20 s: 30 dB alone, 5 dB in double talk (a
        // third of it), and 100 frames with the far end silent.
        for i in 0..3_000u64 {
            let erle = if i < 1_000 {
                10.0
            } else if i % 3 == 0 {
                5.0
            } else {
                30.0
            };
            let far = if (2_500..2_600).contains(&i) {
                0.0
            } else {
                0.1
            };
            m.observe(&frame(i, db(-30.0), db(-30.0 - erle), far));
        }
        assert!((m.first().erle_db().unwrap() - 10.0).abs() <= BIN_DB);
        assert!((m.after().erle_db().unwrap() - 30.0).abs() <= BIN_DB);
        assert_eq!(m.first().frames, 1_000);
        assert_eq!(m.after().frames, 1_900);
    }

    #[test]
    fn a_path_that_no_longer_fits_lowers_the_number() {
        let mut m = ErleMeter::default();
        for i in 0..3_000u64 {
            m.observe(&frame(i, db(-30.0), db(-33.0), 0.1));
        }
        assert!((m.after().erle_db().unwrap() - 3.0).abs() <= BIN_DB);
    }

    #[test]
    fn silence_on_either_side_is_not_counted() {
        let mut m = ErleMeter::default();
        assert_eq!(
            m.observe(&frame(0, db(-30.0), db(-60.0), 0.0)),
            None,
            "far silent"
        );
        assert_eq!(m.observe(&frame(1, 0.0, 0.0, 0.1)), None, "mic silent");
        assert!(m.observe(&frame(2, db(-30.0), db(-60.0), 0.1)).is_some());
        assert_eq!(m.first().erle_db(), None, "under a second");
    }
}
