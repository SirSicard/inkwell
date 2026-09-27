//! A language model the shell registers (`INK_ENGINE_LLM`, Foundation Models on the Mac) and the
//! dictation polish that goes to it: used while registered, an "unavailable" answer kept as a
//! failure (the dictation goes out as written, never with made-up text), and nothing used once it
//! is let go of.

mod common;

use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use ink_core::mock::MockPlatform;
use ink_core::{HotkeyEvent, Llm};
use ink_ffi::dictation::{Block, DictationInbox};
use ink_ffi::external::{InkEngineVTable, KIND_LLM, Registration};
use ink_ffi::llms::PolishModel;
use ink_ffi::runtime::{Core, DictationParts};
use ink_pipeline::chain::DictationSettings;
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::gain_stage::Vad;

/// How the fake model answers.
#[derive(Clone, Copy)]
enum Says {
    Polished,
    Unavailable,
}

struct Model {
    says: Mutex<Says>,
    requests: Mutex<Vec<serde_json::Value>>,
    released: AtomicUsize,
}

unsafe extern "C" fn generate(ctx: *mut c_void, call: u64, request: *const c_char) {
    // SAFETY: ctx is the test's model, alive past its release; the request is valid for the call.
    let (me, request) = unsafe {
        (
            &*(ctx as *const Model),
            std::ffi::CStr::from_ptr(request)
                .to_str()
                .unwrap()
                .to_owned(),
        )
    };
    me.requests
        .lock()
        .unwrap()
        .push(serde_json::from_str(&request).unwrap());
    let answer = match *me.says.lock().unwrap() {
        Says::Polished => r#"{"text":"Polished synthetic words."}"#,
        // A message is ignored by the core: only the kind and code are read (I5).
        Says::Unavailable => {
            r#"{"error":{"kind":"unavailable","code":2,"message":"synthetic words"}}"#
        }
    };
    // Answered from a thread of the engine's own, as a Swift Task would.
    std::thread::spawn(move || {
        let answer = CString::new(answer).unwrap();
        // SAFETY: a NUL-terminated string valid for the call.
        unsafe { ink_ffi::ink_engine_complete(call, answer.as_ptr()) };
    });
}

unsafe extern "C" fn release(ctx: *mut c_void) {
    // SAFETY: as above.
    let me = unsafe { &*(ctx as *const Model) };
    me.released.fetch_add(1, Ordering::SeqCst);
}

fn model_table(me: &Model, info: &CString) -> InkEngineVTable {
    InkEngineVTable {
        kind: KIND_LLM,
        info_json: info.as_ptr(),
        ctx: me as *const Model as *mut c_void,
        release: Some(release),
        generate: Some(generate),
        ..Default::default()
    }
}

fn register(core: &Core, me: &Model, info: &CString) -> Result<String, String> {
    // SAFETY: a valid table whose ctx outlives the core.
    let r =
        unsafe { Registration::from_table(&model_table(me, info), core.shared().shutdown.clone()) }
            .map_err(|e| e.0)?;
    core.register(r)
}

const BLOCK: usize = 160;
const BLOCK_NS: u64 = 10_000_000;

/// A take: room, a press, speech, a release, quiet.
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
    audio(&[0.0; 16_000], &mut t);
}

