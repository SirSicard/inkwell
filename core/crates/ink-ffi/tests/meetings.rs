//! Meetings end to end through the core's commands (S2.8): a meeting started from "devices" (a
//! capture that replays files, as the Mac's would open its mic and tap) and stopped by hand,
//! detection's offer and its answer, a recorded app that lets go of the mic, Ask about the live
//! meeting, the far end's bands, and the retention sweep with its secure delete.

mod common;

use std::ffi::{CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use ink_audio::{DEFAULT_RING_DURATION, FileReplaySource, Pacing, bands_channel};
use ink_core::mock::MockClock;
use ink_core::{
    AppRef, Channel, Clock, EventSink, MeetingDetector, MeetingSignal, NewRecord, PlatformError,
    RecordKind, Store, Transport,
};
use ink_engines::{ModelDir, Registry};
use ink_ffi::capture::{MeetingCapture, MicInfo, Opened};
use ink_ffi::external::{InkEngineVTable, KIND_LLM, Registration};
use ink_ffi::meeting::CaptureSide;
use ink_ffi::runtime::{Core, MeetingPlatform, Parts};
use ink_pipeline::meeting::watchdog::{FarDelivery, Routing};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(30);

/// A "device" capture: each meeting replays the same two files in real time, as a mic and a tap
/// would deliver. It records which app each meeting was opened for.
struct ReplayCapture {
    mic: PathBuf,
    far: PathBuf,
    clock: Arc<dyn Clock>,
    opened_for: Mutex<Vec<Option<String>>>,
}

impl MeetingCapture for ReplayCapture {
    fn open(&self, app: Option<&AppRef>, _: bool) -> Result<Opened, String> {
        self.opened_for
            .lock()
            .unwrap()
            .push(app.map(|a| a.id.clone()));
        let side = |path: &Path, channel| -> Result<CaptureSide, String> {
            let source = FileReplaySource::open(path, channel, self.clock.clone())
                .map_err(|e| e.to_string())?
                .with_pacing(Pacing::RealTime);
            Ok(CaptureSide {
                source: Box::new(source),
                ring: DEFAULT_RING_DURATION,
                start_at: None,
            })
        };
        Ok(Opened {
            sides: vec![
                side(&self.mic, Channel::Mic)?,
                side(&self.far, Channel::Far)?,
            ],
            routing: Routing {
                mic: Transport::BuiltIn,
                far: FarDelivery::WhilePlaying,
            },
            mic: Some(MicInfo {
                name: "Test Mic".into(),
                transport: Transport::BuiltIn,
                reason: "default_input",
            }),
        })
    }
}

/// A detector the test drives: it keeps the core's callback and signals through it.
#[derive(Default)]
struct FakeDetector {
    sink: Mutex<Option<EventSink<MeetingSignal>>>,
}

impl FakeDetector {
    fn signal(&self, signal: MeetingSignal) {
        let sink = self.sink.lock().unwrap().clone().expect("listening");
        sink(signal);
    }
}

impl MeetingDetector for FakeDetector {
    fn start(&self, on_signal: EventSink<MeetingSignal>) -> Result<(), PlatformError> {
        *self.sink.lock().unwrap() = Some(on_signal);
        Ok(())
    }

    fn stop(&self) {
        *self.sink.lock().unwrap() = None;
    }
}

fn app(id: &str, name: &str) -> AppRef {
    AppRef {
        id: id.into(),
        pid: None,
        name: name.into(),
    }
}

struct Rig {
    core: Core,
    events: Arc<Recorder>,
    capture: Arc<ReplayCapture>,
    detector: Arc<FakeDetector>,
    _dir: TempDir,
}

/// A core over an in-memory library whose meetings replay `seconds` of speech per side, with the
/// fake detector, on `clock`.
fn rig(label: &str, seconds: f64, clock: Arc<dyn Clock>) -> Rig {
    let dir = TempDir::new(label);
    let (mic, far) = (dir.path().join("mic.wav"), dir.path().join("far.wav"));
    speech_wav(&mic, seconds, 31);
    speech_wav(&far, seconds, 32);
    let loader = MockLoader::new(Behaviour::Say("words from the final pass".into()));
    let models = ModelDir::new(dir.path().join("models"));
    let row = test_row(ROW_ID);
    install(&models, &row);
    let capture = Arc::new(ReplayCapture {
        mic,
        far,
        clock: clock.clone(),
        opened_for: Mutex::default(),
    });
    let detector = Arc::new(FakeDetector::default());
    let parts = Parts {
        store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
        clock,
        registry: Registry::new(vec![row]).unwrap(),
        models,
        loader: loader.clone(),
        installer: Arc::new(MockInstaller {
            generation: loader.generation.clone(),
            gate: None,
            installs: AtomicUsize::new(0),
        }),
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: MeetingPlatform {
            capture: capture.clone(),
            detector: Some(detector.clone()),
        },
    };
    let (core, events) = start_parts(parts);
    Rig {
        core,
        events,
        capture,
        detector,
        _dir: dir,
    }
}

fn failed_with(events: &Recorder, id: &str) -> Value {
    events
        .wait_for(WAIT, |v| v["type"] == "command.failed" && v["id"] == id)
        .unwrap_or_else(|| panic!("command {id} did not fail: {:?}", events.types()))
}

#[test]
fn a_meeting_from_the_devices_starts_named_and_stops_by_hand_into_its_final_pass() {
    let r = rig("start-stop", 30.0, clock());
    r.events
        .wait_for(WAIT, |v| v["type"] == "meeting.detection")
        .expect("detection says whether it listens");
    r.core
        .command(r#"{"cmd":"meeting.start","title":"Weekly sync","id":"s1"}"#)
        .unwrap();
    let started = r.events.wait_type("meeting.started", WAIT);
    assert_eq!(started["title"], "Weekly sync");
    assert_eq!(started["mic_name"], "Test Mic");
    assert_eq!(started["mic_transport"], "built_in");
    assert_eq!(started["mic_reason"], "default_input");
    assert!(started.get("app").is_none(), "Record now names no app");
    assert_eq!(*r.capture.opened_for.lock().unwrap(), [None]);

    // A second start while it records is refused, and changes nothing.
    r.core
        .command(r#"{"cmd":"meeting.start","id":"s2"}"#)
        .unwrap();
    let refused = failed_with(&r.events, "s2");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("already running")
    );

    std::thread::sleep(Duration::from_millis(1_500));
    r.core
        .command(r#"{"cmd":"meeting.stop","id":"x1"}"#)
        .unwrap();
    r.events.wait_type("meeting.stopped", WAIT);
    let finished = r.events.wait_type("meeting.finished", WAIT);
    assert_eq!(finished["record"], started["record"]);
    // Stopped long before the files ended: capture ended because it was told to.
    let passes: Vec<Value> = r
        .events
        .all()
        .into_iter()
        .filter(|v| v["type"] == "meeting.transcribed")
        .collect();
    assert_eq!(passes.len(), 2);
    for p in &passes {
        let ms = p["pass"]["captured_ms"].as_u64().unwrap();
        assert!(ms > 500 && ms < 10_000, "{p}");
    }
    // Nothing to stop now.
    r.core
        .command(r#"{"cmd":"meeting.stop","id":"x2"}"#)
        .unwrap();
    let nothing = failed_with(&r.events, "x2");
    assert!(nothing["message"].as_str().unwrap().contains("no meeting"));
    // Unknown fields are refused before anything is queued.
    assert!(
        r.core
            .command(r#"{"cmd":"meeting.start","ap":"typo"}"#)
            .is_err()
    );
    assert!(r.core.command(r#"{"cmd":"meeting.dismiss"}"#).is_err());
    r.events.assert_valid();
    r.core.shutdown();
}

#[test]
fn detection_offers_an_app_the_user_answers_and_its_meeting_ends_when_it_lets_go() {
    let clock = Arc::new(MockClock::new(10_000_000_000, 1_790_146_800_000));
    let r = rig("detection", 60.0, clock.clone());
    let listening = r
        .events
        .wait_for(WAIT, |v| v["type"] == "meeting.detection")
        .unwrap();
    assert_eq!(listening["listening"], true);
    // The meetings thread reads the mock clock when a signal arrives: each signal is given time
    // to be taken before the clock moves on.
    let settle = || std::thread::sleep(Duration::from_millis(150));
    let signal = |s: MeetingSignal| {
        r.detector.signal(s);
        settle();
    };
    let poke = || {
        // Any signal makes the meetings thread judge what is pending at the mock clock's time.
        signal(MeetingSignal::MicReleased {
            app: app("nobody", "Nobody"),
        });
    };

    // An app takes the mic; it is offered once it has held it for 3 s, not before.
    signal(MeetingSignal::MicInUse {
        app: app("com.example.call", "Example Call"),
    });
    clock.advance_ns(2_000_000_000);
    poke();
    assert!(
        r.events
            .wait_for(Duration::from_millis(300), |v| v["type"]
                == "meeting.detected")
            .is_none()
    );
    clock.advance_ns(1_500_000_000);
    poke();
    let offered = r.events.wait_type("meeting.detected", WAIT);
    assert_eq!(offered["app"], "com.example.call");
    assert_eq!(offered["app_name"], "Example Call");

    // "Not this one": the offer ends, and is not made again while the app holds the mic.
    r.core
        .command(r#"{"cmd":"meeting.dismiss","app":"com.example.call"}"#)
        .unwrap();
    let ended = r.events.wait_type("meeting.detection_ended", WAIT);
    assert_eq!(ended["dismissed"], true);
    clock.advance_ns(10_000_000_000);
    poke();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(r.events.count("meeting.detected"), 1);

    // The next call: offered, and the user records it.
    signal(MeetingSignal::MicReleased {
        app: app("com.example.call", "Example Call"),
    });
    signal(MeetingSignal::MicInUse {
        app: app("com.example.call", "Example Call"),
    });
    clock.advance_ns(3_500_000_000);
    poke();
    assert!(r.events.wait_count("meeting.detected", 2, WAIT));
    r.core
        .command(r#"{"cmd":"meeting.start","app":"com.example.call"}"#)
        .unwrap();
    let started = r.events.wait_type("meeting.started", WAIT);
    assert_eq!(started["app"], "com.example.call");
    assert_eq!(started["app_name"], "Example Call");
    assert_eq!(
        *r.capture.opened_for.lock().unwrap(),
        [Some("com.example.call".to_owned())]
    );

    // The app lets go of the mic: the meeting ends 15 s later, not sooner.
    std::thread::sleep(Duration::from_millis(500));
    signal(MeetingSignal::MicReleased {
        app: app("com.example.call", "Example Call"),
    });
    clock.advance_ns(10_000_000_000);
    poke();
    assert!(
        r.events
            .wait_for(Duration::from_millis(300), |v| v["type"]
                == "meeting.stopped")
            .is_none()
    );
    clock.advance_ns(6_000_000_000);
    poke();
    r.events.wait_type("meeting.stopped", WAIT);
    r.events.wait_type("meeting.finished", WAIT);
    r.events.assert_valid();
    r.core.shutdown();
}

#[test]
fn the_detection_setting_starts_and_stops_listening() {
    let r = rig("detect-setting", 5.0, clock());
    r.events
        .wait_for(WAIT, |v| {
            v["type"] == "meeting.detection" && v["listening"] == true
        })
        .unwrap();
    r.core
        .command(r#"{"cmd":"setting.set","key":"meetings.detect","value":"off"}"#)
        .unwrap();
    r.events
        .wait_for(WAIT, |v| {
            v["type"] == "meeting.detection" && v["listening"] == false
        })
        .expect("off");
    assert!(
        r.detector.sink.lock().unwrap().is_none(),
        "the detector stopped"
    );
    r.core
        .command(r#"{"cmd":"setting.set","key":"meetings.detect","value":"on"}"#)
        .unwrap();
    assert!(r.events.wait_count("meeting.detection", 3, WAIT));
    assert!(r.detector.sink.lock().unwrap().is_some());
    r.core.shutdown();
}

// --- Ask -------------------------------------------------------------------------------------

struct Model {
    requests: Mutex<Vec<Value>>,
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
    std::thread::spawn(move || {
        let answer = CString::new(r#"{"text":"  They asked about the budget.  "}"#).unwrap();
        // SAFETY: a NUL-terminated string valid for the call.
        unsafe { ink_ffi::ink_engine_complete(call, answer.as_ptr()) };
    });
}

unsafe extern "C" fn release(_: *mut c_void) {}

#[test]
fn ask_answers_about_the_live_meeting_with_the_registered_model() {
    let r = rig("ask", 30.0, clock());
    // No meeting: said so.
    r.core
        .command(r#"{"cmd":"meeting.ask","question":"What was decided?","id":"q0"}"#)
        .unwrap();
    assert!(
        failed_with(&r.events, "q0")["message"]
            .as_str()
            .unwrap()
            .contains("no meeting")
    );
    r.core.command(r#"{"cmd":"meeting.start"}"#).unwrap();
    let started = r.events.wait_type("meeting.started", WAIT);
    // No model registered: said so, never made up.
    r.core
        .command(r#"{"cmd":"meeting.ask","question":"What was decided?","id":"q1"}"#)
        .unwrap();
    assert!(
        failed_with(&r.events, "q1")["message"]
            .as_str()
            .unwrap()
            .contains("no language model")
    );
    let model = Box::leak(Box::new(Model {
        requests: Mutex::default(),
    }));
    let info = CString::new(
        r#"{"id":"test-llm","licence":"MIT","model":"t","local":true,"context_tokens":4096}"#,
    )
    .unwrap();
    let table = InkEngineVTable {
        kind: KIND_LLM,
        info_json: info.as_ptr(),
        ctx: model as *const Model as *mut c_void,
        release: Some(release),
        generate: Some(generate),
        ..Default::default()
    };
    // SAFETY: a valid table whose ctx outlives the core.
    let registration =
        unsafe { Registration::from_table(&table, r.core.shared().shutdown.clone()) }.unwrap();
    r.core.register(registration).unwrap();
    // A live final exists once the live engine... there is none in this rig: the question is
    // answered from what the record holds so far (nothing yet), without a call.
    r.core
        .command(r#"{"cmd":"meeting.ask","question":"What was decided?","id":"q2"}"#)
        .unwrap();
    let answered = r
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "meeting.answered" && v["ref"] == "q2"
        })
        .unwrap();
    assert_eq!(answered["record"], started["record"]);
    assert_eq!(answered["text"], "Nothing has been said yet.");
    assert!(model.requests.lock().unwrap().is_empty());
    // With words in the record, the model is asked and its answer is trimmed.
    let record = ink_core::RecordId(started["record"].as_str().unwrap().to_owned());
    r.core
        .shared()
        .store
        .append_segments(
            &record,
            &[ink_core::Segment {
                channel: Channel::Far,
                start_ms: 1_000,
                end_ms: 2_000,
                text: "What about the budget for the pilot?".into(),
                speaker: None,
            }],
        )
        .unwrap();
    r.core
        .command(r#"{"cmd":"meeting.ask","question":"What did they ask?","id":"q3"}"#)
        .unwrap();
    let answered = r
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "meeting.answered" && v["ref"] == "q3"
        })
        .unwrap();
    assert_eq!(answered["text"], "They asked about the budget.");
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]["user"]
            .as_str()
            .unwrap()
            .contains("What did they ask?")
    );
    assert!(requests[0].get("json_schema").is_none(), "plain text");
    drop(requests);
    r.events.assert_valid();
    r.core.shutdown();
}

// --- Bands -----------------------------------------------------------------------------------

#[test]
fn the_far_end_has_bands_of_its_own_during_a_meeting() {
    let r = rig("far-bands", 30.0, clock());
    let (writer, reader) = bands_channel();
    r.core.lend_far_bands(writer);
    r.core.command(r#"{"cmd":"meeting.start"}"#).unwrap();
    r.events.wait_type("meeting.started", WAIT);
    let until = std::time::Instant::now() + WAIT;
    let mut loud = false;
    while std::time::Instant::now() < until && !loud {
        let s = reader.read();
        loud = s.published > 0 && s.bands.mid > 0.0;
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(loud, "the far end's bands moved");
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    r.events.wait_type("meeting.finished", WAIT);
    let last = reader.read();
    assert_eq!(
        last.bands,
        ink_audio::Bands::default(),
        "idle is a still frame"
    );
    r.core.shutdown();
}

// --- Retention -------------------------------------------------------------------------------

/// S2.8's retention: records older than the setting keeps go whole, their audio too, and their
/// words are gone from the library's files (the store checkpoints its log with TRUNCATE after the
/// delete). Newer records, and one without an end, stay.
#[test]
fn retention_deletes_old_records_whole_and_leaves_no_trace_of_their_words() {
    const DAY: i64 = 86_400_000;
    let dir = TempDir::new("retention");
    let path = dir.path().join("library.sqlite");
    let store = Arc::new(ink_store::SqliteStore::open(&path).unwrap());
    let clock = clock();
    let now = clock.unix_ms();
    let record_of = |kind, days_ago: i64, word: &str, ended: bool, audio: Option<&str>| {
        let id = store
            .create_record(NewRecord {
                kind,
                title: Some(format!("{word} title")),
                started_at_unix_ms: now - days_ago * DAY,
                source_app: None,
                audio_dir: audio.map(Into::into),
            })
            .unwrap();
        store
            .append_segments(
                &id,
                &[ink_core::Segment {
                    channel: Channel::Mic,
                    start_ms: 0,
                    end_ms: 1_000,
                    text: format!("the {word} was said here"),
                    speaker: None,
                }],
            )
            .unwrap();
        if ended {
            store
                .finish_record(&id, now - days_ago * DAY + 60_000)
                .unwrap();
        }
        if let Some(audio) = audio {
            std::fs::create_dir_all(dir.path().join(audio)).unwrap();
            std::fs::write(
                dir.path().join(audio).join("mic-000000-16000x1.pcm"),
                [0u8; 64],
            )
            .unwrap();
        }
        id
    };
    let record = |days_ago, word: &str, ended, audio| {
        record_of(RecordKind::Meeting, days_ago, word, ended, audio)
    };
    let old = record(40, "zebrafinch", true, Some("meetings/old"));
    let old_live = record(50, "quokkabird", false, None);
    let recent = record(3, "marmosetfox", true, Some("meetings/recent"));
    let old_dictation = record_of(RecordKind::Dictation, 45, "okapiwren", true, None);
    // Review (S2.8): an import is the user's own file, perhaps its only copy: never swept.
    let old_import = record_of(
        RecordKind::FileImport,
        60,
        "narwhalbee",
        true,
        Some("imports/old"),
    );
    let parts = Parts {
        store: store.clone(),
        clock,
        registry: Registry::new(Vec::new()).unwrap(),
        models: ModelDir::new(dir.path().join("models")),
        loader: MockLoader::new(Behaviour::Say("x".into())),
        installer: Arc::new(MockInstaller {
            generation: Arc::default(),
            gate: None,
            installs: AtomicUsize::new(0),
        }),
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: MeetingPlatform::default(),
    };
    let (core, events) = start_parts(parts);
    // Forever by default: the launch's sweep deleted nothing.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(events.count("library.swept"), 0);
    core.command(r#"{"cmd":"setting.set","key":"retention.days","value":"30"}"#)
        .unwrap();
    let swept = events.wait_type("library.swept", WAIT);
    assert_eq!(swept["deleted"], 2, "the old meeting and the old dictation");
    assert_eq!(swept["failed"], 0);
    assert_eq!(store.record(&old).unwrap(), None);
    assert_eq!(store.record(&old_dictation).unwrap(), None);
    assert!(
        store.record(&old_import).unwrap().is_some(),
        "an import is never swept"
    );
    assert!(dir.path().join("imports/old").exists(), "nor its file");
    assert!(
        store.record(&old_live).unwrap().is_some(),
        "no end: never swept"
    );
    assert!(store.record(&recent).unwrap().is_some());
    assert!(
        !dir.path().join("meetings/old").exists(),
        "its audio went too"
    );
    assert!(dir.path().join("meetings/recent").exists());
    core.shutdown();
    drop(store);
    // No copy of the deleted words anywhere in the library's files.
    for file in ["library.sqlite", "library.sqlite-wal"] {
        if let Ok(bytes) = std::fs::read(dir.path().join(file)) {
            let found = bytes.windows(10).any(|w| w == b"zebrafinch");
            assert!(!found, "{file} still holds the deleted words");
        }
    }
    let bytes = std::fs::read(dir.path().join("library.sqlite")).unwrap();
    assert!(
        bytes.windows(11).any(|w| w == b"marmosetfox"),
        "a control: kept words are there"
    );
    // Values outside the whitelist are refused.
    let (core, _events) = start_parts(Parts {
        store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
        clock: clock_fn(),
        registry: Registry::new(Vec::new()).unwrap(),
        models: ModelDir::new(dir.path().join("models")),
        loader: MockLoader::new(Behaviour::Say("x".into())),
        installer: Arc::new(MockInstaller {
            generation: Arc::default(),
            gate: None,
            installs: AtomicUsize::new(0),
        }),
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: MeetingPlatform::default(),
    });
    assert!(
        core.command(r#"{"cmd":"setting.set","key":"retention.days","value":"1"}"#)
            .is_err()
    );
    core.shutdown();
}

fn clock_fn() -> Arc<dyn Clock> {
    clock()
}

// --- The meeting's language model ------------------------------------------------------------

struct Summarizer {
    systems: Mutex<Vec<String>>,
}

unsafe extern "C" fn summarize(ctx: *mut c_void, call: u64, request: *const c_char) {
    // SAFETY: ctx is the test's model, alive past its release; the request is valid for the call.
    let (me, request) = unsafe {
        (
            &*(ctx as *const Summarizer),
            std::ffi::CStr::from_ptr(request)
                .to_str()
                .unwrap()
                .to_owned(),
        )
    };
    let request: Value = serde_json::from_str(&request).unwrap();
    let system = request["system"].as_str().unwrap().to_owned();
    let answer = if system.contains("meeting record") {
        assert!(
            request["json_schema"].is_string(),
            "a summary asks for its shape"
        );
        r#"{"headline":"The report goes out on Friday.","body":"Status.","decisions":[],"actions":[]}"#
    } else {
        r#"{"class":"hypothetical","confidence":0.9,"task":null,"due":null,"quote":"words"}"#
    };
    me.systems.lock().unwrap().push(system);
    let answer = CString::new(serde_json::json!({ "text": answer }).to_string()).unwrap();
    std::thread::spawn(move || {
        // SAFETY: a NUL-terminated string valid for the call.
        unsafe { ink_ffi::ink_engine_complete(call, answer.as_ptr()) };
    });
}

/// S2.8, carried item 1: a meeting's final pass writes its summary with the language model the
/// shell registered (Foundation Models on the Mac, a script here), and an untitled meeting takes
/// the headline as its title.
#[test]
fn a_meeting_is_summarized_by_the_registered_model_and_titled_by_its_headline() {
    let dir = TempDir::new("summary");
    let loader = MockLoader::new(Behaviour::Say("I'll send the report on Friday".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader, installer);
    let model = Box::leak(Box::new(Summarizer {
        systems: Mutex::default(),
    }));
    let info = CString::new(
        r#"{"id":"test-llm","licence":"MIT","model":"t","local":true,"context_tokens":4096}"#,
    )
    .unwrap();
    let table = InkEngineVTable {
        kind: KIND_LLM,
        info_json: info.as_ptr(),
        ctx: model as *const Summarizer as *mut c_void,
        release: Some(release),
        generate: Some(summarize),
        ..Default::default()
    };
    // SAFETY: a valid table whose ctx outlives the core.
    let registration =
        unsafe { Registration::from_table(&table, core.shared().shutdown.clone()) }.unwrap();
    core.register(registration).unwrap();
    let (mic, far) = (dir.path().join("mic.wav"), dir.path().join("far.wav"));
    speech_wav(&mic, 3.0, 51);
    speech_wav(&far, 2.0, 52);
    core.command(&format!(
        r#"{{"cmd":"replay_meeting","mic":{:?},"far":{:?},"pacing":"fast"}}"#,
        mic.to_str().unwrap(),
        far.to_str().unwrap()
    ))
    .unwrap();
    let summarized = events.wait_type("meeting.summarized", WAIT);
    let finished = events.wait_type("meeting.finished", WAIT);
    assert_eq!(summarized["record"], finished["record"]);
    let record = ink_core::RecordId(finished["record"].as_str().unwrap().to_owned());
    let store = &core.shared().store;
    assert_eq!(
        store.record(&record).unwrap().unwrap().title.as_deref(),
        Some("The report goes out on Friday.")
    );
    assert!(
        store
            .summary(&record)
            .unwrap()
            .unwrap()
            .text
            .starts_with("The report goes out on Friday.")
    );
    let systems = model.systems.lock().unwrap();
    assert!(systems.iter().any(|s| s.contains("meeting record")));
    assert!(
        systems.iter().any(|s| s.contains("classify ONE sentence")),
        "the promise was judged"
    );
    drop(systems);
    events.assert_valid();
    core.shutdown();
}

// --- Retention after a final pass, off the meeting's worker ------------------------------------

/// Review (S2.8): a start that answers `meeting.finished` is never refused. The retention sweep
/// that follows a final pass runs on its own thread, so a slow one (a thousand old records to
/// delete) never keeps the last meeting "running" (it did, on the meeting's worker).
#[test]
fn a_meeting_can_start_the_moment_the_last_one_finished() {
    const DAY: i64 = 86_400_000;
    let r = rig("back-to-back", 30.0, clock());
    let store = r.core.shared().store.clone();
    let now = r.core.shared().clock.unix_ms();
    for i in 0..1_000 {
        let started = now - 40 * DAY - i;
        let id = store
            .create_record(NewRecord {
                kind: RecordKind::Dictation,
                title: None,
                started_at_unix_ms: started,
                source_app: None,
                audio_dir: None,
            })
            .unwrap();
        store
            .append_segments(
                &id,
                &[ink_core::Segment {
                    channel: Channel::Mic,
                    start_ms: 0,
                    end_ms: 1_000,
                    text: format!("an old dictation {i}"),
                    speaker: None,
                }],
            )
            .unwrap();
        store.finish_record(&id, started + 1_000).unwrap();
    }
    // Set in the library, not by command: nothing is swept until the meeting's pass is over.
    store.set_setting("retention.days", "30").unwrap();

    r.core
        .command(r#"{"cmd":"meeting.start","id":"first"}"#)
        .unwrap();
    r.events.wait_type("meeting.started", WAIT);
    std::thread::sleep(Duration::from_millis(1_000));
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    let first = r.events.wait_type("meeting.finished", WAIT);
    r.core
        .command(r#"{"cmd":"meeting.start","id":"second"}"#)
        .unwrap();
    let second = r.events.wait_for(Duration::from_secs(10), |v| {
        (v["type"] == "meeting.started" && v["record"] != first["record"])
            || (v["type"] == "command.failed" && v["id"] == "second")
    });
    assert_eq!(
        second.as_ref().map(|v| v["type"].clone()),
        Some("meeting.started".into()),
        "{second:?}"
    );
    let swept = r.events.wait_type("library.swept", WAIT);
    assert_eq!(swept["deleted"], 1_000);
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    assert!(r.events.wait_count("meeting.finished", 2, WAIT));
    r.events.assert_valid();
    r.core.shutdown();
}

/// Review (S2.8): a meeting recovered after a crash is followed by a retention sweep, as a live
/// meeting's final pass is. Here the recovered meeting itself is past the setting's 30 days: it
/// had no end until recovery gave it one, so the launch's sweep left it alone, and the sweep after
/// its pass takes it.
#[test]
fn a_recovered_meeting_is_followed_by_a_retention_sweep() {
    use ink_core::{AudioBlock, StreamFormat};

    const DAY: i64 = 86_400_000;
    let dir = TempDir::new("recover-sweep");
    let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    let clock = clock();
    let now = clock.unix_ms();
    let record = store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: Some("Interrupted".into()),
            started_at_unix_ms: now - 40 * DAY,
            source_app: None,
            audio_dir: Some("meetings/crashed".into()),
        })
        .unwrap();
    store.set_setting("retention.days", "30").unwrap();
    // What a crash leaves: three seconds a side on disk, and the marker.
    let audio = dir.path().join("meetings/crashed");
    let chunks = ink_audio::ChunkStore::open(&audio).unwrap();
    for (channel, seed) in [(Channel::Mic, 61), (Channel::Far, 62)] {
        let samples = ink_audio::synth::speech_like(3.0, -30.0, seed);
        let mut writer = chunks.writer(channel, StreamFormat::CANONICAL).unwrap();
        writer
            .write(
                &AudioBlock {
                    samples: &samples,
                    format: StreamFormat::CANONICAL,
                    host_time_ns: 1_000_000_000,
                },
                0,
            )
            .unwrap();
        writer.finish().unwrap();
    }
    ink_ffi::recovery::mark_live(&audio, &record).unwrap();

    let loader = MockLoader::new(Behaviour::Say("words from the final pass".into()));
    let models = ModelDir::new(dir.path().join("models"));
    let row = test_row(ROW_ID);
    install(&models, &row);
    let (core, events) = start_parts(Parts {
        store: store.clone(),
        clock,
        registry: Registry::new(vec![row]).unwrap(),
        models,
        loader: loader.clone(),
        installer: Arc::new(MockInstaller {
            generation: loader.generation.clone(),
            gate: None,
            installs: AtomicUsize::new(0),
        }),
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: MeetingPlatform::default(),
    });
    core.command(r#"{"cmd":"meetings.recover"}"#).unwrap();
    let finished = events.wait_type("meeting.finished", WAIT);
    assert_eq!(finished["record"], record.0.as_str());
    let swept = events.wait_type("library.swept", WAIT);
    assert_eq!(swept["deleted"], 1);
    assert_eq!(store.record(&record).unwrap(), None);
    assert!(!audio.exists(), "its audio went too");
    events.assert_valid();
    core.shutdown();
}
