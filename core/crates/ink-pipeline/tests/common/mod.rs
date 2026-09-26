//! The dictation rig: the whole chain on mocks, driven like the real thing.
//!
//! ```text
//! MockPlatform mic (48 kHz) ─► capture ring ─► MicPath ─► DictationChain ─► MockEngine (via Tap)
//! MockPlatform hotkey ─► queue ─► DictationChain            └─► MemStore, MockPlatform inserter
//! ```
//!
//! Audio goes in 10 ms blocks stamped on the mock clock, which advances with it; hotkey events are
//! stamped on the same clock, so a press lands where it would on a real machine. Everything is
//! synthetic and deterministic.

#![allow(dead_code)] // each test binary uses a different subset

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ink_audio::synth::speech_like;
use ink_audio::{CaptureConsumer, SpeechProbability, capture_ring};
use ink_core::mock::{MemStore, MockEngine, MockPlatform};
use ink_core::{
    AudioSource, CaptureControl, Channel, Clock, EngineError, EventSink, HotkeyBinding,
    HotkeyEvent, HotkeySource, Job, Llm, OfflineEngine, Record, RecordKind, RecordQuery, Store,
    TimedText, TranscribeOptions, Transcript,
};
use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
use ink_pipeline::events::{DictationEvent, VadUnavailable};
use ink_pipeline::gain_stage::Vad;
use ink_pipeline::mic::MicPath;

/// Mock host time advances by this per block fed.
pub const BLOCK: Duration = Duration::from_millis(10);
const BLOCK_FRAMES: usize = 480;

/// A transcript with one segment.
pub fn transcript(text: &str) -> Transcript {
    Transcript {
        segments: vec![TimedText {
            start_ms: 0,
            end_ms: 1_000,
            text: text.into(),
        }],
    }
}

/// 16 kHz → 48 kHz by linear interpolation: enough for a speech-band test signal.
pub fn upsample3(x: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(x.len() * 3);
    for (i, &a) in x.iter().enumerate() {
        let b = x.get(i + 1).copied().unwrap_or(0.0);
        out.extend([a, a + (b - a) / 3.0, a + 2.0 * (b - a) / 3.0]);
    }
    out
}

/// RMS in dBFS.
pub fn rms_dbfs(x: &[f32]) -> f32 {
    let ms = x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len().max(1) as f64;
    (10.0 * ms.log10()) as f32
}

/// Speech-like audio at 48 kHz, scaled so its RMS is exactly `rms` dBFS at 48 kHz.
pub fn speech_48k(seconds: f64, rms: f32, seed: u64) -> Vec<f32> {
    let up = upsample3(&speech_like(seconds, rms, seed));
    let scale = 10f32.powf((rms - rms_dbfs(&up)) / 20.0);
    up.into_iter().map(|s| s * scale).collect()
}

/// The VAD a test installs.
#[derive(Clone, Copy, Debug)]
pub enum VadKind {
    /// Speech wherever a window of the audio it hears is above −50 dBFS RMS: deaf below that, as a
    /// real VAD is, so it only hears the provisional copy.
    Energy,
    /// Never speech: a real VAD's verdict on noise.
    Never,
    /// Every call fails.
    Failing,
    /// None installed.
    Unavailable(VadUnavailable),
}

pub struct EnergyVad;

impl SpeechProbability for EnergyVad {
    fn reset(&mut self) {}
    fn probability(&mut self, window: &[f32; 512]) -> Result<f32, EngineError> {
        Ok(if rms_dbfs(window) > -50.0 { 1.0 } else { 0.0 })
    }
}

pub struct NeverVad;

impl SpeechProbability for NeverVad {
    fn reset(&mut self) {}
    fn probability(&mut self, _: &[f32; 512]) -> Result<f32, EngineError> {
        Ok(0.0)
    }
}

pub struct FailingVad;

impl SpeechProbability for FailingVad {
    fn reset(&mut self) {}
    fn probability(&mut self, _: &[f32; 512]) -> Result<f32, EngineError> {
        Err(EngineError::Failed("scripted VAD failure".into()))
    }
}

