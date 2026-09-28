//! Dictation, live, through the core (S2.7): `dictation.enable` binds the keys on the platform, a
//! press opens the mic and a hold dictates into the focused app; the key settings rebind at once;
//! the mic is let go of when idle; the ink follows the voice while a take is open; the engine is
//! warmed at a take's start. The platform is the mock one: the keys, the mic and insertion.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use ink_audio::{BandsReader, bands_channel};
use ink_core::mock::MockPlatform;
use ink_core::{
    AudioBlock, AudioSink, AudioSource, CaptureControl, Channel, Clock, DeviceId, DeviceInfo,
    FarEndTarget, Permission, PermissionState, PlatformError, SourceStats, StreamFormat,
};
use std::sync::atomic::{AtomicBool, Ordering};

use ink_engines::{ModelDir, Registry};
use ink_ffi::runtime::{Core, Parts};
use ink_ffi::voice::{MIC_IDLE, VoicePlatform};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(10);
/// 10 ms at the mock mic's 48 kHz.
const BLOCK: usize = 480;

struct VoiceRig {
    core: Option<Core>,
    events: Arc<Recorder>,
    /// The mic, the dictation key, insertion and focus.
    platform: Arc<MockPlatform>,
    /// The edit key (a hotkey source holds one binding).
    edit: Arc<MockPlatform>,
    loader: Arc<MockLoader>,
    bands: BandsReader,
    _dir: TempDir,
}

impl VoiceRig {
    fn new(label: &str) -> Self {
        Self::build(label, true)
    }

    fn build(label: &str, with_platform: bool) -> Self {
        Self::build_with(
            label,
            with_platform,
            Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
        )
    }

    /// A rig whose core keeps its settings in `store` (a failing one, say).
    fn with_store(label: &str, store: Arc<dyn ink_core::Store>) -> Self {
        Self::build_with(label, true, store)
    }

