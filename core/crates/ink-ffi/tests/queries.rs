//! The screens' commands (`queries`): permissions, owed, notes, settings, modes and the model
//! catalogue, on their own thread, every event checked against the schema.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

use common::*;
use ink_audio::bands_channel;
use ink_core::mock::MockPlatform;
use ink_core::{
    Channel, NewCommitment, NewRecord, Permission, PermissionProbe, PermissionState, RecordKind,
    Span, Store,
};
use ink_engines::{ModelDir, Registry};
use ink_ffi::queries::{MODES_KEY, SYSTEM_AUDIO_ASKED_KEY};
use ink_ffi::runtime::{Core, Parts};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(5);

struct Rig {
    core: Core,
    events: Arc<Recorder>,
    store: Arc<ink_store::SqliteStore>,
    probe: Arc<MockPlatform>,
    gate: Arc<Gate>,
    _dir: TempDir,
}

/// A core over an in-memory library whose permissions `probe` answers, with the test row
/// installed and model updates held at `gate` until it opens.
fn rig(label: &str) -> Rig {
    let dir = TempDir::new(label);
    let models = ModelDir::new(dir.path().join("models"));
    let row = test_row(ROW_ID);
    install(&models, &row);
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let gate = Arc::new(Gate::default());
    let store = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    let probe = Arc::new(MockPlatform::new());
    let parts = Parts {
        store: store.clone(),
        clock: clock(),
        registry: Registry::new(vec![row]).unwrap(),
        models,
        loader: loader.clone(),
        installer: Arc::new(MockInstaller {
            generation: loader.generation.clone(),
            gate: Some(gate.clone()),
            installs: AtomicUsize::new(0),
        }),
        data_dir: dir.path().to_owned(),
        permissions: probe.clone() as Arc<dyn PermissionProbe>,
        meetings: Default::default(),
    };
    let events = Recorder::new();
    let core = Core::start(parts, events.out()).unwrap();
    core.lend_bands(bands_channel().0);
    events.wait_type("core.ready", WAIT);
    Rig {
        core,
        events,
        store,
        probe,
        gate,
        _dir: dir,
    }
}

impl Rig {
    /// Sends `cmd` and waits for the `n`th event of type `ty`.
    fn ask(&self, cmd: Value, ty: &str, n: usize) -> Value {
        self.core.command(&cmd.to_string()).unwrap();
        assert!(
            self.events.wait_count(ty, n, WAIT),
            "no {ty} #{n}; got {:?}",
            self.events.types()
        );
        self.events
            .all()
            .into_iter()
            .filter(|e| e["type"] == ty)
            .nth(n - 1)
            .unwrap()
    }

    fn finish(self) {
        self.gate.open();
        self.core.shutdown();
        self.events.assert_valid();
    }
}

#[test]
fn a_revoked_permission_shows_in_the_next_check_even_while_a_model_update_holds_the_commands() {
    let rig = rig("permissions");
    rig.probe
        .set_permission(Permission::Microphone, PermissionState::Granted);
    rig.probe
        .set_permission(Permission::SystemAudio, PermissionState::Granted);
    rig.probe
        .set_permission(Permission::Accessibility, PermissionState::Denied);
    let first = rig.ask(
        json!({"cmd": "permissions.check"}),
        "permissions.checked",
        1,
    );
    assert_eq!(first["microphone"], "granted");
    assert_eq!(first["system_audio"], "granted");
    assert_eq!(first["accessibility"], "denied");
    assert_eq!(first["input_monitoring"], "not_determined");

    // The command thread is held by an update's download for as long as it takes.
    rig.core
        .command(&json!({"cmd": "model.update", "model": ROW_ID, "next": ROW_ID}).to_string())
        .unwrap();
    rig.events.wait_type("model.update_started", WAIT);
    assert!(rig.gate.until_waiting(WAIT), "the install is under way");

    // System audio is revoked in System Settings; the shell checks when the user comes back.
    rig.probe
        .set_permission(Permission::SystemAudio, PermissionState::Denied);
    let asked = Instant::now();
    let second = rig.ask(
        json!({"cmd": "permissions.check"}),
        "permissions.checked",
        2,
    );
    assert_eq!(second["system_audio"], "denied");
    assert!(
        asked.elapsed() < Duration::from_secs(5),
        "the card must turn within 5 s: {:?}",
        asked.elapsed()
    );
    assert_eq!(
        rig.events.count("model.update_finished"),
        0,
        "the answer did not wait for the update"
    );
    rig.finish();
}

