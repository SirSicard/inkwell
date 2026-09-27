//! The meeting rig: the whole chain on mocks, driven the way capture drives it.
//!
//! ```text
//! test audio ─► capture ring (per side) ─► SideCapture ─┬─► chunks (temp dir) ─► final pass
//!                                                        └─► MeetingChain ─► live engine (scripted)
//! final pass ─► Final (MockEngine, fixtures made on first sight) ─► MemStore
//! ```
//!
//! Blocks are 10 ms, stamped on the mock clock's timebase from the meeting's start. Everything is
//! synthetic or the committed AMI excerpt, and deterministic.

#![allow(dead_code)] // each test binary uses a different subset

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ink_audio::synth::speech_like;
use ink_audio::{
    CaptureProducer, ChunkStore, SpeechProbability, capture_ring, gain::rms, gain::to_dbfs,
};
use ink_core::mock::{MemStore, MockClock, MockDiarizer, MockEngine};
use ink_core::{
    AsrEvent, AudioBlock, AudioSink, CancelToken, Channel, Clock, Diarizer, EngineError,
    EngineInfo, EngineStream, EventSink, Job, Llm, OfflineEngine, Store, StreamFormat,
    StreamingEngine, TimedText, TranscribeOptions, Transcript,
};
use ink_pipeline::capture::{CaptureIssue, SideCapture};
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::meeting::events::{MeetingEvent, MeetingWarning};
use ink_pipeline::meeting::{
    EndedMeeting, FinalizeError, MeetingChain, MeetingOutcome, MeetingServices, MeetingSettings,
    MeetingStart,
};
use ink_pipeline::speech::VadSource;

pub const RATE: usize = 16_000;
/// Samples per 10 ms block at 16 kHz.
pub const BLOCK: usize = 160;
/// The host time the meeting starts at.
pub const T0_NS: u64 = 5_000_000_000;
/// The wall time it starts at: Wednesday 2026-09-23 09:00 at UTC+2.
pub const T0_UNIX_MS: i64 = 1_790_146_800_000;

/// A temp directory removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ink-pipeline-test-{}-{label}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// RMS in dBFS.
pub fn rms_dbfs(x: &[f32]) -> f32 {
    to_dbfs(rms(x))
}

/// `seconds` of digital silence.
pub fn silence(seconds: f64) -> Vec<f32> {
    vec![0.0; (seconds * RATE as f64).round() as usize]
}

/// Speech-like audio at 16 kHz and `rms` dBFS.
pub fn speech(seconds: f64, rms: f32, seed: u64) -> Vec<f32> {
    speech_like(seconds, rms, seed)
}

/// Joins pieces end to end.
pub fn join(pieces: &[Vec<f32>]) -> Vec<f32> {
    pieces.concat()
}

// ---------------------------------------------------------------------------------------------
// VADs

/// Speech wherever the window it hears is above `threshold` dBFS RMS: deaf below it, as a real VAD
/// is, so it only hears the gain stages' provisional copies.
#[derive(Clone, Copy, Debug)]
pub struct EnergyVad(pub f32);

impl SpeechProbability for EnergyVad {
    fn reset(&mut self) {}
    fn probability(&mut self, window: &[f32; 512]) -> Result<f32, EngineError> {
        Ok(if rms_dbfs(window) > self.0 { 1.0 } else { 0.0 })
    }
}

/// Never speech.
pub struct NeverVad;

impl SpeechProbability for NeverVad {
    fn reset(&mut self) {}
    fn probability(&mut self, _: &[f32; 512]) -> Result<f32, EngineError> {
        Ok(0.0)
    }
}

/// Works like `inner` for `windows` windows, then fails on every call.
pub struct FailAfter {
    pub inner: EnergyVad,
    pub windows: usize,
}

impl SpeechProbability for FailAfter {
    fn reset(&mut self) {}
    fn probability(&mut self, window: &[f32; 512]) -> Result<f32, EngineError> {
        if self.windows == 0 {
            return Err(EngineError::Failed("scripted VAD failure".into()));
        }
        self.windows -= 1;
        self.inner.probability(window)
    }
}

