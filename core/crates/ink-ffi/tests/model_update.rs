//! A model update holds its model exclusively: between unloading it and installing the new files,
//! a job that needs it is refused (with `model.refused`), never served from files being replaced.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::*;
use ink_core::HotkeyEvent;
use ink_core::mock::MockPlatform;
use ink_ffi::dictation::Block;
use ink_ffi::dictation::DictationInbox;
use ink_ffi::runtime::{Core, DictationParts};
use ink_pipeline::chain::DictationSettings;
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::gain_stage::Vad;

const BLOCK: usize = 160;
const BLOCK_NS: u64 = 10_000_000;

fn dictation(core: &Core, platform: &Arc<MockPlatform>) -> DictationInbox {
    core.start_dictation(DictationParts {
        inserter: platform.clone(),
        focus: platform.clone(),
        llm: None,
        settings: DictationSettings::default(),
        vad: Vad::Unavailable(VadUnavailable::ModelMissing),
    })
    .unwrap()
}

/// One take as the pump and the hotkey deliver it, stamped from now on the core's clock: room, a
/// press, 1.5 s of speech, a release, then quiet so the tail ends.
fn take(core: &Core, inbox: &DictationInbox, seed: u64) {
    let mut t = core.shared().clock.now_ns();
    let hotkey = inbox.hotkey_sink();
    let audio = |samples: &[f32], t: &mut u64| {
        for block in samples.chunks(BLOCK) {
            inbox.push_audio(Block {
                samples: block.to_vec(),
                host_time_ns: *t,
                dropped_frames: 0,
            });
            *t += BLOCK_NS;
        }
    };
    audio(&[0.0; 8_000], &mut t);
    hotkey(HotkeyEvent::Pressed { at_ns: t });
    audio(&ink_audio::synth::speech_like(1.5, -30.0, seed), &mut t);
    hotkey(HotkeyEvent::Released { at_ns: t });
    audio(&[0.0; 9_600], &mut t);
}

#[test]
fn a_dictation_during_an_update_is_refused_and_never_loads_the_model() {
    let dir = TempDir::new("update");
    let loader = MockLoader::new(Behaviour::Say("dictated words".into()));
    let gate = Arc::new(Gate::default());
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: Some(gate.clone()),
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader.clone(), installer.clone());
    core.command(r#"{"cmd":"model.warm","job":"dictation_final"}"#)
        .unwrap();
    events.wait_type("model.warmed", Duration::from_secs(5));
    assert_eq!(*loader.journal.loads.lock().unwrap(), [1]);
    let platform = Arc::new(MockPlatform::new());
    let inbox = dictation(&core, &platform);

    // The update unloads the model and stops mid-install, the files half replaced.
    core.command(&format!(
        r#"{{"cmd":"model.update","model":"{ROW_ID}","next":"{ROW_ID}"}}"#
    ))
    .unwrap();
    events.wait_type("model.update_started", Duration::from_secs(5));
    assert!(
        gate.until_waiting(Duration::from_secs(5)),
        "the install began"
    );
    assert!(core.shared().gate.is_held(ROW_ID));
    assert!(
        core.shared().residency.resident().is_empty(),
        "unloaded first"
    );
    assert_eq!(
        loader.generation.load(Ordering::SeqCst),
        0,
        "files being replaced"
    );

    // A dictation now is refused, and nothing is loaded.
    take(&core, &inbox, 11);
    let refused = events.wait_type("model.refused", Duration::from_secs(10));
    assert_eq!(refused["id"], ROW_ID);
    assert_eq!(refused["job"], "dictation_final");
    assert_eq!(refused["reason"], "updating");
    let failed = events.wait_type("dictation.failed", Duration::from_secs(5));
    assert_eq!(failed["stage"], "transcription");
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .contains("being updated"),
        "{failed}"
    );
    assert_eq!(
        *loader.journal.loads.lock().unwrap(),
        [1],
        "no load while the files were being replaced"
    );
    assert!(platform.inserted().is_empty());
    // Commands run one at a time: a warm asked for now waits for the update to finish.
    core.command(r#"{"cmd":"model.warm","job":"dictation_final"}"#)
        .unwrap();

    // The install finishes, the new model is warmed under the hold, and the hold is released.
    gate.open();
    let finished = events.wait_type("model.update_finished", Duration::from_secs(5));
    assert_eq!(finished["ok"], true, "{finished}");
    assert_eq!(finished["no_model_warm"], false);
    assert!(!core.shared().gate.is_held(ROW_ID));
    assert!(events.wait_count("model.warmed", 2, Duration::from_secs(5)));
    let types = events.types();
    let finished_at = types
        .iter()
        .position(|t| t == "model.update_finished")
        .unwrap();
    let last_warm = types.iter().rposition(|t| t == "model.warmed").unwrap();
    assert!(
        last_warm > finished_at,
        "the queued warm ran after the update: {types:?}"
    );
    assert_eq!(
        events.count("model.refused"),
        1,
        "only the take was refused"
    );
    assert_eq!(
        loader.journal.loads.lock().unwrap().first(),
        Some(&1),
        "{:?}",
        loader.journal.loads
    );
    assert!(
        loader.journal.loads.lock().unwrap()[1..]
            .iter()
            .all(|g| *g == 2),
        "every later load read the new files: {:?}",
        loader.journal.loads
    );

    // The next dictation is served by the new model.
    take(&core, &inbox, 12);
    let inserted = events.wait_type("dictation.inserted", Duration::from_secs(10));
    // The default mode writes it as a sentence.
    assert_eq!(inserted["text"], "Dictated words.");
    assert_eq!(
        platform.inserted().len(),
        1,
        "inserted once, by the second take only"
    );
    assert_eq!(installer.installs.load(Ordering::SeqCst), 1);
    core.shutdown();
    events.assert_valid();
}

#[test]
fn an_update_is_refused_while_a_job_is_using_the_model() {
    let dir = TempDir::new("update-busy");
    let busy = Arc::new(Gate::default());
    let loader = MockLoader::new(Behaviour::WaitThen(busy.clone(), "slow words".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader.clone(), installer.clone());
    let platform = Arc::new(MockPlatform::new());
    let inbox = dictation(&core, &platform);
    take(&core, &inbox, 21);
    assert!(
        busy.until_waiting(Duration::from_secs(10)),
        "the take reached the model"
    );

    core.command(&format!(
        r#"{{"cmd":"model.update","model":"{ROW_ID}","next":"{ROW_ID}","id":"u1"}}"#
    ))
    .unwrap();
    let failed = events.wait_type("command.failed", Duration::from_secs(5));
    assert!(
        failed["message"].as_str().unwrap().contains("in use"),
        "{failed}"
    );
    assert_eq!(failed["id"], "u1");
    assert_eq!(events.count("model.update_started"), 0);
    assert_eq!(
        installer.installs.load(Ordering::SeqCst),
        0,
        "nothing was replaced"
    );

    busy.open();
    events.wait_type("dictation.inserted", Duration::from_secs(10));
    core.shutdown();
    events.assert_valid();
}
