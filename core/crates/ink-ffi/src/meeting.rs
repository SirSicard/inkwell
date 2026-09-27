//! A meeting run: capture, the pump, and the worker that owns the meeting chain.
//!
//! ```text
//! source ─realtime─► capture ring ─► pump ─┬─► chunks on disk (what the final pass reads)
//!  (a WAV replay today; devices in S2.8)   ├─► bands (the ink, lock-free copy-out)
//!                                          └─► Mailbox (bounded) ─► worker: MeetingChain, tick,
//!                                                                    stop, final pass
//! ```
//!
//! - **The pump** ("ink-pump") drains each side's ring into its chunks and on to the worker as
//!   canonical audio. It wakes every [`PUMP_INTERVAL`] while capture runs: the realtime producer
//!   cannot signal it (no locks or syscalls there), and between meetings it does not exist. It
//!   never waits on the worker: the [`Mailbox`] drops what does not fit and the pump reports it.
//! - **The worker** ("ink-meeting") starts the chain (which creates the record and stamps the
//!   start), lets capture start only then (audio before the start is outside the meeting), feeds
//!   the chain, wakes at [`MeetingChain::deadline_ns`] for the silent-channel watchdog's
//!   [`tick`](MeetingChain::tick), and at the end stops the chain and runs the final pass.
//!   The meeting chain has no thread of its own; this is it.
//! - **Capture** is any [`AudioSource`] per side ([`CaptureSide`]): a [`FileReplaySource`] for a
//!   replay (architecture rule 7), a device in S2.8. A side has ended when its source drops the
//!   sink it was given, which a replay does after its last block.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ink_audio::{BandAnalyzer, Bands, ChunkStore, FileReplaySource, Pacing, capture_ring};
use ink_core::{
    AudioBlock, AudioSink, AudioSource, CancelToken, Channel, EventSink, Job, RecordId,
    StreamingEngine,
};
use ink_engines::{ExternalEngine, Route};
use ink_pipeline::capture::{CanonicalBlock, CaptureIssue, SideCapture, SideSummary};
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::meeting::events::MeetingEvent;
use ink_pipeline::meeting::{MeetingChain, MeetingServices, MeetingSettings, MeetingStart};
use ink_pipeline::speech::VadSource;

use crate::dictation::dropped_event;
use crate::events::{self, event};
use crate::gate::Routed;
use crate::mailbox::{DEFAULT_AUDIO_CAPACITY, Item, Mailbox, Pop, Pushed};
use crate::runtime::Shared;

/// How often the pump drains the rings while capture runs.
pub const PUMP_INTERVAL: Duration = Duration::from_millis(10);

/// The longest file a fast replay takes: its ring must hold all of it (the ring's limit is 60 s).
pub const FAST_REPLAY_MAX: Duration = Duration::from_secs(55);

/// What a replay meeting plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    /// The mic side's WAV.
    pub mic: PathBuf,
    /// The far side's WAV, if any.
    pub far: Option<PathBuf>,
    /// A title for the record.
    pub title: Option<String>,
    /// Deliver as fast as the ring takes it, rather than in real time.
    pub fast: bool,
}

/// One side of a meeting's capture.
pub struct CaptureSide {
    /// Where the audio comes from; its channel is the side.
    pub source: Box<dyn AudioSource>,
    /// How much audio its ring holds before it drops some ([`DEFAULT_RING_DURATION`]).
    ///
    /// [`DEFAULT_RING_DURATION`]: ink_audio::DEFAULT_RING_DURATION
    pub ring: Duration,
}

