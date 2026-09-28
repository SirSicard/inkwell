//! The worker thread: hotkey callbacks only enqueue, and the owning thread runs the chain.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ink_audio::synth::speech_like;
use ink_core::mock::{MemStore, MockEngine, MockPlatform};
use ink_core::{
    CancelToken, Endpoint, EngineError, EngineInfo, EventSink, HotkeyEvent, InsertOutcome, Job,
    Llm, LlmError, LlmInfo, LlmRequest, LlmResponse, OfflineEngine, PlatformError, TextInserter,
    TimedText, TranscribeOptions, Transcript,
};
use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
use ink_pipeline::events::{DictationEvent, VadUnavailable, Warning};
use ink_pipeline::gain_stage::Vad;
use ink_pipeline::worker::{DictationWorker, Input, MAX_PANICS_WITHOUT_A_TAKE};

const BLOCK: usize = 160;
const BLOCK_NS: u64 = 10_000_000;

/// Room, a press, speech, a release, room: as the pump and the hotkey would deliver them.
fn script() -> Vec<Input> {
    take_script(&mut 1_000_000_000)
}

/// One take's inputs, starting at host time `t`, which moves past them.
fn take_script(t: &mut u64) -> Vec<Input> {
    fn audio(samples: &[f32], inputs: &mut Vec<Input>, t: &mut u64) {
        for block in samples.chunks(BLOCK) {
            inputs.push(Input::Audio {
                samples: block.to_vec(),
                host_time_ns: *t,
                dropped_frames: 0,
            });
            *t += BLOCK_NS;
        }
    }
    let mut inputs = Vec::new();
    audio(&[0.0; 8_000], &mut inputs, t);
    inputs.push(Input::Hotkey(HotkeyEvent::Pressed { at_ns: *t }));
    audio(&speech_like(1.5, -30.0, 9), &mut inputs, t);
    inputs.push(Input::Hotkey(HotkeyEvent::Released { at_ns: *t }));
    audio(&[0.0; 9_600], &mut inputs, t);
    inputs
}

/// Keeps what it is given and answers nothing.
#[derive(Default)]
struct Keep(Mutex<Vec<Vec<f32>>>);

impl OfflineEngine for Keep {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: "keep".into(),
            jobs: vec![Job::DictationFinal],
            licence: "MIT".into(),
        }
    }

    fn transcribe(&self, audio: &[f32], _: &TranscribeOptions) -> Result<Transcript, EngineError> {
        self.0.lock().unwrap().push(audio.to_vec());
        Ok(Transcript::default())
    }
}

fn chain(
    engine: Arc<dyn OfflineEngine>,
    events: &Arc<Mutex<Vec<DictationEvent>>>,
) -> (DictationChain, Arc<MockPlatform>) {
    let platform = Arc::new(MockPlatform::new());
    let chain = chain_with(engine, &platform, platform.clone(), events);
    (chain, platform)
}

fn chain_with(
    engine: Arc<dyn OfflineEngine>,
    platform: &Arc<MockPlatform>,
    inserter: Arc<dyn TextInserter>,
    events: &Arc<Mutex<Vec<DictationEvent>>>,
) -> DictationChain {
    let sink_events = events.clone();
    let sink: EventSink<DictationEvent> = Arc::new(move |e| sink_events.lock().unwrap().push(e));
    DictationChain::new(
        Services {
            engine,
            store: Arc::new(MemStore::new()),
            inserter,
            focus: platform.clone(),
            clock: platform.clock(),
            llm: None,
        },
        DictationSettings::default(),
        Vad::Unavailable(VadUnavailable::ModelMissing),
        sink,
    )
}

#[test]
fn the_worker_runs_a_dictation_from_queued_inputs() {
    // What the engine receives, found by running the same script on this thread.
    let keep = Arc::new(Keep::default());
    let (mut direct, _) = chain(keep.clone(), &Arc::default());
    for input in script() {
        match input {
            Input::Audio {
                samples,
                host_time_ns,
                dropped_frames,
            } => direct.push_audio(&samples, host_time_ns, dropped_frames),
            Input::Hotkey(e) => direct.hotkey(e),
            _ => unreachable!("the script has audio and hotkeys only"),
        }
    }
    let take = keep
        .0
        .lock()
        .unwrap()
        .pop()
        .expect("the take reached the engine");

    let engine = MockEngine::new("mock", &[Job::DictationFinal]).with_fixture(
        &take,
        Transcript {
            segments: vec![TimedText {
                start_ms: 0,
                end_ms: 1_500,
                text: "sent from the worker".into(),
            }],
        },
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let (chain, platform) = chain(Arc::new(engine.clone()), &events);
    let worker = DictationWorker::spawn(chain, platform.clock()).unwrap();
    let hotkeys = worker.hotkey_sink();
    for input in script() {
        match input {
            // Hotkey events arrive through the sink the platform calls on its own thread.
            Input::Hotkey(e) => hotkeys(e),
            other => worker.send(other).unwrap(),
        }
    }
    let chain = worker.stop().unwrap();

    assert!(!chain.is_recording());
    assert_eq!(engine.calls().len(), 1);
    assert_eq!(
        platform.inserted(),
        vec!["Sent from the worker. ".to_owned()]
    );
    let events = events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DictationEvent::Inserted { .. })),
        "{events:?}"
    );
}

