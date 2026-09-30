//! A device that changes under a meeting (S3.5b). A side that can move ([`Follow`], Windows'
//! device loopback) is opened again where it should be and goes on in the same chunks, with the
//! echo search started again. A source that ends by itself is said with its reason. A side left
//! with no source is lost: its silence is then said within the watchdog's limit, never taken for
//! quiet, until it comes back. Replays and scripted sources stand in for the devices; the moves
//! themselves (a headset plugged in mid-call) are the maintainer's checklist
//! (`windows/S3.5b-CHECKLIST.md`).

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use ink_audio::{DEFAULT_RING_DURATION, FileReplaySource, Pacing};
use ink_core::mock::MockClock;
use ink_core::{AudioSink, AudioSource, Channel, Clock, PlatformError, SourceStats, StreamFormat};
use ink_engines::{ModelDir, Registry};
use ink_ffi::capture::{Follow, Reopened};
use ink_ffi::meeting::{CaptureSide, MeetingInfo};
use ink_ffi::runtime::{Core, Parts};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(30);

struct Rig {
    core: Core,
    events: Arc<Recorder>,
    clock: Arc<MockClock>,
    mic: PathBuf,
    far: PathBuf,
    far2: PathBuf,
    _dir: TempDir,
}

fn rig(label: &str) -> Rig {
    let dir = TempDir::new(label);
    let clock = Arc::new(MockClock::new(10_000_000_000, 1_790_146_800_000));
    let [mic, far, far2] = ["mic.wav", "far.wav", "far2.wav"].map(|f| dir.path().join(f));
    speech_wav(&mic, 30.0, 61);
    speech_wav(&far, 30.0, 62);
    speech_wav(&far2, 30.0, 63);
    let loader = MockLoader::new(Behaviour::Say("words from the final pass".into()));
    let models = ModelDir::new(dir.path().join("models"));
    let row = test_row(ROW_ID);
    install(&models, &row);
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
        meetings: Default::default(),
    };
    let (core, events) = start_parts(parts);
    Rig {
        core,
        events,
        clock,
        mic,
        far,
        far2,
        _dir: dir,
    }
}

fn replay(path: &Path, channel: Channel, clock: &Arc<MockClock>) -> Box<dyn AudioSource> {
    let clock: Arc<dyn Clock> = clock.clone();
    Box::new(
        FileReplaySource::open(path, channel, clock)
            .unwrap()
            .with_pacing(Pacing::RealTime),
    )
}

fn side(source: Box<dyn AudioSource>, follow: Option<Box<dyn Follow>>) -> CaptureSide {
    CaptureSide {
        source,
        ring: DEFAULT_RING_DURATION,
        start_at: None,
        follow,
    }
}