#[test]
fn asking_for_system_audio_is_remembered_and_a_refusal_is_reported() {
    let rig = rig("request");
    let requested = rig.ask(
        json!({"cmd": "permission.request", "permission": "system_audio"}),
        "permission.requested",
        1,
    );
    assert_eq!(requested["permission"], "system_audio");
    assert_eq!(rig.probe.requested(), [Permission::SystemAudio]);
    assert_eq!(
        rig.store
            .setting(SYSTEM_AUDIO_ASKED_KEY)
            .unwrap()
            .as_deref(),
        Some("true"),
        "the next launch lets the probe run"
    );
    // Asking for the microphone remembers nothing of the kind.
    rig.ask(
        json!({"cmd": "permission.request", "permission": "microphone"}),
        "permission.requested",
        2,
    );
    assert_eq!(
        rig.probe.requested(),
        [Permission::SystemAudio, Permission::Microphone]
    );
    rig.finish();

    // A platform that cannot ask says so, with the command's id.
    let dir = TempDir::new("no-probe");
    let (core, events) = start(
        &dir,
        &[],
        MockLoader::new(Behaviour::Say("x".into())),
        Arc::new(MockInstaller {
            generation: Arc::default(),
            gate: None,
            installs: AtomicUsize::new(0),
        }),
    );
    core.command(r#"{"cmd":"permission.request","permission":"accessibility","id":"p1"}"#)
        .unwrap();
    let failed = events.wait_type("command.failed", WAIT);
    assert_eq!(failed["command"], "permission.request");
    assert_eq!(failed["id"], "p1");
    core.command(r#"{"cmd":"permissions.check"}"#).unwrap();
    let checked = events.wait_type("permissions.checked", WAIT);
    assert_eq!(checked["system_audio"], "unknown", "nothing is made up");
    core.shutdown();
    events.assert_valid();
}

fn meeting(store: &dyn Store, title: Option<&str>, started: i64) -> ink_core::RecordId {
    store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: title.map(Into::into),
            started_at_unix_ms: started,
            source_app: None,
            audio_dir: None,
        })
        .unwrap()
}

fn promise(text: &str, owner: Option<&str>, due_at: Option<i64>, at_ms: u64) -> NewCommitment {
    NewCommitment {
        recipient: None,
        text: text.into(),
        owner: owner.map(Into::into),
        due: due_at.map(|_| "by Friday".into()),
        due_at_unix_ms: due_at,
        provenance: vec![Span {
            channel: Channel::Mic,
            start_ms: at_ms,
            end_ms: at_ms + 2_000,
        }],
    }
}

#[test]
fn owed_lists_open_commitments_with_their_record_and_merges_and_marking_one_done_drops_it() {
    let rig = rig("owed");
    let store = rig.store.as_ref();
    let call = meeting(store, Some("Planning call"), 1_790_000_000_000);
    let ids = store
        .add_commitments(
            &call,
            &[
                promise(
                    "Send the pilot deck",
                    Some("Alex"),
                    Some(1_790_100_000_000),
                    38_520,
                ),
                promise("Send the deck", Some("Alex"), None, 60_000),
                promise("Book a room", None, None, 44_100),
            ],
        )
        .unwrap();
    store.merge_commitment(&ids[1], &ids[0]).unwrap();
    let untitled = meeting(store, None, 1_790_050_000_000);
    store
        .add_commitments(
            &untitled,
            &[promise(
                "Rerun the numbers",
                Some("Sam"),
                Some(1_790_000_500_000),
                1_000,
            )],
        )
        .unwrap();

    let listed = rig.ask(json!({"cmd": "commitments.list"}), "commitments.listed", 1);
    let items = listed["items"].as_array().unwrap();
    let texts: Vec<&str> = items.iter().map(|i| i["text"].as_str().unwrap()).collect();
    assert_eq!(
        texts,
        ["Rerun the numbers", "Send the pilot deck", "Book a room"],
        "soonest due first, undated last; the merged one is not listed on its own"
    );
    let deck = &items[1];
    assert_eq!(deck["merged"], 1, "said twice");
    assert_eq!(deck["record_title"], "Planning call");
    assert_eq!(deck["record_started_at_unix_ms"], 1_790_000_000_000_i64);
    assert_eq!(deck["owner"], "Alex");
    assert_eq!(deck["due_at_unix_ms"], 1_790_100_000_000_i64);
    assert_eq!(deck["said_at_ms"], 38_520);
    assert_eq!(deck["channel"], "mic");
    assert!(items[0].get("record_title").is_none(), "absent, not null");
    assert!(items[2].get("owner").is_none());

    let done = rig.ask(
        json!({"cmd": "commitment.set_done", "commitment": ids[0].0, "done": true}),
        "commitment.updated",
        1,
    );
    assert_eq!(done["done"], true);
    let listed = rig.ask(
        json!({"cmd": "commitments.list", "limit": 10}),
        "commitments.listed",
        2,
    );
    let texts: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["Rerun the numbers", "Book a room"]);

    rig.core
        .command(r#"{"cmd":"commitment.set_done","commitment":"no-such","done":true,"id":"d9"}"#)
        .unwrap();
    let failed = rig.events.wait_type("command.failed", WAIT);
    assert_eq!(failed["id"], "d9");
    rig.finish();
}

