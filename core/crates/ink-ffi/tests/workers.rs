//! The chains' worker threads: a bounded queue from the pump that drops and reports rather than
//! blocking, deadlines that become ticks, and the unbind instruction when dictation gives up.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use common::*;
use ink_core::HotkeyEvent;
use ink_core::mock::MockPlatform;
use ink_ffi::dictation::{Block, DictationInbox, DictationWorker, Input, WorkerGone};
use ink_ffi::gate::Routed;
use ink_ffi::hub::Hub;
use ink_ffi::mailbox::Pushed;
use ink_ffi::runtime::{Core, DictationParts};
use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::gain_stage::Vad;

const BLOCK: usize = 160;
const BLOCK_NS: u64 = 10_000_000;

fn no_vad() -> Vad {
    Vad::Unavailable(VadUnavailable::ModelMissing)
}

fn installer(loader: &MockLoader) -> Arc<MockInstaller> {
    Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    })
}

fn dictation(core: &Core, platform: &Arc<MockPlatform>) -> DictationInbox {
    core.start_dictation(DictationParts {
        inserter: platform.clone(),
        focus: platform.clone(),
        llm: None,
        settings: DictationSettings::default(),
        vad: no_vad(),
    })
    .unwrap()
}

/// Audio blocks from host time `*t`, which moves past them.
fn audio(inbox: &DictationInbox, samples: &[f32], t: &mut u64) {
    for block in samples.chunks(BLOCK) {
        inbox.push_audio(Block {
            samples: block.to_vec(),
            host_time_ns: *t,
            dropped_frames: 0,
        });
        *t += BLOCK_NS;
    }
}

/// Room, a press, speech, a release; then `quiet_after` of quiet (none: the audio just stops).
fn take(core: &Core, inbox: &DictationInbox, seed: u64, quiet_after: usize) {
    let mut t = core.shared().clock.now_ns();
    let hotkey = inbox.hotkey_sink();
    audio(inbox, &[0.0; 8_000], &mut t);
    hotkey(HotkeyEvent::Pressed { at_ns: t });
    audio(
        inbox,
        &ink_audio::synth::speech_like(1.5, -30.0, seed),
        &mut t,
    );
    hotkey(HotkeyEvent::Released { at_ns: t });
    audio(inbox, &vec![0.0; quiet_after], &mut t);
}

#[test]
fn a_full_queue_drops_and_reports_without_blocking_the_pump() {
    let dir = TempDir::new("queue");
    let busy = Arc::new(Gate::default());
    let loader = MockLoader::new(Behaviour::WaitThen(busy.clone(), "words".into()));
    let (core, _) = start(
        &dir,
        &[test_row(ROW_ID)],
        loader.clone(),
        installer(&loader),
    );
    // A worker of its own, with room for 8 blocks, reporting to its own recorder.
    let events = Recorder::new();
    let hub = Hub::start(events.out()).unwrap();
    let platform = Arc::new(MockPlatform::new());
    let s = core.shared();
    let chain = DictationChain::new(
        Services {
            engine: Arc::new(Routed::new(s.clone(), ink_core::Job::DictationFinal)),
            store: s.store.clone(),
            inserter: platform.clone(),
            focus: platform.clone(),
            clock: s.clock.clone(),
            llm: None,
        },
        DictationSettings::default(),
        no_vad(),
        {
            let e = hub.events();
            Arc::new(move |d| e.emit(ink_ffi::events::dictation(&d)))
        },
    );
    let worker = DictationWorker::spawn(chain, s.clock.clone(), hub.events(), 8).unwrap();
    let inbox = worker.inbox();

    // A take, fed no faster than the worker takes it (a real pump is paced by the device), and
    // the worker then stuck in the engine with it.
    let paced = |samples: &[f32], t: &mut u64| {
        for block in samples.chunks(BLOCK) {
            while inbox.queued_audio() >= 8 {
                if busy.waiting.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                    // The tail ended early and the take is in the engine: the rest is not
                    // needed.
                    return;
                }
                std::thread::yield_now();
            }
            assert!(matches!(
                inbox.push_audio(Block {
                    samples: block.to_vec(),
                    host_time_ns: *t,
                    dropped_frames: 0,
                }),
                Pushed::Queued { after_drop: None }
            ));
            *t += BLOCK_NS;
        }
    };
    let mut t = s.clock.now_ns();
    let hotkey = inbox.hotkey_sink();
    paced(&[0.0; 8_000], &mut t);
    hotkey(HotkeyEvent::Pressed { at_ns: t });
    paced(&ink_audio::synth::speech_like(1.5, -30.0, 5), &mut t);
    hotkey(HotkeyEvent::Released { at_ns: t });
    paced(&[0.0; 4_800], &mut t);
    assert!(
        busy.until_waiting(Duration::from_secs(10)),
        "the worker is in the engine"
    );
    assert_eq!(inbox.dropped().blocks, 0, "nothing dropped while paced");

    // The pump goes on: 50 blocks while the worker cannot take any.
    let mut slowest = Duration::ZERO;
    let mut results = Vec::new();
    for _ in 0..50 {
        let started = Instant::now();
        results.push(inbox.push_audio(Block {
            samples: vec![0.0; BLOCK],
            host_time_ns: t,
            dropped_frames: 0,
        }));
        slowest = slowest.max(started.elapsed());
        t += BLOCK_NS;
    }
    assert!(
        slowest < Duration::from_millis(50),
        "a push waited: {slowest:?}"
    );
    let queued = results
        .iter()
        .filter(|r| matches!(r, Pushed::Queued { .. }))
        .count();
    let dropped = results.iter().filter(|r| **r == Pushed::Dropped).count();
    assert_eq!(queued + dropped, 50);
    assert!(inbox.queued_audio() <= 8, "never more than the bound");
    let after = inbox.dropped();
    assert_eq!(after.blocks, dropped as u64);
    assert_eq!(after.samples, (dropped * BLOCK) as u64);
    assert!(dropped >= 42, "{dropped} dropped");

    // The worker catches up; the next block the queue takes reports the whole stretch.
    busy.open();
    let until = Instant::now() + Duration::from_secs(10);
    while inbox.queued_audio() > 0 {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    let next = inbox.push_audio(Block {
        samples: vec![0.0; BLOCK],
        host_time_ns: t,
        dropped_frames: 0,
    });
    let Pushed::Queued {
        after_drop: Some(stretch),
    } = next
    else {
        panic!("expected the stretch to be reported, got {next:?}");
    };
    assert_eq!(stretch.blocks, after.blocks);
    let report = events.wait_type("audio.dropped", Duration::from_secs(5));
    assert_eq!(report["chain"], "dictation");
    assert_eq!(report["channel"], "mic");
    assert_eq!(report["blocks"], after.blocks);
    assert_eq!(report["samples"], after.samples);
    worker.stop().unwrap();
    hub.stop();
    core.shutdown();
    events.assert_valid();
}