/// What the scripted follow answers next.
#[derive(Clone)]
enum Plan {
    Stay,
    /// Move once to a replay of this file, said as this device; then stay.
    MoveTo(PathBuf, &'static str),
    Fail(&'static str),
}

/// A [`Follow`] the test drives. It records each ask's `again`.
struct Scripted {
    plan: Arc<Mutex<Plan>>,
    asked: Arc<Mutex<Vec<bool>>>,
    clock: Arc<MockClock>,
}

impl Follow for Scripted {
    fn moved(&mut self, again: bool) -> Result<Option<Reopened>, String> {
        self.asked.lock().unwrap().push(again);
        let mut plan = self.plan.lock().unwrap();
        match plan.clone() {
            Plan::Stay => Ok(None),
            Plan::MoveTo(path, name) => {
                *plan = Plan::Stay;
                Ok(Some((
                    replay(&path, Channel::Far, &self.clock),
                    name.into(),
                )))
            }
            Plan::Fail(why) => Err(why.into()),
        }
    }
}

/// A device that delivers nothing and, once `gone` is set, has ended by itself; its stop then
/// says why, as WASAPI's does after `AUDCLNT_E_DEVICE_INVALIDATED`.
struct Unplugged {
    gone: Arc<AtomicBool>,
    sink: Option<Box<dyn AudioSink>>,
}

impl AudioSource for Unplugged {
    fn channel(&self) -> Channel {
        Channel::Far
    }

    fn format(&self) -> StreamFormat {
        StreamFormat::CANONICAL
    }

    fn start(&mut self, sink: Box<dyn AudioSink>) -> Result<(), PlatformError> {
        self.sink = Some(sink);
        Ok(())
    }

    fn stop(&mut self) -> Result<SourceStats, PlatformError> {
        // Dropping the sink, as every source's stop does.
        if self.sink.take().is_some() && self.gone.load(Ordering::Relaxed) {
            return Err(PlatformError::Device(
                "far end: the test output was removed".into(),
            ));
        }
        Ok(SourceStats::default())
    }

    fn ended(&self) -> bool {
        self.sink.is_some() && self.gone.load(Ordering::Relaxed)
    }
}

fn is(v: &Value, ty: &str, pairs: &[(&str, &str)]) -> bool {
    v["type"] == ty && pairs.iter().all(|(k, want)| v[*k] == *want)
}

/// The echo searches started again for a new device (a move, or a side lost).
fn switches(events: &Recorder) -> usize {
    events
        .all()
        .iter()
        .filter(|v| {
            is(
                v,
                "meeting.echo",
                &[("state", "searching"), ("why", "device_switch")],
            )
        })
        .count()
}

fn far_warnings(events: &Recorder) -> Vec<Value> {
    events
        .all()
        .into_iter()
        .filter(|v| {
            is(
                v,
                "meeting.warning",
                &[("kind", "capture"), ("channel", "far")],
            )
        })
        .collect()
}

/// Stops the meeting and returns the far side's `captured_ms` from its final pass.
fn stop(r: &Rig) -> u64 {
    r.core
        .command(r#"{"cmd":"meeting.stop","id":"stop"}"#)
        .unwrap();
    r.events.wait_type("meeting.finished", WAIT);
    r.events
        .all()
        .into_iter()
        .find(|v| v["type"] == "meeting.transcribed" && v["pass"]["channel"] == "far")
        .expect("the far end's pass")["pass"]["captured_ms"]
        .as_u64()
        .unwrap()
}

/// A headset plugged in mid-call: asked after the interval, the far end moves to the replay that
/// stands for the new output, goes on in the same ring and chunks, and the echo search starts
/// again (a new device is a new echo path). Nothing is said to the user: nothing was lost.
#[test]
fn a_far_end_that_moves_goes_on_in_the_same_recording() {
    let r = rig("follow-moves");
    let asked = Arc::new(Mutex::default());
    let follow = Scripted {
        plan: Arc::new(Mutex::new(Plan::MoveTo(r.far2.clone(), "Test Headset"))),
        asked: asked.clone(),
        clock: r.clock.clone(),
    };
    r.core
        .start_meeting(
            vec![
                side(replay(&r.mic, Channel::Mic, &r.clock), None),
                side(
                    replay(&r.far, Channel::Far, &r.clock),
                    Some(Box::new(follow)),
                ),
            ],
            MeetingInfo::default(),
        )
        .unwrap();
    r.events.wait_type("meeting.started", WAIT);
    let switched = r.events.wait_for(WAIT, |v| {
        is(
            v,
            "meeting.echo",
            &[("state", "searching"), ("why", "device_switch")],
        )
    });
    assert!(switched.is_some(), "{:?}", r.events.types());
    assert!(!asked.lock().unwrap()[0], "asked, not told to reopen");
    std::thread::sleep(Duration::from_millis(2_000));
    let far_ms = stop(&r);
    // About 2 s from the first source, then about 2 s more from the second: the ring's sink went
    // on to the new source.
    assert!(far_ms >= 3_000, "far captured {far_ms} ms");
    assert!(
        far_warnings(&r.events).is_empty(),
        "{:?}",
        far_warnings(&r.events)
    );
    assert_eq!(r.events.count("meeting.capture_failed"), 0);
    r.events.assert_valid();
    r.core.shutdown();
}

/// A far end that ends by itself with nothing to open it again (process loopback on Windows): its
/// end is said with the platform's reason at once, and from then on the watchdog expects audio
/// from it, so its silence is said as stopped within the limit. Before it ended, the same silence
/// was a far end with nothing playing.
#[test]
fn a_far_end_that_ends_by_itself_is_said_and_its_silence_is_judged() {
    let r = rig("follow-ends");
    let gone = Arc::new(AtomicBool::new(false));
    r.core
        .start_meeting(
            vec![
                side(replay(&r.mic, Channel::Mic, &r.clock), None),
                side(
                    Box::new(Unplugged {
                        gone: gone.clone(),
                        sink: None,
                    }),
                    None,
                ),
            ],
            MeetingInfo::default(),
        )
        .unwrap();
    r.events.wait_type("meeting.started", WAIT);
    let far_stopped = |v: &Value| {
        is(
            v,
            "meeting.side_state",
            &[("channel", "far"), ("state", "stopped")],
        )
    };
    // Nothing plays: past the limit, still not a failure.
    r.clock.advance_ns(11_000_000_000);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!r.events.all().iter().any(far_stopped));

    gone.store(true, Ordering::Relaxed);
    let warned = r
        .events
        .wait_for(WAIT, |v| {
            is(
                v,
                "meeting.warning",
                &[("kind", "capture"), ("channel", "far")],
            )
        })
        .expect("said");
    assert!(
        warned["message"]
            .as_str()
            .unwrap()
            .contains("the test output was removed"),
        "{warned}"
    );
    r.clock.advance_ns(11_000_000_000);
    assert!(
        r.events.wait_for(WAIT, far_stopped).is_some(),
        "{:?}",
        r.events.types()
    );
    stop(&r);
    assert_eq!(
        far_warnings(&r.events).len(),
        1,
        "said once, not again at stop"
    );
    r.events.assert_valid();
    r.core.shutdown();
}

/// A movable far end whose device goes, with nowhere to open it again: opened again at once,
/// which fails, said once with both reasons; lost, so its silence is said; tried again every
/// interval, and when an output comes back the far end records again and reads as well.
#[test]
fn a_far_end_that_cannot_be_opened_again_is_lost_until_it_comes_back() {
    let r = rig("follow-lost");
    let gone = Arc::new(AtomicBool::new(false));
    let plan = Arc::new(Mutex::new(Plan::Fail(
        "the other side's sound: no output device",
    )));
    let asked = Arc::new(Mutex::default());
    let follow = Scripted {
        plan: plan.clone(),
        asked: asked.clone(),
        clock: r.clock.clone(),
    };
    r.core
        .start_meeting(
            vec![
                side(replay(&r.mic, Channel::Mic, &r.clock), None),
                side(
                    Box::new(Unplugged {
                        gone: gone.clone(),
                        sink: None,
                    }),
                    Some(Box::new(follow)),
                ),
            ],
            MeetingInfo::default(),
        )
        .unwrap();
    r.events.wait_type("meeting.started", WAIT);
    gone.store(true, Ordering::Relaxed);
    let warned = r
        .events
        .wait_for(WAIT, |v| {
            is(
                v,
                "meeting.warning",
                &[("kind", "capture"), ("channel", "far")],
            )
        })
        .expect("said");
    let message = warned["message"].as_str().unwrap();
    assert!(message.contains("the test output was removed"), "{message}");
    assert!(message.contains("no output device"), "{message}");
    assert!(asked.lock().unwrap()[0], "opened again at once");

    r.clock.advance_ns(11_000_000_000);
    let far = |state: &'static str| {
        move |v: &Value| {
            is(
                v,
                "meeting.side_state",
                &[("channel", "far"), ("state", state)],
            )
        }
    };
    assert!(r.events.wait_for(WAIT, far("stopped")).is_some());

    // Still failing at the next tries: not said again.
    std::thread::sleep(Duration::from_millis(4_500));
    assert!(asked.lock().unwrap().len() >= 2);
    assert_eq!(far_warnings(&r.events).len(), 1);

    let before = switches(&r.events);
    *plan.lock().unwrap() = Plan::MoveTo(r.far2.clone(), "Test Speakers");
    assert!(
        r.events.wait_for(WAIT, far("ok")).is_some(),
        "{:?}",
        r.events.types()
    );
    assert!(switches(&r.events) > before, "a new echo path");
    std::thread::sleep(Duration::from_millis(1_000));
    let far_ms = stop(&r);
    assert!(far_ms > 500, "it recorded again: {far_ms} ms");
    assert_eq!(far_warnings(&r.events).len(), 1);
    r.events.assert_valid();
    r.core.shutdown();
}