#[test]
fn a_live_meetings_notes_are_added_updated_and_deleted_and_matched_to_their_command() {
    let rig = rig("notes");
    let record = meeting(rig.store.as_ref(), None, 1_790_000_000_000);
    let added = rig.ask(
        json!({"cmd": "note.add", "record": record.0, "at_ms": 754_000, "text": "Pilot: two teams", "id": "line-1"}),
        "note.added",
        1,
    );
    assert_eq!(added["ref"], "line-1");
    assert_eq!(added["at_ms"], 754_000);
    assert!(added.get("text").is_none(), "the words stay with the shell");
    let note = added["note"].as_str().unwrap().to_owned();

    let updated = rig.ask(
        json!({"cmd": "note.update", "note": note, "text": "Pilot: two teams, six weeks", "id": "line-1.update"}),
        "note.updated",
        1,
    );
    assert_eq!(
        updated["ref"], "line-1.update",
        "matched to the line that sent it"
    );
    let notes = rig.store.notes(&record).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].text, "Pilot: two teams, six weeks");
    assert_eq!(notes[0].at_ms, 754_000);

    let deleted = rig.ask(
        json!({"cmd": "note.delete", "note": note, "id": "line-1.delete"}),
        "note.deleted",
        1,
    );
    assert_eq!(deleted["ref"], "line-1.delete");
    assert!(rig.store.notes(&record).unwrap().is_empty());

    // An update or delete that fails names its command's id, so the shell can roll the line back.
    for (cmd, id) in [
        ("note.update", "line-9.update"),
        ("note.delete", "line-9.delete"),
    ] {
        let before = rig.events.count("command.failed");
        let mut command = json!({"cmd": cmd, "note": "no-such-note", "id": id});
        if cmd == "note.update" {
            command["text"] = json!("zebra quartz");
        }
        rig.core.command(&command.to_string()).unwrap();
        assert!(
            rig.events.wait_count("command.failed", before + 1, WAIT),
            "{cmd}"
        );
        let failed = rig
            .events
            .all()
            .into_iter()
            .filter(|e| e["type"] == "command.failed")
            .nth(before)
            .unwrap();
        assert_eq!(failed["command"], cmd);
        assert_eq!(failed["id"], id);
        assert!(!failed["message"].as_str().unwrap().contains("zebra"));
    }

    let before = rig.events.count("command.failed");
    rig.core
        .command(r#"{"cmd":"note.add","record":"no-such-record","at_ms":0,"text":"zebra quartz","id":"n2"}"#)
        .unwrap();
    assert!(rig.events.wait_count("command.failed", before + 1, WAIT));
    let failed = rig
        .events
        .all()
        .into_iter()
        .filter(|e| e["type"] == "command.failed")
        .nth(before)
        .unwrap();
    assert_eq!(failed["command"], "note.add");
    assert_eq!(failed["id"], "n2");
    assert!(
        !failed["message"].as_str().unwrap().contains("zebra"),
        "an error never quotes the note"
    );
    rig.finish();
}

