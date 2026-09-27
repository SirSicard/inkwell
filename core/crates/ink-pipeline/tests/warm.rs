//! The dictation engine's warm-up on key-down: it runs after a quiet spell, never delays a take,
//! and its answer never reaches anything.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ink_core::mock::MockClock;
use ink_core::{
    CancelToken, Channel, Clock, EngineError, EngineInfo, Job, OfflineEngine, TimedText,
    TranscribeOptions, Transcript,
};
use ink_pipeline::warm::{EngineWarmer, WARM_AUDIO};

/// An engine that answers words for anything (Qwen3-ASR invents text on silence), records each
/// call's length, and holds a call of silence (a warm-up) until cancelled or `release`d.
struct Engine {
    calls: Mutex<Vec<usize>>,
    busy: AtomicUsize,
    hold_warmups: bool,
}

impl Engine {
    fn new(hold_warmups: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::default(),
            busy: AtomicUsize::new(0),
            hold_warmups,
        })
    }

    fn calls(&self) -> Vec<usize> {
        self.calls.lock().unwrap().clone()
    }
}

impl OfflineEngine for Engine {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: "test-asr".into(),
            jobs: vec![Job::DictationFinal],
            licence: "MIT".into(),
        }
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        self.calls.lock().unwrap().push(audio.len());
        self.busy.fetch_add(1, Ordering::SeqCst);
        let silent = audio.iter().all(|&s| s == 0.0);
        if silent && self.hold_warmups {
            while !options.cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(2));
            }
            self.busy.fetch_sub(1, Ordering::SeqCst);
            return Err(EngineError::Cancelled);
        }
        self.busy.fetch_sub(1, Ordering::SeqCst);
        let text = if silent {
            "invented on silence"
        } else {
            "words made up from nothing"
        };
        Ok(Transcript {
            segments: vec![TimedText {
                start_ms: 0,
                end_ms: 500,
                text: text.into(),
            }],
        })
    }
}

fn options() -> TranscribeOptions {
    TranscribeOptions {
        channel: Channel::Mic,
        context: None,
        cancel: CancelToken::new(),
    }
}

