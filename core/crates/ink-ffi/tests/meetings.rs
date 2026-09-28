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
use ink_ffi::capture::{FarScope, MeetingCapture, MicInfo, Opened};
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
    /// An app's own sound cannot be tapped: everything this "Mac" plays is recorded instead.
    tap_fails: std::sync::atomic::AtomicBool,
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
            far: match app {
                None => FarScope::Everything,
                Some(_) if self.tap_fails.load(std::sync::atomic::Ordering::Relaxed) => {
                    FarScope::EverythingInstead("no audio process for that app".into())
                }
                Some(_) => FarScope::App,
            },
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
    let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    rig_with(label, seconds, clock, store)
}

/// [`rig`] over `store`.
fn rig_with(label: &str, seconds: f64, clock: Arc<dyn Clock>, store: Arc<dyn Store>) -> Rig {
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
        tap_fails: Default::default(),
    });
    let detector = Arc::new(FakeDetector::default());
    let parts = Parts {
        store,
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
    assert_eq!(
        started["far_end"], "everything",
        "and records all this Mac plays"
    );
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
    assert_eq!(started["far_end"], "app", "the call's own sound alone");
    assert_eq!(r.events.count("meeting.far_end_fallback"), 0);
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

/// Review (S2.8): the shell hears from the start whether the core listens, whatever the setting
/// says. Off by the setting is announced (it used to be silent, and the shell showed nothing).
#[test]
fn detection_announces_its_state_at_start_even_when_off() {
    let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    store.set_setting("meetings.detect", "off").unwrap();
    let r = rig_with("detect-off", 5.0, clock(), store);
    let said = r
        .events
        .wait_for(WAIT, |v| v["type"] == "meeting.detection")
        .expect("announced");
    assert_eq!(said["listening"], false);
    assert!(said.get("message").is_none(), "{said}");
    assert!(r.detector.sink.lock().unwrap().is_none());
    r.events.assert_valid();
    r.core.shutdown();
}

/// Review (S2.8): a detection setting the core cannot read leaves detection off, and the shell
/// hears so, with why (it was only logged, while the shell could show "Listening"). Turning it on
/// once the library answers starts it.
#[test]
fn a_detection_setting_the_core_cannot_read_is_announced_as_not_listening() {
    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    store.fail(&["setting"]);
    let r = rig_with("detect-unread", 5.0, clock(), store.clone());
    let said = r
        .events
        .wait_for(WAIT, |v| v["type"] == "meeting.detection")
        .expect("announced");
    assert_eq!(said["listening"], false);
    assert!(
        said["message"]
            .as_str()
            .unwrap()
            .starts_with("couldn't read the detection setting"),
        "{said}"
    );
    store.heal();
    r.core
        .command(r#"{"cmd":"setting.set","key":"meetings.detect","value":"on"}"#)
        .unwrap();
    r.events
        .wait_for(WAIT, |v| {
            v["type"] == "meeting.detection" && v["listening"] == true
        })
        .expect("on");
    r.events.assert_valid();
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

/// What a crash leaves in `dir`: a meeting record that started `days_ago` and never ended, three
/// seconds a side on disk, and the marker.
fn interrupted_meeting(
    dir: &Path,
    store: &dyn Store,
    clock: &dyn Clock,
    days_ago: i64,
) -> (ink_core::RecordId, PathBuf) {
    use ink_core::{AudioBlock, StreamFormat};

    const DAY: i64 = 86_400_000;
    let record = store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: Some("Interrupted".into()),
            started_at_unix_ms: clock.unix_ms() - days_ago * DAY,
            source_app: None,
            audio_dir: Some("meetings/crashed".into()),
        })
        .unwrap();
    let audio = dir.join("meetings/crashed");
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
    (record, audio)
}

/// A core over `store` in `dir`, with a final-pass engine and no meeting devices.
fn recovery_core(
    dir: &Path,
    store: Arc<dyn Store>,
    clock: Arc<dyn Clock>,
) -> (Core, Arc<Recorder>) {
    let loader = MockLoader::new(Behaviour::Say("words from the final pass".into()));
    let models = ModelDir::new(dir.join("models"));
    let row = test_row(ROW_ID);
    install(&models, &row);
    start_parts(Parts {
        store,
        clock,
        registry: Registry::new(vec![row]).unwrap(),
        models,
        loader: loader.clone(),
        installer: Arc::new(MockInstaller {
            generation: loader.generation.clone(),
            gate: None,
            installs: AtomicUsize::new(0),
        }),
        data_dir: dir.to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: MeetingPlatform::default(),
    })
}

/// Review (S2.8): a meeting recovered after a crash is followed by a retention sweep, as a live
/// meeting's final pass is. Here the recovered meeting itself is past the setting's 30 days: it
/// had no end until recovery gave it one, so the launch's sweep left it alone, and the sweep after
/// its pass takes it.
#[test]
fn a_recovered_meeting_is_followed_by_a_retention_sweep() {
    let dir = TempDir::new("recover-sweep");
    let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    let clock = clock();
    let (record, audio) = interrupted_meeting(dir.path(), store.as_ref(), clock.as_ref(), 40);
    store.set_setting("retention.days", "30").unwrap();
    let (core, events) = recovery_core(dir.path(), store.clone(), clock);
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

/// PR #86 (Windows CI): a sweep that runs after recovery has marked the meeting ended but before its
/// pass is done (the launch's sweep, running late) leaves it alone; the recovered meeting finishes,
/// and only the sweep after its pass takes it. Made deterministic here: the sweep runs on the
/// recovery's own thread, right after the store marks the record ended.
#[test]
fn a_sweep_during_recovery_never_takes_the_meeting_being_recovered() {
    let dir = TempDir::new("recover-sweep-first");
    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    let clock = clock();
    let (record, audio) = interrupted_meeting(dir.path(), store.as_ref(), clock.as_ref(), 40);
    store.set_setting("retention.days", "30").unwrap();
    let (core, events) = recovery_core(dir.path(), store.clone(), clock);
    let shared = core.shared().clone();
    let in_the_window = Arc::new(Mutex::new(None));
    {
        let (store, record, in_the_window) = (store.clone(), record.clone(), in_the_window.clone());
        store.clone().after("finish_record", move || {
            let ended = store.record(&record).unwrap().unwrap().ended_at_unix_ms;
            assert!(
                ended.is_some(),
                "the record reads as ended: the window is open"
            );
            *in_the_window.lock().unwrap() = Some(ink_ffi::retention::sweep(&shared));
        });
    }
    core.command(r#"{"cmd":"meetings.recover"}"#).unwrap();
    let finished = events.wait_type("meeting.finished", WAIT);
    assert_eq!(finished["record"], record.0.as_str());
    let swept_then = in_the_window
        .lock()
        .unwrap()
        .take()
        .expect("the sweep ran in the window")
        .expect("the setting keeps 30 days");
    assert_eq!(swept_then.deleted, 0, "kept while its pass ran");
    assert_eq!(events.count("meeting.failed"), 0, "{:?}", events.types());
    // Then the sweep after its pass takes it.
    let swept = events
        .wait_for(WAIT, |v| v["type"] == "library.swept" && v["deleted"] == 1)
        .expect("swept after its pass");
    assert_eq!(swept["failed"], 0);
    assert_eq!(store.record(&record).unwrap(), None);
    assert!(!audio.exists(), "its audio went too");
    events.assert_valid();
    core.shutdown();
}

/// Review of 7ab19de: recovery's early end (no audio can be placed: the record is ended and the
/// live transcript stands, with no pass) is held from retention too, from before it marks the
/// record ended; once recovery is over the hold is gone and a sweep takes the record.
#[test]
fn recovery_holds_a_meeting_from_retention_on_its_early_end_too() {
    let dir = TempDir::new("recover-early-end");
    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    let clock = clock();
    let record = store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: Some("Interrupted".into()),
            started_at_unix_ms: clock.unix_ms() - 40 * 86_400_000,
            source_app: None,
            audio_dir: Some("meetings/no-audio".into()),
        })
        .unwrap();
    // A marker and no chunks (and no timeline): nothing to place, so no pass.
    let audio = dir.path().join("meetings/no-audio");
    std::fs::create_dir_all(&audio).unwrap();
    ink_ffi::recovery::mark_live(&audio, &record).unwrap();
    store.set_setting("retention.days", "30").unwrap();
    let (core, events) = recovery_core(dir.path(), store.clone(), clock);
    let shared = core.shared().clone();
    let in_the_window = Arc::new(Mutex::new(None));
    {
        let (record, in_the_window, shared) =
            (record.clone(), in_the_window.clone(), shared.clone());
        store.after("finish_record", move || {
            let held = shared.held_from_sweep(&record);
            *in_the_window.lock().unwrap() = Some((held, ink_ffi::retention::sweep(&shared)));
        });
    }
    core.command(r#"{"cmd":"meetings.recover"}"#).unwrap();
    let failed = events.wait_type("meeting.failed", WAIT);
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.contains("no recorded audio could be placed")),
        "{failed}"
    );
    events.wait_type("meetings.recovered", WAIT);
    let (held, swept) = in_the_window
        .lock()
        .unwrap()
        .take()
        .expect("ended in the window");
    assert!(held, "held before the record was marked ended");
    assert_eq!(swept.expect("30 days").deleted, 0, "kept while held");
    assert!(
        !shared.held_from_sweep(&record),
        "let go once recovery was over"
    );
    assert!(!audio.join(ink_ffi::recovery::LIVE_FILE).exists());
    let swept = ink_ffi::retention::sweep(&shared).expect("30 days");
    assert_eq!(swept.deleted, 1, "then swept");
    assert_eq!(store.record(&record).unwrap(), None);
    events.assert_valid();
    core.shutdown();
}