/// The failed command after the first `before` failures.
fn failure(rig: &Rig, before: usize) -> Value {
    assert!(rig.events.wait_count("command.failed", before + 1, WAIT));
    rig.events
        .all()
        .into_iter()
        .filter(|e| e["type"] == "command.failed")
        .nth(before)
        .unwrap()
}

#[test]
fn a_far_end_speaker_is_named_renamed_and_cleared_and_the_record_reads_the_name() {
    let rig = rig("speakers");
    let record = meeting(rig.store.as_ref(), None, 1_790_000_000_000);
    let line = |channel, start_ms, speaker: Option<&str>| ink_core::Segment {
        channel,
        start_ms,
        end_ms: start_ms + 900,
        text: "one two three".into(),
        speaker: speaker.map(|s| ink_core::SpeakerId(s.into())),
    };
    rig.store
        .append_segments(
            &record,
            &[
                line(Channel::Mic, 0, None),
                line(Channel::Far, 1_000, Some("spk0")),
                line(Channel::Far, 2_000, Some("spk1")),
            ],
        )
        .unwrap();
    let speakers = |n: usize| -> Value {
        rig.ask(
            json!({"cmd": "record.open", "record": record.0, "id": format!("open-{n}")}),
            "library.record",
            n,
        )["speakers"]
            .clone()
    };

    // Named: trimmed, and the name is not echoed (the record carries it, to whoever opens it).
    let named = rig.ask(
        json!({"cmd": "speaker.name", "record": record.0, "speaker": "spk1", "name": "  Robin Example ", "id": "name-1"}),
        "speaker.named",
        1,
    );
    assert_eq!(named["ref"], "name-1");
    assert_eq!(named["record"], record.0.as_str());
    assert_eq!(named["speaker"], "spk1");
    assert_eq!(named["named"], true);
    assert!(named.get("name").is_none(), "the name stays with the shell");
    assert_eq!(
        speakers(1),
        json!([{"speaker": "spk1", "name": "Robin Example"}])
    );

    // Renamed.
    rig.ask(
        json!({"cmd": "speaker.name", "record": record.0, "speaker": "spk1", "name": "Sam Example", "id": "name-2"}),
        "speaker.named",
        2,
    );
    assert_eq!(
        speakers(2),
        json!([{"speaker": "spk1", "name": "Sam Example"}])
    );

    // Cleared, by an empty name (or one of spaces): numbered again.
    let cleared = rig.ask(
        json!({"cmd": "speaker.name", "record": record.0, "speaker": "spk1", "name": "   ", "id": "name-3"}),
        "speaker.named",
        3,
    );
    assert_eq!(cleared["named"], false);
    assert_eq!(speakers(3), json!([]));
    // Clearing a speaker who has no name is no error.
    let again = rig.ask(
        json!({"cmd": "speaker.name", "record": record.0, "speaker": "spk0", "name": "", "id": "name-4"}),
        "speaker.named",
        4,
    );
    assert_eq!(again["named"], false);

    // Refused when it runs, naming the command's id, never the name: a label the transcript's far
    // end does not have (the mic is the user, and has none), or a record that is not there.
    for (label, command) in [
        (
            "an unknown label",
            json!({"cmd": "speaker.name", "record": record.0, "speaker": "spk9", "name": "Zebra Quartz", "id": "bad-1"}),
        ),
        (
            "no record",
            json!({"cmd": "speaker.name", "record": "no-such-record", "speaker": "spk0", "name": "Zebra Quartz", "id": "bad-2"}),
        ),
    ] {
        let before = rig.events.count("command.failed");
        rig.core.command(&command.to_string()).unwrap();
        let failed = failure(&rig, before);
        assert_eq!(failed["command"], "speaker.name", "{label}");
        assert_eq!(failed["id"], command["id"], "{label}");
        let message = failed["message"].as_str().unwrap();
        assert!(
            !message.contains("Zebra"),
            "{label}: an error never quotes the name: {message}"
        );
    }

    // Refused as it is read, as every command whose fields are wrong: a name over two lines (it is
    // written into Ask's transcript, one line per turn), one too long, a missing field, or one it
    // does not take.
    let long = "Zebra".repeat(ink_ffi::queries::MAX_SPEAKER_NAME_CHARS);
    for command in [
        json!({"cmd": "speaker.name", "record": record.0, "speaker": "spk0", "name": "Zebra\nQuartz: hi"}),
        json!({"cmd": "speaker.name", "record": record.0, "speaker": "spk0", "name": long}),
        json!({"cmd": "speaker.name", "record": record.0, "speaker": "spk0"}),
        json!({"cmd": "speaker.name", "record": record.0, "name": "Zebra"}),
        json!({"cmd": "speaker.name", "record": record.0, "speaker": "", "name": "Zebra"}),
        json!({"cmd": "speaker.name", "record": record.0, "speaker": "spk0", "name": "Zebra", "colour": "red"}),
    ] {
        let refused = rig.core.command(&command.to_string()).unwrap_err();
        assert!(refused.starts_with("speaker.name: "), "{refused}");
        assert!(
            !refused.contains("Zebra"),
            "never quotes the name: {refused}"
        );
    }
    assert!(rig.store.speaker_names(&record).unwrap().is_empty());
    rig.finish();
}