impl VadKind {
    pub fn build(self) -> Vad {
        match self {
            Self::Energy => Vad::Installed(Box::new(EnergyVad)),
            Self::Never => Vad::Installed(Box::new(NeverVad)),
            Self::Failing => Vad::Installed(Box::new(FailingVad)),
            Self::Unavailable(why) => Vad::Unavailable(why),
        }
    }
}

/// An engine that keeps every input it receives, then answers through the mock engine.
pub struct Tap {
    pub inner: MockEngine,
    pub inputs: Mutex<Vec<Vec<f32>>>,
}

impl OfflineEngine for Tap {
    fn info(&self) -> ink_core::EngineInfo {
        OfflineEngine::info(&self.inner)
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        self.inputs.lock().unwrap().push(audio.to_vec());
        self.inner.transcribe(audio, options)
    }
}

/// How to build a rig. Clone it to build an identical one.
#[derive(Clone)]
pub struct RigBuilder {
    vad: VadKind,
    settings: DictationSettings,
    llm: Option<Arc<dyn Llm>>,
    store: Option<Arc<dyn Store>>,
}

impl RigBuilder {
    pub fn vad(mut self, vad: VadKind) -> Self {
        self.vad = vad;
        self
    }

    pub fn settings(mut self, edit: impl FnOnce(&mut DictationSettings)) -> Self {
        edit(&mut self.settings);
        self
    }

    pub fn llm(mut self, llm: Arc<dyn Llm>) -> Self {
        self.llm = Some(llm);
        self
    }

    /// A store other than the rig's own `MemStore` (which is then unused).
    pub fn store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn build(self) -> Rig {
        let platform = Arc::new(MockPlatform::new());
        let engine = MockEngine::new("mock-asr", &[Job::DictationFinal]);
        let tap = Arc::new(Tap {
            inner: engine.clone(),
            inputs: Mutex::default(),
        });
        let mem = Arc::new(MemStore::new());
        let store: Arc<dyn Store> = self.store.clone().unwrap_or_else(|| mem.clone());
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let sink: EventSink<DictationEvent> =
            Arc::new(move |e| sink_events.lock().unwrap().push(e));
        let services = Services {
            engine: tap.clone(),
            store: store.clone(),
            inserter: platform.clone(),
            focus: platform.clone(),
            clock: platform.clock(),
            llm: self.llm.clone(),
        };
        let chain = DictationChain::new(services, self.settings.clone(), self.vad.build(), sink);

        let mut source = platform.open_mic(None).unwrap();
        let (producer, consumer) = capture_ring(source.format(), Duration::from_secs(2)).unwrap();
        source.start(Box::new(producer)).unwrap();
        let mic = MicPath::new(source.format()).unwrap();

        let hotkeys = Arc::new(Mutex::new(Vec::new()));
        let queue = hotkeys.clone();
        HotkeySource::start(
            &*platform,
            &HotkeyBinding("right_option".into()),
            Arc::new(move |e| queue.lock().unwrap().push(e)),
        )
        .unwrap();
        platform.clock().advance_ns(1_000_000_000);

        Rig {
            config: self,
            platform,
            engine,
            tap,
            store,
            events,
            chain: RefCell::new(chain),
            capture: RefCell::new(Capture {
                _source: source,
                consumer,
                mic,
            }),
            hotkeys,
        }
    }
}

struct Capture {
    _source: Box<dyn AudioSource>,
    consumer: CaptureConsumer,
    mic: MicPath,
}

pub struct Rig {
    config: RigBuilder,
    pub platform: Arc<MockPlatform>,
    pub engine: MockEngine,
    pub tap: Arc<Tap>,
    pub store: Arc<dyn Store>,
    events: Arc<Mutex<Vec<DictationEvent>>>,
    pub chain: RefCell<DictationChain>,
    capture: RefCell<Capture>,
    hotkeys: Arc<Mutex<Vec<HotkeyEvent>>>,
}

impl Rig {
    pub fn builder() -> RigBuilder {
        RigBuilder {
            vad: VadKind::Energy,
            settings: DictationSettings::default(),
            llm: None,
            store: None,
        }
    }