impl Replay {
    /// Opens the files as capture sides. Errors name the file, never audio.
    pub fn open(&self, shared: &Shared) -> Result<Vec<CaptureSide>, String> {
        let mut paths = vec![(Channel::Mic, self.mic.clone())];
        if let Some(far) = &self.far {
            paths.push((Channel::Far, far.clone()));
        }
        let pacing = if self.fast {
            Pacing::Unpaced
        } else {
            Pacing::RealTime
        };
        let mut sides = Vec::new();
        for (channel, path) in paths {
            let source = FileReplaySource::open(&path, channel, shared.clock.clone())
                .map_err(|e| e.to_string())?
                .with_pacing(pacing);
            let format = source.format();
            let seconds = source.total_frames() as f64 / f64::from(format.sample_rate.max(1));
            let ring = if self.fast {
                if seconds > FAST_REPLAY_MAX.as_secs_f64() {
                    return Err(format!(
                        "{}: {seconds:.0} s is too long for a fast replay (at most {} s)",
                        file_name(&path),
                        FAST_REPLAY_MAX.as_secs()
                    ));
                }
                Duration::from_secs_f64(seconds) + Duration::from_secs(5)
            } else {
                ink_audio::DEFAULT_RING_DURATION
            };
            sides.push(CaptureSide {
                source: Box::new(source),
                ring,
            });
        }
        Ok(sides)
    }
}

/// Anything but audio, for the meeting worker.
enum Input {
    Issue(Channel, CaptureIssue),
    Ended(SideSummary),
    Stop,
}

/// A side's source, and whether it has delivered everything.
struct Source {
    source: Box<dyn AudioSource>,
    done: Arc<AtomicBool>,
}

/// The capture sink a replay pushes into: the ring, plus a flag set when the replay drops it,
/// which it does after its last block. Only an atomic store runs on the realtime thread.
struct Delivered {
    ring: ink_audio::CaptureProducer,
    done: Arc<AtomicBool>,
}

impl AudioSink for Delivered {
    fn push(&mut self, block: &AudioBlock<'_>) {
        self.ring.push(block);
    }
}

impl Drop for Delivered {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
    }
}

/// A meeting in progress, or finished and not yet collected.
pub struct MeetingRun {
    abort: Arc<AtomicBool>,
    cancel: CancelToken,
    mailbox: Arc<Box2>,
    pump: JoinHandle<()>,
    worker: JoinHandle<()>,
}

impl MeetingRun {
    /// Starts the worker, which starts the chain, and the pump, which starts capture once the
    /// chain has. Errors name what failed, never audio.
    pub fn start(
        shared: &Arc<Shared>,
        capture: Vec<CaptureSide>,
        title: Option<String>,
    ) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir_name = format!(
            "{}-{}",
            shared.clock.unix_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let chunks = ChunkStore::open(shared.data_dir.join("meetings").join(&dir_name))
            .map_err(|e| format!("the meeting's audio directory: {e}"))?;
        let mut sources = Vec::new();
        let mut captures = Vec::new();
        for CaptureSide { source, ring } in capture {
            let channel = source.channel();
            let (producer, consumer) =
                capture_ring(source.format(), ring).map_err(|e| e.to_string())?;
            let done = Arc::new(AtomicBool::new(false));
            captures.push((
                SideCapture::new(channel, consumer, chunks.clone()),
                Delivered {
                    ring: producer,
                    done: done.clone(),
                },
            ));
            sources.push(Source { source, done });
        }

        let mailbox = Arc::new(Mailbox::new(DEFAULT_AUDIO_CAPACITY));
        let record: Arc<OnceLock<RecordId>> = Arc::default();
        let abort = Arc::new(AtomicBool::new(false));
        let cancel = CancelToken::new();
        let (go_tx, go_rx) = mpsc::channel::<bool>();
        let worker = {
            let (shared, mailbox, cancel) = (shared.clone(), mailbox.clone(), cancel.clone());
            let record = record.clone();
            let start = MeetingStart {
                title,
                source_app: None,
                audio_dir: Some(format!("meetings/{dir_name}")),
                routing: Default::default(),
            };
            thread::Builder::new()
                .name("ink-meeting".into())
                .spawn(move || worker(&shared, &mailbox, &record, start, chunks, &cancel, go_tx))
                .map_err(|e| format!("the meeting worker did not start: {e}"))?
        };
        let pump = {
            let (shared, mailbox, abort) = (shared.clone(), mailbox.clone(), abort.clone());
            thread::Builder::new()
                .name("ink-pump".into())
                .spawn(move || {
                    pump(
                        &shared, &mailbox, &record, sources, captures, &abort, &go_rx,
                    );
                })
                .map_err(|e| format!("the pump did not start: {e}"))?
        };
        Ok(Self {
            abort,
            cancel,
            mailbox,
            pump,
            worker,
        })
    }

