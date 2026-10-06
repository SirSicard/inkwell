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

use ink_engines::{EngineRow, ModelDir, Registry, SILERO_VAD_ID};
use ink_ffi::runtime::{Core, Parts};
use ink_ffi::voice::{DEFAULT_KEY, MIC_IDLE, VoicePlatform};
use ink_pipeline::tail::TailConfig;
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(10);
/// How long the bands must go unpublished to count as settled: many pump passes.
const SETTLED: Duration = Duration::from_millis(200);
/// 10 ms at the mock mic's 48 kHz.
const BLOCK: usize = 480;
/// Silence fed after a release, in seconds: room for the tail, which the chain waits for in
/// audio (`TAIL`, 300 ms at most), and short of the deadline where it stops waiting (the tail and
/// its grace, 550 ms after the release on the mock clock). See [`VoiceRig::release_take`].
const TAIL_ROOM: f64 = 0.4;

/// Right Option in this platform's one spelling: `right_option` on the Mac, the same key's
/// `right_alt` on Windows (`ink_ffi::hotkey`).
fn right_option() -> String {
    ink_ffi::hotkey::spelling("right_option")
}

struct VoiceRig {
    core: Option<Core>,
    events: Arc<Recorder>,
    /// The mic, the dictation key, insertion and focus.
    platform: Arc<MockPlatform>,
    /// The edit key (a hotkey source holds one binding).
    edit: Arc<MockPlatform>,
    loader: Arc<MockLoader>,
    bands: BandsReader,
    /// The library, when it is the SQLite store ([`build`](Self::build)): what an import writes
    /// before dictation reads it.
    sqlite: Option<Arc<ink_store::SqliteStore>>,
    _dir: TempDir,
}

impl VoiceRig {
    fn new(label: &str) -> Self {
        Self::build(label, true)
    }

    fn build(label: &str, with_platform: bool) -> Self {
        let sqlite = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
        let mut rig = Self::build_with(label, with_platform, sqlite.clone());
        rig.sqlite = Some(sqlite);
        rig
    }

    /// The SQLite library (rigs made by [`build`](Self::build)).
    fn sqlite(&self) -> &ink_store::SqliteStore {
        self.sqlite.as_deref().expect("a rig on the SQLite store")
    }

    /// A rig whose core keeps its settings in `store` (a failing one, say).
    fn with_store(label: &str, store: Arc<dyn ink_core::Store>) -> Self {
        Self::build_with(label, true, store)
    }

    fn build_with(label: &str, with_platform: bool, store: Arc<dyn ink_core::Store>) -> Self {
        Self::build_rows(label, with_platform, store, Vec::new())
    }

    /// A rig whose registry also lists `rows`, none of them installed.
    fn with_rows(label: &str, rows: Vec<EngineRow>) -> Self {
        let sqlite = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
        Self::build_rows(label, true, sqlite, rows)
    }

    fn build_rows(
        label: &str,
        with_platform: bool,
        store: Arc<dyn ink_core::Store>,
        rows: Vec<EngineRow>,
    ) -> Self {
        Self::build_full(
            label,
            with_platform,
            store,
            rows,
            Vec::new(),
            Default::default(),
            "hello world",
        )
    }

    /// A rig whose registry also lists `installed`, installed, with `local` for the core's own
    /// language model, and whose speech engine hears `heard` in every take.
    fn with_local(
        label: &str,
        installed: Vec<EngineRow>,
        local: ink_ffi::runtime::LocalParts,
        heard: &str,
    ) -> Self {
        let sqlite = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
        let mut rig = Self::build_full(
            label,
            true,
            sqlite.clone(),
            Vec::new(),
            installed,
            local,
            heard,
        );
        rig.sqlite = Some(sqlite);
        rig
    }