#[test]
fn shell_settings_are_whitelisted_and_round_trip() {
    let rig = rig("settings");
    let unset = rig.ask(
        json!({"cmd": "setting.get", "key": "onboarding.done"}),
        "setting.value",
        1,
    );
    assert_eq!(unset["key"], "onboarding.done");
    assert!(unset.get("value").is_none());
    let set = rig.ask(
        json!({"cmd": "setting.set", "key": "dictation.polish", "value": "off"}),
        "setting.value",
        2,
    );
    assert_eq!(set["value"], "off");
    assert_eq!(
        rig.store.setting("dictation.polish").unwrap().as_deref(),
        Some("off")
    );
    // An appearance setting: unset, its value is left out (the shell reads its default); a colour
    // of the user's own round-trips as written.
    let unset = rig.ask(
        json!({"cmd": "setting.get", "key": "appearance.mode"}),
        "setting.value",
        3,
    );
    assert_eq!(unset["key"], "appearance.mode");
    assert!(unset.get("value").is_none());
    let colour = rig.ask(
        json!({"cmd": "setting.set", "key": "appearance.you.dark", "value": "#0a1b2c"}),
        "setting.value",
        4,
    );
    assert_eq!(colour["key"], "appearance.you.dark");
    assert_eq!(colour["value"], "#0a1b2c");
    let read = rig.ask(
        json!({"cmd": "setting.get", "key": "appearance.you.dark"}),
        "setting.value",
        5,
    );
    assert_eq!(read["value"], "#0a1b2c");
    // The core's own settings are not the shell's to write, and polish turns on only with the
    // user's consent (consent.allow). An appearance setting takes only its own values.
    for bad in [
        json!({"cmd": "setting.set", "key": SYSTEM_AUDIO_ASKED_KEY, "value": "true"}),
        json!({"cmd": "setting.set", "key": "dictation.polish", "value": "yes"}),
        json!({"cmd": "setting.set", "key": "dictation.polish", "value": "on"}),
        json!({"cmd": "setting.set", "key": "llm.consent.polish", "value": "none"}),
        json!({"cmd": "setting.get", "key": MODES_KEY}),
        json!({"cmd": "setting.set", "key": "appearance.you.dark", "value": "#0A1B2C"}),
        json!({"cmd": "setting.set", "key": "appearance.dots.light", "value": "teal"}),
        json!({"cmd": "setting.set", "key": "appearance.mode", "value": "auto"}),
    ] {
        assert!(rig.core.command(&bad.to_string()).is_err(), "{bad}");
    }
    rig.finish();
}