    /// Whether the meeting is over: its final pass has finished, or it failed.
    pub fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }

    /// Waits for the meeting to end by itself.
    pub fn join(self) {
        if self.pump.join().is_err() {
            log::error!("the pump panicked outside its boundary");
        }
        // The pump closes the mailbox however it ends; closed again here, so that even a pump
        // that could not, the worker sees the end instead of ticking forever.
        self.mailbox.close();
        if self.worker.join().is_err() {
            log::error!("the meeting worker panicked outside its boundary");
        }
    }

    /// Stops capture, cancels the final pass, and waits for both threads. The recorded audio
    /// stays on disk and the live transcript stands.
    pub fn stop(self) {
        self.abort.store(true, Ordering::Release);
        self.cancel.cancel();
        self.join();
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

type Box2 = Mailbox<CanonicalBlock, Input>;

/// Where a side's capture issues go: to the worker, in order with its audio.
fn issues(mailbox: &Box2, channel: Channel) -> impl FnMut(CaptureIssue) + '_ {
    move |issue| {
        let _ = mailbox.push(Input::Issue(channel, issue));
    }
}

/// The pump thread: [`capture`] behind a panic boundary, then the ending the worker waits for,
/// however capture ended.
fn pump(
    shared: &Shared,
    mailbox: &Box2,
    record: &OnceLock<RecordId>,
    sources: Vec<Source>,
    captures: Vec<(SideCapture, Delivered)>,
    abort: &AtomicBool,
    go: &mpsc::Receiver<bool>,
) {
    let captured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        capture(shared, mailbox, sources, captures, abort, go);
    }));
    if captured.is_err() {
        // The payload is not logged: it could quote what was said (I5). The sources and the
        // chunk writers were dropped on the way out, which stops the sources and leaves what was
        // written on disk for the final pass.
        log::error!("the pump panicked; capture stops here and the meeting ends");
        shared.events.emit(event(
            "meeting.capture_failed",
            &[("record", record.get().map(|r| r.0.as_str().into()))],
        ));
    }
    // Only the pump tells the worker that capture ended: done on every path, or the worker
    // would tick forever and shutdown would wait on it.
    let _ = mailbox.push(Input::Stop);
    if let Some(o) = mailbox.close() {
        shared.events.emit(dropped_event("meeting", None, o));
    }
    // Idle is a still frame (architecture rule 9): the last thing drawn is silence.
    shared.publish_bands(Bands::default());
}

