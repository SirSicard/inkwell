//! Meetings end to end through the core's commands (S2.8): a meeting started from "devices" (a
//! capture that replays files, as the Mac's would open its mic and tap) and stopped by hand,
//! detection's offer and its answer, a recorded app that lets go of the mic, Ask about the live
//! meeting, the far end's bands, and the retention sweep with its secure delete.

mod common;

use std::ffi::{CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize};
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
use ink_ffi::devices::Choices;
use ink_ffi::external::{InkEngineVTable, KIND_LLM, Registration};
use ink_ffi::meeting::CaptureSide;
use ink_ffi::runtime::{Core, MeetingPlatform, Parts};
use ink_pipeline::meeting::watchdog::{FarDelivery, Routing};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(30);

/// A "device" capture: each meeting replays the same two files in real time, as a mic and a tap
/// would deliver. It records which app each meeting was opened for, and counts the frames each
/// side of the latest meeting has handed to capture.
struct ReplayCapture {
    mic: PathBuf,
    far: PathBuf,
    clock: Arc<dyn Clock>,
    opened_for: Mutex<Vec<Option<String>>>,
    /// An app's own sound cannot be tapped: everything this "Mac" plays is recorded instead.
    tap_fails: std::sync::atomic::AtomicBool,
    /// An app is heard by loopback of its output device, as Windows hears most apps.
    device_loopback: std::sync::atomic::AtomicBool,
    /// Frames delivered to capture by the latest meeting's mic and far end.
    delivered: [Arc<AtomicU64>; 2],
}

impl ReplayCapture {
    /// Waits until each side of the latest meeting has delivered `frames`, for up to `timeout`.
    /// Capture starts once the meeting's start is on disk, not at `meeting.started`.
    fn wait_delivered(&self, frames: u64, timeout: Duration) -> bool {
        let until = std::time::Instant::now() + timeout;
        while self
            .delivered
            .iter()
            .any(|d| d.load(std::sync::atomic::Ordering::Acquire) < frames)
        {
            if std::time::Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }
}

/// A replay that counts the frames it has pushed into capture's ring.
struct Counted {
    source: FileReplaySource,
    frames: Arc<AtomicU64>,
}

/// The ring's sink, counting each block after the ring has it. Only an atomic add on the
/// realtime thread.
struct CountingSink {
    sink: Box<dyn ink_core::AudioSink>,
    frames: Arc<AtomicU64>,
}

impl ink_core::AudioSink for CountingSink {
    fn push(&mut self, block: &ink_core::AudioBlock<'_>) {
        self.sink.push(block);
        self.frames
            .fetch_add(block.frames() as u64, std::sync::atomic::Ordering::Release);
    }
}

impl ink_core::AudioSource for Counted {
    fn channel(&self) -> Channel {
        self.source.channel()
    }

    fn format(&self) -> ink_core::StreamFormat {
        self.source.format()
    }

    fn start(&mut self, sink: Box<dyn ink_core::AudioSink>) -> Result<(), PlatformError> {
        self.source.start(Box::new(CountingSink {
            sink,
            frames: self.frames.clone(),
        }))
    }

    fn stop(&mut self) -> Result<ink_core::SourceStats, PlatformError> {
        self.source.stop()
    }
}

impl MeetingCapture for ReplayCapture {
    fn open(&self, app: Option<&AppRef>, _: &Choices) -> Result<Opened, String> {
        self.opened_for
            .lock()
            .unwrap()
            .push(app.map(|a| a.id.clone()));
        let side = |path: &Path, channel| -> Result<CaptureSide, String> {
            let source = FileReplaySource::open(path, channel, self.clock.clone())
                .map_err(|e| e.to_string())?
                .with_pacing(Pacing::RealTime);
            let frames = self.delivered[usize::from(channel == Channel::Far)].clone();
            frames.store(0, std::sync::atomic::Ordering::Release);
            Ok(CaptureSide {
                source: Box::new(Counted { source, frames }),
                ring: DEFAULT_RING_DURATION,
                start_at: None,
                follow: None,
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
                wanted: None,
            }),
            far: match app {
                None => FarScope::Everything,
                Some(_) if self.tap_fails.load(std::sync::atomic::Ordering::Relaxed) => {
                    FarScope::EverythingInstead("no audio process for that app".into())
                }
                Some(_)
                    if self
                        .device_loopback
                        .load(std::sync::atomic::Ordering::Relaxed) =>
                {
                    FarScope::Everything
                }
                Some(_) => FarScope::App,
            },
        })
    }