#[test]
fn a_stopped_worker_refuses_input() {
    let (chain, platform) = chain(Arc::new(Keep::default()), &Arc::default());
    let worker = DictationWorker::spawn(chain, platform.clock()).unwrap();
    let hotkeys = worker.hotkey_sink();
    let sender = worker.sender();
    worker.stop().unwrap();
    // The platform may still call the sink after the worker is gone: it must return, not panic.
    hotkeys(HotkeyEvent::Cancelled);
    assert!(sender.send(Input::StreamEnded).is_err());
}

// ---------------------------------------------------------------------------------------------
// The panic boundary: a stage that panics must not leave the hotkey armed and dead
// ---------------------------------------------------------------------------------------------

/// What the engine receives for one take of the script (the chain is deterministic).
fn the_take() -> Vec<f32> {
    let keep = Arc::new(Keep::default());
    let (mut direct, _) = chain(keep.clone(), &Arc::default());
    for input in script() {
        match input {
            Input::Audio {
                samples,
                host_time_ns,
                dropped_frames,
            } => direct.push_audio(&samples, host_time_ns, dropped_frames),
            Input::Hotkey(e) => direct.hotkey(e),
            _ => unreachable!("the script has audio and hotkeys only"),
        }
    }
    keep.0
        .lock()
        .unwrap()
        .pop()
        .expect("the take reached the engine")
}

fn answering(text: &str) -> MockEngine {
    MockEngine::new("mock", &[Job::DictationFinal]).with_fixture(
        &the_take(),
        Transcript {
            segments: vec![TimedText {
                start_ms: 0,
                end_ms: 1_500,
                text: text.into(),
            }],
        },
    )
}

/// Panics on its first `panics` calls, then answers through the mock engine.
struct PanickyEngine {
    panics: AtomicUsize,
    inner: MockEngine,
}

impl OfflineEngine for PanickyEngine {
    fn info(&self) -> EngineInfo {
        OfflineEngine::info(&self.inner)
    }

    fn transcribe(&self, audio: &[f32], o: &TranscribeOptions) -> Result<Transcript, EngineError> {
        if self
            .panics
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            panic!("scripted engine panic");
        }
        self.inner.transcribe(audio, o)
    }
}

/// Panics on its first insertion, then inserts through the mock platform.
struct PanickyInserter {
    armed: AtomicUsize,
    inner: Arc<MockPlatform>,
}

impl TextInserter for PanickyInserter {
    fn insert(&self, text: &str) -> Result<InsertOutcome, PlatformError> {
        if self.armed.swap(0, Ordering::SeqCst) > 0 {
            panic!("scripted inserter panic");
        }
        self.inner.insert(text)
    }
}

/// Runs `takes` takes through a worker and stops it.
fn run_takes(worker: &DictationWorker, takes: usize) {
    let hotkeys = worker.hotkey_sink();
    let mut t = 1_000_000_000;
    for _ in 0..takes {
        for input in take_script(&mut t) {
            match input {
                Input::Hotkey(e) => hotkeys(e),
                other => {
                    let _ = worker.send(other);
                }
            }
        }
    }
}

fn failures(events: &[DictationEvent]) -> Vec<bool> {
    events
        .iter()
        .filter_map(|e| match e {
            DictationEvent::WorkerFailed { recovered } => Some(*recovered),
            _ => None,
        })
        .collect()
}