    fn build_full(
        label: &str,
        with_platform: bool,
        store: Arc<dyn ink_core::Store>,
        rows: Vec<EngineRow>,
        installed: Vec<EngineRow>,
        local: ink_ffi::runtime::LocalParts,
        heard: &str,
    ) -> Self {
        let dir = TempDir::new(label);
        let platform = Arc::new(MockPlatform::new());
        let edit = Arc::new(MockPlatform::new());
        let row = test_row("test-asr");
        let models = ModelDir::new(dir.path().join("models"));
        install(&models, &row);
        for r in &installed {
            install(&models, r);
        }
        let rows = [rows, installed].concat();
        let loader = MockLoader::new(Behaviour::Say(heard.into()));
        let parts = Parts {
            store,
            clock: platform.clock(),
            registry: Registry::new([vec![row], rows].concat()).unwrap(),
            models,
            loader: loader.clone(),
            installer: Arc::new(MockInstaller {
                generation: loader.generation.clone(),
                gate: None,
                installs: Default::default(),
            }),
            data_dir: dir.path().to_owned(),
            permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
            local,
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
            sqlite: None,
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

    /// The events so far, and what the warnings and discards among them said: why a take ended
    /// without the outcome a test waited for.
    fn said(&self) -> String {
        let said: Vec<String> = self
            .events
            .all()
            .into_iter()
            .filter(|v| v["type"] == "dictation.warning" || v["type"] == "dictation.discarded")
            .map(|v| v.to_string())
            .collect();
        format!("{:?}; {said:?}", self.events.types())
    }

    /// Lets go of `key` once the take its press opened has started (`started` takes had started
    /// before that press), then feeds [`TAIL_ROOM`]. The caller waits for the outcome.
    ///
    /// [`feed`](Self::feed) moves the mock clock about 50 times faster than the pump hands the
    /// chain the mic's audio (every 10 ms of real time, later on a loaded machine), so both waits
    /// are for the chain. A release sent at once can reach it before any audio of a stream the
    /// press opened, and then sits at the press: a take of 0 ms. A take starts only once the chain
    /// has heard the minimum hold, which places the release on the stream's timeline. And a clock
    /// moved past the tail's deadline while the take's audio is still on its way ends the take
    /// with what the chain has heard (`tail_cut_short`, then `too_short`). The tail arrives in
    /// audio, so the room completes it however late the pump is, and the clock stays short of the
    /// deadline until the caller has the outcome.
    fn release_take(&self, key: &MockPlatform, started: usize) {
        let room = Duration::from_secs_f64(TAIL_ROOM);
        let tail = ink_audio::take::TAIL;
        assert!(room > tail && room < tail + TailConfig::default().grace);
        assert!(
            self.events
                .wait_count("dictation.started", started + 1, WAIT),
            "{}",
            self.said()
        );
        assert!(key.release());
        self.silence(TAIL_ROOM);
    }

    /// Press, speak, release, and room for the tail; waits for the take to end.
    fn dictate(&self, seconds: f64, seed: u64) -> Value {
        let before = self.events.count("dictation.inserted");
        let started = self.events.count("dictation.started");
        assert!(self.platform.press());
        self.feed(&Self::speech(seconds, seed));
        self.release_take(&self.platform, started);
        assert!(
            self.events
                .wait_count("dictation.inserted", before + 1, WAIT),
            "{}",
            self.said()
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

    /// The bands once nothing has been published for [`SETTLED`]. The pump publishes on its own
    /// passes (every 10 ms, later on a loaded runner), so a fixed sleep can end before a publish
    /// that is already due.
    fn settled_bands(&self) -> ink_audio::BandsSnapshot {
        let until = Instant::now() + WAIT;
        let mut last = self.bands.read();
        loop {
            std::thread::sleep(SETTLED);
            let now = self.bands.read();
            if now.published == last.published {
                return now;
            }
            assert!(
                Instant::now() < until,
                "the bands never stopped being published"
            );
            last = now;
        }
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
    assert_eq!(ready["key"], DEFAULT_KEY);
    assert!(ready.get("edit_key").is_none(), "no edit key by default");
    assert_eq!(
        rig.platform.hotkey_binding().map(|b| b.0).as_deref(),
        Some(DEFAULT_KEY)
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
    rig.release_take(&rig.platform, 0);
    assert!(rig.events.wait_count("dictation.inserted", 1, WAIT));
    assert_eq!(rig.events.count("dictation.started"), 1);
    assert_eq!(rig.platform.inserted(), ["Hello world. "]);
}

/// The voice detector installed while dictation runs reaches it at once, not at the next launch:
/// when the install ends, the core resolves dictation's VAD again. Its row here has a stand-in
/// file, so a build with Silero (the release's `ink-engines/engine-silero`) says it could not load
/// it, which shows the install was acted on; a build without Silero has nothing to load and says
/// nothing new. The take after it is the barrier: the chain handles what was queued before it.
#[test]
fn a_voice_detector_installed_while_dictation_runs_is_taken_at_once() {
    let mut vad = ink_engines::silero_vad();
    vad.files[0].size = 4;
    let rig = VoiceRig::with_rows("vad-installed", vec![vad.clone()]);
    rig.enable();
    let first = rig.events.wait_type("dictation.voice_detection", WAIT);
    assert_eq!(
        (&first["available"], &first["reason"]),
        (&false.into(), &"model_missing".into())
    );

    // The download: its files arrive as the downloader leaves them, and the install ends.
    install(&ModelDir::new(rig._dir.path().join("models")), &vad);
    rig.command(&format!(
        r#"{{"cmd":"model.update","model":"{SILERO_VAD_ID}","next":"{SILERO_VAD_ID}"}}"#
    ));
    let finished = rig.events.wait_type("model.update_finished", WAIT);
    assert_eq!(finished["ok"], true, "{finished}");
    rig.dictate(1.2, 4);

    let said: Vec<Value> = rig
        .events
        .all()
        .into_iter()
        .filter(|e| e["type"] == "dictation.voice_detection")
        .collect();
    if Registry::builtin().unwrap().get(SILERO_VAD_ID).is_some() {
        assert_eq!(said.len(), 2, "{said:?}");
        assert_eq!(said[1]["reason"], "load_failed", "{said:?}");
    } else {
        assert_eq!(said.len(), 1, "nothing to load in this build: {said:?}");
    }
    rig.events.assert_valid();
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
            v["type"] == "dictation.ready" && v["key"] == right_option()
        })
        .expect("rebound");
    assert!(ready.get("ref").is_none());
    assert_eq!(
        rig.platform.hotkey_binding().map(|b| b.0).as_deref(),
        Some(right_option().as_str())
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
    assert_eq!(ready["key"], DEFAULT_KEY);
    assert_eq!(
        rig.edit.hotkey_binding().map(|b| b.0).as_deref(),
        Some("right_command")
    );
    // The dictation key itself as the edit key: refused, and said so.
    rig.command(&format!(
        r#"{{"cmd":"setting.set","key":"dictation.edit_key","value":"{DEFAULT_KEY}"}}"#
    ));
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

/// Any key the Mac can watch is a dictation key: a chord is stored in its one spelling, bound,
/// and named so in dictation.ready. One it cannot watch is refused with the reason, and the key
/// before stays.
#[cfg(target_os = "macos")]
#[test]
fn a_chord_is_a_dictation_key_stored_in_its_one_spelling() {
    let rig = VoiceRig::new("chord-key");
    rig.enable();
    rig.command(r#"{"cmd":"setting.set","key":"dictation.key","value":"Shift+Ctrl+Space"}"#);
    let value = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "setting.value" && v["key"] == "dictation.key"
        })
        .expect("stored");
    assert_eq!(value["value"], "ctrl+shift+space");
    rig.events
        .wait_for(WAIT, |v| {
            v["type"] == "dictation.ready" && v["key"] == "ctrl+shift+space"
        })
        .expect("rebound");
    assert_eq!(
        rig.platform.hotkey_binding().map(|b| b.0).as_deref(),
        Some("ctrl+shift+space")
    );
    let refused = rig
        .core()
        .command(r#"{"cmd":"setting.set","key":"dictation.key","value":"shift+a"}"#)
        .expect_err("a capital is typing");
    assert!(refused.contains("capital"), "{refused}");
    assert_eq!(
        rig.setting("dictation.key").as_deref(),
        Some("ctrl+shift+space")
    );
    // A take on the chord, as on any key: the hold machinery past the tap does not care.
    let inserted = rig.dictate(1.0, 5);
    assert_eq!(inserted["type"], "dictation.inserted", "{inserted}");
}

/// The edit key takes the same shapes; the dictation key under another spelling is still the
/// dictation key.
#[cfg(target_os = "macos")]
#[test]
fn the_edit_key_may_be_a_chord_but_never_the_dictation_key_respelt() {
    let rig = VoiceRig::new("chord-edit-key");
    rig.enable();
    rig.command(r#"{"cmd":"setting.set","key":"dictation.key","value":"ctrl+shift+space"}"#);
    rig.events
        .wait_for(WAIT, |v| v["key"] == "ctrl+shift+space")
        .expect("rebound");
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"cmd+option+e"}"#);
    rig.events
        .wait_for(WAIT, |v| {
            v["type"] == "dictation.ready" && v["edit_key"] == "option+cmd+e"
        })
        .expect("edit chord bound");
    assert_eq!(
        rig.edit.hotkey_binding().map(|b| b.0).as_deref(),
        Some("option+cmd+e")
    );
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"shift+ctrl+space"}"#);
    let same = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "dictation.ready" && v.get("edit_key_error").is_some()
        })
        .expect("refused as the dictation key");
    assert!(same.get("edit_key").is_none(), "{same}");
    assert!(rig.edit.hotkey_binding().is_none());
}

/// consent.allow turns voice edit on with a chord, stored in its one spelling.
#[cfg(target_os = "macos")]
#[test]
fn edit_consent_takes_a_chord_for_its_key() {
    let rig = VoiceRig::new("chord-edit-consent");
    rig.register_local();
    let state = rig.ask(
        r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"Option+Cmd+E","id":"c1"}"#,
        "c1",
    );
    assert_eq!(state["on"], true, "{state}");
    assert_eq!(
        rig.setting("dictation.edit_key").as_deref(),
        Some("option+cmd+e")
    );
    assert!(
        rig.core()
            .command(r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"e"}"#)
            .is_err(),
        "a key the Mac cannot watch"
    );
}

/// A key stored outside setting.set (an older build, an import) reaches the platform as it is,
/// never quietly swapped for the default: the platform holds it, or refuses it by name.
#[test]
fn a_stored_key_reaches_the_platform_as_it_is() {
    let rig = VoiceRig::new("stored-key");
    rig.core()
        .shared()
        .store
        .set_setting("dictation.key", "f13")
        .unwrap();
    let ready = rig.enable();
    assert_eq!(ready["key"], "f13", "{ready}");
    assert_eq!(
        rig.platform.hotkey_binding().map(|b| b.0).as_deref(),
        Some("f13")
    );
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
    rig.release_take(&rig.edit, 0);
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
    rig.release_take(&rig.platform, 0);
    assert!(rig.events.wait_count("dictation.inserted", 1, WAIT));
    // Room after the take (the mic stays open): the ink rests, and nothing more is published.
    // The take's end still publishes one still frame, on the pump's next pass: wait for the count
    // to settle rather than for a fixed time (a loaded runner can run that pass late).
    rig.silence(0.3);
    let after = rig.settled_bands();
    assert_eq!(after.bands, ink_audio::Bands::default(), "a still frame");
    rig.silence(0.3);
    assert_eq!(rig.settled_bands().published, after.published);
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

// --- Modes edited in Settings (feat/core-modes-edit) ------------------------------------------

impl VoiceRig {
    /// Sends a `modes.save` or `modes.delete` (with id `id`) while dictation runs, and waits for
    /// its answer and for the running dictation to take the change.
    fn change_modes(&self, json: &str, id: &str) -> Value {
        let ready = self.events.count("dictation.ready");
        let answer = self.ask(json, id);
        assert_eq!(answer["type"], "modes.listed", "{answer}");
        assert!(
            self.events.wait_count("dictation.ready", ready + 1, WAIT),
            "the running dictation never took the change"
        );
        answer
    }

    /// The default mode on `model` (a `polish_model` id), while dictation runs.
    fn default_on(&self, model: &str, id: &str) -> Value {
        self.change_modes(
            &format!(
                r#"{{"cmd":"modes.save","mode":{{"id":"default","polish_model":"{model}"}},"id":"{id}"}}"#
            ),
            id,
        )
    }

    /// Turns local-only mode on or off, as Settings > AI does, and waits for it to be said.
    fn local_only(&self, on: bool) {
        let value = if on { "on" } else { "off" };
        let said = self.events.count("setting.value");
        self.command(&format!(
            r#"{{"cmd":"setting.set","key":"llm.local_only","value":"{value}"}}"#
        ));
        assert!(
            self.events.wait_count("setting.value", said + 1, WAIT),
            "local-only never changed"
        );
    }

    fn focus_on(&self, app: &str) {
        self.platform.set_focus(ink_core::FocusInfo {
            app: Some(ink_core::AppRef {
                id: app.into(),
                pid: Some(1),
                name: "An app".into(),
            }),
            secure_input: false,
        });
    }
}

/// A mode saved in Settings writes the next take in the app it names; deleted, its app goes back
/// to the default mode. The first save writes 1.0's own document.
#[test]
fn a_saved_mode_reaches_a_running_dictation_at_once() {
    let rig = VoiceRig::new("modes-save");
    rig.enable();
    rig.focus_on("com.example.chat");
    assert_eq!(rig.dictate(1.0, 61)["text"], "Hello world.");
    let listed = rig.change_modes(
        r#"{"cmd":"modes.save","mode":{"name":"Chat","style":"relaxed","apps":["com.example.chat"]},"id":"s1"}"#,
        "s1",
    );
    let chat = listed["modes"][1]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        listed["saved"],
        chat.as_str(),
        "the new mode's id, for the editor"
    );
    assert!(rig.setting(ink_ffi::queries::MODES_KEY).is_some());
    assert_eq!(rig.dictate(1.0, 62)["text"], "hello world");
    rig.change_modes(
        &format!(r#"{{"cmd":"modes.delete","mode":"{chat}","id":"d1"}}"#),
        "d1",
    );
    assert_eq!(rig.dictate(1.0, 63)["text"], "Hello world.");
    rig.events.assert_valid();
}

/// A mode polishes on its own model, found at each take: one on this machine, under on-device
/// consent; a cloud one that consent does not cover gets nothing; one let go of gets nothing and
/// the take says so. The AI setting's model is never used in a mode's place. The listing names
/// every model a mode can pick, where it sends and whether the consent covers it.
#[test]
fn a_mode_polishes_on_its_own_model_only_where_the_consent_covers_it() {
    let rig = VoiceRig::new("modes-model");
    let setting = rig.register_local();
    let own = rig.register_model("other-llm", "other", true, "Hello world, mode!");
    let remote = rig.register_remote();
    rig.allow_on_device("c1");

    let listed = rig.ask(r#"{"cmd":"modes.list","id":"l1"}"#, "l1");
    assert_eq!(
        listed["setting_polish_model"], "engine:local-llm",
        "{listed}"
    );
    let models = listed["polish_models"].as_array().unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        ["engine:local-llm", "engine:other-llm", "engine:remote-llm"]
    );
    assert_eq!(models[1]["to"], "on_device");
    assert_eq!(models[1]["allowed"], true);
    assert_eq!(models[2]["to"], "cloud");
    assert_eq!(models[2]["name"], "remote");
    assert_eq!(
        models[2]["allowed"], false,
        "on-device consent does not cover it"
    );

    rig.enable();
    let unknown = rig.fails(
        r#"{"cmd":"modes.save","mode":{"id":"default","polish_model":"engine:nope"},"id":"u1"}"#,
        "u1",
    );
    assert_eq!(unknown["code"], "model_unknown");

    rig.default_on("engine:other-llm", "s1");
    assert_eq!(rig.dictate(1.0, 64)["text"], "Hello world, mode!");
    assert_eq!(own.calls.load(Ordering::SeqCst), 1);

    rig.default_on("engine:remote-llm", "s2");
    // The consent's refusal, not local-only mode's (which is asked first).
    rig.local_only(false);
    assert_eq!(rig.dictate(1.0, 65)["text"], "Hello world.");
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0, "nothing sent");
    let refused = rig.warnings("polish_not_allowed");
    assert_eq!(refused.len(), 1);
    assert_eq!(
        refused[0]["message"], "remote",
        "names the model's destination"
    );

    rig.default_on("engine:other-llm", "s3");
    rig.unregister("other-llm");
    assert_eq!(rig.dictate(1.0, 66)["text"], "Hello world.");
    assert_eq!(rig.warnings("polish_model_missing").len(), 1);
    assert_eq!(own.calls.load(Ordering::SeqCst), 1, "let go of: not called");
    assert_eq!(
        setting.calls.load(Ordering::SeqCst),
        0,
        "never the AI setting's model in a mode's place"
    );
    // The mode still names it, and the listing no longer lists it.
    let after = rig.ask(r#"{"cmd":"modes.list","id":"l2"}"#, "l2");
    assert_eq!(after["modes"][0]["polish_model"], "engine:other-llm");
    assert_eq!(after["polish_models"].as_array().unwrap().len(), 2);
    rig.events.assert_valid();
}

/// A mode's own model goes through local-only mode like every call: a cloud model the user did
/// agree to is still refused while local-only is on.
#[test]
fn a_mode_s_cloud_model_is_refused_while_local_only_is_on() {
    let rig = VoiceRig::new("modes-local-only");
    let remote = rig.register_remote();
    let state = rig.ask(
        r#"{"cmd":"consent.allow","feature":"polish","to":"cloud","endpoint":"shell engine remote-llm","id":"a1"}"#,
        "a1",
    );
    assert_eq!(state["allowed"], true, "{state}");
    rig.enable();
    rig.default_on("engine:remote-llm", "s1");
    assert_eq!(rig.dictate(1.0, 67)["text"], "Hello world.");
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0, "never called");
    let failed = rig.warnings("polish_failed");
    assert!(
        failed
            .last()
            .and_then(|w| w["message"].as_str())
            .is_some_and(|m| m.contains("local-only")),
        "{failed:?}"
    );
    rig.events.assert_valid();
}

/// The stored modes cannot be read: dictation goes on in the built-in default mode and polishes
/// nothing, even with polish on and its consent given, since the user's modes may send each to a
/// model of its own (or to none) and which cannot be known. `dictation.ready` names the modes.
#[test]
fn unreadable_modes_polish_nothing_at_dictation_time() {
    let rig = VoiceRig::new("modes-unreadable");
    let model = rig.register_local();
    rig.allow_on_device("c1");
    rig.core()
        .shared()
        .store
        .set_setting(ink_ffi::queries::MODES_KEY, "{not json")
        .unwrap();
    let ready = rig.enable();
    assert!(
        ready["settings_error"]
            .as_str()
            .is_some_and(|e| e.contains("the modes")),
        "{ready}"
    );
    assert_eq!(rig.dictate(1.0, 68)["text"], "Hello world.");
    assert_eq!(model.calls.load(Ordering::SeqCst), 0, "nothing sent");
    rig.events.assert_valid();
}

/// The owner's decision (2026-10-05): one polish consent per destination. The AI setting's model
/// on this machine and a Chat mode on a cloud model each polish under their own consent; revoking
/// one stops only the modes on it; revoking the last turns polish off. `consent.state` lists them.
#[test]
fn two_modes_on_two_destinations_polish_each_under_its_own_consent() {
    let rig = VoiceRig::new("consent-per-destination");
    let local = rig.register_local();
    let cloud = rig.register_model("remote-llm", "remote", false, "Hello world, cloud!");
    rig.local_only(false);
    rig.allow_on_device("c1");
    // Not the AI setting's model (that is local-llm): one a mode can pick, so it can be agreed to.
    let state = rig.ask(
        r#"{"cmd":"consent.allow","feature":"polish","to":"cloud","endpoint":"shell engine remote-llm","id":"c2"}"#,
        "c2",
    );
    assert_eq!(state["type"], "consent.state", "{state}");
    assert_eq!(
        state["allowed"], true,
        "the AI setting's model is still covered"
    );
    let consents = state["consents"].as_array().unwrap();
    assert_eq!(consents.len(), 2, "{state}");
    assert_eq!(consents[0]["to"], "on_device");
    assert_eq!(consents[1]["to"], "cloud");
    assert_eq!(consents[1]["endpoint"], "shell engine remote-llm");
    assert_eq!(consents[1]["name"], "remote");
    let stored: Value = serde_json::from_str(&rig.setting("llm.consent.polish").unwrap()).unwrap();
    assert_eq!(stored.as_array().map(Vec::len), Some(2), "{stored}");

    rig.enable();
    let listed = rig.change_modes(
        r#"{"cmd":"modes.save","mode":{"name":"Chat","polish":true,"apps":["com.example.chat"],"polish_model":"engine:remote-llm"},"id":"s1"}"#,
        "s1",
    );
    assert_eq!(
        listed["modes"][1]["polish_model_state"], "ready",
        "{listed}"
    );
    let models = listed["polish_models"].as_array().unwrap();
    assert!(models.iter().all(|m| m["allowed"] == true), "{listed}");

    let dictate_in = |app: &str, seed: u64| {
        rig.focus_on(app);
        rig.dictate(1.0, seed)["text"].as_str().unwrap().to_owned()
    };
    assert_eq!(dictate_in("com.example.chat", 81), "Hello world, cloud!");
    assert_eq!(dictate_in("com.example.mail", 82), "Hello, world!");
    assert_eq!(
        (
            cloud.calls.load(Ordering::SeqCst),
            local.calls.load(Ordering::SeqCst)
        ),
        (1, 1)
    );

    // The cloud consent revoked: Chat goes in as said and asks for it; the rest still polishes.
    let ready = rig.events.count("dictation.ready");
    let state = rig.ask(
        r#"{"cmd":"consent.revoke","feature":"polish","to":"cloud","endpoint":"shell engine remote-llm","id":"r1"}"#,
        "r1",
    );
    assert_eq!(state["consents"].as_array().unwrap().len(), 1, "{state}");
    assert_eq!(state["on"], true);
    assert!(rig.events.wait_count("dictation.ready", ready + 1, WAIT));
    assert_eq!(dictate_in("com.example.chat", 83), "Hello world.");
    assert_eq!(rig.warnings("polish_not_allowed").len(), 1);
    assert_eq!(dictate_in("com.example.mail", 84), "Hello, world!");
    assert_eq!(
        cloud.calls.load(Ordering::SeqCst),
        1,
        "nothing more sent there"
    );
    let listed = rig.ask(r#"{"cmd":"modes.list","id":"l1"}"#, "l1");
    let remote = listed["polish_models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "engine:remote-llm")
        .unwrap()
        .clone();
    assert_eq!(remote["allowed"], false, "{remote}");
    // Revoking it again changes nothing, and is no failure.
    let again = rig.ask(
        r#"{"cmd":"consent.revoke","feature":"polish","to":"cloud","endpoint":"shell engine remote-llm","id":"r2"}"#,
        "r2",
    );
    assert_eq!(again["type"], "consent.state");

    // The last one revoked: polish is off with it.
    let state = rig.ask(
        r#"{"cmd":"consent.revoke","feature":"polish","to":"on_device","id":"r3"}"#,
        "r3",
    );
    assert_eq!(state["on"], false, "{state}");
    assert_eq!(state["consents"].as_array().map(Vec::len), Some(0));
    assert_eq!(rig.setting("dictation.polish").as_deref(), Some("off"));
    assert_eq!(rig.setting("llm.consent.polish").as_deref(), Some("none"));
    let refused = rig.fails(
        r#"{"cmd":"consent.revoke","feature":"edit","to":"on_device","id":"r4"}"#,
        "r4",
    );
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|m| m.contains("one consent"))
    );
    rig.events.assert_valid();
}

/// The single polish consent a build before consents per destination stored reads as one, so
/// nothing is asked again; the next consent is added beside it, and the list is what is stored.
#[test]
fn the_single_polish_consent_an_older_build_stored_is_kept() {
    let rig = VoiceRig::new("consent-migration");
    let local = rig.register_local();
    rig.register_model("remote-llm", "remote", false, "Hello world, cloud!");
    let store = &rig.core().shared().store;
    store.set_setting("dictation.polish", "on").unwrap();
    store
        .set_setting("llm.consent.polish", r#"{"to":"on_device"}"#)
        .unwrap();
    let state = rig.polish_state("g1");
    assert_eq!(state["allowed"], true, "{state}");
    assert_eq!(state["consents"].as_array().map(Vec::len), Some(1));
    // Agreed again for the same destination: the old object becomes a list of one.
    rig.allow_on_device("a0");
    assert_eq!(
        rig.setting("llm.consent.polish").as_deref(),
        Some(r#"[{"to":"on_device"}]"#)
    );
    rig.enable();
    assert_eq!(rig.dictate(1.0, 85)["text"], "Hello, world!");
    assert_eq!(local.calls.load(Ordering::SeqCst), 1);
    rig.ask(
        r#"{"cmd":"consent.allow","feature":"polish","to":"cloud","endpoint":"shell engine remote-llm","id":"c1"}"#,
        "c1",
    );
    let stored: Value = serde_json::from_str(&rig.setting("llm.consent.polish").unwrap()).unwrap();
    assert_eq!(stored[0]["to"], "on_device", "{stored}");
    assert_eq!(stored[1]["endpoint"], "shell engine remote-llm", "{stored}");
    rig.events.assert_valid();
}

/// `modes.listed` says truly whether polish may use each model: a cloud model the user agreed to
/// is not allowed while local-only mode is on, and says that is why.
#[test]
fn a_model_local_only_refuses_is_listed_as_not_allowed() {
    let rig = VoiceRig::new("modes-listed-local-only");
    rig.register_remote();
    let state = rig.ask(
        r#"{"cmd":"consent.allow","feature":"polish","to":"cloud","endpoint":"shell engine remote-llm","id":"a1"}"#,
        "a1",
    );
    assert_eq!(state["type"], "consent.state", "{state}");
    let listed = rig.ask(r#"{"cmd":"modes.list","id":"l1"}"#, "l1");
    let model = &listed["polish_models"][0];
    assert_eq!(model["allowed"], false, "{listed}");
    assert_eq!(model["blocked_local_only"], true);
    assert_eq!(model["model"], "remote");
    rig.local_only(false);
    let listed = rig.ask(r#"{"cmd":"modes.list","id":"l2"}"#, "l2");
    assert_eq!(listed["polish_models"][0]["allowed"], true, "{listed}");
    assert_eq!(listed["polish_models"][0]["blocked_local_only"], false);
    rig.events.assert_valid();
}

/// Over polish consents that cannot be read, a revoke fails and writes nothing: the user could
/// not see what they would be revoking.
#[test]
fn a_revoke_over_unreadable_consents_fails_and_writes_nothing() {
    let rig = VoiceRig::new("consent-revoke-unreadable");
    rig.register_local();
    let store = &rig.core().shared().store;
    store.set_setting("dictation.polish", "on").unwrap();
    store
        .set_setting("llm.consent.polish", "[{not json")
        .unwrap();
    let failed = rig.fails(
        r#"{"cmd":"consent.revoke","feature":"polish","to":"on_device","id":"r1"}"#,
        "r1",
    );
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("couldn't read")),
        "{failed}"
    );
    assert_eq!(
        rig.setting("llm.consent.polish").as_deref(),
        Some("[{not json")
    );
    assert_eq!(rig.setting("dictation.polish").as_deref(), Some("on"));
    rig.events.assert_valid();
}

/// Local-only mode is asked before the consent: the AI setting's cloud model, with a consent that
/// covers it, is refused as local-only (polish_failed), never as a consent the user could give.
#[test]
fn local_only_refuses_a_consented_cloud_model_as_local_only() {
    let rig = VoiceRig::new("local-only-first");
    let remote = rig.register_remote();
    let state = rig.ask(
        r#"{"cmd":"consent.allow","feature":"polish","to":"cloud","endpoint":"shell engine remote-llm","id":"a1"}"#,
        "a1",
    );
    assert_eq!(state["allowed"], true, "{state}");
    rig.enable();
    assert_eq!(rig.dictate(1.0, 86)["text"], "Hello world.");
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
    assert!(rig.warnings("polish_not_allowed").is_empty());
    assert!(
        rig.warnings("polish_failed")
            .last()
            .and_then(|w| w["message"].as_str())
            .is_some_and(|m| m.contains("local-only")),
        "{:?}",
        rig.warnings("polish_failed")
    );
    rig.events.assert_valid();
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
    fn output_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        self.inner.output_devices()
    }
    fn automatic_input(&self) -> Result<Option<ink_core::AutoInput>, PlatformError> {
        self.inner.automatic_input()
    }
    fn watch_devices(
        &self,
        on_change: ink_core::EventSink<ink_core::DeviceChange>,
    ) -> Result<(), PlatformError> {
        self.inner.watch_devices(on_change)
    }
    fn unwatch_devices(&self) {
        self.inner.unwatch_devices();
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

    /// Registers a model on this machine, as the Mac registers Apple's on-device model. Its answer
    /// is a cleanup of what the rig dictates ("hello world"), as polish's answer must be.
    fn register_local(&self) -> &'static RemoteModel {
        self.register_model("local-llm", "on-device", true, "Hello, world!")
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
        r#"{"cmd":"consent.allow","feature":"polish","to":"cloud","endpoint":"shell engine remote-llm","id":"allow"}"#,
        "allow",
    );
    assert_eq!(state["type"], "consent.state", "{state}");
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
    // The user agreed to send edits to this provider: what refuses it here is local-only alone.
    let state = rig.ask(
        r#"{"cmd":"consent.allow","feature":"edit","to":"cloud","endpoint":"shell engine remote-llm","key":"right_command","id":"e0"}"#,
        "e0",
    );
    assert_eq!(state["allowed"], true, "{state}");
    rig.events
        .wait_for(WAIT, |v| v["edit_key"] == "right_command")
        .expect("bound");
    rig.platform.set_selection(Some("teh cat"));
    rig.sync_edit_clock();
    assert!(rig.edit.press());
    rig.feed(&VoiceRig::speech(1.0, 12));
    rig.sync_edit_clock();
    rig.release_take(&rig.edit, 0);
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
// goes: consent.allow records that consent in the core's store and turns polish on; nothing else
// does. A model that changes destination since gets nothing until the user agrees again.

impl VoiceRig {
    fn polish_state(&self, id: &str) -> Value {
        let state = self.ask(
            &format!(r#"{{"cmd":"consent.get","feature":"polish","id":"{id}"}}"#),
            id,
        );
        assert_eq!(state["type"], "consent.state", "{state}");
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
            &format!(
                r#"{{"cmd":"consent.allow","feature":"polish","to":"on_device","id":"{id}"}}"#
            ),
            id,
        );
        assert_eq!(state["type"], "consent.state", "{state}");
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

/// consent.allow records the consent in the core's store and turns polish on; the next take is
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
        rig.setting("llm.consent.polish").as_deref(),
        Some(r#"[{"to":"on_device"}]"#)
    );
    rig.enable();
    rig.dictate(1.0, 51);
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert!(
        rig.platform
            .inserted()
            .last()
            .is_some_and(|s| s.contains("Hello, world!")),
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
    let failed = rig.fails(
        r#"{"cmd":"consent.allow","feature":"polish","to":"on_device","id":"f1"}"#,
        "f1",
    );
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.contains("no language model")),
        "{failed}"
    );
    rig.register_local();
    rig.fails(
        r#"{"cmd":"consent.allow","feature":"polish","to":"cloud","endpoint":"shell engine elsewhere","id":"f2"}"#,
        "f2",
    );
    assert_eq!(rig.setting("dictation.polish"), None, "polish stays off");
    assert_eq!(rig.setting("llm.consent.polish"), None, "nothing recorded");
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
        .wait_for(WAIT, |v| v["type"] == "consent.state" && v["on"] == false)
        .expect("consent.state after off");
    assert_eq!(state["allowed"], false, "{state}");
    assert!(state.get("allowed_to").is_none(), "{state}");
    assert_eq!(rig.setting("llm.consent.polish").as_deref(), Some("none"));
    assert!(
        rig.core()
            .command(r#"{"cmd":"setting.set","key":"dictation.polish","value":"on"}"#)
            .is_err(),
        "only consent.allow turns polish on"
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

    // Local-only mode (on by default) refuses it first, and says so: no consent would let it.
    rig.dictate(1.0, 51);
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0, "nothing sent");
    assert!(rig.warnings("polish_not_allowed").is_empty());
    assert!(
        rig.warnings("polish_failed")
            .last()
            .and_then(|w| w["message"].as_str())
            .is_some_and(|m| m.contains("local-only")),
        "{:?}",
        rig.warnings("polish_failed")
    );

    rig.local_only(false);
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
        .set_setting("llm.consent.polish", "{not json")
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
    let failed = rig.fails(
        r#"{"cmd":"consent.allow","feature":"polish","to":"on_device","id":"t1"}"#,
        "t1",
    );
    assert!(
        failed["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("couldn't")),
        "{failed}"
    );
    assert_eq!(rig.setting("dictation.polish"), None, "switch not saved");
    assert_eq!(rig.setting("llm.consent.polish"), None, "nor the consent");

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
        rig.setting("llm.consent.polish").as_deref(),
        Some(r#"[{"to":"on_device"}]"#),
        "both as they were"
    );
    store.heal();
    rig.events.assert_valid();
}

// --- Voice edit consent (PR #86 review) --------------------------------------------------------
//
// Voice edit sends the selection and the spoken instruction to a language model, so it has a
// consent of its own: consent.allow with feature "edit" records it and sets the edit key in one
// write; the edit key set to off withdraws it; an edit without a covering consent sends nothing
// and changes nothing, and says so.

impl VoiceRig {
    /// Holds the edit key over a selection, says an instruction, and waits for the outcome.
    fn voice_edit(&self, seed: u64) -> Value {
        let outcomes = |r: &Recorder| -> Vec<Value> {
            r.all()
                .into_iter()
                .filter(|v| v["type"] == "dictation.edited" || v["type"] == "dictation.edit_failed")
                .collect()
        };
        let before = outcomes(&self.events).len();
        let started = self.events.count("dictation.started");
        self.platform.set_selection(Some("teh cat"));
        self.sync_edit_clock();
        assert!(self.edit.press());
        self.feed(&VoiceRig::speech(1.0, seed));
        self.sync_edit_clock();
        self.release_take(&self.edit, started);
        let until = Instant::now() + WAIT;
        loop {
            if let Some(v) = outcomes(&self.events).into_iter().nth(before) {
                return v;
            }
            assert!(Instant::now() < until, "no edit outcome: {}", self.said());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn edit_state(&self, id: &str) -> Value {
        let state = self.ask(
            &format!(r#"{{"cmd":"consent.get","feature":"edit","id":"{id}"}}"#),
            id,
        );
        assert_eq!(state["type"], "consent.state", "{state}");
        assert_eq!(state["feature"], "edit", "{state}");
        state
    }
}

/// The edit key set without a consent (an older build's store, or a shell that skipped the step):
/// the edit sends nothing, the selection is left alone, and the reason is not_allowed.
#[test]
fn an_edit_key_without_consent_edits_nothing() {
    let rig = VoiceRig::new("edit-no-consent");
    let model = rig.register_local();
    rig.enable();
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"right_command"}"#);
    rig.events
        .wait_for(WAIT, |v| v["edit_key"] == "right_command")
        .expect("bound");
    let state = rig.edit_state("s1");
    assert_eq!(state["on"], true);
    assert_eq!(state["allowed"], false);
    let outcome = rig.voice_edit(71);
    assert_eq!(outcome["type"], "dictation.edit_failed", "{outcome}");
    assert_eq!(outcome["reason"], "not_allowed", "{outcome}");
    assert_eq!(outcome["message"], "a model on this machine");
    assert_eq!(model.calls.load(Ordering::SeqCst), 0, "nothing sent");
    assert!(
        rig.platform.inserted().is_empty(),
        "the selection is left alone"
    );
    rig.events.assert_valid();
}

/// consent.allow for edit records the consent and sets the key in one write; the edit then goes
/// through. Polish stays off: its consent is its own.
#[test]
fn edit_consent_turns_voice_edit_on_and_leaves_polish_alone() {
    let rig = VoiceRig::new("edit-allow");
    let model = rig.register_local();
    rig.enable();
    let state = rig.ask(
        r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"right_command","id":"a1"}"#,
        "a1",
    );
    assert_eq!(state["feature"], "edit", "{state}");
    assert_eq!(state["on"], true, "{state}");
    assert_eq!(state["allowed"], true, "{state}");
    assert_eq!(
        rig.setting("dictation.edit_key").as_deref(),
        Some("right_command")
    );
    assert_eq!(
        rig.setting("llm.consent.edit").as_deref(),
        Some(r#"{"to":"on_device"}"#)
    );
    assert_eq!(rig.setting("llm.consent.polish"), None);
    assert_eq!(
        rig.polish_state("a2")["allowed"],
        false,
        "polish not allowed by it"
    );
    rig.events
        .wait_for(WAIT, |v| v["edit_key"] == "right_command")
        .expect("bound");
    let outcome = rig.voice_edit(72);
    assert_eq!(outcome["type"], "dictation.edited", "{outcome}");
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    rig.events.assert_valid();
}

/// A key the edit consent refuses (none, or off) is refused before anything is written.
#[test]
fn edit_consent_needs_its_key() {
    let rig = VoiceRig::new("edit-key");
    rig.register_local();
    for bad in [
        r#"{"cmd":"consent.allow","feature":"edit","to":"on_device"}"#,
        r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"off"}"#,
    ] {
        assert!(rig.core().command(bad).is_err(), "{bad}");
    }
    assert_eq!(rig.setting("llm.consent.edit"), None);
}

/// The edit key set to off withdraws the edit consent in the same write, so choosing a key again
/// asks again; a store that refuses the write leaves both as they were.
#[test]
fn turning_voice_edit_off_withdraws_its_consent_or_changes_nothing() {
    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    let rig = VoiceRig::with_store("edit-off", store.clone());
    rig.register_local();
    rig.ask(
        r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"right_option","id":"o1"}"#,
        "o1",
    );
    store.fail(&["set_settings"]);
    rig.fails(
        r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"off","id":"o2"}"#,
        "o2",
    );
    assert_eq!(
        rig.setting("dictation.edit_key").as_deref(),
        Some(right_option().as_str())
    );
    assert_eq!(
        rig.setting("llm.consent.edit").as_deref(),
        Some(r#"{"to":"on_device"}"#)
    );
    store.heal();
    rig.command(r#"{"cmd":"setting.set","key":"dictation.edit_key","value":"off","id":"o3"}"#);
    let state = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "consent.state" && v["feature"] == "edit" && v["on"] == false
        })
        .expect("consent.state after off");
    assert_eq!(state["allowed"], false, "{state}");
    assert_eq!(rig.setting("llm.consent.edit").as_deref(), Some("none"));
    rig.events.assert_valid();
}

/// Edit consent for this Mac, then the model becomes a cloud one: the state says paused and names
/// the provider, and an edit sends nothing and changes nothing.
#[test]
fn an_edit_model_that_moves_to_the_cloud_gets_nothing() {
    let rig = VoiceRig::new("edit-moved");
    rig.register_local();
    rig.enable();
    rig.ask(
        r#"{"cmd":"consent.allow","feature":"edit","to":"on_device","key":"right_command","id":"m1"}"#,
        "m1",
    );
    rig.events
        .wait_for(WAIT, |v| v["edit_key"] == "right_command")
        .expect("bound");
    rig.unregister("local-llm");
    let remote = rig.register_remote();
    // The consent's refusal, not local-only mode's (which is asked first).
    rig.local_only(false);
    let state = rig.edit_state("m2");
    assert_eq!(state["on"], true);
    assert_eq!(state["allowed"], false);
    assert_eq!(state["to"], "cloud");
    let outcome = rig.voice_edit(73);
    assert_eq!(outcome["reason"], "not_allowed", "{outcome}");
    assert_eq!(outcome["message"], "remote");
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0, "nothing sent");
    assert!(rig.platform.inserted().is_empty());
}
// ---------------------------------------------------------------------------------------------
// Inkwell 0.2's snippets, voice commands and hotkey, imported and then used
// ---------------------------------------------------------------------------------------------

/// Imports a synthetic 0.2 folder holding these files into the rig's store, as the import check
/// binary does.
fn import_0_2(rig: &VoiceRig, files: &[(&str, &str)]) {
    let dir = TempDir::new("legacy-0.2");
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let source =
        ink_store::import::Inkwell02::read(dir.path(), &ink_store::import::NoKeychain).unwrap();
    rig.sqlite().import_inkwell02(&source).unwrap();
}

const SNIPPETS_0_2: &str =
    r#"{"snippets":[{"id":"s1","trigger":"hello","expansion":"Greetings"}]}"#;

#[test]
fn an_imported_modifier_hotkey_is_the_key_held_and_imported_snippets_expand() {
    let rig = VoiceRig::new("import-used");
    import_0_2(
        &rig,
        &[
            (
                "settings.json",
                r#"{"hotkey":"right_opt","recording_mode":"ptt","polish_enabled":true}"#,
            ),
            ("snippets.json", SNIPPETS_0_2),
        ],
    );
    let ready = rig.enable();
    assert_eq!(ready["key"], right_option(), "{ready}");
    assert!(ready.get("settings_error").is_none(), "{ready}");
    assert_eq!(
        rig.platform.hotkey_binding().map(|b| b.0).as_deref(),
        Some(right_option().as_str())
    );
    // The mock engine hears "hello world"; the imported snippet expands "hello".
    let inserted = rig.dictate(1.2, 1);
    assert_eq!(inserted["text"], "Greetings world.");
    // Nothing turned polish on: the import's polish_enabled stays under the import's prefix.
    assert_eq!(
        ink_core::Store::setting(rig.sqlite(), "dictation.polish").unwrap(),
        None
    );
    // Nothing to say about a key that carried over.
    let notes = rig.ask(r#"{"cmd":"import.notes","id":"n"}"#, "n");
    assert!(notes.get("key").is_none(), "{notes}");
    rig.events.assert_valid();
}

/// Windows: 0.2's Fn becomes right Ctrl, which dictation holds, and Settings says the key was
/// replaced, naming both (never a dead key that only Settings > Voice reports).
#[cfg(windows)]
#[test]
fn on_windows_an_imported_fn_is_replaced_by_right_ctrl_and_settings_says_so() {
    let rig = VoiceRig::new("import-fn-win");
    import_0_2(
        &rig,
        &[("settings.json", r#"{"hotkey":"fn","recording_mode":"ptt"}"#)],
    );
    assert_eq!(rig.enable()["key"], "right_control");
    let notes = rig.ask(r#"{"cmd":"import.notes","id":"n"}"#, "n");
    assert_eq!(notes["key"]["outcome"], "replaced", "{notes}");
    assert_eq!(notes["key"]["hotkey"], "fn");
    assert_eq!(notes["key"]["key"], "right_control");
    rig.events.assert_valid();
}

#[test]
fn an_unmappable_hotkey_keeps_the_default_and_settings_says_so_once() {
    let rig = VoiceRig::new("import-unmapped");
    import_0_2(
        &rig,
        &[(
            "settings.json",
            r#"{"hotkey":"super+shift+space","recording_mode":"toggle"}"#,
        )],
    );
    assert_eq!(rig.enable()["key"], DEFAULT_KEY);
    let notes = rig.ask(r#"{"cmd":"import.notes","id":"n1"}"#, "n1");
    assert_eq!(notes["key"]["hotkey"], "super+shift+space");
    assert_eq!(notes["key"]["outcome"], "combination");
    assert_eq!(notes["key"]["toggle"], true);
    rig.command(r#"{"cmd":"setting.set","key":"import.key_note","value":"dismissed"}"#);
    let set = rig.events.wait_type("setting.value", WAIT);
    assert_eq!(set["value"], "dismissed");
    let again = rig.ask(r#"{"cmd":"import.notes","id":"n2"}"#, "n2");
    assert!(again.get("key").is_none(), "said once: {again}");
    rig.events.assert_valid();
}

#[test]
fn an_imported_voice_command_is_carried_out() {
    let rig = VoiceRig::new("import-command");
    import_0_2(
        &rig,
        &[(
            "voice-commands.json",
            r#"{"enabled":true,"wake_prefix":"hello","commands":[
                {"id":"c1","triggers":["world"],"action":{"type":"insert_text","text":"Typed by voice"},"enabled":true}]}"#,
        )],
    );
    rig.enable();
    // "hello world" is now the wake word and a trigger.
    assert!(rig.platform.press());
    rig.feed(&VoiceRig::speech(1.2, 1));
    rig.release_take(&rig.platform, 0);
    let command = rig.events.wait_type("dictation.command", WAIT);
    assert_eq!(command["action"], "insert_text");
    assert_eq!(command["carried_out"], true);
    assert_eq!(rig.platform.inserted(), ["Typed by voice "]);
    assert_eq!(rig.events.count("dictation.inserted"), 0, "not a dictation");
    let listed = rig.ask(r#"{"cmd":"voice_commands.list","id":"l"}"#, "l");
    assert_eq!(listed["from_import"], true);
    assert_eq!(listed["commands"][0]["value"], "Typed by voice");
    rig.events.assert_valid();
}

/// The screens' import (`import.run`) while dictation is live: the running dictation holds the
/// imported key and expands the imported snippets at once, without being enabled again.
#[test]
fn an_import_from_the_screens_reaches_a_running_dictation_at_once() {
    let rig = VoiceRig::new("import-run-live");
    let source = TempDir::new("legacy-0.2-live");
    for (name, text) in [
        (
            "settings.json",
            r#"{"hotkey":"right_opt","recording_mode":"ptt"}"#,
        ),
        ("snippets.json", SNIPPETS_0_2),
    ] {
        std::fs::write(source.path().join(name), text).unwrap();
    }
    rig.core().set_import02(ink_ffi::import02::Import02 {
        library: rig.sqlite.clone().unwrap(),
        source: Some(source.path().to_owned()),
        keys: None,
    });
    assert_eq!(rig.enable()["key"], DEFAULT_KEY);
    let finished = rig.ask(r#"{"cmd":"import.run","id":"i"}"#, "i");
    assert_eq!(finished["type"], "import.finished", "{finished}");
    // After the answer, the running dictation is handed its settings again.
    assert!(rig.events.wait_count("dictation.ready", 2, WAIT));
    let types = rig.events.types();
    let answered = types.iter().position(|t| t == "import.finished").unwrap();
    let rebound = types.iter().rposition(|t| t == "dictation.ready").unwrap();
    assert!(answered < rebound, "answered first: {types:?}");
    let ready = rig
        .events
        .all()
        .into_iter()
        .rfind(|v| v["type"] == "dictation.ready")
        .unwrap();
    assert_eq!(ready["key"], right_option(), "{ready}");
    assert_eq!(
        rig.platform.hotkey_binding().map(|b| b.0).as_deref(),
        Some(right_option().as_str())
    );
    assert_eq!(rig.dictate(1.2, 1)["text"], "Greetings world.");
    rig.events.assert_valid();
}

#[test]
fn a_snippet_saved_in_settings_reaches_a_running_dictation_at_once() {
    let rig = VoiceRig::new("snippet-saved");
    import_0_2(&rig, &[("snippets.json", SNIPPETS_0_2)]);
    rig.enable();
    let listed = rig.ask(r#"{"cmd":"snippets.list","id":"l"}"#, "l");
    assert_eq!(listed["from_import"], true);
    let saved = rig.ask(
        r#"{"cmd":"snippets.save","id":"s","snippets":[
            {"id":"s1","trigger":"hello","expansion":"Hi there","category":"","enabled":true}]}"#,
        "s",
    );
    assert_eq!(saved["type"], "snippets.listed");
    assert_eq!(saved["from_import"], false);
    // The save rebinds nothing and hands the running chain its new list.
    rig.events.wait_count("dictation.ready", 2, WAIT);
    assert_eq!(rig.dictate(1.2, 1)["text"], "Hi there world.");
    // A save that cannot be read is refused before it is queued, and changes nothing.
    let refused = rig.core().command(
        r#"{"cmd":"snippets.save","id":"bad","snippets":[{"id":"x","trigger":"a","expansion":"y"},{"id":"x","trigger":"b","expansion":"z"}]}"#,
    );
    assert!(refused.is_err());
    let listed = rig.ask(r#"{"cmd":"snippets.list","id":"l2"}"#, "l2");
    assert_eq!(listed["snippets"][0]["expansion"], "Hi there");
    rig.events.assert_valid();
}

// --- The core's own language model (Windows' Qwen3; feat/core-local-llm) -----------------------
//
// Once one is downloaded it is the AI setting's model while no provider is chosen, under the
// on-device consent like Apple's model on the Mac: one tap, and the core fails closed without it.

fn chat_row() -> EngineRow {
    language_row("test-chat", ink_engines::LanguageSize::Default, "Test Chat")
}

impl VoiceRig {
    /// Press, speak, release, and room for the tail; waits for the take to end however it ends.
    fn take(&self, seconds: f64, seed: u64) {
        let ended = |r: &Self| {
            [
                "dictation.inserted",
                "dictation.discarded",
                "dictation.failed",
            ]
            .iter()
            .map(|t| r.events.count(t))
            .sum::<usize>()
        };
        let before = ended(self);
        let started = self.events.count("dictation.started");
        assert!(self.platform.press());
        self.feed(&Self::speech(seconds, seed));
        self.release_take(&self.platform, started);
        let until = Instant::now() + WAIT;
        while ended(self) == before {
            assert!(
                Instant::now() < until,
                "the take never ended: {}",
                self.said()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn the_downloaded_model_polishes_only_under_the_on_device_consent() {
    let local = MockLocalLoader::new("Hello, world!");
    let rig = VoiceRig::with_local(
        "local-polish",
        vec![chat_row()],
        local.parts(None),
        "hello world",
    );
    let state = rig.polish_state("p1");
    assert_eq!(state["to"], "on_device", "{state}");
    assert_eq!(state["name"], "Test Chat", "{state}");
    assert_eq!(state["allowed"], false);
    rig.enable();
    // Without the consent, polish is off: nothing is loaded or sent.
    assert_eq!(rig.dictate(1.0, 81)["text"], "Hello world.");
    assert_eq!((local.loads(), local.calls()), (0, 0));

    // One tap: the on-device consent, which turns polish on.
    let allowed = rig.allow_on_device("a1");
    assert_eq!(allowed["allowed_to"], "on_device", "{allowed}");
    assert_eq!(rig.dictate(1.0, 82)["text"], "Hello, world!");
    assert_eq!(local.calls(), 1);

    // A consent for somewhere else only (an earlier cloud provider's) does not cover it: the
    // take goes in as said, and nothing is sent.
    store_polish_consents(
        rig.core(),
        &[ink_pipeline::consent::LlmConsent::Cloud {
            endpoint: "https://api.example.com/v1".into(),
            name: "Example".into(),
        }],
    );
    assert_eq!(rig.dictate(1.0, 83)["text"], "Hello world.");
    assert_eq!(local.calls(), 1, "nothing sent");
    assert_eq!(rig.warnings("polish_not_allowed").len(), 1);
    rig.events.assert_valid();
}

#[test]
fn a_take_that_will_polish_on_the_downloaded_model_warms_it_at_its_start() {
    let local = MockLocalLoader::new("Hello, world!");
    // The speech engine hears nothing, so no take has text to polish: only a warm-up loads it.
    let rig = VoiceRig::with_local("local-warm", vec![chat_row()], local.parts(None), "");
    rig.enable();
    rig.take(1.0, 91);
    assert_eq!(local.loads(), 0, "polish off: not warmed");

    rig.allow_on_device("a1");
    rig.take(1.0, 92);
    let until = Instant::now() + WAIT;
    while local.loads() == 0 {
        assert!(Instant::now() < until, "never warmed");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        (local.loads(), local.calls()),
        (1, 0),
        "loaded, never called"
    );
    assert_eq!(
        rig.core().shared().local.resident(),
        ["test-chat".to_owned()],
        "it stays loaded for the next take"
    );

    // Unloaded, then a take in a mode that does not polish: nothing loads it again.
    rig.core().shared().local.unload("test-chat").unwrap();
    rig.change_modes(
        r#"{"cmd":"modes.save","mode":{"id":"default","polish":false},"id":"m1"}"#,
        "m1",
    );
    rig.take(1.0, 93);
    // The warm-up is decided on its own thread: give it the time the first one took.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(local.loads(), 1, "not warmed for a take it will not polish");
    assert!(rig.core().shared().local.resident().is_empty());
    rig.events.assert_valid();
}