    /// As Windows' plan: an app heard by device loopback is known before anything opens.
    fn planned_far(&self, _app: &AppRef) -> Option<FarScope> {
        self.device_loopback
            .load(std::sync::atomic::Ordering::Relaxed)
            .then_some(FarScope::Everything)
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
    rig_with_keys(label, seconds, clock, store, None)
}

fn rig_with_keys(
    label: &str,
    seconds: f64,
    clock: Arc<dyn Clock>,
    store: Arc<dyn Store>,
    keys: Option<Arc<dyn ink_core::HotkeySource>>,
) -> Rig {
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
        device_loopback: Default::default(),
        delivered: Default::default(),
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
        local: Default::default(),
        meetings: MeetingPlatform {
            capture: capture.clone(),
            detector: Some(detector.clone()),
            keys,
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

/// The user agrees, through the consent step's commands, that summaries and Ask may send the
/// transcript where the registered model goes (`ref` names the commands).
fn allow_meetings(core: &Core, events: &Recorder, reference: &str) {
    let get = format!("{reference}-get");
    core.command(&format!(
        r#"{{"cmd":"consent.get","feature":"meetings","id":"{get}"}}"#
    ))
    .unwrap();
    let state = events
        .wait_for(WAIT, |v| v["type"] == "consent.state" && v["ref"] == get)
        .expect("consent.state");
    let mut allow = serde_json::json!({
        "cmd": "consent.allow", "feature": "meetings", "to": state["to"], "id": reference
    });
    if let Some(endpoint) = state.get("endpoint") {
        allow["endpoint"] = endpoint.clone();
    }
    core.command(&allow.to_string()).unwrap();
    let allowed = events
        .wait_for(WAIT, |v| {
            v["type"] == "consent.state" && v["ref"] == reference
        })
        .expect("allowed");
    assert_eq!(allowed["allowed"], true, "{allowed}");
    assert_eq!(allowed["on"], true, "{allowed}");
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

    // Stopped by hand once each side has delivered a second (the files are 16 kHz). Capture
    // starts once the worker has the meeting's start on disk (its timeline and crash marker, both
    // synced), not at `meeting.started`: on a slow CI runner that came a second after the event.
    assert!(
        r.capture.wait_delivered(16_000, WAIT),
        "each side delivered a second"
    );
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
        // Every frame delivered before the stop is on disk, and nothing near the files' 30 s.
        let ms = p["pass"]["captured_ms"].as_u64().unwrap();
        assert!((1_000..10_000).contains(&ms), "{p}");
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
    // With words in the record, and the user's OK, the model is asked and its answer is trimmed.
    allow_meetings(&r.core, &r.events, "allow");
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

/// Registers a local language model that records what it is sent (answering as for Ask).
fn register_model(core: &Core) -> &'static Model {
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
        unsafe { Registration::from_table(&table, core.shared().shutdown.clone()) }.unwrap();
    core.register(registration).unwrap();
    model
}

/// Starts a meeting whose record already holds a line, and gives its record.
fn meeting_with_words(r: &Rig) -> ink_core::RecordId {
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
    record
}

/// Owner decision (2026-09-28): a meeting's summary and Ask send its transcript to a language
/// model, so they run only with the user's consent for where it goes (the `meetings` feature).
/// Without it nothing is sent: Ask answers that it needs the user's OK in Settings, and the
/// meeting finishes normally with no summary, saying why. Consent for polish and voice edit does
/// not cover it.
#[test]
fn ask_and_the_summary_send_nothing_without_the_meetings_consent() {
    let r = rig("meetings-no-consent", 30.0, clock());
    let model = register_model(&r.core);
    // Polish and voice edit allowed: not summaries and Ask.
    r.core
        .command(r#"{"cmd":"consent.allow","feature":"polish","to":"on_device","id":"p"}"#)
        .unwrap();
    r.core
        .command(
            r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"right_command","id":"e"}"#,
        )
        .unwrap();
    for id in ["p", "e"] {
        let state = r
            .events
            .wait_for(WAIT, |v| v["type"] == "consent.state" && v["ref"] == id)
            .unwrap();
        assert_eq!(state["allowed"], true, "{state}");
    }
    r.core
        .command(r#"{"cmd":"consent.get","feature":"meetings","id":"m"}"#)
        .unwrap();
    let meetings = r
        .events
        .wait_for(WAIT, |v| v["type"] == "consent.state" && v["ref"] == "m")
        .unwrap();
    assert_eq!(meetings["allowed"], false, "{meetings}");
    assert_eq!(meetings["on"], false, "{meetings}");
    assert_eq!(meetings["to"], "on_device", "names where it would go");

    let record = meeting_with_words(&r);
    r.core
        .command(r#"{"cmd":"meeting.ask","question":"What did they ask?","id":"q1"}"#)
        .unwrap();
    let refused = failed_with(&r.events, "q1");
    assert_eq!(refused["message"], ink_ffi::asking::NEEDS_CONSENT);
    assert!(
        ink_ffi::asking::NEEDS_CONSENT.contains("Settings > AI"),
        "says where to give it"
    );

    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    let finished = r.events.wait_type("meeting.finished", WAIT);
    assert_eq!(finished["record"], record.0.as_str());
    let warning = r
        .events
        .wait_for(Duration::ZERO, |v| {
            v["type"] == "meeting.warning" && v["kind"] == "summary_not_allowed"
        })
        .expect("the missing summary is said");
    assert_eq!(warning["message"], "a model on this machine", "{warning}");
    assert_eq!(r.events.count("meeting.summarized"), 0);
    assert_eq!(
        r.events.count("meeting.failed"),
        0,
        "{:?}",
        r.events.types()
    );
    assert_eq!(r.core.shared().store.summary(&record).unwrap(), None);
    assert!(
        model.requests.lock().unwrap().is_empty(),
        "nothing reached the model"
    );
    r.events.assert_valid();
    r.core.shutdown();
}

/// Owner decision (2026-09-28): allowing summaries and Ask turns them on for that destination
/// only (not polish or voice edit), and turning them off withdraws the consent in the same
/// write, so Ask is refused again and `setting.set` cannot turn them back on.
#[test]
fn the_meetings_consent_is_given_and_withdrawn() {
    let r = rig("meetings-consent", 30.0, clock());
    let model = register_model(&r.core);
    allow_meetings(&r.core, &r.events, "allow");
    for feature in ["polish", "edit"] {
        r.core
            .command(&format!(
                r#"{{"cmd":"consent.get","feature":"{feature}","id":"{feature}"}}"#
            ))
            .unwrap();
        let state = r
            .events
            .wait_for(WAIT, |v| {
                v["type"] == "consent.state" && v["ref"] == feature
            })
            .unwrap();
        assert_eq!(state["allowed"], false, "{feature}: {state}");
    }
    let _ = meeting_with_words(&r);
    r.core
        .command(r#"{"cmd":"meeting.ask","question":"What did they ask?","id":"q1"}"#)
        .unwrap();
    r.events
        .wait_for(WAIT, |v| {
            v["type"] == "meeting.answered" && v["ref"] == "q1"
        })
        .expect("answered with consent");
    assert_eq!(model.requests.lock().unwrap().len(), 1);

    // Off: the consent goes with the switch.
    r.core
        .command(r#"{"cmd":"setting.set","key":"meetings.llm","value":"off","id":"off"}"#)
        .unwrap();
    let state = r
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "consent.state" && v["feature"] == "meetings" && v.get("ref").is_none()
        })
        .expect("the state after the switch");
    assert_eq!(state["on"], false, "{state}");
    assert_eq!(state["allowed"], false, "{state}");
    assert_eq!(
        r.core
            .shared()
            .store
            .setting("llm.consent.meetings")
            .unwrap()
            .as_deref(),
        Some("none")
    );
    r.core
        .command(r#"{"cmd":"meeting.ask","question":"What did they ask?","id":"q2"}"#)
        .unwrap();
    assert_eq!(
        failed_with(&r.events, "q2")["message"],
        ink_ffi::asking::NEEDS_CONSENT
    );
    assert_eq!(model.requests.lock().unwrap().len(), 1, "nothing more sent");
    // And only the consent step turns it on again.
    let refused = r
        .core
        .command(r#"{"cmd":"setting.set","key":"meetings.llm","value":"on","id":"on"}"#)
        .expect_err("setting.set cannot turn it on");
    assert!(refused.contains("consent.allow"), "{refused}");
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    r.events.wait_type("meeting.finished", WAIT);
    assert!(
        r.events
            .wait_for(Duration::ZERO, |v| {
                v["type"] == "meeting.warning" && v["kind"] == "summary_not_allowed"
            })
            .is_some()
    );
    assert_eq!(model.requests.lock().unwrap().len(), 1, "no summary sent");
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
        local: Default::default(),
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
        local: Default::default(),
        meetings: MeetingPlatform::default(),
    });
    assert!(
        core.command(r#"{"cmd":"setting.set","key":"retention.days","value":"1"}"#)
            .is_err()
    );
    core.shutdown();
}

/// Retention never sweeps what an import brought in: records another source's import wrote (a
/// meeting with its audio, a dictation) are kept however old they are, while a meeting and a
/// dictation made here, as old, are deleted.
#[test]
fn a_sweep_keeps_what_an_import_brought_in() {
    const DAY: i64 = 86_400_000;
    let dir = TempDir::new("sweep-imports");
    let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    let clock = clock();
    let old = clock.unix_ms() - 40 * DAY;
    let made_here = |kind| {
        let id = store
            .create_record(NewRecord {
                kind,
                title: None,
                started_at_unix_ms: old,
                source_app: None,
                audio_dir: None,
            })
            .unwrap();
        store.finish_record(&id, old + 60_000).unwrap();
        id
    };
    let (meeting, dictation) = (
        made_here(RecordKind::Meeting),
        made_here(RecordKind::Dictation),
    );
    // Written by the importing tool before the import, as `RecordImport` asks.
    let audio = dir.path().join("meetings/imported");
    std::fs::create_dir_all(&audio).unwrap();
    std::fs::write(audio.join("mic-000000-16000x1.pcm"), [0u8; 64]).unwrap();
    let imported = |kind, audio_dir: Option<&str>| ink_store::import::RecordImport {
        record: NewRecord {
            kind,
            title: None,
            started_at_unix_ms: old,
            source_app: None,
            audio_dir: audio_dir.map(Into::into),
        },
        ended_at_unix_ms: Some(old + 60_000),
        revision: 2,
        segments: vec![ink_core::Segment {
            channel: Channel::Mic,
            start_ms: 0,
            end_ms: 1_000,
            text: "an imported line".into(),
            speaker: None,
        }],
        summary: None,
        speaker_names: vec![],
        commitments: vec![],
    };
    let imports = store
        .import_records(
            "import.example-source",
            "{}",
            &[
                imported(RecordKind::Meeting, Some("meetings/imported")),
                imported(RecordKind::Dictation, None),
            ],
        )
        .unwrap();
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
        local: Default::default(),
        meetings: MeetingPlatform::default(),
    };
    // Forever until now: the launch's sweep deletes nothing, so the change's sweep is the one.
    let (core, events) = start_parts(parts);
    core.command(r#"{"cmd":"setting.set","key":"retention.days","value":"30"}"#)
        .unwrap();
    let swept = events.wait_type("library.swept", WAIT);
    assert_eq!(
        swept["deleted"], 2,
        "the meeting and the dictation made here"
    );
    assert_eq!(swept["failed"], 0);
    assert_eq!(store.record(&meeting).unwrap(), None);
    assert_eq!(store.record(&dictation).unwrap(), None);
    for id in &imports {
        assert!(store.record(id).unwrap().is_some(), "an import is kept");
        assert_eq!(store.segments(id).unwrap().len(), 1);
    }
    assert!(audio.exists(), "and the imported meeting's audio");
    events.assert_valid();
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
    allow_meetings(&core, &events, "allow");
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
        local: Default::default(),
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
    // This sweep takes it, or the launch's did already if it ran late (sweeps are serialised).
    ink_ffi::retention::sweep(&shared).expect("30 days");
    assert_eq!(store.record(&record).unwrap(), None, "then swept");
    events.assert_valid();
    core.shutdown();
}

/// Review of 55e6485: the sweep decides under the holds' lock and deletes outside it. While it
/// deletes a record (here, inside the store's delete_record), a hold on another record is taken at
/// once (never waits on the delete), and a hold on the record being deleted is refused, not taken.
#[test]
fn a_sweep_deleting_a_record_blocks_no_hold_and_refuses_one_on_that_record() {
    use std::sync::mpsc;
    let dir = TempDir::new("sweep-holds");
    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    let clock = clock();
    let old = clock.unix_ms() - 40 * 86_400_000;
    let record = store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: Some("Old".into()),
            started_at_unix_ms: old,
            source_app: None,
            audio_dir: None,
        })
        .unwrap();
    store.finish_record(&record, old + 1_000).unwrap();
    let (core, events) = recovery_core(dir.path(), store.clone(), clock);
    let shared = core.shared().clone();
    let during = Arc::new(Mutex::new(None));
    {
        let (shared, record, during) = (shared.clone(), record.clone(), during.clone());
        store.after("delete_record", move || {
            // Each on its own thread, bounded: a hold that waited on this delete would never
            // return while we are inside it.
            let try_hold = |id: ink_core::RecordId| {
                let (tx, rx) = mpsc::channel();
                let shared = shared.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(shared.hold_from_sweep(&id).map(|_| ()));
                });
                rx.recv_timeout(Duration::from_secs(5))
            };
            let other = try_hold(ink_core::RecordId("another-meeting".into()));
            let same = try_hold(record.clone());
            *during.lock().unwrap() = Some((other, same));
        });
    }
    // Set once the hook is in place: the launch's sweep keeps everything unless it runs late,
    // and then it is the one that runs the hook.
    store.set_setting("retention.days", "30").unwrap();
    // This sweep, or the launch's if it runs late (sweeps are serialised): whichever deletes the
    // record runs the hook inside its delete, once.
    ink_ffi::retention::sweep(&shared).expect("30 days");
    let (other, same) = during
        .lock()
        .unwrap()
        .take()
        .expect("ran inside the delete");
    assert_eq!(
        other,
        Ok(Ok(())),
        "another record's hold never waits on a delete"
    );
    assert_eq!(
        same,
        Ok(Err(ink_ffi::retention::BeingSwept(record.clone()))),
        "a hold on the record being deleted is refused"
    );
    // Once deleted, the record is no longer marked: a hold on its id is taken (and it is gone).
    assert!(shared.hold_from_sweep(&record).is_ok());
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

/// PR #86 (macOS CI): a `meetings.recover` that arrives while a recovery runs is never refused
/// ("recovery is already running" once answered one that came just after `meetings.recovered`,
/// while the finished run's thread was still returning). It makes the run go one more round.
/// Deterministic: the second ask is sent from inside the first round, and the round waits until
/// the meetings thread has handled it (a later command on that thread has been answered).
#[test]
fn a_recover_sent_during_a_recovery_is_one_more_round_not_refused() {
    let dir = TempDir::new("recover-during");
    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    let clock = clock();
    let (record, audio) = interrupted_meeting(dir.path(), store.as_ref(), clock.as_ref(), 0);
    let (core, events) = recovery_core(dir.path(), store.clone(), clock);
    let core = Arc::new(core);
    {
        let (core, events) = (core.clone(), events.clone());
        store.after("record", move || {
            core.command(r#"{"cmd":"meetings.recover","id":"second"}"#)
                .unwrap();
            // Handled in order on the meetings thread: once this one is answered, so was the ask.
            core.command(r#"{"cmd":"meeting.stop","id":"probe"}"#)
                .unwrap();
            events
                .wait_for(WAIT, |v| {
                    v["type"] == "command.failed" && v["id"] == "probe"
                })
                .expect("the probe was answered");
        });
    }
    core.command(r#"{"cmd":"meetings.recover","id":"first"}"#)
        .unwrap();
    assert!(
        events.wait_count("meetings.recovered", 2, WAIT),
        "{:?}",
        events.types()
    );
    let rounds: Vec<_> = events
        .all()
        .into_iter()
        .filter(|v| v["type"] == "meetings.recovered")
        .map(|v| v["meetings"].clone())
        .collect();
    assert_eq!(
        rounds,
        [serde_json::json!(1), serde_json::json!(0)],
        "the second found nothing left"
    );
    assert!(
        !events
            .all()
            .iter()
            .any(|v| v["type"] == "command.failed" && v["command"] == "meetings.recover"),
        "never refused: {:?}",
        events.types()
    );
    assert!(!audio.join(ink_ffi::recovery::LIVE_FILE).exists());
    assert!(store.record(&record).unwrap().is_some());
    events.assert_valid();
    Arc::into_inner(core)
        .expect("the hook let go of the core")
        .shutdown();
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
    // The user agreed to send meetings to this model: local-only refuses it all the same.
    allow_meetings(&r.core, &r.events, "allow");
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

// --- Call policies: Always, Ask, Never; Stop and delete -------------------------------------

/// A rig on a mock clock, driven as the detection test drives it: the meetings thread reads the
/// clock when a signal arrives, so each signal is given time to be taken before the clock moves.
struct Driven {
    r: Rig,
    clock: Arc<MockClock>,
}

impl Driven {
    fn new(label: &str, store: Arc<dyn Store>) -> Self {
        let clock = Arc::new(MockClock::new(10_000_000_000, 1_790_146_800_000));
        let r = rig_with(label, 60.0, clock.clone(), store);
        Self { r, clock }
    }

    fn signal(&self, s: MeetingSignal) {
        self.r.detector.signal(s);
        std::thread::sleep(Duration::from_millis(150));
    }

    /// Any signal makes the meetings thread judge what is pending at the clock's time.
    fn poke(&self) {
        self.signal(MeetingSignal::MicReleased {
            app: app("nobody", "Nobody"),
        });
    }

    /// `id` takes the mic and holds it past the hold.
    fn hold(&self, id: &str, name: &str) {
        self.signal(MeetingSignal::MicInUse { app: app(id, name) });
        self.clock.advance_ns(3_500_000_000);
        self.poke();
    }

    fn command(&self, json: &str) {
        self.r.core.command(json).unwrap();
    }

    fn listening(&self, on: bool) {
        self.r
            .events
            .wait_for(WAIT, |v| {
                v["type"] == "meeting.detection" && v["listening"] == on
            })
            .unwrap_or_else(|| panic!("listening {on}: {:?}", self.r.events.types()));
    }

    fn answer(&self, ty: &str, reference: &str) -> Value {
        self.r
            .events
            .wait_for(WAIT, |v| v["type"] == ty && v["ref"] == reference)
            .unwrap_or_else(|| panic!("no {ty} for {reference}: {:?}", self.r.events.types()))
    }
}

fn memory_store() -> Arc<dyn Store> {
    Arc::new(ink_store::SqliteStore::open_in_memory().unwrap())
}

/// The meeting directories under the rig's library.
fn meeting_dirs(r: &Rig) -> Vec<PathBuf> {
    match std::fs::read_dir(r._dir.path().join("meetings")) {
        Ok(entries) => entries.filter_map(Result::ok).map(|e| e.path()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Never records silently: an Always app's call starts with no offer, through the same start as
/// Record, and its `meeting.started` (the event the shells show the recording indicator from)
/// says it was its policy and until when it can be stopped and deleted. It ends as any recorded
/// app's meeting does.
#[test]
fn an_always_app_is_recorded_without_asking_and_shows_as_any_recording() {
    let d = Driven::new("always", memory_store());
    d.listening(true);
    d.command(
        r#"{"cmd":"meetings.calls.set","app":"com.example.call","policy":"always","id":"c1"}"#,
    );
    let listed = d.answer("meetings.calls", "c1");
    assert_eq!(listed["default"], "ask");
    assert_eq!(
        listed["apps"],
        serde_json::json!([{"app": "com.example.call", "policy": "always", "chosen": true}])
    );

    d.hold("com.example.call", "Example Call");
    let started = d.r.events.wait_type("meeting.started", WAIT);
    assert_eq!(started["auto"], true, "{started}");
    assert_eq!(started["app"], "com.example.call");
    assert_eq!(started["app_name"], "Example Call");
    assert_eq!(started["far_end"], "app");
    assert_eq!(
        started["delete_until_unix_ms"].as_i64(),
        Some(d.clock.unix_ms() + 60_000),
        "a minute after the start"
    );
    assert_eq!(
        d.r.events.count("meeting.detected"),
        0,
        "recorded, not asked"
    );
    assert_eq!(
        *d.r.capture.opened_for.lock().unwrap(),
        [Some("com.example.call".to_owned())]
    );
    // Seen: the list now names it, for Settings.
    let seen =
        d.r.events
            .wait_for(WAIT, |v| {
                v["type"] == "meetings.calls"
                    && v.get("ref").is_none()
                    && v["apps"][0]["app_name"] == "Example Call"
            })
            .expect("the app seen joins the list");
    assert_eq!(seen["apps"][0]["policy"], "always");
    assert!(seen["apps"][0]["seen_unix_ms"].is_i64());
    // Saved: a new core reads the same choice.
    let stored =
        d.r.core
            .shared()
            .store
            .setting(ink_ffi::calls::APPS_KEY)
            .unwrap()
            .unwrap();
    assert!(stored.contains("\"policy\":\"always\""), "{stored}");

    // It ends as a recorded app's meeting does: 15 s after its app lets go.
    std::thread::sleep(Duration::from_millis(500));
    d.signal(MeetingSignal::MicReleased {
        app: app("com.example.call", "Example Call"),
    });
    d.clock.advance_ns(16_000_000_000);
    d.poke();
    d.r.events.wait_type("meeting.stopped", WAIT);
    d.r.events.wait_type("meeting.finished", WAIT);
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

/// "Stop and delete" in the first minute: capture ends, no final pass runs, and the record and its
/// audio directory are gone as if the meeting had never been made.
#[test]
fn stop_and_delete_in_the_first_minute_leaves_nothing_of_the_meeting() {
    let r = rig("discard", 30.0, clock());
    r.core
        .command(r#"{"cmd":"meeting.start","id":"s"}"#)
        .unwrap();
    let started = r.events.wait_type("meeting.started", WAIT);
    assert!(started.get("auto").is_none(), "the user started it");
    assert!(
        started["delete_until_unix_ms"].is_i64(),
        "any start may be deleted"
    );
    assert!(r.capture.wait_delivered(16_000, WAIT));
    let record = ink_core::RecordId(started["record"].as_str().unwrap().to_owned());
    let store = r.core.shared().store.clone();
    assert!(store.record(&record).unwrap().is_some());
    let dirs = meeting_dirs(&r);
    assert_eq!(dirs.len(), 1);
    assert!(
        dirs[0].join("live.json").is_file(),
        "marked live while it records"
    );

    r.core
        .command(r#"{"cmd":"meeting.discard","id":"d1"}"#)
        .unwrap();
    r.events.wait_type("meeting.stopped", WAIT);
    let gone = r.events.wait_type("meeting.discarded", WAIT);
    assert_eq!(gone["record"], started["record"]);
    assert_eq!(gone["audio_left"], false);
    assert_eq!(gone["scrubbed"], true);
    assert!(
        store.record(&record).unwrap().is_none(),
        "the record is gone"
    );
    assert!(
        store.segments(&record).unwrap_or_default().is_empty(),
        "and its words"
    );
    assert!(meeting_dirs(&r).is_empty(), "and its audio, marker and all");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(r.events.count("meeting.finished"), 0, "no final pass");
    assert_eq!(r.events.count("meeting.transcribed"), 0);

    // Nothing left to delete; a new meeting starts at once.
    r.core
        .command(r#"{"cmd":"meeting.discard","id":"d2"}"#)
        .unwrap();
    assert!(
        failed_with(&r.events, "d2")["message"]
            .as_str()
            .unwrap()
            .contains("no meeting")
    );
    r.core.command(r#"{"cmd":"meeting.start"}"#).unwrap();
    assert!(r.events.wait_count("meeting.started", 2, WAIT));
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    r.events.wait_type("meeting.finished", WAIT);
    r.events.assert_valid();
    r.core.shutdown();
}

/// After the first minute only Stop is left: the delete is refused with its code, and the meeting
/// goes on to be stopped and finished as usual.
#[test]
fn stop_and_delete_is_refused_after_the_first_minute() {
    let d = Driven::new("discard-late", memory_store());
    d.command(r#"{"cmd":"meeting.start","id":"s"}"#);
    let started = d.r.events.wait_type("meeting.started", WAIT);
    assert!(d.r.capture.wait_delivered(8_000, WAIT));
    d.clock.advance_ns(60_000_000_000);
    d.command(r#"{"cmd":"meeting.discard","id":"late"}"#);
    let refused = failed_with(&d.r.events, "late");
    assert_eq!(refused["code"], "delete_window_over", "{refused}");
    d.command(r#"{"cmd":"meeting.stop"}"#);
    let finished = d.r.events.wait_type("meeting.finished", WAIT);
    assert_eq!(finished["record"], started["record"]);
    assert_eq!(d.r.events.count("meeting.discarded"), 0);
    let record = ink_core::RecordId(started["record"].as_str().unwrap().to_owned());
    assert!(d.r.core.shared().store.record(&record).unwrap().is_some());
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

/// A Never app is neither offered nor recorded; made Ask while it holds the mic, it is offered at
/// once; made Never from the offer ("Never for this app"), the offer goes as if dismissed.
#[test]
fn a_never_app_is_offered_once_it_becomes_ask_and_never_withdraws_its_offer() {
    let d = Driven::new("never", memory_store());
    d.listening(true);
    d.command(r#"{"cmd":"meetings.calls.set","app":"com.example.chat","policy":"never","id":"n"}"#);
    d.answer("meetings.calls", "n");
    d.hold("com.example.chat", "Example Chat");
    d.clock.advance_ns(30_000_000_000);
    d.poke();
    assert_eq!(d.r.events.count("meeting.detected"), 0, "Never: no offer");
    assert_eq!(
        d.r.events.count("meeting.started"),
        0,
        "and nothing recorded"
    );

    d.command(r#"{"cmd":"meetings.calls.set","app":"com.example.chat","policy":"ask","id":"a"}"#);
    let offered = d.r.events.wait_type("meeting.detected", WAIT);
    assert_eq!(offered["app"], "com.example.chat");
    assert!(offered.get("message").is_none());
    let listed = d.answer("meetings.calls", "a");
    assert_eq!(listed["apps"][0]["policy"], "ask");

    d.command(
        r#"{"cmd":"meetings.calls.set","app":"com.example.chat","policy":"never","id":"n2"}"#,
    );
    let ended = d.r.events.wait_type("meeting.detection_ended", WAIT);
    assert_eq!(ended["app"], "com.example.chat");
    assert_eq!(ended["dismissed"], true);

    // `default` clears the choice: the app follows the default (Ask) and is offered again.
    d.command(
        r#"{"cmd":"meetings.calls.set","app":"com.example.chat","policy":"default","id":"c"}"#,
    );
    assert!(d.r.events.wait_count("meeting.detected", 2, WAIT));
    let listed = d.answer("meetings.calls", "c");
    assert_eq!(listed["apps"][0]["chosen"], false);
    // A choice the core refuses changes nothing.
    assert!(
        d.r.core
            .command(r#"{"cmd":"meetings.calls.set","app":" padded","policy":"always"}"#)
            .is_err()
    );
    assert!(
        d.r.core
            .command(r#"{"cmd":"meetings.calls.set","app":"x","policy":"sometimes"}"#)
            .is_err()
    );
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

/// The old switch "Offer to record calls" off becomes the default Never at launch: detection is
/// off, as before. An Always app turns it on, and only that app is recorded; the old switch on
/// over Never is Ask.
#[test]
fn the_old_switch_off_migrates_to_never_and_an_always_app_turns_detection_on() {
    let store = memory_store();
    store.set_setting("meetings.detect", "off").unwrap();
    let d = Driven::new("migrate", store.clone());
    d.listening(false);
    assert_eq!(
        store
            .setting(ink_ffi::calls::DEFAULT_KEY)
            .unwrap()
            .as_deref(),
        Some("never"),
        "migrated"
    );
    d.command(r#"{"cmd":"setting.get","key":"meetings.calls.default"}"#);
    d.command(r#"{"cmd":"setting.get","key":"meetings.detect"}"#);
    let value = |key: &str| {
        d.r.events
            .wait_for(WAIT, |v| v["type"] == "setting.value" && v["key"] == key)
            .unwrap()["value"]
            .clone()
    };
    assert_eq!(value("meetings.calls.default"), "never");
    assert_eq!(value("meetings.detect"), "off");

    d.command(r#"{"cmd":"meetings.calls.set","app":"com.example.call","policy":"always"}"#);
    d.listening(true);
    d.hold("com.example.other", "Other");
    assert_eq!(
        d.r.events.count("meeting.detected"),
        0,
        "the default is Never"
    );
    d.hold("com.example.call", "Example Call");
    let started = d.r.events.wait_type("meeting.started", WAIT);
    assert_eq!(started["app"], "com.example.call");
    assert_eq!(started["auto"], true);
    d.command(r#"{"cmd":"meeting.stop"}"#);
    d.r.events.wait_type("meeting.finished", WAIT);

    // The old switch on, over Never: Ask (the shell's toggle keeps working until it moves).
    d.command(r#"{"cmd":"setting.set","key":"meetings.detect","value":"on","id":"on"}"#);
    assert!(
        d.r.events
            .wait_for(WAIT, |v| v["type"] == "meetings.calls"
                && v["default"] == "ask")
            .is_some()
    );
    assert_eq!(
        store
            .setting(ink_ffi::calls::DEFAULT_KEY)
            .unwrap()
            .as_deref(),
        Some("ask")
    );
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

/// A crash after "Stop and delete" and before the delete was done: the next launch's recovery
/// deletes the meeting (record and audio) and never finishes it.
#[test]
fn recovery_deletes_a_meeting_stopped_to_be_deleted() {
    let r = rig("discard-recover", 5.0, clock());
    let store = r.core.shared().store.clone();
    let record = store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: None,
            started_at_unix_ms: 1_790_146_800_000,
            source_app: Some("com.example.call".into()),
            audio_dir: Some("meetings/1790146800000-0".into()),
        })
        .unwrap();
    store
        .append_segments(
            &record,
            &[ink_core::Segment {
                channel: Channel::Mic,
                start_ms: 0,
                end_ms: 1_000,
                text: "words said by mistake".into(),
                speaker: None,
            }],
        )
        .unwrap();
    let dir = r._dir.path().join("meetings/1790146800000-0");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("mic-000000.pcm"), [0u8; 64]).unwrap();
    ink_ffi::recovery::mark_discard(&dir, &record).unwrap();

    r.core.command(r#"{"cmd":"meetings.recover"}"#).unwrap();
    let gone = r.events.wait_type("meeting.discarded", WAIT);
    assert_eq!(gone["record"], record.0.as_str());
    r.events.wait_type("meetings.recovered", WAIT);
    assert!(store.record(&record).unwrap().is_none());
    assert!(!dir.exists(), "its audio and marker are gone");
    assert_eq!(r.events.count("meeting.finished"), 0, "never finished");
    r.events.assert_valid();
    r.core.shutdown();
}

/// A stop by hand during an Always call is the user's: the call, its app still on the mic, is not
/// recorded again by itself (it is offered); Stop and delete is a stop by hand too.
#[test]
fn a_stopped_always_call_is_offered_not_recorded_again() {
    let d = Driven::new("always-stop", memory_store());
    d.listening(true);
    d.command(r#"{"cmd":"meetings.calls.set","app":"com.example.call","policy":"always"}"#);
    d.hold("com.example.call", "Example Call");
    d.r.events.wait_type("meeting.started", WAIT);
    assert!(d.r.capture.wait_delivered(4_000, WAIT));
    d.command(r#"{"cmd":"meeting.discard","id":"d"}"#);
    d.r.events.wait_type("meeting.discarded", WAIT);
    d.clock.advance_ns(5_000_000_000);
    d.poke();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(d.r.events.count("meeting.started"), 1, "not recorded again");
    assert_eq!(
        d.r.events.count("meeting.detected"),
        0,
        "dismissed for this call, as Not this one"
    );
    // The next call is recorded again.
    d.signal(MeetingSignal::MicReleased {
        app: app("com.example.call", "Example Call"),
    });
    d.hold("com.example.call", "Example Call");
    assert!(d.r.events.wait_count("meeting.started", 2, WAIT));
    d.command(r#"{"cmd":"meeting.stop"}"#);
    d.r.events.wait_type("meeting.finished", WAIT);
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

/// Stored choices the core cannot read: every app is at most asked about, the list says why, and
/// a choice is refused until it says to start the list over.
#[test]
fn unreadable_choices_are_set_aside_and_only_replaced_when_asked() {
    let store = memory_store();
    store
        .set_setting(ink_ffi::calls::DEFAULT_KEY, "always")
        .unwrap();
    store
        .set_setting(
            ink_ffi::calls::APPS_KEY,
            "{\"apps\": [{\"app\": \"x\", \"policy\": 7}]}",
        )
        .unwrap();
    let d = Driven::new("calls-unreadable", store.clone());
    d.listening(true);
    d.command(r#"{"cmd":"meetings.calls.list","id":"l"}"#);
    let listed = d.answer("meetings.calls", "l");
    assert!(listed["message"].is_string(), "{listed}");
    // Always lowered to Ask: offered, not recorded.
    d.hold("com.example.call", "Example Call");
    d.r.events.wait_type("meeting.detected", WAIT);
    assert_eq!(d.r.events.count("meeting.started"), 0);
    d.command(r#"{"cmd":"meetings.calls.set","app":"com.example.call","policy":"never","id":"n"}"#);
    let refused = failed_with(&d.r.events, "n");
    assert_eq!(refused["code"], "list_unreadable", "{refused}");
    assert!(
        store
            .setting(ink_ffi::calls::APPS_KEY)
            .unwrap()
            .unwrap()
            .contains("\"policy\": 7"),
        "the stored text is untouched"
    );
    d.command(
        r#"{"cmd":"meetings.calls.set","app":"com.example.call","policy":"never","replace_unreadable":true,"id":"r"}"#,
    );
    let listed = d.answer("meetings.calls", "r");
    // Started over under Always: the default is Ask now, written, and said.
    assert_eq!(listed["default"], "ask", "{listed}");
    assert!(
        listed["message"].as_str().unwrap().contains("Ask now"),
        "{listed}"
    );
    assert_eq!(
        store
            .setting(ink_ffi::calls::DEFAULT_KEY)
            .unwrap()
            .as_deref(),
        Some("ask")
    );
    assert_eq!(listed["apps"][0]["policy"], "never");
    d.r.events.wait_type("meeting.detection_ended", WAIT);
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

/// An Always app whose own sound cannot be recorded alone (the Mac's tap fails, so everything it
/// plays would be recorded; or Windows hears it by loopback of its output device) is never
/// recorded by itself: it is offered, saying why, and no meeting.started comes. Windows' plan
/// says so before anything opens; the Mac's tap only once opened, so its capture is opened and
/// closed unstarted. A tap on Record records it, as today, with the fallback said.
#[test]
fn an_always_app_that_cannot_be_recorded_alone_is_offered_instead() {
    use std::sync::atomic::Ordering::Relaxed;
    for (label, mac) in [("not-alone-mac", true), ("not-alone-win", false)] {
        let d = Driven::new(label, memory_store());
        if mac {
            d.r.capture.tap_fails.store(true, Relaxed);
        } else {
            d.r.capture.device_loopback.store(true, Relaxed);
        }
        d.listening(true);
        d.command(r#"{"cmd":"meetings.calls.set","app":"com.example.call","policy":"always"}"#);
        d.hold("com.example.call", "Example Call");
        let offered = d.r.events.wait_type("meeting.detected", WAIT);
        assert_eq!(offered["app"], "com.example.call");
        assert_eq!(
            offered["message"].as_str(),
            Some(ink_ffi::control::NOT_ALONE),
            "{label}"
        );
        assert_eq!(
            d.r.events.count("meeting.started"),
            0,
            "{label}: nothing started"
        );
        let opened: &[Option<String>] = if mac {
            &[Some("com.example.call".to_owned())]
        } else {
            &[]
        };
        assert_eq!(
            *d.r.capture.opened_for.lock().unwrap(),
            opened,
            "{label}: opened only where only opening tells, and never started"
        );
        // The user's tap records it, everything included, and says so.
        d.command(r#"{"cmd":"meeting.start","app":"com.example.call"}"#);
        let started = d.r.events.wait_type("meeting.started", WAIT);
        assert!(started.get("auto").is_none());
        assert_eq!(started["far_end"], "everything", "{label}");
        d.command(r#"{"cmd":"meeting.stop"}"#);
        d.r.events.wait_type("meeting.finished", WAIT);
        d.r.events.assert_valid();
        d.r.core.shutdown();
    }
}

/// A meeting whose worker failed (its record could not be made, so it never started) refuses
/// Stop and delete at once: the shell is not left waiting for a meeting.discarded that cannot
/// come.
#[test]
fn a_meeting_that_failed_to_start_refuses_to_be_deleted() {
    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    let r = rig_with("discard-failed", 5.0, clock(), store.clone());
    store.fail(&["create_record"]);
    let run =
        ink_ffi::meeting::MeetingRun::start(r.core.shared(), Vec::new(), Default::default(), None)
            .unwrap();
    let failed = r.events.wait_type("meeting.failed", WAIT);
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .starts_with("the meeting could not start"),
        "{failed}"
    );
    let until = std::time::Instant::now() + WAIT;
    while !run.is_over() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(run.is_over());
    assert!(!run.discard(), "refused: nothing will be deleted");
    run.join();
    assert_eq!(r.events.count("meeting.discarded"), 0);
    r.events.assert_valid();
    r.core.shutdown();
}

/// Starting an unreadable list over lowers the default from what the store says, not what the
/// meetings thread last read: a Never the store holds stands; a store that cannot be read lowers
/// it (fail closed: never Always on a guess).
#[test]
fn starting_over_lowers_the_default_from_what_the_store_says() {
    for (label, after_load, lowered) in [
        ("store-never", "never", false),
        ("store-ask", "ask", false),
        ("store-fails", "", true),
    ] {
        let store = FailingStore::new(memory_store());
        store
            .set_setting(ink_ffi::calls::DEFAULT_KEY, "always")
            .unwrap();
        store
            .set_setting(ink_ffi::calls::APPS_KEY, "{\"apps\": 7}")
            .unwrap();
        let d = Driven::new(label, store.clone());
        d.listening(true);
        // Behind the meetings thread's back: no reload says so.
        if after_load.is_empty() {
            store.fail(&["setting"]);
        } else {
            store
                .inner
                .set_setting(ink_ffi::calls::DEFAULT_KEY, after_load)
                .unwrap();
        }
        d.command(
            r#"{"cmd":"meetings.calls.set","app":"com.example.call","policy":"never","replace_unreadable":true,"id":"r"}"#,
        );
        let listed = d.answer("meetings.calls", "r");
        store.heal();
        let stored = store.inner.setting(ink_ffi::calls::DEFAULT_KEY).unwrap();
        if lowered {
            assert_eq!(stored.as_deref(), Some("ask"), "{label}");
            assert_eq!(listed["default"], "ask", "{listed}: this thread's too");
            assert!(
                listed["message"].as_str().unwrap().contains("Ask now"),
                "{listed}"
            );
        } else {
            assert_eq!(
                stored.as_deref(),
                Some(after_load),
                "{label}: the store's stands"
            );
            assert!(listed.get("message").is_none(), "{listed}");
            // And is this thread's at once: never the stale Always it last read.
            assert_eq!(listed["default"], after_load, "{listed}");
        }
        assert_eq!(listed["apps"][0]["policy"], "never", "{listed}");
        d.r.events.assert_valid();
        d.r.core.shutdown();
    }
}

/// Removing an exception under Always never starts recording merely because Settings changed.
#[test]
fn removing_a_call_app_keeps_recordings_without_starting_capture() {
    let store = memory_store();
    store
        .set_setting(ink_ffi::calls::DEFAULT_KEY, "always")
        .unwrap();
    let d = Driven::new("remove-app", store.clone());
    d.listening(true);
    d.command(r#"{"cmd":"meetings.calls.set","app":"com.example.chat","policy":"never","id":"n"}"#);
    d.answer("meetings.calls", "n");
    d.hold("com.example.chat", "Example Chat");
    d.command(r#"{"cmd":"meetings.calls.remove","app":"com.example.chat","expected_default":"always","id":"r"}"#);
    let listed = d.answer("meetings.calls", "r");
    assert_eq!(listed["apps"], serde_json::json!([]));
    assert_eq!(listed["default"], "always");
    assert_eq!(d.r.events.count("meeting.started"), 0);
    d.r.events.wait_type("meeting.detected", WAIT);
    // The list query and removal share the observation worker: no stale row is restored.
    d.command(r#"{"cmd":"meetings.calls.list","id":"q"}"#);
    assert_eq!(
        d.answer("meetings.calls", "q")["apps"],
        serde_json::json!([])
    );
    d.command(r#"{"cmd":"meeting.start","app":"com.example.chat","id":"s"}"#);
    let started = d.r.events.wait_type("meeting.started", WAIT);
    let record = ink_core::RecordId(started["record"].as_str().unwrap().to_owned());
    d.command(r#"{"cmd":"meetings.calls.remove","app":"com.example.chat","expected_default":"always","id":"r2"}"#);
    d.answer("meetings.calls", "r2");
    assert!(store.record(&record).unwrap().is_some());
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

#[test]
fn removing_a_call_app_with_a_failed_save_keeps_its_rule_and_seen_row() {
    let store = FailingStore::new(memory_store());
    let d = Driven::new("remove-failure", store.clone());
    d.listening(true);
    d.command(r#"{"cmd":"meetings.calls.set","app":"com.example.chat","policy":"never","id":"n"}"#);
    d.answer("meetings.calls", "n");
    d.hold("com.example.chat", "Example Chat");
    store.fail(&["set_settings"]);
    d.command(r#"{"cmd":"meetings.calls.remove","app":"com.example.chat","expected_default":"ask","id":"r"}"#);
    let failed = failed_with(&d.r.events, "r");
    assert_eq!(failed["command"], "meetings.calls.remove");
    store.heal();
    d.command(r#"{"cmd":"meetings.calls.list","id":"q"}"#);
    let listed = d.answer("meetings.calls", "q");
    assert_eq!(listed["apps"][0]["policy"], "never");
    assert_eq!(listed["apps"][0]["chosen"], true);
    assert_eq!(d.r.events.count("meeting.started"), 0);
    assert!(
        d.r.core
            .command(r#"{"cmd":"meetings.calls.remove","app":" padded"}"#)
            .is_err()
    );
    assert!(
        d.r.core
            .command(r#"{"cmd":"meetings.calls.remove","app":"com.example.chat","policy":"never"}"#)
            .is_err()
    );
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

#[test]
fn removing_a_call_app_persists_and_the_next_call_can_readd_it() {
    let store = memory_store();
    let d = Driven::new("remove-readd", store.clone());
    d.listening(true);
    d.hold("com.example.chat", "Example Chat");
    d.command(r#"{"cmd":"meetings.calls.remove","app":"com.example.chat","expected_default":"ask","id":"r"}"#);
    assert_eq!(
        d.answer("meetings.calls", "r")["apps"],
        serde_json::json!([])
    );
    assert!(
        ink_ffi::calls::load(store.as_ref())
            .unwrap()
            .apps()
            .is_empty()
    );
    d.signal(MeetingSignal::MicReleased {
        app: app("com.example.chat", "Example Chat"),
    });
    d.hold("com.example.chat", "Example Chat");
    d.command(r#"{"cmd":"meetings.calls.list","id":"q"}"#);
    let listed = d.answer("meetings.calls", "q");
    assert_eq!(listed["apps"][0]["app"], "com.example.chat");
    assert_eq!(listed["apps"][0]["chosen"], false);
    assert_eq!(listed["apps"][0]["policy"], "ask");
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

#[test]
fn removing_a_call_app_refuses_to_replace_an_unreadable_list() {
    let store = memory_store();
    let original = r#"{"apps": [{"app": "com.example.chat", "policy": "bogus"}]}"#;
    store
        .set_setting(ink_ffi::calls::APPS_KEY, original)
        .unwrap();
    let d = Driven::new("remove-unreadable", store.clone());
    d.listening(true);
    d.command(r#"{"cmd":"meetings.calls.remove","app":"com.example.chat","expected_default":"ask","id":"r"}"#);
    assert_eq!(
        failed_with(&d.r.events, "r")["command"],
        "meetings.calls.remove"
    );
    assert_eq!(
        store.setting(ink_ffi::calls::APPS_KEY).unwrap().as_deref(),
        Some(original)
    );
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

#[test]
fn removing_a_call_app_refuses_a_default_the_user_did_not_confirm() {
    let store = memory_store();
    store
        .set_setting(ink_ffi::calls::DEFAULT_KEY, "always")
        .unwrap();
    let d = Driven::new("remove-default-race", store.clone());
    d.listening(true);
    d.command(r#"{"cmd":"meetings.calls.set","app":"com.example.chat","policy":"never","id":"n"}"#);
    d.answer("meetings.calls", "n");
    // The shell optimistically shows Never, but its default save failed.
    d.command(r#"{"cmd":"meetings.calls.remove","app":"com.example.chat","expected_default":"never","id":"r"}"#);
    assert_eq!(
        failed_with(&d.r.events, "r")["command"],
        "meetings.calls.remove"
    );
    assert_eq!(
        ink_ffi::calls::load(store.as_ref())
            .unwrap()
            .policy("com.example.chat"),
        ink_ffi::calls::CallPolicy::Never
    );
    // Also check the stored default when the observation worker has not reloaded it yet.
    store
        .set_setting(ink_ffi::calls::DEFAULT_KEY, "ask")
        .unwrap();
    d.command(r#"{"cmd":"meetings.calls.remove","app":"com.example.chat","expected_default":"always","id":"r2"}"#);
    assert_eq!(
        failed_with(&d.r.events, "r2")["command"],
        "meetings.calls.remove"
    );
    assert_eq!(
        ink_ffi::calls::load(store.as_ref())
            .unwrap()
            .policy("com.example.chat"),
        ink_ffi::calls::CallPolicy::Never
    );
    d.r.events.assert_valid();
    d.r.core.shutdown();
}

#[derive(Default)]
struct FakeShortcut {
    held: Mutex<Option<(String, EventSink<ink_core::HotkeyEvent>)>>,
}

impl ink_core::HotkeySource for FakeShortcut {
    fn start(
        &self,
        binding: &ink_core::HotkeyBinding,
        sink: EventSink<ink_core::HotkeyEvent>,
    ) -> Result<(), PlatformError> {
        *self.held.lock().unwrap() = Some((binding.0.clone(), sink));
        Ok(())
    }
    fn stop(&self) {
        *self.held.lock().unwrap() = None;
    }
}

impl FakeShortcut {
    fn sink(&self) -> EventSink<ink_core::HotkeyEvent> {
        self.held
            .lock()
            .unwrap()
            .as_ref()
            .expect("meeting hook held")
            .1
            .clone()
    }
    fn binding(&self) -> Option<String> {
        self.held.lock().unwrap().as_ref().map(|v| v.0.clone())
    }
}

fn shortcut_state(r: &Rig, id: &str, suspended: bool) -> Value {
    r.core
        .command(
            &serde_json::json!({"cmd":"meetings.shortcut.suspend","suspended":suspended,"id":id})
                .to_string(),
        )
        .unwrap();
    r.events
        .wait_for(WAIT, |v| {
            v["type"] == "meetings.shortcut.state" && v["ref"] == id
        })
        .expect("shortcut acknowledgement")
}

#[cfg(windows)]
#[test]
fn global_meeting_shortcut_toggles_once_and_capture_suspension_invalidates_queued_keys() {
    use ink_core::HotkeyEvent::{Pressed, Released};
    let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    let keys = Arc::new(FakeShortcut::default());
    let r = rig_with_keys("shortcut-toggle", 30.0, clock(), store, Some(keys.clone()));
    let ready = r.events.wait_type("meetings.shortcut.state", WAIT);
    assert_eq!(ready["active"], true);
    assert_eq!(keys.binding().as_deref(), Some("ctrl+shift+r"));
    let old = keys.sink();
    old(Pressed { at_ns: 1 });
    let started = r.events.wait_type("meeting.started", WAIT);
    assert!(
        started.get("auto").is_none(),
        "manual shortcut start is never automatic"
    );
    assert!(started.get("app").is_none());
    old(Pressed { at_ns: 2 });
    let paused = shortcut_state(&r, "pause", true);
    assert_eq!(paused["suspended"], true);
    assert_eq!(paused["active"], false);
    assert!(keys.binding().is_none());
    assert_eq!(
        r.events.count("meeting.stopped"),
        0,
        "a held repeat never stopped the recording"
    );
    old(Released { at_ns: 3 });
    old(Pressed { at_ns: 4 });
    shortcut_state(&r, "barrier", true);
    assert_eq!(
        r.events.count("meeting.stopped"),
        0,
        "a queued old binding cannot fire after acknowledgement"
    );
    assert_eq!(shortcut_state(&r, "resume", false)["active"], true);
    let current = keys.sink();
    current(Pressed { at_ns: 5 });
    r.events.wait_type("meeting.stopped", WAIT);
    current(Pressed { at_ns: 6 });
    shortcut_state(&r, "end-barrier", true);
    assert_eq!(
        r.capture.opened_for.lock().unwrap().len(),
        1,
        "a held stop never immediately starts a new recording"
    );
    r.events.assert_valid();
    r.core.shutdown();
    assert!(keys.binding().is_none());
}

#[cfg(windows)]
#[test]
fn global_meeting_shortcut_failed_save_and_unreadable_conflict_checks_preserve_settings() {
    let inner = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    inner.set_setting("meetings.key", "f13").unwrap();
    let store = FailingStore::new(inner.clone());
    let keys = Arc::new(FakeShortcut::default());
    let r = rig_with_keys(
        "shortcut-failures",
        1.0,
        clock(),
        store.clone(),
        Some(keys.clone()),
    );
    assert_eq!(
        r.events.wait_type("meetings.shortcut.state", WAIT)["active"],
        true
    );
    store.fail(&["set_setting"]);
    r.core
        .command(r#"{"cmd":"setting.set","key":"meetings.key","value":"f14","id":"save-failed"}"#)
        .unwrap();
    failed_with(&r.events, "save-failed");
    assert_eq!(keys.binding().as_deref(), Some("f13"));
    assert_eq!(
        inner.setting("meetings.key").unwrap().as_deref(),
        Some("f13")
    );
    store.heal();
    r.core
        .command(r#"{"cmd":"setting.set","key":"dictation.key","value":"F13","id":"clash"}"#)
        .unwrap();
    assert!(
        failed_with(&r.events, "clash")["message"]
            .as_str()
            .unwrap()
            .contains("meetings")
    );
    assert!(inner.setting("dictation.key").unwrap().is_none());
    r.core.command(r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"F13","id":"edit-clash"}"#).unwrap();
    assert!(
        failed_with(&r.events, "edit-clash")["message"]
            .as_str()
            .unwrap()
            .contains("meetings")
    );
    assert!(inner.setting("dictation.edit_key").unwrap().is_none());
    store.fail(&["setting"]);
    r.core
        .command(r#"{"cmd":"setting.set","key":"dictation.key","value":"f15","id":"read-failed"}"#)
        .unwrap();
    failed_with(&r.events, "read-failed");
    assert!(inner.setting("dictation.key").unwrap().is_none());
    let paused = shortcut_state(&r, "unreadable-pause", true);
    assert_eq!(paused["active"], false);
    let resumed = shortcut_state(&r, "unreadable-resume", false);
    assert_eq!(resumed["active"], false);
    assert!(resumed["error"].as_str().unwrap().contains("read"));
    assert!(keys.binding().is_none());
    store.heal();
    assert_eq!(shortcut_state(&r, "healed", false)["active"], true);
    r.events.assert_valid();
    r.core.shutdown();
}
