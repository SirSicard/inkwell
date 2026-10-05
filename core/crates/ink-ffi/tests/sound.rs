//! Settings > Sound through the core: `audio.devices` and the `audio.input` / `audio.output`
//! settings, the coalesced device watch (`audio.devices_changed`), dictation letting go of its
//! idle mic when the mic it would open changes, the chosen mic's fallback said once per spell, the
//! mic test, and a meeting that opens its mic again only when its own mic goes
//! (`meeting.mic_switched`). The platform is the mock one, with scripted plugs and unplugs.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use ink_audio::{DEFAULT_RING_DURATION, FileReplaySource, Pacing};
use ink_core::mock::MockPlatform;
use ink_core::{AudioSource, CaptureControl, Channel, Clock, DeviceId, DeviceInfo, Transport};
use ink_engines::{ModelDir, Registry};
use ink_ffi::capture::{Follow, FollowMic, MicInfo};
use ink_ffi::devices::{self, Choices, InputChoice};
use ink_ffi::meeting::{CaptureSide, MeetingInfo};
use ink_ffi::runtime::{Core, Parts};
use ink_ffi::voice::VoicePlatform;
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(10);
const MS: u64 = 1_000_000;
/// The most mock time [`Rig::released`] moves on: past the grace after a press, never near
/// [`MIC_IDLE`](ink_ffi::voice::MIC_IDLE).
const RELEASE_BUDGET: u64 = 5_000 * MS;
const _: () = assert!(RELEASE_BUDGET < ink_ffi::voice::MIC_IDLE.as_nanos() as u64 / 10);

fn device(id: &str, name: &str, transport: Transport, is_default: bool) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.into()),
        name: name.into(),
        transport,
        is_default,
    }
}

fn built_in() -> DeviceInfo {
    device("built-in", "Built-in Microphone", Transport::BuiltIn, true)
}

fn usb(id: &str) -> DeviceInfo {
    device(id, "USB Microphone", Transport::Usb, false)
}

fn buds() -> DeviceInfo {
    device("buds", "Example Buds", Transport::Bluetooth, false)
}

struct Rig {
    core: Option<Core>,
    events: Arc<Recorder>,
    platform: Arc<MockPlatform>,
    store: Arc<dyn ink_core::Store>,
    _dir: TempDir,
}

impl Rig {
    /// A core over `platform`, given to it as dictation's (and so Sound's) platform unless
    /// `with_platform` is false.
    fn new(label: &str, platform: MockPlatform, with_platform: bool) -> Self {
        let dir = TempDir::new(label);
        let platform = Arc::new(platform);
        let row = test_row("test-asr");
        let models = ModelDir::new(dir.path().join("models"));
        install(&models, &row);
        let loader = MockLoader::new(Behaviour::Say("hello world".into()));
        let store: Arc<dyn ink_core::Store> =
            Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
        let parts = Parts {
            store: store.clone(),
            clock: platform.clock(),
            registry: Registry::new(vec![row]).unwrap(),
            models,
            loader: loader.clone(),
            installer: Arc::new(MockInstaller {
                generation: loader.generation.clone(),
                gate: None,
                installs: Default::default(),
            }),
            data_dir: dir.path().to_owned(),
            permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
            meetings: Default::default(),
        };
        let (core, events) = start_parts(parts);
        if with_platform {
            core.set_voice_platform(VoicePlatform {
                capture: platform.clone(),
                keys: platform.clone(),
                edit_keys: Arc::new(MockPlatform::new()),
                inserter: platform.clone(),
                focus: platform.clone(),
            });
        }
        platform.clock().advance_ns(1_000 * MS);
        Self {
            core: Some(core),
            events,
            platform,
            store,
            _dir: dir,
        }
    }

    fn core(&self) -> &Core {
        self.core.as_ref().unwrap()
    }

    /// Sends `json` with id `id` and waits for the first event answering it.
    fn ask(&self, json: &str, id: &str) -> Value {
        self.core().command(json).unwrap();
        self.events
            .wait_for(WAIT, |v| v["ref"] == id || v["id"] == id)
            .unwrap_or_else(|| panic!("no answer to {json}: {:?}", self.events.types()))
    }