#[test]
fn a_panicking_engine_is_reported_and_the_next_take_still_works() {
    let engine = Arc::new(PanickyEngine {
        panics: AtomicUsize::new(1),
        inner: answering("second time lucky"),
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let (chain, platform) = chain(engine, &events);
    let worker = DictationWorker::spawn(chain, platform.clock()).unwrap();
    run_takes(&worker, 2);
    let health = worker.health();
    let chain = worker
        .stop()
        .expect("the worker thread itself never panics");

    let events = events.lock().unwrap();
    assert_eq!(failures(&events), vec![true], "{events:?}");
    assert!(!chain.is_recording(), "back to idle");
    assert_eq!(platform.inserted(), vec!["Second time lucky. ".to_owned()]);
    assert_eq!(health.dropped_inputs(), 0);
}

#[test]
fn a_panicking_inserter_is_reported_and_the_next_take_still_works() {
    let platform = Arc::new(MockPlatform::new());
    let inserter = Arc::new(PanickyInserter {
        armed: AtomicUsize::new(1),
        inner: platform.clone(),
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let chain = chain_with(
        Arc::new(answering("typed at last")),
        &platform,
        inserter,
        &events,
    );
    let worker = DictationWorker::spawn(chain, platform.clock()).unwrap();
    run_takes(&worker, 2);
    worker.stop().unwrap();

    let events = events.lock().unwrap();
    assert_eq!(failures(&events), vec![true], "{events:?}");
    assert_eq!(platform.inserted(), vec!["Typed at last. ".to_owned()]);
}

#[test]
fn a_worker_that_keeps_panicking_stops_and_counts_what_it_can_no_longer_take() {
    let engine = Arc::new(PanickyEngine {
        panics: AtomicUsize::new(usize::MAX),
        inner: answering("never"),
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let (chain, platform) = chain(engine, &events);
    let worker = DictationWorker::spawn(chain, platform.clock()).unwrap();
    let health = worker.health();
    run_takes(&worker, MAX_PANICS_WITHOUT_A_TAKE as usize);
    // Wait for the worker to give up: the last failure is terminal.
    let sink = worker.hotkey_sink();
    for _ in 0..500 {
        if !health.is_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!health.is_running(), "the worker stopped");
    let expected: Vec<bool> = (1..=MAX_PANICS_WITHOUT_A_TAKE)
        .map(|n| n < MAX_PANICS_WITHOUT_A_TAKE)
        .collect();
    assert_eq!(failures(&events.lock().unwrap()), expected);

    // A key press into a dead worker is not lost silently: it is counted.
    let before = health.dropped_inputs();
    sink(HotkeyEvent::Pressed { at_ns: 0 });
    assert!(worker.send(Input::StreamEnded).is_err());
    assert_eq!(health.dropped_inputs(), before + 2);
    assert!(platform.inserted().is_empty());
    worker.stop().unwrap();
}

#[test]
fn a_panic_while_stopping_still_stops() {
    let engine = Arc::new(PanickyEngine {
        panics: AtomicUsize::new(usize::MAX),
        inner: answering("never"),
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let (chain, platform) = chain(engine, &events);
    let worker = DictationWorker::spawn(chain, platform.clock()).unwrap();
    // A take still open at stop: stopping processes it, and the engine panics.
    let mut t = 1_000_000_000;
    let hotkeys = worker.hotkey_sink();
    for input in take_script(&mut t) {
        match input {
            Input::Hotkey(HotkeyEvent::Released { .. }) => break,
            Input::Hotkey(e) => hotkeys(e),
            other => worker.send(other).unwrap(),
        }
    }
    let (done, stopped) = std::sync::mpsc::channel();
    std::thread::spawn(move || done.send(worker.stop().is_ok()));
    let stopped = stopped.recv_timeout(std::time::Duration::from_secs(10));
    assert_eq!(stopped, Ok(true), "stop returned");
    assert_eq!(failures(&events.lock().unwrap()), vec![false]);
}

/// Every log line of this test binary.
fn logged() -> &'static Mutex<Vec<String>> {
    static LINES: std::sync::OnceLock<Mutex<Vec<String>>> = std::sync::OnceLock::new();
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    let lines = LINES.get_or_init(Mutex::default);
    INSTALLED.get_or_init(|| {
        struct Capture;
        impl log::Log for Capture {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                let line = format!("{} {}", record.level(), record.args());
                // Never panics in a logger: a failed assertion elsewhere must not cascade.
                logged()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(line);
            }
            fn flush(&self) {}
        }
        static CAPTURE: Capture = Capture;
        log::set_logger(&CAPTURE).expect("no other logger in this binary");
        log::set_max_level(log::LevelFilter::Trace);
    });
    lines
}

#[test]
fn a_sink_that_panics_while_reporting_a_failure_is_logged_before_the_worker_stops() {
    let lines = logged();
    let engine: Arc<dyn OfflineEngine> = Arc::new(PanickyEngine {
        panics: AtomicUsize::new(usize::MAX),
        inner: answering("never"),
    });
    let platform = Arc::new(MockPlatform::new());
    // The shell's sink panics on the failure report, as a broken UI bridge might.
    let sink: EventSink<DictationEvent> = Arc::new(|e| {
        if matches!(e, DictationEvent::WorkerFailed { .. }) {
            panic!("scripted sink panic");
        }
    });
    let chain = DictationChain::new(
        Services {
            engine,
            store: Arc::new(MemStore::new()),
            inserter: platform.clone(),
            focus: platform.clone(),
            clock: platform.clock(),
            llm: None,
        },
        DictationSettings::default(),
        Vad::Unavailable(VadUnavailable::ModelMissing),
        sink,
    );
    let worker = DictationWorker::spawn(chain, platform.clock()).unwrap();
    let health = worker.health();
    run_takes(&worker, 1);
    for _ in 0..500 {
        if !health.is_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!health.is_running(), "the worker stopped");
    let lines = lines
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("ERROR") && l.contains("could not be reported")),
        "{lines:#?}"
    );
    worker.stop().unwrap();
}

// ---------------------------------------------------------------------------------------------
// Polish and the next take
// ---------------------------------------------------------------------------------------------

/// How long [`SlowLlm`] takes: longer than a cold on-device polish (about 1.4 s).
const SLOW: Duration = Duration::from_millis(1_500);

/// Answers "Polished take N." to its Nth call after [`SLOW`], watching its token meanwhile.
#[derive(Default)]
struct SlowLlm {
    calls: AtomicUsize,
    cancelled: AtomicUsize,
}

impl Llm for SlowLlm {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "slow".into(),
            model: "slow".into(),
            endpoint: Endpoint::InProcess,
        }
    }

    fn complete(&self, _: &LlmRequest, cancel: &CancelToken) -> Result<LlmResponse, LlmError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let started = Instant::now();
        while started.elapsed() < SLOW {
            if cancel.is_cancelled() {
                self.cancelled.fetch_add(1, Ordering::SeqCst);
                return Err(LlmError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(LlmResponse {
            text: format!("Polished take {n}."),
        })
    }
}

/// A press while a working polish runs does not cancel it: that take keeps its polish, and the
/// pressed take, queued with its audio meanwhile, is processed after it.
#[test]
fn a_press_while_polish_runs_leaves_that_take_polished_and_is_processed_after_it() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let platform = Arc::new(MockPlatform::new());
    let llm = Arc::new(SlowLlm::default());
    let mut settings = DictationSettings::default();
    settings.modes.modes[0].polish_enabled = true;
    settings.polish_consent = Some(ink_pipeline::consent::PolishConsent::OnDevice);
    let sink_events = events.clone();
    let chain = DictationChain::new(
        Services {
            engine: Arc::new(answering("sent from the worker")),
            store: Arc::new(MemStore::new()),
            inserter: platform.clone(),
            focus: platform.clone(),
            clock: platform.clock(),
            llm: Some(llm.clone()),
        },
        settings,
        Vad::Unavailable(VadUnavailable::ModelMissing),
        Arc::new(move |e| sink_events.lock().unwrap().push(e)),
    );
    let worker = DictationWorker::spawn(chain, platform.clock()).unwrap();
    let hotkeys = worker.hotkey_sink();
    let send = |inputs: Vec<Input>| {
        for input in inputs {
            match input {
                Input::Hotkey(e) => hotkeys(e),
                other => worker.send(other).unwrap(),
            }
        }
    };
    let mut t = 1_000_000_000;
    send(take_script(&mut t));
    let until = Instant::now() + Duration::from_secs(20);
    while llm.calls.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < until, "polish never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    // The next take, press included, while the first take's polish is still being written.
    send(take_script(&mut t));
    assert!(
        platform.inserted().is_empty(),
        "the second take was queued while the first was being polished"
    );
    worker.stop().unwrap();

    assert_eq!(
        platform.inserted(),
        vec![
            "Polished take 1. ".to_owned(),
            "Polished take 2. ".to_owned()
        ]
    );
    assert_eq!(
        llm.cancelled.load(Ordering::SeqCst),
        0,
        "nothing was cancelled"
    );
    let events = events.lock().unwrap();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, DictationEvent::Warning(Warning::PolishFailed(_)))),
        "{events:?}"
    );
}