/// A factory handing out a VAD per use. `make(n)` builds the `n`th instance (0 and 1 are the live
/// mic and far end, then one per final-pass side, in that order).
pub fn vad_source(
    make: impl Fn(usize) -> Box<dyn SpeechProbability> + Send + Sync + 'static,
) -> VadSource {
    let made = Arc::new(AtomicUsize::new(0));
    VadSource::Installed(Arc::new(move || {
        Ok(make(made.fetch_add(1, Ordering::SeqCst)))
    }))
}

/// The energy VAD at −50 dBFS for every use.
pub fn energy_vad() -> VadSource {
    vad_source(|_| Box::new(EnergyVad(-50.0)))
}

pub fn no_vad() -> VadSource {
    VadSource::Unavailable(VadUnavailable::ModelMissing)
}

// ---------------------------------------------------------------------------------------------
// Engines

/// What the final-pass engine answers for one call.
pub type Answer =
    Arc<dyn Fn(Channel, usize, &[f32]) -> Result<Transcript, EngineError> + Send + Sync>;

/// A transcript with one segment over the whole call.
pub fn words(text: &str, len_samples: usize) -> Transcript {
    Transcript {
        segments: vec![TimedText {
            start_ms: 0,
            end_ms: (len_samples / 16) as u64,
            text: text.into(),
        }],
    }
}

/// The default answer: "mic words N" or "far words N", numbering each side's calls.
pub fn numbered() -> Answer {
    Arc::new(|channel, n, audio| {
        let side = match channel {
            Channel::Mic => "mic",
            Channel::Far => "far",
        };
        Ok(words(&format!("{side} words {n} alpha beta"), audio.len()))
    })
}

/// The final-pass engine: answers through a [`MockEngine`], registering each input as a fixture
/// the first time it is seen, so the mock records every call's level and channel.
pub struct Final {
    pub mock: MockEngine,
    answer: Answer,
    counts: Mutex<[usize; 2]>,
    pub inputs: Mutex<Vec<(Channel, Vec<f32>)>>,
}

impl Final {
    pub fn new(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            mock: MockEngine::new("mock-final", &[Job::MeetingFinal]),
            answer,
            counts: Mutex::default(),
            inputs: Mutex::default(),
        })
    }
}

impl OfflineEngine for Final {
    fn info(&self) -> EngineInfo {
        OfflineEngine::info(&self.mock)
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let side = usize::from(options.channel == Channel::Far);
        let n = {
            let mut counts = self.counts.lock().unwrap();
            counts[side] += 1;
            counts[side]
        };
        self.inputs
            .lock()
            .unwrap()
            .push((options.channel, audio.to_vec()));
        let transcript = (self.answer)(options.channel, n, audio)?;
        self.mock.add_fixture(audio, transcript);
        self.mock.transcribe(audio, options)
    }
}

/// What a live engine received, per side.
pub type Received = Arc<Mutex<Vec<(Channel, Vec<f32>)>>>;
/// Finals to add per side: (start_ms, end_ms, text) of the stream.
pub type Extra = Arc<Mutex<Vec<(Channel, u64, u64, String)>>>;

/// A live engine: a final for each burst of sound in its stream (from the first sample above
/// −60 dBFS to the last before 300 ms under it), a partial on every push, and a record of the
/// level of what it received. Its finals are stamped in its own stream's time, as a real engine's
/// are.
pub struct Onsets {
    pub received: Received,
    /// Extra finals to report when a side's stream finishes, at (start_ms, end_ms) of the stream.
    pub extra: Extra,
    /// The words its partials and finals end with.
    pub words: Arc<Mutex<String>>,
}

impl Onsets {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            received: Arc::default(),
            extra: Arc::default(),
            words: Arc::new(Mutex::new("one two".into())),
        })
    }

    pub fn received(&self, channel: Channel) -> Vec<f32> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter(|(c, _)| *c == channel)
            .flat_map(|(_, a)| a.iter().copied())
            .collect()
    }
}

impl StreamingEngine for Onsets {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: "onsets".into(),
            jobs: vec![Job::LivePartials],
            licence: "MIT".into(),
        }
    }

    fn open_stream(
        &self,
        channel: Channel,
        events: EventSink<AsrEvent>,
    ) -> Result<Box<dyn EngineStream>, EngineError> {
        Ok(Box::new(OnsetStream {
            received: self.received.clone(),
            extra: self.extra.clone(),
            words: self.words.clone(),
            channel,
            events,
            at: 0,
            burst: None,
            quiet: 0,
            count: 0,
        }))
    }
}