    /// Feeds 48 kHz mono in 10 ms blocks, pumping everything through after each block.
    pub fn feed(&self, samples: &[f32]) {
        let clock = self.platform.clock();
        for block in samples.chunks(BLOCK_FRAMES) {
            assert!(self.platform.feed(Channel::Mic, block, clock.now_ns()));
            clock.advance_ns(BLOCK.as_nanos() as u64);
            self.pump();
        }
    }

    /// Advances the clock without audio (a stalled mic), then lets the chain check its deadline.
    pub fn stall(&self, duration: Duration) {
        self.platform.clock().advance_ns(duration.as_nanos() as u64);
        self.chain.borrow_mut().tick();
    }

    /// Feeds `then` block by block (silence after it runs out) until something is inserted, and
    /// returns how many blocks that took, or `None` after `max_blocks`.
    pub fn blocks_until_inserted(&self, then: &[f32], max_blocks: usize) -> Option<usize> {
        let before = self.inserted().len();
        let quiet = [0.0f32; BLOCK_FRAMES];
        let mut blocks = then.chunks(BLOCK_FRAMES);
        for n in 1..=max_blocks {
            self.feed(blocks.next().unwrap_or(&quiet));
            if self.inserted().len() > before {
                return Some(n);
            }
        }
        None
    }

    pub fn silence(&self, seconds: f64) {
        self.feed(&vec![0.0; (seconds * 48_000.0) as usize]);
    }

    fn pump(&self) {
        {
            let mut cap = self.capture.borrow_mut();
            let Capture { consumer, mic, .. } = &mut *cap;
            let mut chain = self.chain.borrow_mut();
            while let Some(b) = consumer.pop() {
                let dropped = b.dropped_frames_before;
                let out = mic.push(&b.block).unwrap();
                chain.push_audio(out.samples, out.host_time_ns, dropped);
            }
        }
        self.deliver_hotkeys();
        self.chain.borrow_mut().tick();
    }

    fn deliver_hotkeys(&self) {
        let pending: Vec<HotkeyEvent> = self.hotkeys.lock().unwrap().drain(..).collect();
        for e in pending {
            self.chain.borrow_mut().hotkey(e);
        }
    }

    pub fn press(&self) {
        assert!(self.platform.press());
        self.deliver_hotkeys();
    }

    pub fn release(&self) {
        assert!(self.platform.release());
        self.deliver_hotkeys();
    }

    /// Half a second of room, press, `speech`, release, and enough room for the tail.
    pub fn dictate(&self, speech: &[f32]) {
        self.silence(0.5);
        self.press();
        self.feed(speech);
        self.release();
        self.silence(0.6);
    }

    /// What the engine receives when this rig's configuration dictates `speech`: found by running
    /// an identical rig once (the chain is deterministic, so the input repeats exactly).
    pub fn engine_input_for(&self, speech: &[f32]) -> Vec<f32> {
        let probe = self.config.clone().build();
        probe.dictate(speech);
        let inputs = probe.tap.inputs.lock().unwrap();
        assert_eq!(
            inputs.len(),
            1,
            "the probe dictation reached the engine once"
        );
        inputs[0].clone()
    }

    /// Registers `speech`'s engine input as a mock fixture answering `text`.
    pub fn teach(&self, speech: &[f32], text: &str) {
        let input = self.engine_input_for(speech);
        self.engine.add_fixture(&input, transcript(text));
    }

    /// A synthetic dictation of `seconds` at `rms` dBFS that the engine answers with `text`.
    pub fn dictate_fixture(&self, text: &str, seconds: f64, rms: f32) -> Vec<f32> {
        let speech = speech_48k(seconds, rms, 7);
        self.teach(&speech, text);
        self.dictate(&speech);
        speech
    }

    pub fn events(&self) -> Vec<DictationEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn clear_events(&self) {
        self.events.lock().unwrap().clear();
    }

    pub fn inserted(&self) -> Vec<String> {
        self.platform.inserted()
    }

    pub fn engine_inputs(&self) -> Vec<Vec<f32>> {
        self.tap.inputs.lock().unwrap().clone()
    }

    pub fn dictation_records(&self) -> Vec<Record> {
        self.store
            .records(&RecordQuery {
                kind: Some(RecordKind::Dictation),
                before: None,
                limit: 100,
            })
            .unwrap()
    }
}