/// hotkey.check answers whether this computer can watch a binding, before the shell stores it:
/// its one spelling when it can, why not when it cannot. Nothing is stored.
#[test]
fn a_key_is_checked_before_it_is_stored() {
    let rig = rig("hotkey-check");
    let ok = rig.ask(
        json!({"cmd": "hotkey.check", "binding": "f13", "id": "k1"}),
        "hotkey.checked",
        1,
    );
    assert_eq!(ok["binding"], "f13");
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!(ok["canonical"], "f13");
    assert!(ok.get("reason").is_none(), "{ok}");
    assert_eq!(ok["ref"], "k1");
    let refused = rig.ask(
        json!({"cmd": "hotkey.check", "binding": "a", "id": "k2"}),
        "hotkey.checked",
        2,
    );
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(refused.get("canonical").is_none(), "{refused}");
    let reason = refused["reason"].as_str().unwrap_or_default();
    assert!(
        reason.starts_with(|c: char| c.is_lowercase()) && !reason.ends_with('.'),
        "words to show after \"can't use that:\": {reason:?}"
    );
    assert_eq!(rig.store.setting("dictation.key").unwrap(), None);
    // A key setting's refusal gives the reason, never the settings table's placeholder.
    for value in ["off", "a"] {
        let refused = rig
            .core
            .command(
                &json!({"cmd": "setting.set", "key": "dictation.key", "value": value}).to_string(),
            )
            .expect_err("not a key this computer watches");
        assert!(!refused.contains("<key>"), "{refused}");
        assert!(refused.contains("can't be"), "{refused}");
    }
    for bad in [
        json!({"cmd": "hotkey.check"}),
        json!({"cmd": "hotkey.check", "binding": 13}),
        json!({"cmd": "hotkey.check", "binding": "f13", "key": "dictation.key"}),
    ] {
        assert!(rig.core.command(&bad.to_string()).is_err(), "{bad}");
    }
    rig.finish();
}

