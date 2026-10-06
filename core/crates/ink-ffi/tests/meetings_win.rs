//! Windows meetings end to end through the core (S3.5b), without a desktop: the core's Windows
//! capture ([`WinMeetingCapture`]) over replay sources where WASAPI would open the routed mic and
//! the far end, and a detector that says what the audio session manager says (an executable that
//! holds the mic, and its process). Detection offers, the user answers, the far end is planned as
//! on Windows, the meeting records, stops, and its final pass writes the record. This rig
//! registers only the mock speech engine, so the far end stays one voice and there is no summary,
//! said: what a core without a diarizer or a language model does, on any OS. Whether Windows'
//! registry lists a diarizer is its build's (`engine-nemo`), not this test's.
//!
//! Real devices, real calls, the Drop on screen and a `kill -9` of the app are the maintainer's
//! checklist (`windows/S3.5b-CHECKLIST.md`); a killed core finishing its meeting at the next
//! launch runs on Windows too, in `crash_recovery.rs`.
#![cfg(windows)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use ink_audio::{FileReplaySource, Pacing};
use ink_core::mock::MockClock;
use ink_core::{
    AppRef, AudioSource, Channel, Clock, DeviceId, DeviceInfo, EventSink, FarEndTarget,
    MeetingDetector, MeetingSignal, PlatformError, RecordId, Transport,
};
use ink_engines::{ModelDir, Registry};
use ink_ffi::capture::{FarHears, WinDevices, WinMeetingCapture};
use ink_ffi::devices::{InputChoice, MicReason, OutputChoice, Picked};
use ink_ffi::runtime::{Core, MeetingPlatform, Parts};

const WAIT: Duration = Duration::from_secs(30);

/// Where WASAPI would open devices, two WAV files replayed in real time. It records what it was
/// asked for; Zoom can be made not to run, and the far end's output made to change.
struct ReplayDevices {
    mic: PathBuf,
    far: PathBuf,
    clock: Arc<dyn Clock>,
    asked: Mutex<Vec<String>>,
    zoom_gone: AtomicBool,
    /// What `far_moved` answers: the call now plays on another output.
    moved: AtomicBool,
    /// Each `far_moved` question: the output it was asked about.
    moves_asked: Mutex<Vec<String>>,
    /// Device-loopback far ends opened so far: each is on the output "out-N".
    outputs: AtomicUsize,
}

impl ReplayDevices {
    fn replay(&self, path: &Path, channel: Channel) -> Result<Box<dyn AudioSource>, PlatformError> {
        let source = FileReplaySource::open(path, channel, self.clock.clone())
            .map_err(|e| PlatformError::Device(e.to_string()))?
            .with_pacing(Pacing::RealTime);
        Ok(Box::new(source))
    }

    fn ask(&self, what: String) {
        self.asked.lock().unwrap().push(what);
    }

    /// A device-loopback far end on the next output.
    fn on_device(&self) -> FarHears {
        let n = self.outputs.fetch_add(1, Ordering::Relaxed);
        FarHears::Device {
            id: format!("out-{n}"),
            name: format!("Speakers {n}"),
        }
    }
}

/// The capture holds its devices; the test keeps a handle to read what they were asked.
struct Devices(Arc<ReplayDevices>);

impl WinDevices for Devices {
    fn pick_mic(&self, choice: &InputChoice) -> Result<Picked, String> {
        let what = match choice {
            InputChoice::Auto => "auto",
            InputChoice::Device(_) => "a device",
        };
        self.0.ask(format!("pick {what}"));
        Ok(Picked {
            device: DeviceInfo {
                id: DeviceId("{0.0.1.00000000}.{usb-mic}".into()),
                name: "Microphone (USB Audio)".into(),
                transport: Transport::Usb,
                is_default: true,
            },
            reason: MicReason::Auto(ink_core::AutoReason::DefaultInput),
            wanted: None,
        })
    }

    fn open_mic(&self, device: &DeviceId) -> Result<Box<dyn AudioSource>, PlatformError> {
        self.0.ask(format!("mic {}", device.0));
        self.0.replay(&self.0.mic, Channel::Mic)
    }

    /// No output is chosen in these tests: the default.
    fn pinned_output(&self, _: &OutputChoice) -> Result<Option<String>, PlatformError> {
        Ok(None)
    }

