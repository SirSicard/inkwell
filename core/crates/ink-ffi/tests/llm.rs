//! A language model the shell registers (`INK_ENGINE_LLM`, Foundation Models on the Mac) and the
//! dictation polish that goes to it: used while registered, an "unavailable" answer kept as a
//! failure (the dictation goes out as written, never with made-up text), nothing used once it is
//! let go of, a model that never answers cut off at polish's budget, and a working polish never cut
//! short by the next take's press.

mod common;

use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use ink_core::mock::MockPlatform;
use ink_core::{HotkeyEvent, Llm};
use ink_ffi::dictation::{Block, DictationInbox};
use ink_ffi::external::{InkEngineVTable, KIND_LLM, Registration};
use ink_ffi::llms::PolishModel;
use ink_ffi::runtime::{Core, DictationParts};
use ink_pipeline::chain::DictationSettings;
use ink_pipeline::consent::LlmConsent;
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::gain_stage::Vad;

/// How the fake model answers.
#[derive(Clone, Copy)]
enum Says {
    Polished,
    Unavailable,
    /// Nothing, ever: a hung model.
    Never,
    /// "Synthetic words, take N." for its Nth request, after `SLOW`: a cold but working model.
    /// A cleanup of what was said, as polish's answer must be (the take says "synthetic words").
    Slow,
}

/// How long a `Says::Slow` model takes: longer than a cold on-device polish (about 1.4 s).
const SLOW: Duration = Duration::from_millis(1_500);

struct Model {
    says: Mutex<Says>,
    requests: Mutex<Vec<serde_json::Value>>,
    released: AtomicUsize,
    /// Calls the core told the model it gave up on.
    cancels: AtomicUsize,
}