    fn build_with(label: &str, with_platform: bool, store: Arc<dyn ink_core::Store>) -> Self {
        let dir = TempDir::new(label);
        let platform = Arc::new(MockPlatform::new());
        let edit = Arc::new(MockPlatform::new());
        let row = test_row("test-asr");
        let models = ModelDir::new(dir.path().join("models"));
        install(&models, &row);
        let loader = MockLoader::new(Behaviour::Say("hello world".into()));
        let parts = Parts {
            store,
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
        let events = Recorder::new();
        let (writer, bands) = bands_channel();
        let core = Core::start(parts, events.out()).unwrap();
        core.lend_bands(writer);
        events.wait_type("core.ready", WAIT);
        if with_platform {
            core.set_voice_platform(VoicePlatform {
                capture: platform.clone(),
                keys: platform.clone(),
                edit_keys: edit.clone(),
                inserter: platform.clone(),
                focus: platform.clone(),
            });
        }
        platform.clock().advance_ns(1_000_000_000);
        Self {
            core: Some(core),
            events,
            platform,
            edit,
            loader,
            bands,
            _dir: dir,
        }
    }

    fn core(&self) -> &Core {
        self.core.as_ref().unwrap()
    }

    fn command(&self, json: &str) {
        self.core().command(json).unwrap();
    }

    /// Sends `json` with id `id` and waits for the event answering it.
    fn ask(&self, json: &str, id: &str) -> Value {
        self.command(json);
        self.events
            .wait_for(WAIT, |v| v["ref"] == id)
            .unwrap_or_else(|| panic!("no answer to {json}: {:?}", self.events.types()))
    }

    fn enable(&self) -> Value {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = format!("on{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let answer = self.ask(&format!(r#"{{"cmd":"dictation.enable","id":"{id}"}}"#), &id);
        assert_eq!(answer["type"], "dictation.ready", "{answer}");
        answer
    }

    /// The edit key's mock has a clock of its own: set it to the mic's, so its presses land where
    /// they would on one machine.
    fn sync_edit_clock(&self) {
        let (mic, edit) = (self.platform.clock(), self.edit.clock());
        edit.advance_ns(mic.now_ns().saturating_sub(edit.now_ns()));
    }

    /// As [`feed`](Self::feed), at the pace of real time, so the chain keeps up as it would.
    fn feed_paced(&self, samples: &[f32]) {
        for block in samples.chunks(BLOCK) {
            self.feed(block);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Feeds the mock mic 10 ms at a time on the mock clock, waiting (in real time) for the mic
    /// to be open first.
    fn feed(&self, samples: &[f32]) {
        let clock = self.platform.clock();
        for block in samples.chunks(BLOCK) {
            let until = Instant::now() + WAIT;
            while !self.platform.feed(Channel::Mic, block, clock.now_ns()) {
                assert!(Instant::now() < until, "the mic never opened");
                std::thread::sleep(Duration::from_millis(1));
            }
            clock.advance_ns(10_000_000);
            // Real time for the pump, which drains every 10 ms.
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    fn silence(&self, seconds: f64) {
        self.feed(&vec![0.0; (seconds * 48_000.0) as usize]);
    }

    fn speech(seconds: f64, seed: u64) -> Vec<f32> {
        // 16 kHz synthetic speech, repeated to 48 kHz: enough for the mock engine and the ink.
        ink_audio::synth::speech_like(seconds, -25.0, seed)
            .into_iter()
            .flat_map(|s| [s, s, s])
            .collect()
    }

    /// Press, speak, release, and room for the tail; waits for the take to end.
    fn dictate(&self, seconds: f64, seed: u64) -> Value {
        let before = self.events.count("dictation.inserted");
        assert!(self.platform.press());
        self.feed(&Self::speech(seconds, seed));
        assert!(self.platform.release());
        self.silence(0.6);
        assert!(
            self.events
                .wait_count("dictation.inserted", before + 1, WAIT),
            "{:?}",
            self.events.types()
        );
        self.events
            .all()
            .into_iter()
            .filter(|v| v["type"] == "dictation.inserted")
            .nth(before)
            .unwrap()
    }

    fn mic_open(&self) -> bool {
        self.platform
            .feed(Channel::Mic, &[0.0; 1], self.platform.clock().now_ns())
    }
}

impl Drop for VoiceRig {
    fn drop(&mut self) {
        if let Some(core) = self.core.take() {
            core.shutdown();
        }
    }
}

#[test]
fn enabling_holds_the_default_key_and_a_hold_dictates_into_the_focused_app() {
    let rig = VoiceRig::new("dictate");
    let ready = rig.enable();
    assert_eq!(ready["key"], "fn");
    assert!(ready.get("edit_key").is_none(), "no edit key by default");
    assert_eq!(
        rig.platform.hotkey_binding().map(|b| b.0).as_deref(),
        Some("fn")
    );
    assert!(
        !rig.mic_open(),
        "the mic opens at the first press, not before"
    );
    let inserted = rig.dictate(1.2, 1);
    assert_eq!(inserted["text"], "Hello world.");
    assert_eq!(rig.platform.inserted(), ["Hello world. "]);
    let started = rig.events.wait_type("dictation.started", WAIT);
    assert_eq!(started["edit"], false);
    assert_eq!(started["take"], 0);
    rig.events.assert_valid();
}

/// The Verify line "a second press never wipes the take", through the key sink and the mailbox:
/// the key-down arrives twice during one hold, and one dictation goes in.
#[test]
fn a_second_press_never_wipes_the_take() {
    let rig = VoiceRig::new("second-press");
    rig.enable();
    assert!(rig.platform.press());
    rig.feed(&VoiceRig::speech(0.8, 2));
    assert!(rig.platform.press(), "the key-down again, mid-hold");
    rig.feed(&VoiceRig::speech(0.8, 3));
    assert!(rig.platform.release());
    rig.silence(0.6);
    assert!(rig.events.wait_count("dictation.inserted", 1, WAIT));
    assert_eq!(rig.events.count("dictation.started"), 1);
    assert_eq!(rig.platform.inserted(), ["Hello world. "]);
}

#[test]
fn without_a_platform_dictation_says_it_is_unsupported() {
    let rig = VoiceRig::build("unsupported", false);
    let answer = rig.ask(r#"{"cmd":"dictation.enable","id":"on"}"#, "on");
    assert_eq!(answer["type"], "dictation.off");
    assert_eq!(answer["reason"], "unsupported");
}

#[test]
fn without_accessibility_dictation_is_off_and_says_why() {
    let rig = VoiceRig::new("no-ax");
    rig.platform
        .set_permission(Permission::Accessibility, PermissionState::Denied);
    let answer = rig.ask(r#"{"cmd":"dictation.enable","id":"on"}"#, "on");
    assert_eq!(answer["type"], "dictation.off", "{answer}");
    assert_eq!(answer["reason"], "needs_accessibility");
    assert!(rig.platform.hotkey_binding().is_none());
    // Granted since: enabling again binds the key.
    rig.platform
        .set_permission(Permission::Accessibility, PermissionState::Granted);
    let again = rig.ask(r#"{"cmd":"dictation.enable","id":"again"}"#, "again");
    assert_eq!(again["type"], "dictation.ready", "{again}");
    assert!(rig.platform.hotkey_binding().is_some());
}

#[test]
fn changing_the_key_setting_rebinds_it_at_once() {
    let rig = VoiceRig::new("rebind");
    rig.enable();
    rig.command(r#"{"cmd":"setting.set","key":"dictation.key","value":"right_option"}"#);
    let ready = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "dictation.ready" && v["key"] == "right_option"
        })
        .expect("rebound");
    assert!(ready.get("ref").is_none());
    assert_eq!(
        rig.platform.hotkey_binding().map(|b| b.0).as_deref(),
        Some("right_option")
    );
    // A key the platform cannot hold is refused before it is stored.
    assert!(
        rig.core()
            .command(r#"{"cmd":"setting.set","key":"dictation.key","value":"left_option"}"#)
            .is_err()
    );
}

/// The shell's on/off switch is stored for the shell to read at launch; storing it rebinds
/// nothing (the shell sends dictation.disable or dictation.enable itself).
#[test]
fn the_on_off_switch_is_stored_and_rebinds_nothing() {
    let rig = VoiceRig::new("switch");
    rig.enable();
    let before = rig.events.count("dictation.ready");
    rig.command(r#"{"cmd":"setting.set","key":"dictation.enabled","value":"off"}"#);
    let value = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "setting.value" && v["key"] == "dictation.enabled"
        })
        .expect("stored");
    assert_eq!(value["value"], "off");
    // A rebind would have answered with dictation.ready; the key setting's would come after this.
    rig.command(r#"{"cmd":"setting.set","key":"dictation.key","value":"right_shift"}"#);
    rig.events
        .wait_for(WAIT, |v| {
            v["type"] == "dictation.ready" && v["key"] == "right_shift"
        })
        .expect("the key setting rebinds");
    assert_eq!(rig.events.count("dictation.ready"), before + 1);
}

#[test]
fn the_edit_key_is_held_on_its_own_and_never_the_dictation_key() {
    let rig = VoiceRig::new("edit-key");
    rig.enable();
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"right_command"}"#);
    let ready = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "dictation.ready" && v["edit_key"] == "right_command"
        })
        .expect("edit key bound");
    assert_eq!(ready["key"], "fn");
    assert_eq!(
        rig.edit.hotkey_binding().map(|b| b.0).as_deref(),
        Some("right_command")
    );
    // The dictation key itself as the edit key: refused, and said so.
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"fn"}"#);
    let same = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "dictation.ready" && v.get("edit_key_error").is_some()
        })
        .expect("refused");
    assert!(same.get("edit_key").is_none());
    assert!(rig.edit.hotkey_binding().is_none());
    // Off: let go of.
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"off"}"#);
    assert!(rig.events.wait_count("dictation.ready", 4, WAIT));
    assert!(rig.edit.hotkey_binding().is_none());
}

/// Taking the edit key as the dictation key lets go of the edit key first: one key, one tap.
#[test]
fn the_edit_keys_key_taken_for_dictation_is_let_go_of_as_the_edit_key() {
    let rig = VoiceRig::new("swap-keys");
    rig.enable();
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"right_command"}"#);
    rig.events
        .wait_for(WAIT, |v| v["edit_key"] == "right_command")
        .expect("edit key bound");
    rig.command(r#"{"cmd":"setting.set","key":"dictation.key","value":"right_command"}"#);
    let ready = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "dictation.ready" && v["key"] == "right_command"
        })
        .expect("rebound");
    assert!(ready.get("edit_key").is_none(), "{ready}");
    assert!(ready.get("edit_key_error").is_some(), "{ready}");
    assert!(rig.edit.hotkey_binding().is_none());
    assert_eq!(
        rig.platform.hotkey_binding().map(|b| b.0).as_deref(),
        Some("right_command")
    );
}