    fn open_far(
        &self,
        target: &FarEndTarget,
        _pinned: Option<&str>,
    ) -> Result<(Box<dyn AudioSource>, FarHears), PlatformError> {
        let far = || self.0.replay(&self.0.far, Channel::Far);
        match target {
            FarEndTarget::AllOutput => {
                self.0.ask("far: the default output".into());
                Ok((far()?, self.0.on_device()))
            }
            FarEndTarget::Apps(apps) => {
                let [app] = apps.as_slice() else {
                    return Err(PlatformError::Failed("one app at a time".into()));
                };
                self.0.ask(format!("far: {} (pid {:?})", app.id, app.pid));
                // S0.4's plan: Zoom and the browsers alone (process loopback), every other app
                // on its output device (device loopback).
                if app.id.eq_ignore_ascii_case("Zoom.exe") {
                    if self.0.zoom_gone.load(Ordering::Relaxed) {
                        return Err(PlatformError::Device("Zoom.exe is not running".into()));
                    }
                    return Ok((far()?, FarHears::App));
                }
                Ok((far()?, self.0.on_device()))
            }
        }
    }

    fn far_moved(
        &self,
        _: &FarEndTarget,
        endpoint: &str,
        _pinned: Option<&str>,
    ) -> Result<bool, PlatformError> {
        self.0.moves_asked.lock().unwrap().push(endpoint.to_owned());
        Ok(self.0.moved.swap(false, Ordering::Relaxed))
    }
}

/// A detector the test drives, as the audio session manager's would report.
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

/// An app as Windows' detector names it: the executable, the process holding the mic, its stem.
fn exe(id: &str, pid: u32) -> AppRef {
    AppRef {
        id: id.into(),
        pid: Some(pid),
        name: id.trim_end_matches(".exe").into(),
    }
}

struct Rig {
    core: Core,
    events: Arc<Recorder>,
    devices: Arc<ReplayDevices>,
    detector: Arc<FakeDetector>,
    clock: Arc<MockClock>,
    _dir: TempDir,
}

fn rig(label: &str, seconds: f64) -> Rig {
    let dir = TempDir::new(label);
    let clock = Arc::new(MockClock::new(10_000_000_000, 1_790_146_800_000));
    let (mic, far) = (dir.path().join("mic.wav"), dir.path().join("far.wav"));
    speech_wav(&mic, seconds, 51);
    speech_wav(&far, seconds, 52);
    let loader = MockLoader::new(Behaviour::Say("words from the final pass".into()));
    let models = ModelDir::new(dir.path().join("models"));
    let row = test_row(ROW_ID);
    install(&models, &row);
    let devices = Arc::new(ReplayDevices {
        mic,
        far,
        clock: clock.clone(),
        asked: Mutex::default(),
        zoom_gone: AtomicBool::new(false),
        moved: AtomicBool::new(false),
        moves_asked: Mutex::default(),
        outputs: AtomicUsize::new(0),
    });
    let detector = Arc::new(FakeDetector::default());
    let parts = Parts {
        store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
        clock: clock.clone(),
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
            capture: Arc::new(WinMeetingCapture::new(Devices(devices.clone()))),
            detector: Some(detector.clone()),
        },
    };
    let (core, events) = start_parts(parts);
    let listening = events
        .wait_for(WAIT, |v| v["type"] == "meeting.detection")
        .expect("detection says whether it listens");
    assert_eq!(listening["listening"], true);
    Rig {
        core,
        events,
        devices,
        detector,
        clock,
        _dir: dir,
    }
}

impl Rig {
    /// A signal, given time to be taken before the mock clock moves on.
    fn signal(&self, signal: MeetingSignal) {
        self.detector.signal(signal);
        std::thread::sleep(Duration::from_millis(150));
    }

    /// `app` takes the mic and holds it past the offer's 3 s: the core offers it.
    fn offered(&self, app: AppRef, offers: usize) -> serde_json::Value {
        self.signal(MeetingSignal::MicInUse { app });
        self.clock.advance_ns(3_500_000_000);
        // Any signal makes the meetings thread judge what is pending at the mock clock's time.
        self.signal(MeetingSignal::MicReleased {
            app: exe("nobody.exe", 1),
        });
        assert!(self.events.wait_count("meeting.detected", offers, WAIT));
        self.events
            .all()
            .into_iter()
            .filter(|v| v["type"] == "meeting.detected")
            .nth(offers - 1)
            .unwrap()
    }