struct OnsetStream {
    received: Received,
    extra: Extra,
    words: Arc<Mutex<String>>,
    channel: Channel,
    events: EventSink<AsrEvent>,
    at: u64,
    /// Start of the burst in progress, and its last loud sample.
    burst: Option<(u64, u64)>,
    quiet: u64,
    count: usize,
}

const LOUD: f32 = 0.001; // −60 dBFS

impl OnsetStream {
    fn close(&mut self) {
        if let Some((start, last)) = self.burst.take() {
            self.count += 1;
            let side = match self.channel {
                Channel::Mic => "you",
                Channel::Far => "them",
            };
            (self.events)(AsrEvent::Final(TimedText {
                start_ms: start / 16,
                end_ms: (last + 1) / 16,
                text: format!("live {side} {} {}", self.count, self.words.lock().unwrap()),
            }));
        }
    }
}

impl EngineStream for OnsetStream {
    fn push(&mut self, audio: &[f32]) -> Result<(), EngineError> {
        self.received
            .lock()
            .unwrap()
            .push((self.channel, audio.to_vec()));
        for &s in audio {
            if s.abs() > LOUD {
                self.burst = Some(match self.burst {
                    Some((start, _)) => (start, self.at),
                    None => (self.at, self.at),
                });
                self.quiet = 0;
            } else if self.burst.is_some() {
                self.quiet += 1;
                if self.quiet >= 4_800 {
                    self.close();
                }
            }
            self.at += 1;
        }
        (self.events)(AsrEvent::Partial {
            text: format!("partial {} {}", self.at, self.words.lock().unwrap()),
        });
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), EngineError> {
        self.close();
        let extra: Vec<_> = self
            .extra
            .lock()
            .unwrap()
            .iter()
            .filter(|(c, ..)| *c == self.channel)
            .cloned()
            .collect();
        for (_, start_ms, end_ms, text) in extra {
            (self.events)(AsrEvent::Final(TimedText {
                start_ms,
                end_ms,
                text,
            }));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// The rig

/// Everything a meeting test sets up.
pub struct RigBuilder {
    pub vad: VadSource,
    pub answer: Answer,
    pub diarizer: Option<Arc<dyn Diarizer>>,
    pub llm: Option<Arc<dyn Llm>>,
    pub store: Option<Arc<dyn Store>>,
    pub clock: Option<Arc<dyn Clock>>,
    pub settings: MeetingSettings,
    pub mic_format: StreamFormat,
    pub far_format: StreamFormat,
    pub title: Option<String>,
    /// A live engine in place of the rig's `Onsets`.
    pub live_engine: Option<Arc<dyn StreamingEngine>>,
    /// The capture's routing, for the watchdog.
    pub routing: ink_pipeline::meeting::watchdog::Routing,
    /// A mock clock that moves 10 ms with every 10 ms fed (the default clock stands still, so
    /// the watchdog never sees time pass).
    pub moving_clock: bool,
    /// No live engine at all: the live transcript stays empty (as on a machine whose live engine
    /// runs in the shell).
    pub no_live_engine: bool,
    /// A final-pass engine in place of the rig's scripted one (a real model, locally).
    pub offline: Option<Arc<dyn OfflineEngine>>,
}

impl Default for RigBuilder {
    fn default() -> Self {
        Self {
            vad: energy_vad(),
            answer: numbered(),
            diarizer: None,
            llm: None,
            store: None,
            clock: None,
            settings: MeetingSettings::default(),
            mic_format: StreamFormat::CANONICAL,
            far_format: StreamFormat::CANONICAL,
            title: Some("Weekly sync".into()),
            live_engine: None,
            routing: Default::default(),
            moving_clock: false,
            no_live_engine: false,
            offline: None,
        }
    }
}

pub struct Rig {
    pub chain: Option<MeetingChain>,
    pub mic: Option<SideCapture>,
    pub far: Option<SideCapture>,
    mic_in: CaptureProducer,
    far_in: CaptureProducer,
    pub mic_format: StreamFormat,
    pub far_format: StreamFormat,
    pub dir: TempDir,
    pub chunks: ChunkStore,
    pub store: Arc<dyn Store>,
    pub mem: Arc<MemStore>,
    pub engine: Arc<Final>,
    pub live: Arc<Onsets>,
    pub events: Arc<Mutex<Vec<MeetingEvent>>>,
    pub issues: Vec<(Channel, CaptureIssue)>,
    /// Frames fed per side, in the side's own format.
    frames: [u64; 2],
    /// The moving clock, when the rig has one.
    pub clock: Option<Arc<MockClock>>,
}

impl RigBuilder {
    pub fn build(self) -> Rig {
        let dir = TempDir::new("meeting");
        let chunks = ChunkStore::open(dir.path().join("record")).unwrap();
        let mem = Arc::new(MemStore::new());
        let store: Arc<dyn Store> = self.store.clone().unwrap_or_else(|| mem.clone());
        let moving = self
            .moving_clock
            .then(|| Arc::new(MockClock::new(T0_NS, T0_UNIX_MS)));
        let clock: Arc<dyn Clock> = match (&self.clock, &moving) {
            (Some(clock), _) => clock.clone(),
            (None, Some(moving)) => moving.clone(),
            (None, None) => Arc::new(MockClock::new(T0_NS, T0_UNIX_MS)),
        };
        let engine = Final::new(self.answer.clone());
        let live = Onsets::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let sink: EventSink<MeetingEvent> = Arc::new(move |e| sink_events.lock().unwrap().push(e));
        let services = MeetingServices {
            live: (!self.no_live_engine).then(|| {
                self.live_engine
                    .clone()
                    .unwrap_or_else(|| live.clone() as Arc<dyn StreamingEngine>)
            }),
            offline: self
                .offline
                .clone()
                .unwrap_or_else(|| engine.clone() as Arc<dyn OfflineEngine>),
            diarizer: self.diarizer.clone(),
            store: store.clone(),
            clock,
            llm: self.llm.clone(),
        };
        let chain = MeetingChain::start(
            services,
            self.settings.clone(),
            self.vad.clone(),
            sink,
            MeetingStart {
                title: self.title.clone(),
                source_app: Some("com.example.meet".into()),
                audio_dir: Some("record".into()),
                routing: self.routing,
            },
        )
        .unwrap();
        assert_eq!(
            chain.start_ns(),
            T0_NS,
            "the rig's clock starts the meeting"
        );
        let ring = Duration::from_secs(2);
        let (mic_in, mic_out) = capture_ring(self.mic_format, ring).unwrap();
        let (far_in, far_out) = capture_ring(self.far_format, ring).unwrap();
        Rig {
            chain: Some(chain),
            mic: Some(SideCapture::new(Channel::Mic, mic_out, chunks.clone())),
            far: Some(SideCapture::new(Channel::Far, far_out, chunks.clone())),
            mic_in,
            far_in,
            mic_format: self.mic_format,
            far_format: self.far_format,
            dir,
            chunks,
            store,
            mem,
            engine,
            live,
            events,
            issues: Vec::new(),
            frames: [0, 0],
            clock: moving,
        }
    }
}

impl Rig {
    pub fn chain(&mut self) -> &mut MeetingChain {
        self.chain.as_mut().expect("the meeting is live")
    }

    /// Pushes one device block of `channel` (interleaved in the side's format) into its ring,
    /// stamped where its first frame falls in the meeting, then drains both sides into the chain.
    pub fn push_block(&mut self, channel: Channel, interleaved: &[f32]) {
        self.push_block_unpumped(channel, interleaved);
        self.pump();
    }

    /// Pushes a block into its ring without draining: the pump is stalled.
    pub fn push_block_unpumped(&mut self, channel: Channel, interleaved: &[f32]) {
        let (format, side) = match channel {
            Channel::Mic => (self.mic_format, 0),
            Channel::Far => (self.far_format, 1),
        };
        let host_time_ns =
            T0_NS + self.frames[side] * 1_000_000_000 / u64::from(format.sample_rate);
        self.frames[side] += (interleaved.len() / usize::from(format.channels)) as u64;
        let block = AudioBlock {
            samples: interleaved,
            format,
            host_time_ns,
        };
        match channel {
            Channel::Mic => self.mic_in.push(&block),
            Channel::Far => self.far_in.push(&block),
        }
    }

    /// Skips `frames` of `channel`'s device time: audio lost upstream of the ring.
    pub fn lose(&mut self, channel: Channel, frames: u64) {
        self.frames[usize::from(channel == Channel::Far)] += frames;
    }

    /// Drains both rings into the chain, as the pump does.
    pub fn pump(&mut self) {
        let chain = self.chain.as_mut().expect("live");
        for (channel, side) in [
            (Channel::Mic, self.mic.as_mut()),
            (Channel::Far, self.far.as_mut()),
        ] {
            let Some(side) = side else { continue };
            let mut issues = Vec::new();
            side.drain(
                &mut |b| chain.push_audio(b.channel, &b.samples, b.host_time_ns, b.dropped_frames),
                &mut |i| issues.push(i),
            );
            for issue in issues {
                chain.capture_issue(channel, issue.clone());
                self.issues.push((channel, issue));
            }
        }
    }

    /// Feeds one side only, in 10 ms blocks: the other side's device delivers nothing.
    pub fn feed_side(&mut self, channel: Channel, signal: &[f32]) {
        let format = match channel {
            Channel::Mic => self.mic_format,
            Channel::Far => self.far_format,
        };
        for piece in signal.chunks(BLOCK) {
            let device = to_device(piece, format);
            self.push_block(channel, &device);
            self.advance(10_000_000);
        }
    }

    /// Moves the moving clock on (nothing without one).
    pub fn advance(&mut self, ns: u64) {
        if let Some(clock) = &self.clock {
            clock.advance_ns(ns);
        }
    }

    /// Lets the chain judge its sides with no audio arriving, as its owner does at its deadline.
    pub fn tick(&mut self) {
        self.chain().tick();
    }

    /// Feeds two 16 kHz mono signals side by side in 10 ms blocks, converted to each side's format
    /// (48 kHz by repeating samples, stereo by duplicating them).
    pub fn feed(&mut self, mic: &[f32], far: &[f32]) {
        let n = mic.len().max(far.len());
        let mut at = 0;
        while at < n {
            let end = (at + BLOCK).min(n);
            for (channel, signal) in [(Channel::Mic, mic), (Channel::Far, far)] {
                let piece: Vec<f32> = (at..end)
                    .map(|i| signal.get(i).copied().unwrap_or(0.0))
                    .collect();
                let format = match channel {
                    Channel::Mic => self.mic_format,
                    Channel::Far => self.far_format,
                };
                let device = to_device(&piece, format);
                self.push_block(channel, &device);
            }
            self.advance(10_000_000);
            at = end;
        }
    }

    /// Ends capture: drains and closes both sides, then stops the chain.
    pub fn stop(&mut self) -> EndedMeeting {
        let mut chain = self.chain.take().expect("live");
        for (channel, side) in [
            (Channel::Mic, self.mic.take()),
            (Channel::Far, self.far.take()),
        ] {
            let Some(side) = side else { continue };
            let mut issues = Vec::new();
            let summary = side.finish(
                &mut |b| chain.push_audio(b.channel, &b.samples, b.host_time_ns, b.dropped_frames),
                &mut |i| issues.push(i),
            );
            for issue in issues {
                chain.capture_issue(channel, issue.clone());
                self.issues.push((channel, issue));
            }
            chain.capture_ended(summary);
        }
        chain.stop()
    }

    /// Stops and runs the final pass.
    pub fn finish(&mut self) -> Result<MeetingOutcome, FinalizeError> {
        let ended = self.stop();
        ended.finalize(&self.chunks, &CancelToken::new())
    }

    pub fn events(&self) -> Vec<MeetingEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn warnings(&self) -> Vec<MeetingWarning> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                MeetingEvent::Warning(w) => Some(w),
                _ => None,
            })
            .collect()
    }
}

