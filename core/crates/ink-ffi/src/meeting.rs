//! A meeting run: capture, the pump, and the worker that owns the meeting chain.
//!
//! ```text
//! source ─realtime─► capture ring ─► pump ─┬─► chunks on disk (what the final pass reads)
//!  (a device, or a WAV replay)             ├─► bands (the ink, lock-free copy-out: one per side)
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
//!   replay (architecture rule 7), the mic and a process tap for a real meeting
//!   ([`capture`](crate::capture)). A side has ended when its source drops the sink it was given,
//!   which a replay does after its last block; a device's meeting ends when it is told to
//!   ([`MeetingRun::end`]).
//! - **What it runs on** besides the speech engines ([`engines`](crate::engines)): the installed
//!   VAD, the diarizer for the final pass, and the language model the shell registered, each
//!   looked up when the meeting starts.
//! - **A crash** leaves a marker beside the chunks ([`recovery`](crate::recovery)) until the final
//!   pass has run, so the next launch can finish what a killed one could not.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ink_audio::{BandAnalyzer, Bands, ChunkStore, FileReplaySource, Pacing, StartAt, capture_ring};
use ink_core::{
    AudioBlock, AudioSink, AudioSource, CancelToken, Channel, EventSink, Job, RecordId,
    StreamingEngine,
};
use ink_engines::{ExternalEngine, Route};
use ink_pipeline::capture::{CanonicalBlock, CaptureIssue, SideCapture, SideSummary};
use ink_pipeline::meeting::events::{MeetingEvent, MeetingWarning};
use ink_pipeline::meeting::watchdog::Routing;
use ink_pipeline::meeting::{MeetingChain, MeetingServices, MeetingSettings, MeetingStart};

use crate::capture::{FarScope, MicInfo, transport_name};

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

/// What is known about a meeting when it starts, for its record and `meeting.started`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeetingInfo {
    /// Its title, when the shell knows one (a calendar event, a replay's name).
    pub title: Option<String>,
    /// The app it records, when there is one: its id (a bundle id on the Mac) and its name.
    pub app: Option<(String, String)>,
    /// How its capture is routed, for the watchdog.
    pub routing: Routing,
    /// The mic it records, when the platform said.
    pub mic: Option<MicInfo>,
    /// What its far end records.
    pub far: FarScope,
}