    fn asked(&self) -> Vec<String> {
        self.devices.asked.lock().unwrap().clone()
    }
}

/// Teams: offered by its executable, recorded when the user says so, its far end the output device
/// it plays to (said as everything), stopped by hand into a final pass that writes the record.
#[test]
fn a_teams_call_is_offered_recorded_and_blotted_into_a_record() {
    let r = rig("win-teams", 60.0);
    let offer = r.offered(exe("ms-teams.exe", 300), 1);
    assert_eq!(offer["app"], "ms-teams.exe");
    assert_eq!(offer["app_name"], "ms-teams");

    r.core
        .command(r#"{"cmd":"meeting.start","app":"ms-teams.exe","id":"meeting.start"}"#)
        .unwrap();
    let started = r.events.wait_type("meeting.started", WAIT);
    assert_eq!(started["app"], "ms-teams.exe");
    assert_eq!(started["mic_name"], "Microphone (USB Audio)");
    assert_eq!(started["mic_transport"], "usb");
    assert_eq!(started["mic_reason"], "default_input");
    assert_eq!(
        started["far_end"], "everything",
        "device loopback hears everything its device plays"
    );
    assert_eq!(r.events.count("meeting.far_end_fallback"), 0, "by plan");
    assert_eq!(
        r.asked(),
        [
            "pick auto",
            "mic {0.0.1.00000000}.{usb-mic}",
            // The offer's own process reached the plan.
            "far: ms-teams.exe (pid Some(300))",
        ]
    );

    std::thread::sleep(Duration::from_millis(2_000));
    r.core
        .command(r#"{"cmd":"meeting.stop","id":"meeting.stop"}"#)
        .unwrap();
    r.events.wait_type("meeting.stopped", WAIT);
    let finished = r.events.wait_type("meeting.finished", WAIT);
    assert_eq!(finished["record"], started["record"]);
    assert_eq!(finished["revision"], 2, "the final pass wrote the record");

    // No diarizer row in this rig: the far end stays one voice, and nothing says labels were
    // tried.
    assert_eq!(r.events.count("meeting.diarized"), 0);
    let record = RecordId(finished["record"].as_str().unwrap().to_owned());
    let segments = r.core.shared().store.segments(&record).unwrap();
    assert!(
        segments.iter().any(|s| s.channel == Channel::Far),
        "{segments:?}"
    );
    assert!(segments.iter().all(|s| s.speaker.is_none()), "{segments:?}");
    // Nor a language model: no summary, and the pass says so.
    assert!(
        r.events
            .all()
            .iter()
            .any(|v| v["type"] == "meeting.warning" && v["kind"] == "summary_unavailable"),
        "{:?}",
        r.events.types()
    );
    r.events.assert_valid();
    r.core.shutdown();
}

/// Zoom is heard alone (process loopback); a Zoom that is gone by the time the user answers is
/// recorded from the default output instead, and that is said.
#[test]
fn zoom_is_heard_alone_and_a_zoom_that_is_gone_falls_back_and_says_so() {
    let r = rig("win-zoom", 60.0);
    r.offered(exe("Zoom.exe", 210), 1);
    r.core
        .command(r#"{"cmd":"meeting.start","app":"Zoom.exe"}"#)
        .unwrap();
    let started = r.events.wait_type("meeting.started", WAIT);
    assert_eq!(started["far_end"], "app", "the call's own sound alone");
    std::thread::sleep(Duration::from_millis(500));
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    r.events.wait_type("meeting.finished", WAIT);

    // The next call: Zoom's process is gone when the answer comes.
    r.signal(MeetingSignal::MicReleased {
        app: exe("Zoom.exe", 210),
    });
    r.devices.zoom_gone.store(true, Ordering::Relaxed);
    r.offered(exe("Zoom.exe", 211), 2);
    r.core
        .command(r#"{"cmd":"meeting.start","app":"Zoom.exe"}"#)
        .unwrap();
    assert!(r.events.wait_count("meeting.started", 2, WAIT));
    let fallback = r.events.wait_type("meeting.far_end_fallback", WAIT);
    // The identity as the core keeps it on Windows: lowercased where it came in.
    assert_eq!(fallback["app"], "zoom.exe");
    assert!(
        fallback["message"]
            .as_str()
            .unwrap()
            .contains("Zoom.exe is not running"),
        "{fallback}"
    );
    let second = r
        .events
        .all()
        .into_iter()
        .filter(|v| v["type"] == "meeting.started")
        .nth(1)
        .unwrap();
    assert_eq!(second["far_end"], "everything");
    assert_eq!(
        r.asked()[r.asked().len() - 2..],
        ["far: zoom.exe (pid Some(211))", "far: the default output"]
    );
    std::thread::sleep(Duration::from_millis(500));
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    assert!(r.events.wait_count("meeting.finished", 2, WAIT));
    r.events.assert_valid();
    r.core.shutdown();
}

/// A headset plugged in mid-call (S3.5b): Teams now plays on another output, so its far end, a
/// device loopback, is opened again for Teams and goes on; the echo search starts again, and
/// nothing is said to the user because nothing was lost. Zoom (process loopback) is never asked.
#[test]
fn a_teams_call_whose_output_changes_is_followed() {
    let r = rig("win-follow", 60.0);
    r.offered(exe("ms-teams.exe", 300), 1);
    r.core
        .command(r#"{"cmd":"meeting.start","app":"ms-teams.exe"}"#)
        .unwrap();
    r.events.wait_type("meeting.started", WAIT);
    r.devices.moved.store(true, Ordering::Relaxed);
    let switched = r.events.wait_for(WAIT, |v| {
        v["type"] == "meeting.echo" && v["state"] == "searching" && v["why"] == "device_switch"
    });
    assert!(switched.is_some(), "{:?}", r.events.types());
    let far_opens: Vec<String> = r
        .asked()
        .into_iter()
        .filter(|a| a.starts_with("far: "))
        .collect();
    assert_eq!(
        far_opens,
        [
            "far: ms-teams.exe (pid Some(300))",
            "far: ms-teams.exe (pid Some(300))"
        ],
        "opened again for Teams, where it plays now"
    );
    assert_eq!(r.devices.moves_asked.lock().unwrap()[0], "out-0");
    std::thread::sleep(Duration::from_millis(2_500));
    assert_eq!(
        r.devices.moves_asked.lock().unwrap().last().unwrap(),
        "out-1",
        "asked about its new output from then on"
    );
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    r.events.wait_type("meeting.finished", WAIT);
    assert!(
        !r.events
            .all()
            .iter()
            .any(|v| v["type"] == "meeting.warning" && v["kind"] == "capture"),
        "{:?}",
        r.events.types()
    );
    r.events.assert_valid();
    r.core.shutdown();
}

/// The shell may name an app in any case: meeting.dismiss and meeting.start find the offer by the
/// identity as the core keeps it on Windows (lowercased), and every event says it so; the name is
/// the one detection gave.
#[test]
fn an_offered_app_is_answered_whatever_case_the_shell_names_it_in() {
    let r = rig("win-case", 60.0);
    let offered = r.offered(exe("Zoom.exe", 210), 1);
    assert_eq!(offered["app"], "zoom.exe");
    assert_eq!(offered["app_name"], "Zoom");
    r.core
        .command(r#"{"cmd":"meeting.dismiss","app":"ZOOM.EXE","id":"d"}"#)
        .unwrap();
    let ended = r.events.wait_type("meeting.detection_ended", WAIT);
    assert_eq!(ended["app"], "zoom.exe");
    assert_eq!(ended["dismissed"], true, "{ended}");

    // The next call, started (meeting.start) in yet another case.
    r.signal(MeetingSignal::MicReleased {
        app: exe("Zoom.exe", 210),
    });
    r.offered(exe("Zoom.exe", 211), 2);
    r.core
        .command(r#"{"cmd":"meeting.start","app":"ZOOM.exe"}"#)
        .unwrap();
    let started = r.events.wait_type("meeting.started", WAIT);
    assert_eq!(started["app"], "zoom.exe");
    assert_eq!(started["app_name"], "Zoom", "the offer's own: it was found");
    assert_eq!(started["far_end"], "app");
    std::thread::sleep(Duration::from_millis(500));
    r.core.command(r#"{"cmd":"meeting.stop"}"#).unwrap();
    r.events.wait_type("meeting.finished", WAIT);
    r.events.assert_valid();
    r.core.shutdown();
}