/// Starts the sources once the chain has started, and drains them until they end or the meeting
/// is stopped.
fn capture(
    shared: &Shared,
    mailbox: &Box2,
    mut sources: Vec<Source>,
    captures: Vec<(SideCapture, Delivered)>,
    abort: &AtomicBool,
    go: &mpsc::Receiver<bool>,
) {
    // The worker says whether the chain started; if it did not, nothing is captured.
    if go.recv() != Ok(true) {
        return;
    }
    let mut sides = Vec::new();
    for (source, (side, sink)) in sources.iter_mut().zip(captures) {
        let channel = source.source.channel();
        if let Err(e) = source.source.start(Box::new(sink)) {
            let _ = mailbox.push(Input::Issue(channel, CaptureIssue::Convert(e.to_string())));
            source.done.store(true, Ordering::Release);
        }
        sides.push((channel, side));
    }
    let mut analyzer = BandAnalyzer::new();
    let mut feed = |block: CanonicalBlock| {
        if block.channel == Channel::Mic
            && let Some(b) = analyzer.process(&block.samples)
        {
            shared.publish_bands(b);
        }
        let n = block.samples.len();
        if let Pushed::Queued {
            after_drop: Some(o),
        } = mailbox.push_audio(block, n)
        {
            shared.events.emit(dropped_event("meeting", None, o));
        }
    };
    loop {
        // Read before draining: a source marked done has pushed its last block, so the drain
        // after this read takes everything.
        let finished = sources.iter().all(|s| s.done.load(Ordering::Acquire));
        let aborting = abort.load(Ordering::Acquire);
        if aborting {
            for s in &mut sources {
                // Joins the replay thread; a failure only means it had already ended badly.
                let _ = s.source.stop();
            }
        }
        for (channel, side) in &mut sides {
            side.drain(&mut feed, &mut issues(mailbox, *channel));
        }
        if finished || aborting {
            break;
        }
        thread::sleep(PUMP_INTERVAL);
    }
    for (channel, side) in sides {
        let summary = side.finish(&mut feed, &mut issues(mailbox, channel));
        let _ = mailbox.push(Input::Ended(summary));
    }
}

/// **Worker.** The router's live-partials engine for a meeting starting now, if one is installed:
/// a streaming engine the shell registered (on the Mac, FluidAudio's Parakeet). Without one the
/// meeting has no live transcript, only its final pass.
fn live_engine(shared: &Shared) -> Option<Arc<dyn StreamingEngine>> {
    match shared.router.route(Job::LivePartials) {
        Ok(Route::External {
            engine: ExternalEngine::Streaming(engine),
            ..
        }) => Some(engine),
        Ok(other) => {
            // A registry row for live partials: this build has no streaming adapter for one.
            log::warn!(
                "live partials route to {}, which this build cannot stream; no live transcript",
                other.id()
            );
            None
        }
        Err(e) => {
            // Usually nothing registered for live partials (Parakeet's models missing, say). The
            // error names the job, never any text.
            log::warn!("no live transcript for this meeting: {e}");
            None
        }
    }
}

fn worker(
    shared: &Arc<Shared>,
    mailbox: &Box2,
    record: &Arc<OnceLock<RecordId>>,
    start: MeetingStart,
    chunks: ChunkStore,
    cancel: &CancelToken,
    go: mpsc::Sender<bool>,
) {
    let sink: EventSink<MeetingEvent> = {
        let (events, record) = (shared.events.clone(), record.clone());
        Arc::new(move |e| {
            if let MeetingEvent::Started { record: r } = &e {
                let _ = record.set(r.clone());
            }
            let r = record.get().cloned().unwrap_or(RecordId(String::new()));
            events.emit(events::meeting(&r, &e));
        })
    };
    let services = MeetingServices {
        live: live_engine(shared),
        offline: Arc::new(Routed::new(shared.clone(), Job::MeetingFinal)),
        diarizer: None,
        store: shared.store.clone(),
        clock: shared.clock.clone(),
        llm: None,
    };
    let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let vad = VadSource::Unavailable(VadUnavailable::ModelMissing);
        let mut chain =
            match MeetingChain::start(services, MeetingSettings::default(), vad, sink, start) {
                Ok(chain) => chain,
                Err(e) => {
                    let _ = go.send(false);
                    failed(shared, None, &format!("the meeting could not start: {e}"));
                    return;
                }
            };
        let _ = go.send(true);
        loop {
            let deadline = chain.deadline_ns().map(|d| {
                Instant::now() + Duration::from_nanos(d.saturating_sub(shared.clock.now_ns()))
            });
            match mailbox.pop(deadline) {
                Pop::Item(Item::Audio(b, _)) => {
                    chain.push_audio(b.channel, &b.samples, b.host_time_ns, b.dropped_frames);
                }
                Pop::Item(Item::Other(Input::Issue(channel, issue))) => {
                    chain.capture_issue(channel, issue);
                }
                Pop::Item(Item::Other(Input::Ended(summary))) => chain.capture_ended(summary),
                Pop::Item(Item::Other(Input::Stop)) | Pop::Closed => break,
                Pop::TimedOut => chain.tick(),
            }
        }
        let ended = chain.stop();
        if let Err(e) = ended.finalize(&chunks, cancel) {
            failed(shared, Some(ended.record()), &e.to_string());
        }
    }));
    if body.is_err() {
        // The payload is not logged: it could quote what was said (I5).
        log::error!("the meeting worker panicked; the meeting stops here");
        mailbox.close();
        let r = record.get().map_or("", |r| r.0.as_str()).to_owned();
        shared.events.emit(event(
            "meeting.worker_failed",
            &[("record", Some(r.into()))],
        ));
    }
}