/// A voice edit through the core, with no language model registered: the selection is left
/// alone and the shell hears why.
#[test]
fn an_edit_without_a_language_model_leaves_the_selection_alone() {
    let rig = VoiceRig::new("edit-no-model");
    rig.enable();
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"right_command"}"#);
    rig.events
        .wait_for(WAIT, |v| v["edit_key"] == "right_command")
        .expect("bound");
    rig.platform.set_selection(Some("teh cat"));
    rig.sync_edit_clock();
    assert!(rig.edit.press());
    rig.feed(&VoiceRig::speech(1.0, 4));
    rig.sync_edit_clock();
    assert!(rig.edit.release());
    rig.silence(0.6);
    let failed = rig
        .events
        .wait_for(WAIT, |v| v["type"] == "dictation.edit_failed")
        .expect("edit failed");
    assert_eq!(failed["reason"], "model", "{failed}");
    assert!(rig.platform.inserted().is_empty());
    let started = rig.events.wait_type("dictation.started", WAIT);
    assert_eq!(started["edit"], true);
}

#[test]
fn the_mic_is_let_go_of_when_idle_and_opened_again_at_the_next_press() {
    let rig = VoiceRig::new("mic-idle");
    // The owner's decision (2026-09-28): one minute, not three.
    assert_eq!(MIC_IDLE, Duration::from_secs(60));
    rig.enable();
    rig.dictate(1.0, 5);
    assert!(rig.mic_open(), "open for the next take's lead");
    // Short of the minute it stays open (the mic thread wakes every 10 ms; give it several).
    rig.platform
        .clock()
        .advance_ns(MIC_IDLE.as_nanos() as u64 - 2_000_000_000);
    std::thread::sleep(Duration::from_millis(100));
    assert!(rig.mic_open(), "still open before the minute is up");
    rig.platform.clock().advance_ns(3_000_000_000);
    let until = Instant::now() + WAIT;
    while rig.mic_open() {
        assert!(Instant::now() < until, "the mic was never let go of");
        std::thread::sleep(Duration::from_millis(5));
    }
    // The next press opens it again, and the take goes in.
    rig.dictate(1.0, 6);
    assert_eq!(rig.platform.inserted().len(), 2);
}

