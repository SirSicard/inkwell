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
use ink_core::{Channel, Clock, Permission, PermissionState};

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
        let dir = TempDir::new(label);
        let platform = Arc::new(MockPlatform::new());
        let edit = Arc::new(MockPlatform::new());
        let row = test_row("test-asr");
        let models = ModelDir::new(dir.path().join("models"));
        install(&models, &row);
        let loader = MockLoader::new(Behaviour::Say("hello world".into()));
        let parts = Parts {
            store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
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
    rig.enable();
    rig.dictate(1.0, 5);
    assert!(rig.mic_open(), "open for the next take's lead");
    rig.platform
        .clock()
        .advance_ns(MIC_IDLE.as_nanos() as u64 + 1_000_000_000);
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