#[test]
fn the_dictation_worker_ticks_a_tail_whose_audio_stopped() {
    let dir = TempDir::new("tick");
    let loader = MockLoader::new(Behaviour::Say("ticked".into()));
    let (core, events) = start(
        &dir,
        &[test_row(ROW_ID)],
        loader.clone(),
        installer(&loader),
    );
    let platform = Arc::new(MockPlatform::new());
    let inbox = dictation(&core, &platform);
    // No audio after the release: only the tail's deadline can end this take.
    let released = Instant::now();
    take(&core, &inbox, 7, 0);
    let inserted = events.wait_type("dictation.inserted", Duration::from_secs(10));
    assert_eq!(inserted["text"], "Ticked.");
    let warning = events.wait_type("dictation.warning", Duration::ZERO);
    assert_eq!(warning["kind"], "tail_cut_short", "ended by the deadline");
    // The deadline is the release (stamped up to 2 s ahead) plus the tail and its grace.
    assert!(
        released.elapsed() >= Duration::from_millis(300),
        "{:?}",
        released.elapsed()
    );
    core.shutdown();
    events.assert_valid();
}

#[test]
fn a_worker_that_keeps_panicking_tells_the_shell_to_unbind_the_hotkey() {
    let dir = TempDir::new("panic");
    let loader = MockLoader::new(Behaviour::Panic);
    let (core, events) = start(
        &dir,
        &[test_row(ROW_ID)],
        loader.clone(),
        installer(&loader),
    );
    let platform = Arc::new(MockPlatform::new());
    let inbox = dictation(&core, &platform);
    for (n, seed) in [31, 32, 33].into_iter().enumerate() {
        take(&core, &inbox, seed, 9_600);
        assert!(events.wait_count("dictation.worker_failed", n + 1, Duration::from_secs(10)));
    }
    let failures: Vec<_> = events
        .all()
        .into_iter()
        .filter(|v| v["type"] == "dictation.worker_failed")
        .collect();
    assert_eq!(failures.len(), 3);
    for recovered in &failures[..2] {
        assert_eq!(recovered["recovered"], true);
        assert_eq!(recovered["unbind_hotkey"], false);
    }
    assert_eq!(failures[2]["recovered"], false);
    assert_eq!(
        failures[2]["unbind_hotkey"], true,
        "the shell must unbind the hotkey"
    );

    // The worker is gone: nothing reaches it any more, and the pump is told so.
    let until = Instant::now() + Duration::from_secs(5);
    while inbox.send(Input::StreamEnded).is_ok() {
        assert!(Instant::now() < until, "the worker did not stop");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(inbox.send(Input::StreamEnded), Err(WorkerGone));
    let pushed = inbox.push_audio(Block {
        samples: vec![0.0; BLOCK],
        host_time_ns: 0,
        dropped_frames: 0,
    });
    assert_eq!(pushed, Pushed::Closed);
    core.shutdown();
    events.assert_valid();
}