#[test]
fn the_ink_follows_the_voice_while_a_take_is_open() {
    let rig = VoiceRig::new("bands");
    rig.enable();
    let before = rig.bands.read().published;
    assert!(rig.platform.press());
    rig.feed_paced(&VoiceRig::speech(1.0, 7));
    let during = rig.bands.read();
    assert!(during.published > before, "bands while dictating");
    assert!(
        during.bands.low + during.bands.mid + during.bands.high > 0.0,
        "{during:?}"
    );
    assert!(rig.platform.release());
    rig.silence(0.6);
    assert!(rig.events.wait_count("dictation.inserted", 1, WAIT));
    // Room after the take (the mic stays open): the ink rests, and nothing more is published.
    // (The pump's drain in flight when the take ended may still publish; let it finish.)
    std::thread::sleep(Duration::from_millis(50));
    rig.silence(0.3);
    std::thread::sleep(Duration::from_millis(50));
    let after = rig.bands.read();
    assert_eq!(after.bands, ink_audio::Bands::default(), "a still frame");
    rig.silence(0.3);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(rig.bands.read().published, after.published);
}

#[test]
fn a_take_after_a_quiet_spell_warms_the_engine_on_its_own_thread() {
    let rig = VoiceRig::new("warm");
    rig.enable();
    rig.dictate(1.0, 8);
    let threads = rig.loader.journal.threads.lock().unwrap().clone();
    assert!(
        threads.iter().any(|t| t.as_deref() == Some("ink-warm")),
        "{threads:?}"
    );
    assert!(
        threads
            .iter()
            .any(|t| t.as_deref() == Some("ink-dictation")),
        "{threads:?}"
    );
    assert_eq!(rig.platform.inserted(), ["Hello world. "]);
}

#[test]
fn a_mic_that_cannot_open_says_so_and_the_next_press_tries_again() {
    let rig = VoiceRig::new("mic-denied");
    rig.enable();
    rig.platform
        .set_permission(Permission::Microphone, PermissionState::Denied);
    assert!(rig.platform.press());
    let failed = rig.events.wait_type("dictation.mic_failed", WAIT);
    assert!(failed["message"].as_str().is_some());
    assert!(rig.platform.release());
    rig.platform
        .set_permission(Permission::Microphone, PermissionState::Granted);
    rig.dictate(1.0, 9);
    assert_eq!(rig.platform.inserted(), ["Hello world. "]);
}

#[test]
fn disabling_lets_go_of_the_keys_and_the_mic() {
    let rig = VoiceRig::new("disable");
    rig.enable();
    rig.dictate(1.0, 10);
    let off = rig.ask(r#"{"cmd":"dictation.disable","id":"off"}"#, "off");
    assert_eq!(off["type"], "dictation.off");
    assert_eq!(off["reason"], "disabled");
    assert!(rig.platform.hotkey_binding().is_none());
    assert!(!rig.mic_open());
    // And on again.
    rig.enable();
    rig.dictate(1.0, 11);
}

#[test]
fn shutting_down_with_dictation_on_stops_everything() {
    let mut rig = VoiceRig::new("shutdown");
    rig.enable();
    assert!(rig.platform.press());
    rig.feed(&VoiceRig::speech(0.5, 12));
    let core = rig.core.take().unwrap();
    let started = Instant::now();
    core.shutdown();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!rig.mic_open());
    assert!(rig.platform.hotkey_binding().is_none());
}