/// One side of a meeting's capture.
pub struct CaptureSide {
    /// Where the audio comes from; its channel is the side.
    pub source: Box<dyn AudioSource>,
    /// How much audio its ring holds before it drops some ([`DEFAULT_RING_DURATION`]).
    ///
    /// [`DEFAULT_RING_DURATION`]: ink_audio::DEFAULT_RING_DURATION
    pub ring: Duration,
    /// For a replay: where its host times start, set to the meeting's start before capture
    /// starts, so the final pass reads the same audio on every run of the same files. A device
    /// stamps its own times and has none.
    pub start_at: Option<StartAt>,
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
            let start_at = StartAt::new();
            let source = FileReplaySource::open(&path, channel, shared.clock.clone())
                .map_err(|e| e.to_string())?
                .with_pacing(pacing)
                .starting_at(start_at.clone());
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
                start_at: Some(start_at),
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

/// A side's source, whether it has delivered everything, and (a replay's) where its host times
/// start.
struct Source {
    source: Box<dyn AudioSource>,
    done: Arc<AtomicBool>,
    start_at: Option<StartAt>,
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

/// What [`MeetingRun::end`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Capture is stopping; the final pass follows.
    Ended,
    /// It had already been told to end.
    AlreadyEnding,
    /// Capture had already ended (by itself, or long ago): nothing is being recorded.
    NotCapturing,
}

/// Called once when a meeting's capture has ended, however it ended (the pump's last act).
pub type CaptureEnded = Box<dyn FnOnce() + Send>;

/// A meeting in progress, or finished and not yet collected.
pub struct MeetingRun {
    abort: Arc<AtomicBool>,
    /// Set by [`end`](Self::end): capture is stopping, the final pass will run.
    ending: AtomicBool,
    cancel: CancelToken,
    mailbox: Arc<Box2>,
    pump: JoinHandle<()>,
    worker: JoinHandle<()>,
    /// The meeting's record, once its chain has started.
    record: Arc<OnceLock<RecordId>>,
    /// Set by the worker before the shell hears the meeting is over (`meeting.finished`, or a
    /// failure): from then on the worker only tidies up (the crash marker, a sweep's ask), so a
    /// new meeting may start and wait for it.
    over: Arc<AtomicBool>,
}

impl MeetingRun {
    /// Starts the worker, which starts the chain, and the pump, which starts capture once the
    /// chain has. `ended` runs when capture has ended. Errors name what failed, never audio.
    pub fn start(
        shared: &Arc<Shared>,
        capture: Vec<CaptureSide>,
        info: MeetingInfo,
        ended: Option<CaptureEnded>,
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
        for CaptureSide {
            source,
            ring,
            start_at,
        } in capture
        {
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
            sources.push(Source {
                source,
                done,
                start_at,
            });
        }

        let mailbox = Arc::new(Mailbox::new(DEFAULT_AUDIO_CAPACITY));
        let record: Arc<OnceLock<RecordId>> = Arc::default();
        let abort = Arc::new(AtomicBool::new(false));
        let cancel = CancelToken::new();
        let over = Arc::new(AtomicBool::new(false));
        // The worker's answer once the chain has started: the meeting's start (host time), or
        // `None` when it did not start.
        let (go_tx, go_rx) = mpsc::channel::<Option<u64>>();
        let worker = {
            let (shared, mailbox, cancel) = (shared.clone(), mailbox.clone(), cancel.clone());
            let (record, over) = (record.clone(), over.clone());
            let start = MeetingStart {
                title: info.title.clone(),
                source_app: info.app.as_ref().map(|(id, _)| id.clone()),
                audio_dir: Some(format!("meetings/{dir_name}")),
                routing: info.routing,
            };
            thread::Builder::new()
                .name("ink-meeting".into())
                .spawn(move || {
                    worker(
                        &shared, &mailbox, &record, start, &info, chunks, &cancel, go_tx, &over,
                    );
                })
                .map_err(|e| format!("the meeting worker did not start: {e}"))?
        };
        let pump = {
            let (shared, mailbox, abort) = (shared.clone(), mailbox.clone(), abort.clone());
            let record = record.clone();
            thread::Builder::new()
                .name("ink-pump".into())
                .spawn(move || {
                    pump(
                        &shared, &mailbox, &record, sources, captures, &abort, &go_rx,
                    );
                    if let Some(ended) = ended {
                        ended();
                    }
                })
                .map_err(|e| format!("the pump did not start: {e}"))?
        };
        Ok(Self {
            abort,
            ending: AtomicBool::new(false),
            cancel,
            mailbox,
            pump,
            worker,
            record,
            over,
        })
    }

    /// The meeting's record, once its chain has started.
    pub fn record(&self) -> Option<&RecordId> {
        self.record.get()
    }

    /// **Any thread.** Ends the meeting: capture stops, the pump hands on what it has, and the
    /// worker runs the final pass (which is not cancelled). Returns at once.
    pub fn end(&self) -> Ending {
        if self.pump.is_finished() {
            return Ending::NotCapturing;
        }
        if self.ending.swap(true, Ordering::AcqRel) {
            return Ending::AlreadyEnding;
        }
        self.abort.store(true, Ordering::Release);
        Ending::Ended
    }

    /// Whether capture is still running: not ended, and not told to end.
    pub fn is_capturing(&self) -> bool {
        !self.pump.is_finished() && !self.ending.load(Ordering::Acquire)
    }

    /// Whether the meeting's worker has returned.
    pub fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }

    /// Whether the meeting is over: its final pass has finished, or it failed. True from before
    /// the shell hears so, so a start that answers `meeting.finished` is never refused as "a
    /// meeting is already running"; [`join`](Self::join) then waits only for the worker's tidying
    /// up.
    pub fn is_over(&self) -> bool {
        self.over.load(Ordering::Acquire) || self.is_finished()
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

/// Where a side's capture issues go: to the worker, in order with its audio. When the worker
/// can no longer take one (it failed, and its mailbox is closed), the issue is told straight to the
/// shell, as the chain would have told it: never dropped.
fn issues<'a>(
    shared: &'a Shared,
    mailbox: &'a Box2,
    record: &'a OnceLock<RecordId>,
    channel: Channel,
) -> impl FnMut(CaptureIssue) + 'a {
    move |issue| {
        if let Err(Input::Issue(channel, issue)) = mailbox.push(Input::Issue(channel, issue)) {
            log::warn!("meeting: {issue} (after the meeting's worker stopped)");
            let r = record.get().cloned().unwrap_or(RecordId(String::new()));
            shared.events.emit(events::meeting(
                &r,
                &MeetingEvent::Warning(MeetingWarning::Capture { channel, issue }),
            ));
        }
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
    go: &mpsc::Receiver<Option<u64>>,
) {
    let captured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        capture(shared, mailbox, record, sources, captures, abort, go);
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
    shared.publish_far_bands(Bands::default());
}

/// Starts the sources once the chain has started, and drains them until they end or the meeting
/// is stopped.
fn capture(
    shared: &Shared,
    mailbox: &Box2,
    record: &OnceLock<RecordId>,
    mut sources: Vec<Source>,
    captures: Vec<(SideCapture, Delivered)>,
    abort: &AtomicBool,
    go: &mpsc::Receiver<Option<u64>>,
) {
    // The worker says whether the chain started, and when; if it did not, nothing is captured.
    let Ok(Some(t0)) = go.recv() else {
        return;
    };
    let mut sides = Vec::new();
    for (source, (side, sink)) in sources.iter_mut().zip(captures) {
        let channel = source.source.channel();
        // A replay's first frame is the meeting's first (architecture rule 7: the same files give
        // the final pass the same audio on every run).
        if let Some(start_at) = &source.start_at {
            start_at.set(t0);
        }
        if let Err(e) = source.source.start(Box::new(sink)) {
            issues(shared, mailbox, record, channel)(CaptureIssue::Convert(e.to_string()));
            source.done.store(true, Ordering::Release);
        }
        sides.push((channel, side));
    }
    // One analyzer per side: your drop pulses with the mic, theirs with the far end.
    let (mut near, mut far) = (BandAnalyzer::new(), BandAnalyzer::new());
    let mut feed = |block: CanonicalBlock| {
        match block.channel {
            Channel::Mic => {
                if let Some(b) = near.process(&block.samples) {
                    shared.publish_bands(b);
                }
            }
            Channel::Far => {
                if let Some(b) = far.process(&block.samples) {
                    shared.publish_far_bands(b);
                }
            }
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
            side.drain(&mut feed, &mut issues(shared, mailbox, record, *channel));
        }
        if finished || aborting {
            break;
        }
        thread::sleep(PUMP_INTERVAL);
    }
    for (channel, side) in sides {
        let summary = side.finish(&mut feed, &mut issues(shared, mailbox, record, channel));
        // Refused only when the worker failed: then no final pass reads the summary.
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

/// The events of a meeting's chain, as the shell gets them: `meeting.started` also names the
/// meeting's title, its app and its mic (the chain knows only its record).
pub(crate) fn meeting_sink(
    shared: &Shared,
    record: &Arc<OnceLock<RecordId>>,
    info: &MeetingInfo,
) -> EventSink<MeetingEvent> {
    let (events, record) = (shared.events.clone(), record.clone());
    let info = info.clone();
    Arc::new(move |e| {
        if let MeetingEvent::Started { record: r } = &e {
            let _ = record.set(r.clone());
            events.emit(started(r, &info));
            if let (FarScope::EverythingInstead(why), Some((id, name))) = (&info.far, &info.app) {
                // Other apps' sound is in this recording: said once, right after the start.
                events.emit(event(
                    "meeting.far_end_fallback",
                    &[
                        ("record", Some(r.0.as_str().into())),
                        ("app", Some(id.as_str().into())),
                        ("app_name", Some(name.as_str().into())),
                        ("message", Some(why.as_str().into())),
                    ],
                ));
            }
            return;
        }
        let r = record.get().cloned().unwrap_or(RecordId(String::new()));
        events.emit(events::meeting(&r, &e));
    })
}

/// `meeting.started`, with what the shell shows of a starting meeting.
pub fn started(record: &RecordId, info: &MeetingInfo) -> serde_json::Value {
    event(
        "meeting.started",
        &[
            ("record", Some(record.0.as_str().into())),
            ("title", info.title.clone().map(Into::into)),
            ("app", info.app.as_ref().map(|(id, _)| id.as_str().into())),
            (
                "app_name",
                info.app.as_ref().map(|(_, name)| name.as_str().into()),
            ),
            (
                "mic_name",
                info.mic.as_ref().map(|m| m.name.as_str().into()),
            ),
            (
                "mic_transport",
                info.mic
                    .as_ref()
                    .map(|m| transport_name(m.transport).into()),
            ),
            ("mic_reason", info.mic.as_ref().map(|m| m.reason.into())),
            ("far_end", Some(info.far.name().into())),
        ],
    )
}

/// **Worker.** What a meeting starting now runs on (see [`engines`](crate::engines)), and its
/// settings: the summary sized for the language model.
pub(crate) fn services(shared: &Arc<Shared>) -> (MeetingServices, MeetingSettings) {
    let services = MeetingServices {
        live: live_engine(shared),
        offline: Arc::new(Routed::new(shared.clone(), Job::MeetingFinal)),
        diarizer: crate::engines::diarizer(shared),
        store: shared.store.clone(),
        clock: shared.clock.clone(),
        llm: crate::engines::llm(shared),
    };
    let settings = MeetingSettings {
        summary: crate::engines::summary_options(shared),
        ..MeetingSettings::default()
    };
    (services, settings)
}

#[allow(clippy::too_many_arguments)]
fn worker(
    shared: &Arc<Shared>,
    mailbox: &Box2,
    record: &Arc<OnceLock<RecordId>>,
    start: MeetingStart,
    info: &MeetingInfo,
    chunks: ChunkStore,
    cancel: &CancelToken,
    go: mpsc::Sender<Option<u64>>,
    over: &Arc<AtomicBool>,
) {
    let sink = {
        let (inner, over) = (meeting_sink(shared, record, info), over.clone());
        let sink: EventSink<MeetingEvent> = Arc::new(move |e: MeetingEvent| {
            if matches!(e, MeetingEvent::Finished { .. }) {
                over.store(true, Ordering::Release);
            }
            inner(e);
        });
        sink
    };
    let (services, settings) = services(shared);
    let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let vad = crate::engines::vad_source(shared);
        let warn = sink.clone();
        let mut chain = match MeetingChain::start(services, settings, vad, sink, start) {
            Ok(chain) => chain,
            Err(e) => {
                let _ = go.send(None);
                over.store(true, Ordering::Release);
                failed(shared, None, &format!("the meeting could not start: {e}"));
                return;
            }
        };
        // Where the record's timeline starts, beside its chunks: the player places each chunk by
        // its host time against it. Without it the player estimates from the first chunk, so a
        // failure costs precision, not the meeting.
        if let Err(e) = crate::library::write_timeline(chunks.dir(), chain.start_ns()) {
            log::warn!("meeting: the timeline start could not be written: {e}");
        }
        // Until the final pass has run: retention leaves it alone (the record reads as ended from
        // chain.stop, before the pass), with or without the marker below.
        let hold = shared.hold_from_sweep(chain.record());
        // Until the final pass has run: a launch after a crash finds it and finishes the meeting.
        let live = crate::recovery::mark_live(chunks.dir(), chain.record());
        if let Err(e) = &live {
            log::warn!("meeting: the crash-recovery marker could not be written: {e}");
            // Said, not only logged: the user may want to know this one is not protected.
            warn(MeetingEvent::Warning(MeetingWarning::NotCrashProtected(
                e.to_string(),
            )));
        }
        let _ = go.send(Some(chain.start_ns()));
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
        let result = ended.finalize(&chunks, cancel);
        if let Err(e) = &result {
            over.store(true, Ordering::Release);
            failed(shared, Some(ended.record()), &e.to_string());
        }
        // Cancelled (the app quitting mid-pass): the marker stays, and the next launch runs the
        // pass again; so it does when the store could not mark the record ended (the next launch
        // ends it). Otherwise the meeting is done, well or not.
        let cancelled = matches!(result, Err(ink_pipeline::meeting::FinalizeError::Cancelled));
        if live.is_ok() && crate::recovery::marker_goes(ended.record_ended(), cancelled) {
            crate::recovery::clear_live(chunks.dir());
        }
        drop(hold);
        if result.is_ok() {
            // The library changed: a retention setting applies to it now, not at next launch.
            // Asked of the retention thread: a sweep here would keep this meeting "running".
            shared.sweep_soon();
        }
    }));
    if body.is_err() {
        over.store(true, Ordering::Release);
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
            let info = MeetingInfo::default();
            thread::spawn(move || {
                worker(
                    &shared,
                    &mailbox,
                    &record,
                    start,
                    &info,
                    chunks,
                    &cancel,
                    go_tx,
                    &Arc::default(),
                );
            })
        };
        assert!(matches!(go_rx.recv(), Ok(Some(_))));

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