/// The Mac's answers: the canonical spelling, and the parser's own words.
#[cfg(target_os = "macos")]
#[test]
fn the_mac_check_spells_a_chord_one_way_and_says_why_it_refuses() {
    let rig = rig("hotkey-check-mac");
    let chord = rig.ask(
        json!({"cmd": "hotkey.check", "binding": " Shift+Ctrl+Space "}),
        "hotkey.checked",
        1,
    );
    assert_eq!(chord["binding"], " Shift+Ctrl+Space ");
    assert_eq!(chord["canonical"], "ctrl+shift+space");
    let left = rig.ask(
        json!({"cmd": "hotkey.check", "binding": "left_option"}),
        "hotkey.checked",
        2,
    );
    assert_eq!(left["ok"], false);
    assert!(
        left["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("left-hand modifier"),
        "{left}"
    );
    rig.finish();
}

#[test]
fn modes_come_from_the_store_then_the_import_then_the_default() {
    let rig = rig("modes");
    let default = rig.ask(json!({"cmd": "modes.list"}), "modes.listed", 1);
    assert_eq!(default["default_id"], "default");
    assert_eq!(default["modes"][0]["name"], "Default");
    assert_eq!(default["modes"][0]["apps"], json!([]));

    rig.store
        .set_setting(
            ink_store::import::MODES_KEY,
            &json!({"default_id": "d", "modes": [
                {"id": "d", "name": "Everywhere else", "style": "formal", "polish_enabled": true, "apps": []},
                {"id": "c", "name": "Chat", "style": "casual", "apps": ["com.example.chat"]}]})
            .to_string(),
        )
        .unwrap();
    let imported = rig.ask(json!({"cmd": "modes.list"}), "modes.listed", 2);
    assert_eq!(imported["modes"][1]["apps"], json!(["com.example.chat"]));
    assert_eq!(imported["modes"][0]["polish"], true);

    rig.store
        .set_setting(
            MODES_KEY,
            &json!({"default_id": "only", "modes": [{"id": "only", "name": "Only", "style": "relaxed"}]})
                .to_string(),
        )
        .unwrap();
    let own = rig.ask(json!({"cmd": "modes.list"}), "modes.listed", 3);
    assert_eq!(own["default_id"], "only");
    assert_eq!(own["modes"].as_array().unwrap().len(), 1);

    rig.store.set_setting(MODES_KEY, "{broken").unwrap();
    rig.core.command(r#"{"cmd":"modes.list"}"#).unwrap();
    let failed = rig.events.wait_type("command.failed", WAIT);
    assert_eq!(failed["message"], "the stored modes cannot be read");
    assert!(failed.get("code").is_none(), "{failed}");
    rig.finish();
}

#[test]
fn a_save_refused_over_an_unreadable_list_says_so_by_its_code() {
    let rig = rig("list-unreadable");
    rig.store
        .set_setting(ink_pipeline::snippets::SETTING_KEY, "damaged")
        .unwrap();
    rig.core
        .command(r#"{"cmd":"snippets.save","id":"s","snippets":[]}"#)
        .unwrap();
    let failed = rig.events.wait_type("command.failed", WAIT);
    assert_eq!(failed["id"], "s");
    assert_eq!(failed["code"], ink_ffi::phrases::LIST_UNREADABLE);
    assert_eq!(failed["message"], ink_ffi::phrases::REFUSED_SNIPPETS);
    rig.finish();
}

#[test]
fn the_catalogue_lists_this_oses_models_with_their_rates_and_whether_they_are_installed() {
    let rig = rig("catalogue");
    let listed = rig.ask(json!({"cmd": "models.list"}), "models.listed", 1);
    let models = listed["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], ROW_ID);
    assert_eq!(models[0]["installed"], true);
    let jobs = models[0]["jobs"].as_array().unwrap();
    assert!(jobs.contains(&json!({"job": "dictation_final", "wer": 5.0})));
    rig.finish();
}

/// What serves a job is a screen's question (Settings > Models asks it after each download, and
/// when an engine comes or goes): it is answered while a model update holds the command thread for
/// its download, not once the download ends.
#[test]
fn what_serves_a_job_is_answered_while_a_model_update_holds_the_commands() {
    let rig = rig("route");
    rig.core
        .command(&json!({"cmd": "model.update", "model": ROW_ID, "next": ROW_ID}).to_string())
        .unwrap();
    rig.events.wait_type("model.update_started", WAIT);
    assert!(rig.gate.until_waiting(WAIT), "the install is under way");

    let routed = rig.ask(
        json!({"cmd": "engine.route", "job": "dictation_final"}),
        "engine.routed",
        1,
    );
    assert_eq!(routed["job"], "dictation_final");
    assert_eq!(routed["id"], ROW_ID);
    assert_eq!(routed["source"], "registry");
    assert_eq!(
        rig.events.count("model.update_finished"),
        0,
        "the answer did not wait for the update"
    );
    rig.finish();
}

/// The Mac's Parakeet, which the core only downloads (the shell runs it), is in the built-in
/// catalogue on macOS with its size and whether it is installed, and filling no job; Windows does
/// not list it.
#[test]
fn the_catalogue_lists_the_macs_parakeet_on_macos_only() {
    let dir = TempDir::new("catalogue-parakeet");
    let models = ModelDir::new(dir.path().join("models"));
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let (core, events) = start_parts(Parts {
        store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
        clock: clock(),
        registry: Registry::builtin().unwrap(),
        models: models.clone(),
        loader: loader.clone(),
        installer: Arc::new(MockInstaller {
            generation: loader.generation.clone(),
            gate: None,
            installs: AtomicUsize::new(0),
        }),
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: Default::default(),
    });
    let parakeet = |n: usize| {
        core.command(r#"{"cmd":"models.list"}"#).unwrap();
        assert!(events.wait_count("models.listed", n, WAIT));
        let listed = events
            .all()
            .into_iter()
            .filter(|e| e["type"] == "models.listed")
            .nth(n - 1)
            .unwrap();
        listed["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == ink_engines::PARAKEET_COREML_ID)
            .cloned()
    };
    if cfg!(not(target_os = "macos")) {
        assert_eq!(parakeet(1), None);
        core.shutdown();
        events.assert_valid();
        return;
    }
    let row = ink_engines::parakeet_tdt_v3_coreml();
    assert_eq!(
        parakeet(1),
        Some(json!({
            "id": "parakeet-tdt-0.6b-v3-coreml",
            "licence": "CC-BY-4.0",
            "size_bytes": 483_105_645,
            "installed": false,
            "jobs": [],
        }))
    );
    // Laid out as the downloader leaves it (sparse files of the right sizes, and the marker).
    for f in &row.files {
        let path = models.file_path(&row, f);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(&path)
            .unwrap()
            .set_len(f.size)
            .unwrap();
    }
    std::fs::write(models.marker_path(&row), &row.revision).unwrap();
    assert_eq!(parakeet(2).unwrap()["installed"], true);
    core.shutdown();
    events.assert_valid();
}