#[test]
fn modes_listed_and_dictation_agree_on_the_default_mode_polishing() {
    let rig = VoiceRig::new("modes");
    rig.command(r#"{"cmd":"modes.list"}"#);
    let listed = rig.events.wait_type("modes.listed", WAIT);
    // With no modes stored, "Polish my words" alone decides: the default mode polishes.
    assert_eq!(listed["modes"][0]["polish"], true, "{listed}");
}

/// A mic that changes format mid-stream once `flip` is set (a USB mic unplugged and replaced by
/// the built-in one, a Bluetooth headset switching profile): the mic path cannot go on with it.
struct FlakyCapture {
    inner: Arc<MockPlatform>,
    flip: Arc<AtomicBool>,
}

struct FlakySource {
    inner: Box<dyn AudioSource>,
    flip: Arc<AtomicBool>,
}

struct FlakySink {
    inner: Box<dyn AudioSink>,
    flip: Arc<AtomicBool>,
}

impl AudioSink for FlakySink {
    fn push(&mut self, block: &AudioBlock<'_>) {
        if self.flip.load(Ordering::Acquire) {
            self.inner.push(&AudioBlock {
                format: StreamFormat {
                    sample_rate: 44_100,
                    channels: block.format.channels,
                },
                ..*block
            });
        } else {
            self.inner.push(block);
        }
    }
}

impl AudioSource for FlakySource {
    fn channel(&self) -> Channel {
        self.inner.channel()
    }
    fn format(&self) -> StreamFormat {
        self.inner.format()
    }
    fn start(&mut self, sink: Box<dyn AudioSink>) -> Result<(), PlatformError> {
        self.inner.start(Box::new(FlakySink {
            inner: sink,
            flip: self.flip.clone(),
        }))
    }
    fn stop(&mut self) -> Result<SourceStats, PlatformError> {
        self.inner.stop()
    }
}

impl CaptureControl for FlakyCapture {
    fn input_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        self.inner.input_devices()
    }
    fn default_output(&self) -> Result<Option<DeviceInfo>, PlatformError> {
        self.inner.default_output()
    }
    fn open_mic(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioSource>, PlatformError> {
        Ok(Box::new(FlakySource {
            inner: self.inner.open_mic(device)?,
            flip: self.flip.clone(),
        }))
    }
    fn open_far_end(&self, target: &FarEndTarget) -> Result<Box<dyn AudioSource>, PlatformError> {
        self.inner.open_far_end(target)
    }
}

/// The mic failing in the middle of a take (its format changed under it) is said
/// (dictation.mic_failed), the take ends with what it heard (never hangs "Listening"), the mic is
/// let go of, and the next press opens it again.
#[test]
fn a_mic_that_fails_mid_take_says_so_ends_the_take_and_the_next_press_reopens_it() {
    let flip = Arc::new(AtomicBool::new(false));
    // Built without a platform, then given one whose capture wraps the rig's mock mic.
    let rig = VoiceRig::build("mic-mid-take", false);
    rig.core().set_voice_platform(VoicePlatform {
        capture: Arc::new(FlakyCapture {
            inner: rig.platform.clone(),
            flip: flip.clone(),
        }),
        keys: rig.platform.clone(),
        edit_keys: rig.edit.clone(),
        inserter: rig.platform.clone(),
        focus: rig.platform.clone(),
    });
    rig.enable();
    assert!(rig.platform.press());
    rig.feed(&VoiceRig::speech(1.0, 13));
    flip.store(true, Ordering::Release);
    // The next blocks arrive in another format: the pump gives up on this stream.
    let clock = rig.platform.clock();
    for _ in 0..5 {
        rig.platform
            .feed(Channel::Mic, &[0.0; BLOCK], clock.now_ns());
        clock.advance_ns(10_000_000);
    }
    let failed = rig.events.wait_type("dictation.mic_failed", WAIT);
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.contains("format")),
        "{failed}"
    );
    // The take ended with what it had: processed (here, inserted), never left open.
    let ended = rig
        .events
        .wait_for(WAIT, |v| {
            [
                "dictation.inserted",
                "dictation.discarded",
                "dictation.failed",
            ]
            .contains(&v["type"].as_str().unwrap_or(""))
        })
        .expect("the take ended");
    assert_eq!(ended["type"], "dictation.inserted", "{ended}");
    let until = Instant::now() + WAIT;
    while rig.mic_open() {
        assert!(Instant::now() < until, "the failed mic was never let go of");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(rig.platform.release());
    // The next press opens the mic again and dictates.
    flip.store(false, Ordering::Release);
    rig.dictate(1.0, 14);
    assert_eq!(rig.platform.inserted().len(), 2);
}

// --- Local-only mode (S2.8 review) ------------------------------------------------------------

/// A registered language model, counting its calls: one that says it is not on this machine
/// ([`VoiceRig::register_remote`]), or one on it ([`VoiceRig::register_local`]).
struct RemoteModel {
    calls: std::sync::atomic::AtomicUsize,
    answer: &'static str,
}