fn failed(shared: &Shared, record: Option<&RecordId>, message: &str) {
    log::warn!("meeting: {message}");
    shared.events.emit(event(
        "meeting.failed",
        &[
            ("record", record.map(|r| r.0.as_str().into())),
            ("message", Some(message.into())),
        ],
    ));
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ink_core::Clock;
    use ink_core::mock::MockClock;
    use serde_json::Value;

    use super::*;
    use crate::runtime::testing;

    fn wait(events: &Mutex<Vec<Value>>, pred: impl Fn(&Value) -> bool, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        while Instant::now() < until {
            if events.lock().unwrap().iter().any(&pred) {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// The meeting chain has no thread of its own: the worker must wake at its deadline and tick
    /// it, or a mic that stops delivering is never noticed while nothing else arrives.
    #[test]
    fn the_meeting_worker_ticks_the_watchdog_when_no_audio_arrives() {
        let dir = std::env::temp_dir().join(format!("ink-ffi-tick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let clock = Arc::new(MockClock::new(5_000_000_000, 1_790_146_800_000));
        let (core, events) = testing::core(clock.clone(), dir.clone());
        let shared = core.shared().clone();
        let mailbox: Arc<Box2> = Arc::new(Mailbox::new(16));
        let chunks = ChunkStore::open(dir.join("meetings/tick")).unwrap();
        let (go_tx, go_rx) = mpsc::channel();
        let cancel = CancelToken::new();
        let handle = {
            let (shared, mailbox, cancel) = (shared.clone(), mailbox.clone(), cancel.clone());
            let start = MeetingStart {
                title: None,
                source_app: None,
                audio_dir: Some("meetings/tick".into()),
                routing: Default::default(),
            };
            let record = Arc::default();
            thread::spawn(move || worker(&shared, &mailbox, &record, start, chunks, &cancel, go_tx))
        };
        assert_eq!(go_rx.recv(), Ok(true));

        // One block of signal from the mic, then nothing.
        let block = ink_audio::synth::speech_like(0.01, -30.0, 1);
        let n = block.len();
        let pushed = mailbox.push_audio(
            CanonicalBlock {
                channel: Channel::Mic,
                samples: block,
                host_time_ns: clock.now_ns(),
                dropped_frames: 0,
            },
            n,
        );
        assert_eq!(pushed, Pushed::Queued { after_drop: None });
        let stopped = |v: &Value| {
            v["type"] == "meeting.side_state" && v["channel"] == "mic" && v["state"] == "stopped"
        };
        // Past the mic's deadline (10 s after its last block), the worker is woken by something
        // that judges no side (a capture issue), and only its tick can notice the stopped mic.
        // Repeated in case the worker had not taken the block yet when time moved.
        let mut noticed = false;
        for _ in 0..5 {
            clock.advance_ns(11_000_000_000);
            let _ = mailbox.push(Input::Issue(
                Channel::Far,
                CaptureIssue::Convert("a test issue".into()),
            ));
            if wait(&events, stopped, Duration::from_secs(1)) {
                noticed = true;
                break;
            }
        }
        assert!(noticed, "{:?}", events.lock().unwrap());

        let _ = mailbox.push(Input::Stop);
        handle.join().unwrap();
        core.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