#[test]
fn dictation_polish_goes_to_the_registered_model_and_never_fakes_an_answer() {
    let dir = TempDir::new("llm");
    let loader = MockLoader::new(Behaviour::Say("synthetic words".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader, installer);

    let model = Model {
        says: Mutex::new(Says::Polished),
        requests: Mutex::default(),
        released: AtomicUsize::new(0),
    };
    let info = CString::new(
        r#"{"id":"apple-foundation-models","licence":"Apple","model":"system","local":true}"#,
    )
    .unwrap();
    register(&core, &model, &info).unwrap();
    let registered = events.wait_type("engine.registered", Duration::from_secs(5));
    assert_eq!(registered["kind"], "llm");
    assert_eq!(registered["jobs"], serde_json::json!([]));
    // One id space across kinds: the registry model's id is refused, and so is a second model
    // under the same id.
    let clash = CString::new(format!(
        r#"{{"id":"{ROW_ID}","licence":"Apple","model":"system","local":true}}"#
    ))
    .unwrap();
    assert!(register(&core, &model, &clash).is_err());
    assert!(register(&core, &model, &info).is_err());
    assert_eq!(
        model.released.load(Ordering::SeqCst),
        0,
        "refusals never release"
    );

    // Where it runs is the shell's declaration, for the local-only guard.
    let polish = PolishModel::new(core.shared().llms.clone());
    assert_eq!(polish.info().endpoint, ink_core::Endpoint::InProcess);
    assert_eq!(polish.info().model, "system");

    let platform = Arc::new(MockPlatform::new());
    let mut settings = DictationSettings::default();
    settings.modes.modes[0].polish_enabled = true;
    let inbox = core
        .start_dictation(DictationParts {
            inserter: platform.clone(),
            focus: platform.clone(),
            llm: None,
            settings,
            vad: Vad::Unavailable(VadUnavailable::ModelMissing),
        })
        .unwrap();

    // Registered: the take is polished by it.
    take(&core, &inbox, 1);
    assert!(events.wait_count("dictation.inserted", 1, Duration::from_secs(20)));
    assert_eq!(
        platform.inserted().last().map(|s| s.trim().to_owned()),
        Some("Polished synthetic words.".into())
    );
    let request = model.requests.lock().unwrap()[0].clone();
    let user = request["user"].as_str().unwrap().to_lowercase();
    assert!(user.contains("synthetic words"), "{user}");
    assert!(
        request["system"]
            .as_str()
            .unwrap()
            .contains("speech-to-text")
    );

    // Unavailable (Apple Intelligence turned off, say): the text goes out as written, and the
    // warning says why, without the words.
    *model.says.lock().unwrap() = Says::Unavailable;
    take(&core, &inbox, 2);
    assert!(events.wait_count("dictation.inserted", 2, Duration::from_secs(20)));
    let inserted = platform.inserted();
    assert!(
        inserted.last().unwrap().contains("ynthetic words")
            && !inserted.last().unwrap().contains("Polished"),
        "{inserted:?}"
    );
    let warning = events
        .wait_for(Duration::from_secs(5), |v| {
            v["type"] == "dictation.warning" && v["kind"] == "polish_failed"
        })
        .expect("polish_failed");
    let message = warning["message"].as_str().unwrap();
    assert!(message.contains("unavailable"), "{message}");
    assert!(
        !message.contains("synthetic"),
        "no words in the warning: {message}"
    );

    // Let go of: released at once (no call holds it), and the next take is not polished.
    core.command(r#"{"cmd":"engine.unregister","engine":"apple-foundation-models"}"#)
        .unwrap();
    events.wait_type("engine.unregistered", Duration::from_secs(5));
    assert_eq!(model.released.load(Ordering::SeqCst), 1);
    let asked = model.requests.lock().unwrap().len();
    take(&core, &inbox, 3);
    assert!(events.wait_count("dictation.inserted", 3, Duration::from_secs(20)));
    assert_eq!(
        model.requests.lock().unwrap().len(),
        asked,
        "never asked again"
    );
    assert!(events.wait_count("dictation.warning", 2, Duration::from_secs(5)));

    core.shutdown();
    assert_eq!(model.released.load(Ordering::SeqCst), 1, "released once");
    events.assert_valid();
}

#[test]
fn a_model_still_registered_at_shutdown_is_released_before_it_returns() {
    let dir = TempDir::new("llm-shutdown");
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, _events) = start(&dir, &[], loader, installer);
    let model = Model {
        says: Mutex::new(Says::Polished),
        requests: Mutex::default(),
        released: AtomicUsize::new(0),
    };
    let info =
        CString::new(r#"{"id":"m","licence":"Apple","model":"system","local":false}"#).unwrap();
    register(&core, &model, &info).unwrap();
    // A model that says it is not local is reported as remote, for local-only mode to refuse.
    let polish = PolishModel::new(core.shared().llms.clone());
    assert!(!polish.info().endpoint.is_local());
    drop(polish);
    let stopped = core.shutdown();
    assert_eq!(stopped.engines_released, 1);
    assert_eq!(model.released.load(Ordering::SeqCst), 1);
}