unsafe extern "C" fn remote_generate(
    ctx: *mut std::ffi::c_void,
    call: u64,
    _request: *const std::ffi::c_char,
) {
    // SAFETY: ctx is the test's leaked model, alive for the process.
    let me = unsafe { &*(ctx as *const RemoteModel) };
    me.calls.fetch_add(1, Ordering::SeqCst);
    let answer = me.answer;
    std::thread::spawn(move || {
        let answer = std::ffi::CString::new(format!(r#"{{"text":"{answer}"}}"#)).unwrap();
        // SAFETY: a NUL-terminated string valid for the call.
        unsafe { ink_ffi::ink_engine_complete(call, answer.as_ptr()) };
    });
}

unsafe extern "C" fn remote_release(_: *mut std::ffi::c_void) {}

impl VoiceRig {
    /// Registers a model whose info says `"local": false`.
    fn register_remote(&self) -> &'static RemoteModel {
        self.register_model(
            "remote-llm",
            "remote",
            false,
            "Rewritten by a remote model.",
        )
    }

    /// Registers a model on this machine, as the Mac registers Apple's on-device model.
    fn register_local(&self) -> &'static RemoteModel {
        self.register_model("local-llm", "on-device", true, "Polished on this machine.")
    }

    fn register_model(
        &self,
        id: &str,
        name: &str,
        local: bool,
        answer: &'static str,
    ) -> &'static RemoteModel {
        use ink_ffi::external::{InkEngineVTable, KIND_LLM, Registration};
        let model: &'static RemoteModel = Box::leak(Box::new(RemoteModel {
            calls: Default::default(),
            answer,
        }));
        let info = std::ffi::CString::new(format!(
            r#"{{"id":"{id}","licence":"MIT","model":"{name}","local":{local}}}"#
        ))
        .unwrap();
        let table = InkEngineVTable {
            kind: KIND_LLM,
            info_json: info.as_ptr(),
            ctx: model as *const RemoteModel as *mut std::ffi::c_void,
            release: Some(remote_release),
            generate: Some(remote_generate),
            ..Default::default()
        };
        // SAFETY: a valid table whose ctx outlives the core.
        let registration =
            unsafe { Registration::from_table(&table, self.core().shared().shutdown.clone()) }
                .unwrap();
        self.core().register(registration).unwrap();
        self.events
            .wait_for(WAIT, |v| v["type"] == "engine.registered" && v["id"] == id)
            .expect("registered");
        model
    }

    /// Lets go of a registered model.
    fn unregister(&self, id: &str) {
        self.command(&format!(r#"{{"cmd":"engine.unregister","engine":"{id}"}}"#));
        self.events
            .wait_for(WAIT, |v| {
                v["type"] == "engine.unregistered" && v["id"] == id
            })
            .expect("unregistered");
    }

    fn setting(&self, key: &str) -> Option<String> {
        self.core().shared().store.setting(key).unwrap()
    }
}

/// While local-only is on (the default), dictation's polish never calls a model that is not
/// local: the take goes in as said, and the refusal is said.
#[test]
fn dictation_polish_refuses_a_model_that_is_not_local() {
    let rig = VoiceRig::new("local-only-polish");
    let remote = rig.register_remote();
    // The user agreed to send to this provider: what refuses it here is local-only mode alone.
    let state = rig.ask(
        r#"{"cmd":"polish.allow","to":"cloud","endpoint":"shell engine remote-llm","id":"allow"}"#,
        "allow",
    );
    assert_eq!(state["type"], "polish.state", "{state}");
    assert_eq!(state["allowed"], true, "{state}");
    rig.enable();
    rig.dictate(1.0, 11);
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0, "never called");
    assert!(
        rig.platform
            .inserted()
            .last()
            .is_some_and(|s| !s.contains("Rewritten")),
        "{:?}",
        rig.platform.inserted()
    );
    let warning = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "dictation.warning" && v["kind"] == "polish_failed"
        })
        .expect("the refusal is said");
    assert!(
        warning["message"]
            .as_str()
            .is_some_and(|m| m.contains("local-only")),
        "{warning}"
    );
}

/// While local-only is on, a voice edit never sends the selection to a model that is not local:
/// the selection is left alone, and the refusal is said.
#[test]
fn a_voice_edit_refuses_a_model_that_is_not_local() {
    let rig = VoiceRig::new("local-only-edit");
    let remote = rig.register_remote();
    rig.enable();
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"right_command"}"#);
    rig.events
        .wait_for(WAIT, |v| v["edit_key"] == "right_command")
        .expect("bound");
    rig.platform.set_selection(Some("teh cat"));
    rig.sync_edit_clock();
    assert!(rig.edit.press());
    rig.feed(&VoiceRig::speech(1.0, 12));
    rig.sync_edit_clock();
    assert!(rig.edit.release());
    rig.silence(0.6);
    let failed = rig
        .events
        .wait_for(WAIT, |v| v["type"] == "dictation.edit_failed")
        .expect("edit failed");
    assert_eq!(failed["reason"], "model", "{failed}");
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.contains("local-only")),
        "{failed}"
    );
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0, "never called");
    assert!(
        rig.platform.inserted().is_empty(),
        "the selection is left alone"
    );
}

// --- Polish consent (owner decision, 2026-09-28) ----------------------------------------------
//
// Polish sends a dictation to a language model, so it is off until the user agrees to where it
// goes: polish.allow records that consent in the core's store and turns polish on; nothing else
// does. A model that changes destination since gets nothing until the user agrees again.