/// 16 kHz mono to a device format: each sample repeated for a higher rate that is a multiple of
/// 16 kHz, and copied to every channel.
pub fn to_device(mono16k: &[f32], format: StreamFormat) -> Vec<f32> {
    let repeat = (format.sample_rate / 16_000).max(1) as usize;
    let channels = usize::from(format.channels);
    let mut out = Vec::with_capacity(mono16k.len() * repeat * channels);
    for &s in mono16k {
        for _ in 0..repeat * channels {
            out.push(s);
        }
    }
    out
}

/// A diarizer with fixed turns, as the mock.
pub fn diarizer(turns: &[(&str, u64, u64)]) -> Arc<MockDiarizer> {
    Arc::new(MockDiarizer::new(
        turns
            .iter()
            .map(|&(s, a, b)| ink_core::SpeakerTurn {
                speaker: ink_core::SpeakerId(s.into()),
                start_ms: a,
                end_ms: b,
            })
            .collect(),
    ))
}

// ---------------------------------------------------------------------------------------------
// A store that fails on demand

/// A [`MemStore`] whose named methods fail with a backend error once switched on.
#[derive(Default)]
pub struct FlakyStore {
    pub inner: MemStore,
    failing: Mutex<Vec<&'static str>>,
}

impl FlakyStore {
    /// From now on, the methods named fail.
    pub fn fail(&self, methods: &[&'static str]) {
        self.failing.lock().unwrap().extend_from_slice(methods);
    }

    fn check(&self, method: &'static str) -> Result<(), ink_core::StoreError> {
        if self.failing.lock().unwrap().contains(&method) {
            Err(ink_core::StoreError::Backend(format!(
                "scripted {method} failure"
            )))
        } else {
            Ok(())
        }
    }
}

use ink_core::{
    Commitment, CommitmentId, NewCommitment, NewRecord, Note, NoteId, Record, RecordId,
    RecordQuery, SearchHit, Segment, SpeakerId, StoreError, Summary,
};

impl Store for FlakyStore {
    fn create_record(&self, record: NewRecord) -> Result<RecordId, StoreError> {
        self.check("create_record")?;
        self.inner.create_record(record)
    }
    fn record(&self, id: &RecordId) -> Result<Option<Record>, StoreError> {
        self.check("record")?;
        self.inner.record(id)
    }
    fn records(&self, query: &RecordQuery) -> Result<Vec<Record>, StoreError> {
        self.check("records")?;
        self.inner.records(query)
    }
    fn set_title(&self, id: &RecordId, title: &str) -> Result<(), StoreError> {
        self.check("set_title")?;
        self.inner.set_title(id, title)
    }
    fn finish_record(&self, id: &RecordId, at: i64) -> Result<(), StoreError> {
        self.check("finish_record")?;
        self.inner.finish_record(id, at)
    }
    fn delete_record(&self, id: &RecordId) -> Result<(), StoreError> {
        self.check("delete_record")?;
        self.inner.delete_record(id)
    }
    fn append_segments(&self, id: &RecordId, segments: &[Segment]) -> Result<(), StoreError> {
        self.check("append_segments")?;
        self.inner.append_segments(id, segments)
    }
    fn segments(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError> {
        self.check("segments")?;
        self.inner.segments(id)
    }
    fn supersede(&self, id: &RecordId, segments: &[Segment]) -> Result<u32, StoreError> {
        self.check("supersede")?;
        self.inner.supersede(id, segments)
    }
    fn save_removed(&self, id: &RecordId, lines: &[Segment]) -> Result<(), StoreError> {
        self.check("save_removed")?;
        self.inner.save_removed(id, lines)
    }
    fn removed(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError> {
        self.check("removed")?;
        self.inner.removed(id)
    }
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError> {
        self.check("search")?;
        self.inner.search(query, limit)
    }
    fn add_note(&self, id: &RecordId, at_ms: u64, text: &str) -> Result<NoteId, StoreError> {
        self.check("add_note")?;
        self.inner.add_note(id, at_ms, text)
    }
    fn update_note(&self, id: &NoteId, text: &str) -> Result<(), StoreError> {
        self.check("update_note")?;
        self.inner.update_note(id, text)
    }
    fn delete_note(&self, id: &NoteId) -> Result<(), StoreError> {
        self.check("delete_note")?;
        self.inner.delete_note(id)
    }
    fn notes(&self, id: &RecordId) -> Result<Vec<Note>, StoreError> {
        self.check("notes")?;
        self.inner.notes(id)
    }
    fn save_summary(&self, id: &RecordId, summary: &Summary) -> Result<(), StoreError> {
        self.check("save_summary")?;
        self.inner.save_summary(id, summary)
    }
    fn summary(&self, id: &RecordId) -> Result<Option<Summary>, StoreError> {
        self.check("summary")?;
        self.inner.summary(id)
    }
    fn set_speaker_name(&self, id: &RecordId, s: &SpeakerId, name: &str) -> Result<(), StoreError> {
        self.check("set_speaker_name")?;
        self.inner.set_speaker_name(id, s, name)
    }
    fn speaker_names(&self, id: &RecordId) -> Result<Vec<(SpeakerId, String)>, StoreError> {
        self.check("speaker_names")?;
        self.inner.speaker_names(id)
    }
    fn add_commitments(
        &self,
        id: &RecordId,
        items: &[NewCommitment],
    ) -> Result<Vec<CommitmentId>, StoreError> {
        self.check("add_commitments")?;
        self.inner.add_commitments(id, items)
    }
    fn commitments(&self, id: &RecordId) -> Result<Vec<Commitment>, StoreError> {
        self.check("commitments")?;
        self.inner.commitments(id)
    }
    fn open_commitments(&self, limit: usize) -> Result<Vec<Commitment>, StoreError> {
        self.check("open_commitments")?;
        self.inner.open_commitments(limit)
    }
    fn set_commitment_done(&self, id: &CommitmentId, done: bool) -> Result<(), StoreError> {
        self.check("set_commitment_done")?;
        self.inner.set_commitment_done(id, done)
    }
    fn merge_commitment(&self, id: &CommitmentId, into: &CommitmentId) -> Result<(), StoreError> {
        self.check("merge_commitment")?;
        self.inner.merge_commitment(id, into)
    }
    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.check("setting")?;
        self.inner.setting(key)
    }
    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.check("set_setting")?;
        self.inner.set_setting(key, value)
    }
}

// ---------------------------------------------------------------------------------------------
// A diarizer whose turns depend on the audio it is given

/// What [`Hearing`] received.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Heard {
    /// FNV-1a over every sample's bits, in the order received.
    pub hash: u64,
    /// Samples received.
    pub samples: u64,
    /// The largest piece handed over at once.
    pub largest: usize,
}