fn wait_until(what: impl Fn() -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn a_take_starting_after_a_quiet_spell_warms_the_engine_with_silence() {
    let clock = Arc::new(MockClock::new(1_000_000_000, 0));
    let engine = Engine::new(false);
    let warmer =
        EngineWarmer::start(engine.clone(), clock.clone(), Duration::from_secs(30)).unwrap();
    assert!(warmer.key_down(), "never decoded: warm");
    assert!(wait_until(|| warmer.warmups() == 1));
    assert_eq!(engine.calls(), [WARM_AUDIO], "half a second of silence");
    // Warm now: the next take's start asks for nothing.
    assert!(!warmer.key_down());
    // After the quiet spell, again.
    clock.advance_ns(31_000_000_000);
    assert!(warmer.key_down());
    assert!(wait_until(|| warmer.warmups() == 2));
    warmer.stop();
}

#[test]
fn a_decode_keeps_the_engine_warm() {
    let clock = Arc::new(MockClock::new(1_000_000_000, 0));
    let engine = Engine::new(false);
    let warmer =
        EngineWarmer::start(engine.clone(), clock.clone(), Duration::from_secs(30)).unwrap();
    let chain_engine = warmer.engine();
    chain_engine.transcribe(&[0.1; 16_000], &options()).unwrap();
    clock.advance_ns(10_000_000_000);
    assert!(!warmer.key_down(), "decoded 10 s ago");
    warmer.stop();
    assert_eq!(engine.calls(), [16_000]);
}

/// The take's decode cancels a warm-up in progress and does not wait for its end.
#[test]
fn a_take_never_waits_for_a_warm_up() {
    let clock = Arc::new(MockClock::new(1_000_000_000, 0));
    let engine = Engine::new(true);
    let warmer =
        EngineWarmer::start(engine.clone(), clock.clone(), Duration::from_secs(30)).unwrap();
    assert!(warmer.key_down());
    assert!(
        wait_until(|| engine.busy.load(Ordering::SeqCst) == 1),
        "warming"
    );
    let started = Instant::now();
    let text = warmer
        .engine()
        .transcribe(&[0.1; 16_000], &options())
        .unwrap()
        .text();
    assert_eq!(text, "words made up from nothing");
    assert!(wait_until(|| warmer.warmups() == 1));
    assert_eq!(warmer.yielded(), 1, "the warm-up gave way");
    // Held until cancelled: it ended only because the take cancelled it, long before its budget.
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    warmer.stop();
}

/// Once a take's decode has begun, a warm-up asked for meanwhile is skipped: it would only take
/// turns with the take.
#[test]
fn no_warm_up_starts_once_a_take_is_decoding() {
    let clock = Arc::new(MockClock::new(1_000_000_000, 0));
    let engine = Engine::new(false);
    let warmer =
        EngineWarmer::start(engine.clone(), clock.clone(), Duration::from_secs(30)).unwrap();
    warmer.engine().transcribe(&[0.1; 160], &options()).unwrap();
    // The engine is warm now, so even a request that raced the decode finds nothing to do.
    warmer.key_down();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(warmer.warmups(), 0);
    assert_eq!(engine.calls(), [160]);
    warmer.stop();
}

/// Starts of several takes before the warm-up ran are one warm-up.
#[test]
fn requests_that_queue_up_are_one_warm_up() {
    let clock = Arc::new(MockClock::new(1_000_000_000, 0));
    let engine = Engine::new(true);
    let warmer =
        EngineWarmer::start(engine.clone(), clock.clone(), Duration::from_secs(30)).unwrap();
    for _ in 0..5 {
        warmer.key_down();
    }
    assert!(wait_until(|| engine.busy.load(Ordering::SeqCst) == 1));
    // Let the held warm-up go by stopping the warmer (which cancels it).
    warmer.stop();
    assert_eq!(engine.calls().len(), 1);
}

/// Stopping cancels a warm-up in progress rather than waiting out its budget.
#[test]
fn stopping_cancels_a_warm_up_in_progress() {
    let clock = Arc::new(MockClock::new(1_000_000_000, 0));
    let engine = Engine::new(true);
    let warmer =
        EngineWarmer::start(engine.clone(), clock.clone(), Duration::from_secs(30)).unwrap();
    warmer.key_down();
    assert!(wait_until(|| engine.busy.load(Ordering::SeqCst) == 1));
    let started = Instant::now();
    warmer.stop();
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(clock.now_ns(), 1_000_000_000);
}

/// Whatever the engine makes of the warm-up's silence goes nowhere: the chain behind the warmer
/// inserts only what its take said.
#[test]
fn a_warm_up_answer_is_never_inserted() {
    use ink_core::mock::{MemStore, MockPlatform};
    use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
    use ink_pipeline::gain_stage::Vad;

    let platform = Arc::new(MockPlatform::new());
    let engine = Engine::new(false);
    let warmer =
        EngineWarmer::start(engine.clone(), platform.clock(), Duration::from_secs(30)).unwrap();
    let mut chain = DictationChain::new(
        Services {
            engine: warmer.engine(),
            store: Arc::new(MemStore::new()),
            inserter: platform.clone(),
            focus: platform.clone(),
            clock: platform.clock(),
            llm: None,
        },
        DictationSettings::default(),
        Vad::Unavailable(ink_pipeline::events::VadUnavailable::ModelMissing),
        Arc::new(|_| {}),
    );
    // The warm-up runs to its end (its answer: "invented on silence").
    assert!(warmer.key_down());
    assert!(wait_until(|| warmer.warmups() == 1));
    let clock = platform.clock();
    let speech = ink_audio::synth::speech_like(1.0, -25.0, 1);
    let feed = |chain: &mut DictationChain, samples: &[f32]| {
        for block in samples.chunks(160) {
            chain.push_audio(block, clock.now_ns(), 0);
            clock.advance_ns(10_000_000);
        }
    };
    feed(&mut chain, &[0.0; 8_000]);
    chain.hotkey(ink_core::HotkeyEvent::Pressed {
        at_ns: platform.clock().now_ns(),
    });
    feed(&mut chain, &speech);
    chain.hotkey(ink_core::HotkeyEvent::Released {
        at_ns: platform.clock().now_ns(),
    });
    feed(&mut chain, &[0.0; 9_600]);
    assert_eq!(platform.inserted(), ["Words made up from nothing. "]);
    warmer.stop();
}

/// A handle still held elsewhere (a chain's event sink) never keeps the thread alive, and asks
/// for nothing once the warmer stopped.
#[test]
fn stopping_ends_the_thread_while_a_handle_is_still_held() {
    let clock = Arc::new(MockClock::new(1_000_000_000, 0));
    let engine = Engine::new(false);
    let warmer =
        EngineWarmer::start(engine.clone(), clock.clone(), Duration::from_secs(30)).unwrap();
    let handle = warmer.handle();
    warmer.stop();
    assert!(!handle.key_down(), "nothing is listening");
    assert!(engine.calls().is_empty());
}

/// A stop that races a warm-up request never waits out a whole warm-up: the thread checks the
/// stop under the same lock it starts a decode under. Repeated, since the race is a timing one.
#[test]
fn a_stop_racing_a_warm_up_request_returns_at_once() {
    for _ in 0..50 {
        let clock = Arc::new(MockClock::new(1_000_000_000, 0));
        let engine = Engine::new(true);
        let warmer =
            EngineWarmer::start(engine.clone(), clock.clone(), Duration::from_secs(30)).unwrap();
        warmer.key_down();
        let started = Instant::now();
        warmer.stop();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
    }
}
