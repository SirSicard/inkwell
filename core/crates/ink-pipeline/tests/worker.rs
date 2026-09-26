//! The worker thread: hotkey callbacks only enqueue, and the owning thread runs the chain.

use std::sync::{Arc, Mutex};

use ink_audio::synth::speech_like;
use ink_core::mock::{MemStore, MockEngine, MockPlatform};
use ink_core::{
    EngineError, EngineInfo, EventSink, HotkeyEvent, Job, OfflineEngine, TimedText,
    TranscribeOptions, Transcript,
};
use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
use ink_pipeline::events::{DictationEvent, VadUnavailable};
use ink_pipeline::gain_stage::Vad;
use ink_pipeline::worker::{DictationWorker, Input};

const BLOCK: usize = 160;
const BLOCK_NS: u64 = 10_000_000;

/// Room, a press, speech, a release, room: as the pump and the hotkey would deliver them.
fn script() -> Vec<Input> {
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
    let mut t = 1_000_000_000;
    audio(&[0.0; 8_000], &mut inputs, &mut t);
    inputs.push(Input::Hotkey(HotkeyEvent::Pressed { at_ns: t }));
    audio(&speech_like(1.5, -30.0, 9), &mut inputs, &mut t);
    inputs.push(Input::Hotkey(HotkeyEvent::Released { at_ns: t }));
    audio(&[0.0; 9_600], &mut inputs, &mut t);
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
    let sink_events = events.clone();
    let sink: EventSink<DictationEvent> = Arc::new(move |e| sink_events.lock().unwrap().push(e));
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
    (chain, platform)
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