/// A diarizer whose turns are a function of its input stream alone: each whole second of audio
/// is given to one of three speakers by its level (6 dB bands), and runs of one speaker are
/// merged. Two feeds of the same stream, however it is cut up, get the same turns; and it records
/// a hash of everything it received.
#[derive(Default)]
pub struct Hearing {
    pub heard: Mutex<Heard>,
}

impl Hearing {
    /// Turns for a stream whose 1 s block levels (dBFS) are `levels`.
    fn turns(levels: &[f32]) -> Vec<ink_core::SpeakerTurn> {
        let mut turns: Vec<ink_core::SpeakerTurn> = Vec::new();
        for (k, &db) in levels.iter().enumerate() {
            let band = ((-db / 6.0).floor() as i64).rem_euclid(3);
            let speaker = ink_core::SpeakerId(format!("spk{band}"));
            let (start, end) = (k as u64 * 1_000, (k as u64 + 1) * 1_000);
            match turns.last_mut() {
                Some(last) if last.speaker == speaker && last.end_ms == start => last.end_ms = end,
                _ => turns.push(ink_core::SpeakerTurn {
                    speaker,
                    start_ms: start,
                    end_ms: end,
                }),
            }
        }
        turns
    }

    /// Takes one piece of the stream: hashes it, and adds it to the 1 s block levels.
    fn take(&self, piece: &[f32], block: &mut Vec<f32>, levels: &mut Vec<f32>) {
        let mut heard = self.heard.lock().unwrap();
        for &s in piece {
            for b in s.to_bits().to_le_bytes() {
                heard.hash = (heard.hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
            }
            block.push(s);
            if block.len() == RATE {
                levels.push(rms_dbfs(block));
                block.clear();
            }
        }
        heard.samples += piece.len() as u64;
        heard.largest = heard.largest.max(piece.len());
    }
}

impl Diarizer for Hearing {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: "hearing".into(),
            jobs: vec![Job::Diarization],
            licence: "MIT".into(),
        }
    }

    fn diarize(
        &self,
        audio: &mut dyn ink_core::DiarizeInput,
        cancel: &CancelToken,
    ) -> Result<Vec<ink_core::SpeakerTurn>, EngineError> {
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        let (mut block, mut levels) = (Vec::new(), Vec::new());
        while let Some(window) = audio.next_window() {
            self.take(window, &mut block, &mut levels);
        }
        Ok(Self::turns(&levels))
    }

    fn open_stream(
        &self,
        _: EventSink<ink_core::SpeakerTurn>,
    ) -> Result<Box<dyn EngineStream>, EngineError> {
        Err(EngineError::Unsupported("live labels"))
    }
}