impl VoiceRig {
    fn polish_state(&self, id: &str) -> Value {
        let state = self.ask(&format!(r#"{{"cmd":"polish.get","id":"{id}"}}"#), id);
        assert_eq!(state["type"], "polish.state", "{state}");
        state
    }

    /// Sends `json` (with id `id`) and waits for its command.failed.
    fn fails(&self, json: &str, id: &str) -> Value {
        self.command(json);
        self.events
            .wait_for(WAIT, |v| v["type"] == "command.failed" && v["id"] == id)
            .unwrap_or_else(|| panic!("{json} did not fail: {:?}", self.events.types()))
    }

    fn warnings(&self, kind: &str) -> Vec<Value> {
        self.events
            .all()
            .into_iter()
            .filter(|v| v["type"] == "dictation.warning" && v["kind"] == kind)
            .collect()
    }

    fn allow_on_device(&self, id: &str) -> Value {
        let state = self.ask(
            &format!(r#"{{"cmd":"polish.allow","to":"on_device","id":"{id}"}}"#),
            id,
        );
        assert_eq!(state["type"], "polish.state", "{state}");
        state
    }
}

/// A fresh install: polish off, no consent; the state names where polish would send.
#[test]
fn polish_starts_off_with_no_consent_and_names_where_it_would_send() {
    let rig = VoiceRig::new("polish-fresh");
    let none = rig.polish_state("p1");
    assert_eq!(none["on"], false);
    assert_eq!(none["allowed"], false);
    assert!(none.get("to").is_none(), "no model, no destination: {none}");
    assert!(none.get("allowed_to").is_none(), "{none}");

    rig.register_local();
    let local = rig.polish_state("p2");
    assert_eq!(local["to"], "on_device", "{local}");
    assert_eq!(local["name"], "on-device", "{local}");
    assert!(local.get("endpoint").is_none(), "{local}");
    assert_eq!(local["allowed"], false);
    rig.events.assert_valid();
}

/// polish.allow records the consent in the core's store and turns polish on; the next take is
/// polished.
#[test]
fn polish_allow_records_the_consent_and_turns_polish_on() {
    let rig = VoiceRig::new("polish-allow");
    let model = rig.register_local();
    let state = rig.allow_on_device("a1");
    assert_eq!(state["on"], true, "{state}");
    assert_eq!(state["allowed"], true, "{state}");
    assert_eq!(state["allowed_to"], "on_device", "{state}");
    assert_eq!(rig.setting("dictation.polish").as_deref(), Some("on"));
    assert_eq!(
        rig.setting("dictation.polish_consent").as_deref(),
        Some(r#"{"to":"on_device"}"#)
    );
    rig.enable();
    rig.dictate(1.0, 51);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert!(
        rig.platform
            .inserted()
            .last()
            .is_some_and(|s| s.contains("Polished on this machine")),
        "{:?}",
        rig.platform.inserted()
    );
    rig.events.assert_valid();
}

/// Consent is for what the user was shown: a destination that is not the model's now (it changed
/// while they read), or no model at all, records nothing and fails, so the shell asks again.
#[test]
fn polish_allow_refuses_a_destination_that_is_not_the_models() {
    let rig = VoiceRig::new("polish-mismatch");
    let failed = rig.fails(r#"{"cmd":"polish.allow","to":"on_device","id":"f1"}"#, "f1");
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.contains("no language model")),
        "{failed}"
    );
    rig.register_local();
    rig.fails(
        r#"{"cmd":"polish.allow","to":"cloud","endpoint":"shell engine elsewhere","id":"f2"}"#,
        "f2",
    );
    assert_eq!(rig.setting("dictation.polish"), None, "polish stays off");
    assert_eq!(
        rig.setting("dictation.polish_consent"),
        None,
        "nothing recorded"
    );
    rig.events.assert_valid();
}

/// Off withdraws the consent, so turning polish on again asks again; and setting.set can never
/// turn it on.
#[test]
fn turning_polish_off_withdraws_the_consent() {
    let rig = VoiceRig::new("polish-off");
    rig.register_local();
    rig.allow_on_device("o1");
    rig.command(r#"{"cmd":"setting.set","key":"dictation.polish","value":"off","id":"o2"}"#);
    let state = rig
        .events
        .wait_for(WAIT, |v| v["type"] == "polish.state" && v["on"] == false)
        .expect("polish.state after off");
    assert_eq!(state["allowed"], false, "{state}");
    assert!(state.get("allowed_to").is_none(), "{state}");
    assert_eq!(
        rig.setting("dictation.polish_consent").as_deref(),
        Some("none")
    );
    assert!(
        rig.core()
            .command(r#"{"cmd":"setting.set","key":"dictation.polish","value":"on"}"#)
            .is_err(),
        "only polish.allow turns polish on"
    );
    rig.events.assert_valid();
}

/// Consent for this Mac, then the model becomes a cloud one: the state says polish is paused and
/// names the provider, and a take sends nothing, goes in as said, and says why.
#[test]
fn a_model_that_moves_to_the_cloud_gets_nothing_until_the_user_agrees_again() {
    let rig = VoiceRig::new("polish-moved");
    rig.register_local();
    rig.allow_on_device("m1");
    rig.enable();
    rig.unregister("local-llm");
    let remote = rig.register_remote();

    let state = rig.polish_state("m2");
    assert_eq!(state["on"], true, "{state}");
    assert_eq!(state["allowed"], false, "paused: {state}");
    assert_eq!(state["to"], "cloud", "{state}");
    assert_eq!(state["name"], "remote", "{state}");
    assert_eq!(state["endpoint"], "shell engine remote-llm", "{state}");
    assert_eq!(state["allowed_to"], "on_device", "{state}");

    rig.dictate(1.0, 52);
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0, "nothing sent");
    assert!(
        rig.platform
            .inserted()
            .last()
            .is_some_and(|s| !s.contains("Rewritten")),
        "{:?}",
        rig.platform.inserted()
    );
    let warnings = rig.warnings("polish_not_allowed");
    assert_eq!(warnings.len(), 1, "{:?}", rig.events.types());
    assert_eq!(warnings[0]["message"], "remote", "names the provider");
    rig.events.assert_valid();
}

/// "Polish my words" stored on without a consent (a store an older build wrote) and no modes
/// stored, so the default mode polishes: nothing is sent, and the take says why.
#[test]
fn neither_the_default_mode_nor_a_stored_switch_polishes_without_consent() {
    let rig = VoiceRig::new("polish-default-mode");
    let model = rig.register_local();
    rig.core()
        .shared()
        .store
        .set_setting("dictation.polish", "on")
        .unwrap();
    rig.enable();
    rig.dictate(1.0, 53);
    assert_eq!(model.calls.load(Ordering::SeqCst), 0, "nothing sent");
    assert_eq!(rig.warnings("polish_not_allowed").len(), 1);
    let state = rig.polish_state("d1");
    assert_eq!(state["on"], true);
    assert_eq!(state["allowed"], false);
    rig.events.assert_valid();
}

/// A 0.2 import where polish was on (the global switch and a mode marked Polish) turns nothing on:
/// the importer writes only its own keys, and neither is the switch or a consent.
#[test]
fn a_02_import_with_polish_on_turns_nothing_on() {
    let rig = VoiceRig::new("polish-import");
    let model = rig.register_local();
    let store = &rig.core().shared().store;
    // What the importer writes for a 0.2 install with polish on.
    store
        .set_setting(
            &format!("{}polish_enabled", ink_store::import::SETTINGS_PREFIX),
            "true",
        )
        .unwrap();
    store
        .set_setting(
            ink_store::import::MODES_KEY,
            r#"{"default_id":"d","modes":[{"id":"d","name":"Default","style":"formal","polish_enabled":true,"apps":[]}]}"#,
        )
        .unwrap();
    let state = rig.polish_state("i1");
    assert_eq!(state["on"], false, "{state}");
    assert!(state.get("allowed_to").is_none(), "{state}");
    rig.enable();
    rig.dictate(1.0, 54);
    assert_eq!(model.calls.load(Ordering::SeqCst), 0, "nothing sent");
    rig.events.assert_valid();
}

/// A stored consent that does not read is no consent: polish sends nothing, and both dictation
/// and the polish state say what could not be read.
#[test]
fn an_unreadable_consent_is_no_consent_and_says_so() {
    let rig = VoiceRig::new("polish-unreadable");
    let model = rig.register_local();
    let store = &rig.core().shared().store;
    store.set_setting("dictation.polish", "on").unwrap();
    store
        .set_setting("dictation.polish_consent", "{not json")
        .unwrap();
    let ready = rig.enable();
    assert!(
        ready["settings_error"]
            .as_str()
            .is_some_and(|e| e.contains("the polish consent")),
        "{ready}"
    );
    let state = rig.polish_state("u1");
    assert_eq!(state["allowed"], false);
    assert!(
        state["error"]
            .as_str()
            .is_some_and(|e| e.starts_with("couldn't read")),
        "{state}"
    );
    rig.dictate(1.0, 55);
    assert_eq!(model.calls.load(Ordering::SeqCst), 0, "nothing sent");
    rig.events.assert_valid();
}

/// The switch and the consent are written together: a store that refuses the write leaves both
/// as they were, and the command fails where the shell sees it. Turning polish on saves neither
/// the consent nor the switch; turning it off leaves polish on with its consent, never off with a
/// stale consent.
#[test]
fn the_switch_and_the_consent_are_saved_together_or_not_at_all() {
    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    let rig = VoiceRig::with_store("polish-atomic", store.clone());
    rig.register_local();

    store.fail(&["set_settings"]);
    let failed = rig.fails(r#"{"cmd":"polish.allow","to":"on_device","id":"t1"}"#, "t1");
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("couldn't")),
        "{failed}"
    );
    assert_eq!(rig.setting("dictation.polish"), None, "switch not saved");
    assert_eq!(
        rig.setting("dictation.polish_consent"),
        None,
        "nor the consent"
    );

    store.heal();
    rig.allow_on_device("t2");
    store.fail(&["set_settings"]);
    let failed = rig.fails(
        r#"{"cmd":"setting.set","key":"dictation.polish","value":"off","id":"t3"}"#,
        "t3",
    );
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("couldn't turn polish off")),
        "{failed}"
    );
    assert_eq!(rig.setting("dictation.polish").as_deref(), Some("on"));
    assert_eq!(
        rig.setting("dictation.polish_consent").as_deref(),
        Some(r#"{"to":"on_device"}"#),
        "both as they were"
    );
    store.heal();
    rig.events.assert_valid();
}