impl Model {
    fn new(says: Says) -> Self {
        Self {
            says: Mutex::new(says),
            requests: Mutex::default(),
            released: AtomicUsize::new(0),
            cancels: AtomicUsize::new(0),
        }
    }
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
    let n = {
        let mut requests = me.requests.lock().unwrap();
        requests.push(serde_json::from_str(&request).unwrap());
        requests.len()
    };
    let (answer, delay) = match *me.says.lock().unwrap() {
        Says::Never => return,
        Says::Polished => (r#"{"text":"Polished synthetic words."}"#.to_owned(), None),
        // A message is ignored by the core: only the kind and code are read (I5).
        Says::Unavailable => (
            r#"{"error":{"kind":"unavailable","code":2,"message":"synthetic words"}}"#.to_owned(),
            None,
        ),
        Says::Slow => (
            format!(r#"{{"text":"Synthetic words, take {n}."}}"#),
            Some(SLOW),
        ),
    };
    // Answered from a thread of the engine's own, as a Swift Task would.
    std::thread::spawn(move || {
        if let Some(delay) = delay {
            std::thread::sleep(delay);
        }
        let answer = CString::new(answer).unwrap();
        // SAFETY: a NUL-terminated string valid for the call.
        unsafe { ink_ffi::ink_engine_complete(call, answer.as_ptr()) };
    });
}

unsafe extern "C" fn cancel(ctx: *mut c_void, _call: u64) {
    // SAFETY: as above.
    let me = unsafe { &*(ctx as *const Model) };
    me.cancels.fetch_add(1, Ordering::SeqCst);
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
        cancel: Some(cancel),
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

    let model = Model::new(Says::Polished);
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
    let polish = PolishModel::new(core.shared().llms.clone(), core.shared().local_only.clone());
    assert_eq!(polish.info().endpoint, ink_core::Endpoint::InProcess);
    assert_eq!(polish.info().model, "system");

    let platform = Arc::new(MockPlatform::new());
    let mut settings = DictationSettings::default();
    settings.modes.modes[0].polish_enabled = true;
    settings.polish_consents = vec![LlmConsent::OnDevice];
    store_polish_consents(&core, &settings.polish_consents);
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
    let model = Model::new(Says::Polished);
    let info =
        CString::new(r#"{"id":"m","licence":"Apple","model":"system","local":false}"#).unwrap();
    register(&core, &model, &info).unwrap();
    // A model that says it is not local is reported as remote, for local-only mode to refuse.
    let polish = PolishModel::new(core.shared().llms.clone(), core.shared().local_only.clone());
    assert!(!polish.info().endpoint.is_local());
    drop(polish);
    let stopped = core.shutdown();
    assert_eq!(stopped.engines_released, 1);
    assert_eq!(model.released.load(Ordering::SeqCst), 1);
}

/// A core with a model registered as `apple-foundation-models` and dictation started with polish
/// on and `budget` for it.
fn polishing(
    dir: &TempDir,
    model: &Model,
    budget: Duration,
) -> (Core, Arc<Recorder>, Arc<MockPlatform>, DictationInbox) {
    let loader = MockLoader::new(Behaviour::Say("synthetic words".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(dir, &[test_row(ROW_ID)], loader, installer);
    let info = CString::new(
        r#"{"id":"apple-foundation-models","licence":"Apple","model":"system","local":true}"#,
    )
    .unwrap();
    register(&core, model, &info).unwrap();
    events.wait_type("engine.registered", Duration::from_secs(5));
    let platform = Arc::new(MockPlatform::new());
    let mut settings = DictationSettings::default();
    settings.modes.modes[0].polish_enabled = true;
    settings.polish_consents = vec![LlmConsent::OnDevice];
    settings.polish_budget = budget;
    store_polish_consents(&core, &settings.polish_consents);
    let inbox = core
        .start_dictation(DictationParts {
            inserter: platform.clone(),
            focus: platform.clone(),
            llm: None,
            settings,
            vad: Vad::Unavailable(VadUnavailable::ModelMissing),
        })
        .unwrap();
    (core, events, platform, inbox)
}

/// The take went out as written, and the warning says polish ran out of its time, without the
/// words: `polish_timed_out`, apart from a cancel's `polish_failed`.
fn assert_unpolished_and_warned(events: &Recorder, platform: &MockPlatform) {
    let inserted = platform.inserted();
    let last = inserted.last().unwrap();
    assert!(
        last.contains("ynthetic words") && !last.contains("Polished"),
        "{inserted:?}"
    );
    let warning = events
        .wait_for(Duration::from_secs(5), |v| {
            v["type"] == "dictation.warning" && v["kind"] == "polish_timed_out"
        })
        .expect("polish_timed_out");
    assert!(warning.get("message").is_none(), "no text: {warning}");
    assert!(
        !events
            .all()
            .iter()
            .any(|v| v["type"] == "dictation.warning" && v["kind"] == "polish_failed"),
        "a timeout is not reported as a cancel too"
    );
}

/// A polish in flight when the core shuts down is cancelled, not timed out: `polish_failed` with
/// "cancelled", as before, and never `polish_timed_out`.
#[test]
fn a_polish_cut_short_by_shutdown_is_a_cancel_not_a_timeout() {
    let dir = TempDir::new("llm-shutdown");
    let model = Model::new(Says::Never);
    let (core, events, _platform, inbox) = polishing(&dir, &model, Duration::from_secs(60));
    take(&core, &inbox, 1);
    let until = Instant::now() + Duration::from_secs(20);
    while model.requests.lock().unwrap().is_empty() {
        assert!(Instant::now() < until, "polish never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    core.shutdown();
    let warnings: Vec<_> = events
        .all()
        .into_iter()
        .filter(|v| v["type"] == "dictation.warning")
        .collect();
    assert!(
        warnings
            .iter()
            .any(|v| v["kind"] == "polish_failed" && v["message"] == "cancelled"),
        "{warnings:?}"
    );
    assert!(
        !warnings.iter().any(|v| v["kind"] == "polish_timed_out"),
        "{warnings:?}"
    );
    events.assert_valid();
}

#[test]
fn a_model_that_never_answers_costs_a_take_its_polish_budget_not_the_generate_timeout() {
    let dir = TempDir::new("llm-hang");
    let model = Model::new(Says::Never);
    let (core, events, platform, inbox) = polishing(&dir, &model, Duration::from_millis(300));

    let started = Instant::now();
    take(&core, &inbox, 1);
    assert!(
        events.wait_count("dictation.inserted", 1, Duration::from_secs(10)),
        "the take waited on the model past its budget"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_unpolished_and_warned(&events, &platform);
    assert_eq!(
        model.cancels.load(Ordering::SeqCst),
        1,
        "the model is told the call was given up"
    );

    // The next take is processed as usual, and polished once the model answers again.
    *model.says.lock().unwrap() = Says::Polished;
    take(&core, &inbox, 2);
    assert!(events.wait_count("dictation.inserted", 2, Duration::from_secs(20)));
    assert_eq!(
        platform.inserted().last().map(|s| s.trim().to_owned()),
        Some("Polished synthetic words.".into())
    );
    core.shutdown();
    events.assert_valid();
}

/// A press while a working polish runs does not cancel it: that take keeps its polish, and the
/// pressed take is processed after it. Its audio waits in the queue meanwhile (tens of seconds of
/// room), so nothing is lost by waiting.
#[test]
fn a_press_while_polish_runs_leaves_that_take_polished_and_is_processed_after_it() {
    let dir = TempDir::new("llm-press");
    let model = Model::new(Says::Slow);
    let (core, events, platform, inbox) =
        polishing(&dir, &model, ink_pipeline::chain::POLISH_BUDGET);

    take(&core, &inbox, 1);
    let until = Instant::now() + Duration::from_secs(20);
    while model.requests.lock().unwrap().is_empty() {
        assert!(Instant::now() < until, "polish never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    // The next take, press included, while the first take's polish is still being written.
    take(&core, &inbox, 2);
    assert!(
        platform.inserted().is_empty(),
        "the second take was queued while the first was being polished"
    );
    assert!(events.wait_count("dictation.inserted", 2, Duration::from_secs(20)));
    let inserted: Vec<String> = platform
        .inserted()
        .iter()
        .map(|s| s.trim().to_owned())
        .collect();
    assert_eq!(
        inserted,
        ["Synthetic words, take 1.", "Synthetic words, take 2."]
    );
    assert_eq!(
        model.cancels.load(Ordering::SeqCst),
        0,
        "nothing was cancelled"
    );
    assert!(
        !events
            .all()
            .iter()
            .any(|v| v["type"] == "dictation.warning" && v["kind"] == "polish_failed")
    );
    core.shutdown();
    events.assert_valid();
}

/// S2.8 review item 5, after merging S2.7: dictation's polish (and voice edit, which calls the
/// same model) goes through the local-only guard. A registered model that says it is not local
/// is never called while local-only is on; the take goes in as written, and the refusal is said.
#[test]
fn dictation_polish_never_calls_a_model_that_is_not_local_while_local_only_is_on() {
    let dir = TempDir::new("llm-local-only");
    let loader = MockLoader::new(Behaviour::Say("synthetic words".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader, installer);
    let model = Model::new(Says::Polished);
    let info =
        CString::new(r#"{"id":"remote-model","licence":"MIT","model":"remote","local":false}"#)
            .unwrap();
    register(&core, &model, &info).unwrap();
    events.wait_type("engine.registered", Duration::from_secs(5));
    assert!(core.shared().local_only.is_on(), "on by default");
    let platform = Arc::new(MockPlatform::new());
    let mut settings = DictationSettings::default();
    settings.modes.modes[0].polish_enabled = true;
    // The user agreed to this provider: what refuses it here is local-only mode alone.
    settings.polish_consents = vec![LlmConsent::Cloud {
        endpoint: "shell engine remote-model".into(),
        name: "remote".into(),
    }];
    store_polish_consents(&core, &settings.polish_consents);
    let inbox = core
        .start_dictation(DictationParts {
            inserter: platform.clone(),
            focus: platform.clone(),
            llm: None,
            settings,
            vad: Vad::Unavailable(VadUnavailable::ModelMissing),
        })
        .unwrap();
    take(&core, &inbox, 1);
    assert!(events.wait_count("dictation.inserted", 1, Duration::from_secs(20)));
    let inserted = platform.inserted();
    assert!(
        inserted.last().is_some_and(|s| !s.contains("Polished")),
        "{inserted:?}"
    );
    assert!(model.requests.lock().unwrap().is_empty(), "never called");
    let warning = events
        .wait_for(Duration::from_secs(5), |v| {
            v["type"] == "dictation.warning" && v["kind"] == "polish_failed"
        })
        .expect("the refusal is said");
    assert!(
        warning["message"]
            .as_str()
            .is_some_and(|m| m.contains("local-only")),
        "{warning}"
    );
    events.assert_valid();
    core.shutdown();
}

/// The polish model picks among the registered models at each call, so a caller's consent check
/// runs on the model that call picked (`complete_if`): with only a cloud model registered, a check
/// that allows this machine alone sends nothing, and the refusal names where it would have gone.
#[test]
fn the_polish_model_checks_the_model_it_picked_before_sending() {
    let dir = TempDir::new("llm-consent-pick");
    let loader = MockLoader::new(Behaviour::Say("synthetic words".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader, installer);
    let model = Model::new(Says::Polished);
    let info =
        CString::new(r#"{"id":"cloud-model","licence":"MIT","model":"cloud","local":false}"#)
            .unwrap();
    register(&core, &model, &info).unwrap();
    events.wait_type("engine.registered", Duration::from_secs(5));
    // Local-only off, so the only thing that can refuse is the check.
    core.shared().local_only.set(false);
    let polish = PolishModel::new(core.shared().llms.clone(), core.shared().local_only.clone());
    let request = ink_core::LlmRequest {
        system: "Polish.".into(),
        user: "synthetic words".into(),
        max_tokens: 64,
        temperature: 0.0,
        json_schema: None,
    };
    let refused = polish.complete_if(&request, &ink_core::CancelToken::new(), &|i| {
        LlmConsent::OnDevice.covers(i)
    });
    assert_eq!(
        refused,
        Err(ink_core::LlmError::NotAllowed {
            refused: polish.info()
        })
    );
    assert!(model.requests.lock().unwrap().is_empty(), "nothing sent");
    let cloud = LlmConsent::for_model(&polish.info());
    let sent = polish.complete_if(&request, &ink_core::CancelToken::new(), &|i| {
        cloud.covers(i)
    });
    assert!(sent.is_ok(), "{sent:?}");
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    core.shutdown();
}