    /// `setting.set`: its `setting.value` (which carries no `ref`), or the `command.failed`.
    fn set(&self, key: &str, value: &str, id: &str) -> Value {
        let before = self.events.count("setting.value");
        self.core()
            .command(&format!(
                r#"{{"cmd":"setting.set","key":"{key}","value":"{value}","id":"{id}"}}"#
            ))
            .unwrap();
        let until = Instant::now() + WAIT;
        loop {
            let all = self.events.all();
            if let Some(failed) = all
                .iter()
                .find(|v| v["type"] == "command.failed" && v["id"] == id)
            {
                return failed.clone();
            }
            if let Some(value) = all
                .iter()
                .filter(|v| v["type"] == "setting.value")
                .nth(before)
            {
                return value.clone();
            }
            assert!(
                Instant::now() < until,
                "no answer to setting.set {key}: {:?}",
                self.events.types()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn devices(&self, id: &str) -> Value {
        let v = self.ask(&format!(r#"{{"cmd":"audio.devices","id":"{id}"}}"#), id);
        assert_eq!(v["type"], "audio.devices", "{v}");
        v
    }

    /// Moves the mock clock on until the sound thread has read the burst and said the
    /// `n`th `audio.devices_changed` (the thread may take a notification after a move, so one
    /// move is not always enough).
    fn settled(&self, n: usize) -> Value {
        let until = Instant::now() + WAIT;
        while self.events.count("audio.devices_changed") < n {
            assert!(Instant::now() < until, "{:?}", self.events.types());
            self.platform.clock().advance_ns(400 * MS);
            std::thread::sleep(Duration::from_millis(50));
        }
        self.events
            .all()
            .into_iter()
            .filter(|v| v["type"] == "audio.devices_changed")
            .nth(n - 1)
            .unwrap()
    }

    fn mic_open(&self) -> bool {
        self.platform
            .feed(Channel::Mic, &[0.0; 1], self.platform.clock().now_ns())
    }

    fn until(&self, what: &str, f: impl Fn() -> bool) {
        let until = Instant::now() + WAIT;
        while !f() {
            assert!(Instant::now() < until, "{what}: {:?}", self.events.types());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Moves the mock clock on (past [`STALE_GRACE`](ink_ffi::voice::STALE_GRACE) after the last
    /// press) until dictation has let go of its idle mic.
    ///
    /// At most [`RELEASE_BUDGET`] of mock time, far short of the minute without a take that lets
    /// an idle mic go anyway: what lets it go here is the device change.
    fn released(&self) {
        let until = Instant::now() + WAIT;
        let mut moved = 0;
        while self.mic_open() {
            assert!(
                Instant::now() < until,
                "the idle mic is never let go of: {:?}",
                self.events.types()
            );
            if moved < RELEASE_BUDGET {
                self.platform.clock().advance_ns(200 * MS);
                moved += 200 * MS;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn enable_dictation(&self) {
        let v = self.ask(r#"{"cmd":"dictation.enable","id":"on"}"#, "on");
        assert_eq!(v["type"], "dictation.ready", "{v}");
    }

    /// A press and its release: the mic opens and stays open, idle.
    fn tap(&self) {
        assert!(self.platform.press());
        self.until("the mic opens", || self.mic_open());
        assert!(self.platform.release());
    }

    fn opens(&self) -> Vec<String> {
        self.platform.mic_opens().into_iter().map(|d| d.0).collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(core) = self.core.take() {
            core.shutdown();
        }
    }
}

fn windows_like() -> MockPlatform {
    MockPlatform::new()
        .with_devices(vec![built_in(), usb("usb")], None)
        .with_outputs(Some(vec![
            device("speakers", "Speakers", Transport::BuiltIn, true),
            device("dock", "Dock Audio", Transport::Usb, false),
        ]))
}

/// The answer names the devices, the choice, Automatic's pick and the mic in use and why; a
/// device is set only while it is connected, remembered by name, and found by name on a new id.
#[test]
fn audio_devices_lists_the_choice_and_what_records_now() {
    let rig = Rig::new("sound-devices", windows_like(), true);
    let v = rig.devices("d1");
    assert_eq!(v["inputs"].as_array().unwrap().len(), 2);
    assert_eq!(v["inputs"][0]["id"], "built-in");
    assert_eq!(v["inputs"][0]["is_default"], true);
    assert_eq!(v["input"], "auto");
    assert!(v.get("wanted").is_none());
    assert_eq!(v["automatic"]["name"], "Built-in Microphone");
    assert_eq!(v["automatic"]["reason"], "default_input");
    assert_eq!(v["using"]["reason"], "default_input");
    assert_eq!(v["outputs"].as_array().unwrap().len(), 2);
    assert_eq!(v["output"], "default");
    assert_eq!(v["output_using"]["reason"], "default_output");

    let set = rig.set("audio.input", "usb", "s1");
    assert_eq!(set["type"], "setting.value", "{set}");
    assert_eq!(set["value"], "usb");
    let v = rig.devices("d2");
    assert_eq!(v["input"], "usb");
    assert_eq!(v["wanted"]["name"], "USB Microphone");
    assert_eq!(v["wanted"]["transport"], "usb");
    assert_eq!(
        (v["using"]["id"].as_str(), v["using"]["reason"].as_str()),
        (Some("usb"), Some("chosen"))
    );
    assert_eq!(v["automatic"]["id"], "built-in", "Automatic is still shown");

    // Not connected: refused, and the choice stays. Not a device token (two lines): refused
    // as the command is read, before it is queued.
    let failed = rig.set("audio.input", "nope", "s2");
    assert_eq!(failed["type"], "command.failed", "{failed}");
    let err = rig
        .core()
        .command(r#"{"cmd":"setting.set","key":"audio.input","value":"a\nb"}"#)
        .unwrap_err();
    assert!(err.contains("takes one of: auto, <device>"), "{err}");
    assert_eq!(rig.devices("d3")["input"], "usb");

    let set = rig.set("audio.output", "dock", "s4");
    assert_eq!(set["type"], "setting.value", "{set}");
    let v = rig.devices("d4");
    assert_eq!(v["output_using"]["reason"], "chosen");
    assert_eq!(v["output_wanted"]["name"], "Dock Audio");

    // The same mic on another port: a new id, found by its remembered name and transport.
    rig.platform.unplug("usb");
    rig.platform.plug(usb("usb-on-another-port"));
    let v = rig.devices("d5");
    assert_eq!(v["using"]["id"], "usb-on-another-port");
    assert_eq!(v["using"]["reason"], "chosen");
    // Gone altogether: Automatic stands in, said as such.
    rig.platform.unplug("usb-on-another-port");
    let v = rig.devices("d6");
    assert_eq!(v["using"]["id"], "built-in");
    assert_eq!(v["using"]["reason"], "chosen_missing");
    assert_eq!(v["wanted"]["id"], "usb", "the choice is kept");
    rig.events.assert_valid();
}

/// macOS has no output picker: no outputs listed, and audio.output takes only default. Without a
/// platform there is nothing to list.
#[test]
fn without_an_output_picker_or_a_platform_there_is_less_to_list() {
    let rig = Rig::new("sound-mac", MockPlatform::new().with_outputs(None), true);
    let v = rig.devices("d1");
    assert!(v.get("outputs").is_none(), "{v}");
    assert!(v.get("output").is_none());
    let failed = rig.set("audio.output", "speakers", "s1");
    assert_eq!(failed["type"], "command.failed");
    assert!(
        failed["message"].as_str().unwrap().contains("only default"),
        "{failed}"
    );
    assert_eq!(
        rig.set("audio.output", "default", "s2")["type"],
        "setting.value"
    );
    rig.events.assert_valid();
    drop(rig);

    let bare = Rig::new("sound-bare", MockPlatform::new(), false);
    let failed = bare.ask(r#"{"cmd":"audio.devices","id":"d"}"#, "d");
    assert_eq!(failed["type"], "command.failed");
    // Automatic needs no device to be set.
    assert_eq!(
        bare.set("audio.input", "auto", "s")["type"],
        "setting.value"
    );
    assert_eq!(
        bare.set("audio.input", "usb", "s2")["type"],
        "command.failed"
    );
}

/// The platform is watched once it is given; a burst of changes is said once, after it goes
/// quiet; once the core is shut down nothing watches.
#[test]
fn a_burst_of_device_changes_is_said_once() {
    let rig = Rig::new("sound-watch", windows_like(), true);
    rig.until("the platform is watched", || {
        rig.platform.watching_devices()
    });
    rig.platform.plug(buds());
    rig.platform.set_default_output(device(
        "buds-out",
        "Example Buds",
        Transport::Bluetooth,
        true,
    ));
    rig.platform.set_default_input("buds");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        rig.events.count("audio.devices_changed"),
        0,
        "not quiet yet"
    );
    let changed = rig.settled(1);
    assert_eq!(changed["inputs"][0]["id"], "buds");
    assert_eq!(changed["using"]["id"], "buds");
    assert!(changed.get("ref").is_none());
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(rig.events.count("audio.devices_changed"), 1);
    // A choice written is said too.
    rig.set("audio.input", "usb", "s1");
    assert_eq!(rig.settled(2)["input"], "usb");
    // The same platform given again keeps one watcher, the new one.
    rig.core().set_voice_platform(VoicePlatform {
        capture: rig.platform.clone(),
        keys: rig.platform.clone(),
        edit_keys: Arc::new(MockPlatform::new()),
        inserter: rig.platform.clone(),
        focus: rig.platform.clone(),
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(rig.platform.watching_devices());
    rig.platform
        .plug(device("desk", "Desk Microphone", Transport::Usb, false));
    assert_eq!(rig.settled(3)["inputs"].as_array().unwrap().len(), 4);
    rig.events.assert_valid();
    let platform = rig.platform.clone();
    drop(rig);
    assert!(!platform.watching_devices(), "unwatched at shutdown");
}

/// Dictation's idle mic stays open through a change that leaves its pick alone, and is let go of
/// when the mic it would open now is another: the next press opens that one.
#[test]
fn dictation_lets_go_of_its_idle_mic_only_when_the_pick_changes() {
    let rig = Rig::new("sound-dictation", windows_like(), true);
    rig.enable_dictation();
    rig.tap();
    assert_eq!(rig.opens(), ["built-in"]);

    // Another mic plugged in, not the default: Automatic still picks the built-in one.
    rig.platform
        .plug(device("desk", "Desk Microphone", Transport::Usb, false));
    rig.settled(1);
    rig.platform.clock().advance_ns(1_500 * MS);
    std::thread::sleep(Duration::from_millis(100));
    assert!(rig.mic_open(), "the same pick keeps the mic and its lead");

    // Within a second of a press, a change waits: a press may still be on its way to the chain.
    rig.tap();
    rig.set("audio.input", "usb", "s1");
    std::thread::sleep(Duration::from_millis(200));
    assert!(rig.mic_open(), "not within the grace after a press");
    rig.released();
    rig.tap();
    assert_eq!(rig.opens(), ["built-in", "usb"]);
    assert_eq!(rig.events.count("dictation.mic_failed"), 0);
    rig.events.assert_valid();
}

/// A chosen mic that is not connected is said when a mic opens in its place, once per spell: the
/// spell ends when it is seen again, and another absence is a new spell.
#[test]
fn a_missing_chosen_mic_is_said_once_per_spell() {
    let rig = Rig::new(
        "sound-spell",
        MockPlatform::new().with_devices(vec![built_in(), buds()], None),
        true,
    );
    rig.set("audio.input", "buds", "s1");
    rig.enable_dictation();
    rig.platform.unplug("buds");
    rig.settled(1);
    rig.tap();
    let said = rig.events.wait_type("audio.input_fallback", WAIT);
    assert_eq!(said["wanted"]["id"], "buds");
    assert_eq!(said["wanted"]["name"], "Example Buds");
    assert_eq!(said["mic_name"], "Built-in Microphone");

    // Still missing, another stand-in (a new default): not said again.
    rig.platform
        .plug(device("desk", "Desk Microphone", Transport::Usb, true));
    rig.settled(2);
    rig.released();
    rig.tap();
    assert_eq!(rig.opens(), ["built-in", "desk"]);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rig.events.count("audio.input_fallback"), 1);

    // Back: the spell is over, and the idle mic gives way to it.
    rig.platform.plug(buds());
    rig.settled(3);
    rig.released();
    rig.tap();
    assert_eq!(rig.opens(), ["built-in", "desk", "buds"]);
    // Gone again, within a second of that press: the dead mic opens again at once on the
    // stand-in, and that is a new spell.
    rig.platform.unplug("buds");
    assert!(rig.events.wait_count("audio.input_fallback", 2, WAIT));
    assert_eq!(rig.opens(), ["built-in", "desk", "buds", "desk"]);
    rig.events.assert_valid();
}

/// A mic whose device goes is noticed by dictation itself, without the watcher (no clock is moved
/// past a burst's quiet time here, so this is the source's own end). Between takes it is let go
/// of quietly: opened again at once only for a press just made, once per press; a take's end is no
/// press. In a take it ends the take, said.
#[test]
fn dictation_notices_its_mic_going_by_itself() {
    let rig = Rig::new("sound-dead-mic", windows_like(), true);
    rig.enable_dictation();
    let after = |n: usize| {
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(rig.opens().len(), n, "{:?}", rig.opens());
        assert_eq!(
            rig.events.count("dictation.mic_failed"),
            0,
            "nothing to say"
        );
    };

    // More than a second after the last press, nothing held: let go of quietly, not opened
    // again; the next press opens the other mic.
    rig.tap();
    rig.until("the press is over", || {
        rig.events.count("dictation.short_press_ignored") > 0
    });
    rig.platform.clock().advance_ns(1_500 * MS);
    assert!(rig.platform.unplug("built-in"));
    after(1);
    rig.tap();
    assert_eq!(rig.opens(), ["built-in", "usb"]);

    // While a key is held: opened again at once, on what the choice picks now, once.
    rig.platform
        .plug(device("desk", "Desk Microphone", Transport::Usb, false));
    rig.platform.clock().advance_ns(1_500 * MS);
    assert!(rig.platform.press());
    assert!(rig.platform.unplug("usb"));
    rig.until("opened again", || rig.opens().len() == 3);
    assert!(rig.platform.release());
    assert_eq!(rig.opens()[2], "desk");
    // Its new device ends too, within the same press's grace: not opened again (no loop), and
    // with no mic left nothing is said until a press needs one.
    assert!(rig.platform.unplug("desk"));
    after(3);
    rig.platform.plug(usb("usb"));
    rig.tap();
    assert_eq!(rig.opens()[3], "usb");

    // In a take: hold, speak until the take has started, then pull the mic.
    let speech: Vec<f32> = ink_audio::synth::speech_like(3.0, -25.0, 5)
        .into_iter()
        .flat_map(|s| [s, s, s])
        .collect();
    assert!(rig.platform.press());
    let clock = rig.platform.clock();
    for block in speech.chunks(480) {
        if rig.events.count("dictation.started") > 0 {
            break;
        }
        rig.until("the mic opens", || {
            rig.platform.feed(Channel::Mic, block, clock.now_ns())
        });
        clock.advance_ns(10 * MS);
        std::thread::sleep(Duration::from_micros(500));
    }
    assert!(rig.events.wait_count("dictation.started", 1, WAIT));
    assert!(rig.platform.unplug("usb"));
    let failed = rig.events.wait_type("dictation.mic_failed", WAIT);
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .contains("USB Microphone"),
        "{failed}"
    );
    rig.platform.release();
    rig.events.assert_valid();
}

/// The rig's mock behind a gate: dictation's mic thread (`ink-voice`) blocks in its next
/// `input_devices` once armed, until opened. Everything else passes straight through.
struct Gated {
    inner: Arc<MockPlatform>,
    armed: std::sync::atomic::AtomicBool,
    blocked: std::sync::atomic::AtomicBool,
    open: (std::sync::Mutex<bool>, std::sync::Condvar),
}

impl Gated {
    fn new(inner: Arc<MockPlatform>) -> Self {
        Self {
            inner,
            armed: Default::default(),
            blocked: Default::default(),
            open: Default::default(),
        }
    }

    fn release_gate(&self) {
        *self.open.0.lock().unwrap() = true;
        self.open.1.notify_all();
    }
}

impl CaptureControl for Gated {
    fn input_devices(&self) -> Result<Vec<DeviceInfo>, ink_core::PlatformError> {
        use std::sync::atomic::Ordering;
        if std::thread::current().name() == Some("ink-voice")
            && self.armed.swap(false, Ordering::AcqRel)
        {
            self.blocked.store(true, Ordering::Release);
            let mut open = self.open.0.lock().unwrap();
            while !*open {
                open = self.open.1.wait(open).unwrap();
            }
        }
        self.inner.input_devices()
    }
    fn output_devices(&self) -> Result<Vec<DeviceInfo>, ink_core::PlatformError> {
        self.inner.output_devices()
    }
    fn default_output(&self) -> Result<Option<DeviceInfo>, ink_core::PlatformError> {
        self.inner.default_output()
    }
    fn automatic_input(&self) -> Result<Option<ink_core::AutoInput>, ink_core::PlatformError> {
        self.inner.automatic_input()
    }
    fn open_mic(
        &self,
        device: Option<&DeviceId>,
    ) -> Result<Box<dyn AudioSource>, ink_core::PlatformError> {
        self.inner.open_mic(device)
    }
    fn open_far_end(
        &self,
        target: &ink_core::FarEndTarget,
    ) -> Result<Box<dyn AudioSource>, ink_core::PlatformError> {
        self.inner.open_far_end(target)
    }
    fn watch_devices(
        &self,
        on_change: ink_core::EventSink<ink_core::DeviceChange>,
    ) -> Result<(), ink_core::PlatformError> {
        self.inner.watch_devices(on_change)
    }
    fn unwatch_devices(&self) {
        self.inner.unwatch_devices();
    }
}

/// A press that arrives while the idle mic is being picked again keeps the mic (its lead with
/// it), and the change is looked at again: once the key is let go of and the grace has passed,
/// the mic gives way to the new pick.
#[test]
fn a_press_during_the_pick_keeps_the_mic_and_looks_again_later() {
    use std::sync::atomic::Ordering;
    let rig = Rig::new("sound-press-in-pick", windows_like(), false);
    let gated = Arc::new(Gated::new(rig.platform.clone()));
    rig.core().set_voice_platform(VoicePlatform {
        capture: gated.clone(),
        keys: rig.platform.clone(),
        edit_keys: Arc::new(MockPlatform::new()),
        inserter: rig.platform.clone(),
        focus: rig.platform.clone(),
    });
    rig.enable_dictation();
    rig.tap();
    rig.platform.clock().advance_ns(1_500 * MS);
    gated.armed.store(true, Ordering::Release);
    rig.set("audio.input", "usb", "s1");
    rig.until("the pick is under way", || {
        gated.blocked.load(Ordering::Acquire)
    });
    // The press lands during the pick.
    assert!(rig.platform.press());
    gated.release_gate();
    std::thread::sleep(Duration::from_millis(200));
    assert!(rig.mic_open(), "the press keeps the mic");
    assert_eq!(rig.opens(), ["built-in"]);
    // Still held: nothing changes however long it is held.
    rig.platform.clock().advance_ns(1_500 * MS);
    std::thread::sleep(Duration::from_millis(200));
    assert!(rig.mic_open(), "never while a key is held");
    assert!(rig.platform.release());
    // Looked at again (the flag was set again): let go of, and the next press opens the choice.
    rig.released();
    rig.tap();
    assert_eq!(rig.opens(), ["built-in", "usb"]);
    rig.events.assert_valid();
}

/// Feeds a 1 kHz tone at -12 dBFS to the mock mic in real time until `stop`.
fn tone(platform: Arc<MockPlatform>, stop: Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    let block: Vec<f32> = (0..480)
        .map(|i| 0.25 * (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / 48_000.0).sin())
        .collect();
    let clock = platform.clock();
    while !stop.load(Ordering::Relaxed) {
        platform.feed(Channel::Mic, &block, clock.now_ns());
        clock.advance_ns(10 * MS);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The test opens the chosen mic, reports its level about ten times a second, and ends when its
/// time is up, hearing the tone; one at a time; audio.test_stop ends it early, hearing nothing.
#[test]
fn the_mic_test_reports_a_level_and_ends_on_time_or_when_stopped() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let rig = Rig::new("sound-test", windows_like(), true);
    rig.set("audio.input", "usb", "s1");
    let started = rig.ask(r#"{"cmd":"audio.test","seconds":1,"id":"t1"}"#, "t1");
    assert_eq!(started["type"], "audio.test_started", "{started}");
    assert_eq!(started["mic_name"], "USB Microphone");
    assert_eq!(started["mic_reason"], "chosen");
    assert_eq!(started["seconds"], 1);
    assert_eq!(rig.opens(), ["usb"]);
    let busy = rig.ask(r#"{"cmd":"audio.test","id":"t2"}"#, "t2");
    assert_eq!(busy["type"], "command.failed", "{busy}");

    let stop = Arc::new(AtomicBool::new(false));
    let feeder = {
        let (platform, stop) = (rig.platform.clone(), stop.clone());
        std::thread::spawn(move || tone(platform, stop))
    };
    let tested = rig
        .events
        .wait_for(Duration::from_secs(5), |v| v["type"] == "audio.tested")
        .expect("the test ends");
    stop.store(true, Ordering::Relaxed);
    feeder.join().unwrap();
    assert_eq!(tested["ended"], "done", "{tested}");
    assert_eq!(tested["heard"], true);
    // -12 dBFS peak is a -15 dBFS RMS tone: 0.75 on the meter.
    let peak = tested["peak"].as_f64().unwrap();
    assert!((0.7..0.8).contains(&peak), "{peak}");
    let levels: Vec<f64> = rig
        .events
        .all()
        .into_iter()
        .filter(|v| v["type"] == "audio.test_level")
        .map(|v| {
            assert_eq!(v["ref"], "t1");
            v["level"].as_f64().unwrap()
        })
        .collect();
    // About ten in its second; fewer on a loaded machine, never many more.
    assert!((3..=12).contains(&levels.len()), "{levels:?}");
    assert!(levels.iter().any(|l| *l > 0.7), "{levels:?}");
    assert!(!rig.mic_open(), "the test's mic is closed");

    let started = rig.ask(r#"{"cmd":"audio.test","id":"t3"}"#, "t3");
    assert_eq!(started["seconds"], 15);
    rig.core()
        .command(r#"{"cmd":"audio.test_stop","id":"x"}"#)
        .unwrap();
    let tested = rig
        .events
        .wait_for(WAIT, |v| v["type"] == "audio.tested" && v["ref"] == "t3")
        .unwrap();
    assert_eq!(tested["ended"], "stopped");
    assert_eq!(tested["heard"], false);
    let none = rig.ask(r#"{"cmd":"audio.test_stop","id":"y"}"#, "y");
    assert_eq!(none["type"], "command.failed");
    rig.events.assert_valid();
}

/// A test whose mic is unplugged ends as failed, saying why.
#[test]
fn a_mic_test_whose_mic_goes_fails_saying_why() {
    let rig = Rig::new("sound-test-gone", windows_like(), true);
    rig.ask(r#"{"cmd":"audio.test","id":"t"}"#, "t");
    rig.platform.unplug("built-in");
    let tested = rig
        .events
        .wait_for(WAIT, |v| v["type"] == "audio.tested")
        .unwrap();
    assert_eq!(tested["ended"], "failed");
    assert!(
        tested["message"].as_str().unwrap().contains("built-in"),
        "{tested}"
    );
    rig.events.assert_valid();
}

/// The meeting's mic side as the platforms open it: the choice resolved over the mock, followed
/// by [`FollowMic`].
fn meeting_mic(rig: &Rig) -> (CaptureSide, MicInfo) {
    let platform = rig.platform.clone();
    let open = Arc::new(move |choice: &InputChoice| {
        let picked = devices::pick_mic(platform.as_ref(), choice)?;
        let source = platform
            .open_mic(Some(&picked.device.id))
            .map_err(|e| e.to_string())?;
        Ok((source, picked.mic_info(picked.device.transport)))
    });
    let choices = Choices::new(rig.store.clone());
    let (source, info) = open(&choices.input()).unwrap();
    let follow = FollowMic::new(open, choices, info.clone());
    (
        CaptureSide {
            source,
            ring: DEFAULT_RING_DURATION,
            start_at: None,
            follow: Some(Box::new(follow) as Box<dyn Follow>),
        },
        info,
    )
}

fn far_side(rig: &Rig) -> CaptureSide {
    let path = rig._dir.path().join("far.wav");
    speech_wav(&path, 20.0, 7);
    let clock: Arc<dyn ink_core::Clock> = rig.platform.clock();
    let source: Box<dyn AudioSource> = Box::new(
        FileReplaySource::open(&path, Channel::Far, clock)
            .unwrap()
            .with_pacing(Pacing::RealTime),
    );
    CaptureSide {
        source,
        ring: DEFAULT_RING_DURATION,
        start_at: None,
        follow: None,
    }
}

/// A meeting keeps its mic through a new mic and a new default; when its own mic goes it opens the
/// mic again from the choice (Automatic standing in for the chosen mic), says so, and starts the
/// echo search again. A mic test is refused while it records.
#[test]
fn a_meeting_moves_its_mic_only_when_its_own_mic_goes() {
    let rig = Rig::new("sound-meeting", windows_like(), true);
    rig.set("audio.input", "usb", "s1");
    let (mic, info) = meeting_mic(&rig);
    assert_eq!(info.reason, "chosen");
    rig.core()
        .start_meeting(
            vec![mic, far_side(&rig)],
            MeetingInfo {
                mic: Some(info),
                ..MeetingInfo::default()
            },
        )
        .unwrap();
    let started = rig.events.wait_type("meeting.started", WAIT);
    assert_eq!(started["mic_reason"], "chosen");

    let refused = rig.ask(r#"{"cmd":"audio.test","id":"t"}"#, "t");
    assert_eq!(refused["type"], "command.failed");
    assert_eq!(refused["code"], "meeting_recording");

    // A new mic, made the default: the meeting stays on its own through the pump's next asks.
    rig.platform
        .plug(device("desk", "Desk Microphone", Transport::Usb, true));
    std::thread::sleep(ink_ffi::meeting::FOLLOW_INTERVAL + Duration::from_millis(500));
    assert_eq!(rig.opens(), ["usb"]);
    assert_eq!(rig.events.count("meeting.mic_switched"), 0);

    rig.platform.unplug("usb");
    let switched = rig.events.wait_type("meeting.mic_switched", WAIT);
    assert_eq!(switched["from_name"], "USB Microphone");
    assert_eq!(switched["from_transport"], "usb");
    assert_eq!(switched["mic_name"], "Desk Microphone");
    assert_eq!(switched["mic_reason"], "chosen_missing");
    assert_eq!(switched["record"], started["record"]);
    assert_eq!(rig.opens(), ["usb", "desk"]);
    let said = rig.events.wait_type("audio.input_fallback", WAIT);
    assert_eq!(said["wanted"]["id"], "usb");
    assert!(
        rig.events
            .wait_for(WAIT, |v| v["type"] == "meeting.echo"
                && v["why"] == "device_switch")
            .is_some(),
        "{:?}",
        rig.events.types()
    );

    rig.core()
        .command(r#"{"cmd":"meeting.stop","id":"stop"}"#)
        .unwrap();
    rig.events
        .wait_type("meeting.finished", Duration::from_secs(30));
    rig.events.assert_valid();
}