/// A meeting whose record reads as ended but whose crash marker is still there (a pass cancelled
/// by the app quitting, after the record was ended) is kept until recovery has finished it.
#[test]
fn an_ended_meeting_with_its_crash_marker_is_never_swept() {
    let dir = TempDir::new("marker-kept");
    let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    let clock = clock();
    let (record, audio) = interrupted_meeting(dir.path(), store.as_ref(), clock.as_ref(), 40);
    store
        .finish_record(&record, clock.unix_ms() - 40 * 86_400_000 + 3_000)
        .unwrap();
    store.set_setting("retention.days", "30").unwrap();
    let (core, events) = recovery_core(dir.path(), store.clone(), clock);
    let swept = ink_ffi::retention::sweep(core.shared()).expect("30 days");
    assert_eq!(swept.deleted, 0);
    assert!(store.record(&record).unwrap().is_some());
    assert!(audio.join(ink_ffi::recovery::LIVE_FILE).is_file());
    events.assert_valid();
    core.shutdown();
}

/// Review (S2.8): when the store will not mark a recovered meeting ended, its pass still runs, but
/// the marker stays (the record still reads as live, and is never swept): the next launch tries
/// again, and once the record is ended, the marker goes.
#[test]
fn a_recovered_meeting_the_store_could_not_end_keeps_its_marker() {
    let dir = TempDir::new("recover-unended");
    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    let clock = clock();
    let (record, audio) = interrupted_meeting(dir.path(), store.as_ref(), clock.as_ref(), 0);
    let marker = audio.join(ink_ffi::recovery::LIVE_FILE);
    store.fail(&["finish_record"]);
    let (core, events) = recovery_core(dir.path(), store.clone(), clock);
    core.command(r#"{"cmd":"meetings.recover"}"#).unwrap();
    let finished = events.wait_type("meeting.finished", WAIT);
    assert_eq!(finished["record"], record.0.as_str());
    events.wait_type("meetings.recovered", WAIT);
    assert!(marker.is_file(), "kept for the next launch");
    assert_eq!(
        store.record(&record).unwrap().unwrap().ended_at_unix_ms,
        None
    );

    // The next try, with a store that works.
    store.heal();
    core.command(r#"{"cmd":"meetings.recover"}"#).unwrap();
    assert!(events.wait_count("meetings.recovered", 2, WAIT));
    assert!(!marker.exists(), "ended, so done");
    assert!(
        store
            .record(&record)
            .unwrap()
            .unwrap()
            .ended_at_unix_ms
            .is_some()
    );
    events.assert_valid();
    core.shutdown();
}

/// Review (S2.8): when the meetings cannot even be looked for, recovery says so once
/// (`meetings.recovered` with a message), instead of finding nothing without a word.
#[test]
fn recovery_says_when_it_could_not_look_for_interrupted_meetings() {
    let dir = TempDir::new("recover-unlisted");
    std::fs::write(dir.path().join("meetings"), "not a directory").unwrap();
    let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    let (core, events) = recovery_core(dir.path(), store, clock());
    core.command(r#"{"cmd":"meetings.recover"}"#).unwrap();
    let done = events.wait_type("meetings.recovered", WAIT);
    assert_eq!(done["meetings"], 0);
    assert!(
        done["message"]
            .as_str()
            .unwrap()
            .starts_with("couldn't look for meetings a crash interrupted")
    );
    events.assert_valid();
    core.shutdown();
}

// --- Local-only mode --------------------------------------------------------------------------

/// Review (S2.8): local-only mode is enforced in code, not left to the shell. The setting
/// (`llm.local_only`) is on unless turned off; while it is on, a registered model that says it is
/// not local is never called, for Ask or for a meeting's summary, and the refusal is said. Turned
/// off, the same model answers.
#[test]
fn local_only_refuses_a_model_that_is_not_local_for_ask_and_the_summary() {
    let r = rig("local-only", 30.0, clock());
    let model = Box::leak(Box::new(Model {
        requests: Mutex::default(),
    }));
    let info =
        CString::new(r#"{"id":"remote-llm","licence":"MIT","model":"t","local":false}"#).unwrap();
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
    r.core.command(r#"{"cmd":"meeting.start"}"#).unwrap();
    let started = r.events.wait_type("meeting.started", WAIT);
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

    // On by default: refused, and said.
    r.core
        .command(r#"{"cmd":"meeting.ask","question":"What did they ask?","id":"q1"}"#)
        .unwrap();
    let refused = failed_with(&r.events, "q1");
    assert!(
        refused["message"].as_str().unwrap().contains("local-only"),
        "{refused}"
    );
    assert!(model.requests.lock().unwrap().is_empty(), "never called");

    // Off: the same model answers.
    r.core
        .command(r#"{"cmd":"setting.set","key":"llm.local_only","value":"off"}"#)
        .unwrap();
    r.events
        .wait_for(WAIT, |v| {
            v["type"] == "setting.value" && v["key"] == "llm.local_only"
        })
        .unwrap();
    r.core
        .command(r#"{"cmd":"meeting.ask","question":"What did they ask?","id":"q2"}"#)
        .unwrap();
    r.events
        .wait_for(WAIT, |v| {
            v["type"] == "meeting.answered" && v["ref"] == "q2"
        })
        .expect("answered with local-only off");
    let asked = model.requests.lock().unwrap().len();
    assert_eq!(asked, 1);

    // On again: the meeting's summary is refused too, and said.
    r.core
        .command(r#"{"cmd":"setting.set","key":"llm.local_only","value":"on"}"#)
        .unwrap();
    assert!(r.events.wait_count("setting.value", 2, WAIT));
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    r.events.wait_type("meeting.finished", WAIT);
    let warning = r
        .events
        .wait_for(Duration::ZERO, |v| {
            v["type"] == "meeting.warning" && v["kind"] == "summary_failed"
        })
        .expect("the summary was refused");
    assert!(
        warning["message"].as_str().unwrap().contains("local-only"),
        "{warning}"
    );
    assert_eq!(model.requests.lock().unwrap().len(), asked, "never called");
    r.events.assert_valid();
    r.core.shutdown();
}

// --- The far end, honestly -------------------------------------------------------------------

/// Review (S2.8, security): a call whose app cannot be recorded alone records everything this Mac
/// plays instead; the shell is told at once (`meeting.far_end_fallback`, right after
/// `meeting.started`), never only the log.
#[test]
fn a_call_that_cannot_be_heard_alone_says_it_records_everything_this_mac_plays() {
    let r = rig("far-fallback", 30.0, clock());
    r.capture
        .tap_fails
        .store(true, std::sync::atomic::Ordering::Relaxed);
    r.core
        .command(r#"{"cmd":"meeting.start","app":"com.example.call"}"#)
        .unwrap();
    let started = r.events.wait_type("meeting.started", WAIT);
    assert_eq!(started["far_end"], "everything");
    let fallback = r.events.wait_type("meeting.far_end_fallback", WAIT);
    assert_eq!(fallback["record"], started["record"]);
    assert_eq!(fallback["app"], "com.example.call");
    assert_eq!(
        fallback["app_name"], "com.example.call",
        "no offer: named by its id"
    );
    assert_eq!(fallback["message"], "no audio process for that app");
    assert!(
        r.events.seq_of("meeting.far_end_fallback") > r.events.seq_of("meeting.started"),
        "said right after the start"
    );
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    r.events.wait_type("meeting.finished", WAIT);
    assert_eq!(r.events.count("meeting.far_end_fallback"), 1, "once");
    r.events.assert_valid();
    r.core.shutdown();
}