    /// Review (S2.8): a meeting whose crash-recovery marker cannot be written still records, and
    /// says it is not protected (`meeting.warning`, not only a log line).
    #[test]
    fn a_meeting_without_its_crash_marker_says_it_is_not_protected() {
        let dir = std::env::temp_dir().join(format!("ink-ffi-unprotected-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let clock = Arc::new(MockClock::new(5_000_000_000, 1_790_146_800_000));
        let (core, events) = testing::core(clock.clone(), dir.clone());
        let shared = core.shared().clone();
        let mailbox: Arc<Box2> = Arc::new(Mailbox::new(16));
        let chunks = ChunkStore::open(dir.join("meetings/unprotected")).unwrap();
        // The marker's name is taken by a directory: its rename fails.
        std::fs::create_dir_all(chunks.dir().join(crate::recovery::LIVE_FILE)).unwrap();
        let (go_tx, go_rx) = mpsc::channel();
        let handle = {
            let (shared, mailbox) = (shared.clone(), mailbox.clone());
            let start = MeetingStart {
                title: None,
                source_app: None,
                audio_dir: Some("meetings/unprotected".into()),
                routing: Default::default(),
            };
            thread::spawn(move || {
                worker(
                    &shared,
                    &mailbox,
                    &Arc::default(),
                    start,
                    &MeetingInfo::default(),
                    chunks,
                    &CancelToken::new(),
                    go_tx,
                    &Arc::default(),
                );
            })
        };
        assert!(matches!(go_rx.recv(), Ok(Some(_))), "it records anyway");
        let warned = |v: &Value| {
            v["type"] == "meeting.warning"
                && v["kind"] == "not_crash_protected"
                && v["record"].as_str().is_some_and(|r| !r.is_empty())
        };
        assert!(
            wait(&events, warned, Duration::from_secs(5)),
            "{:?}",
            events.lock().unwrap()
        );
        let _ = mailbox.push(Input::Stop);
        handle.join().unwrap();
        core.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review (S2.8): a capture issue the worker can no longer take (it failed and closed its
    /// mailbox) is told straight to the shell, as the chain would have, never dropped.
    #[test]
    fn a_capture_issue_the_worker_cannot_take_still_reaches_the_shell() {
        let dir = std::env::temp_dir().join(format!("ink-ffi-issue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let clock = Arc::new(MockClock::new(5_000_000_000, 1_790_146_800_000));
        let (core, events) = testing::core(clock, dir.clone());
        let mailbox: Box2 = Mailbox::new(4);
        let record = OnceLock::from(RecordId("rec-issue".into()));
        mailbox.close();
        issues(core.shared(), &mailbox, &record, Channel::Far)(CaptureIssue::ChunkWrite(
            "no space left on device".into(),
        ));
        let told = |v: &Value| {
            v["type"] == "meeting.warning"
                && v["kind"] == "capture"
                && v["record"] == "rec-issue"
                && v["channel"] == "far"
        };
        assert!(
            wait(&events, told, Duration::from_secs(5)),
            "{:?}",
            events.lock().unwrap()
        );
        core.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
