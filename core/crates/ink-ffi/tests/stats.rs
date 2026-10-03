//! The Stats screen's commands through the core: `stats.get` answered by one `stats.counted` that
//! matches the schema and echoes the command's id, counted from the store on the screens' thread.
//! The counting itself is tested in `src/stats.rs` with fixed clocks.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use common::*;
use ink_core::{Channel, NewCommitment, NewRecord, RecordId, RecordKind, Segment, Store};
use ink_ffi::runtime::Core;
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(10);
const MINUTE: i64 = 60_000;

fn core(dir: &TempDir) -> (Core, Arc<Recorder>) {
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    start(dir, &[test_row(ROW_ID)], loader, installer)
}

fn ask(core: &Core, events: &Recorder, command: Value, id: &str) -> Value {
    let mut command = command;
    command["id"] = id.into();
    core.command(&command.to_string()).unwrap();
    events
        .wait_for(WAIT, |v| v["ref"] == id || v["id"] == id)
        .unwrap_or_else(|| panic!("no answer to {id}: {:?}", events.types()))
}

fn utc() -> Value {
    json!([{"from_unix_ms": 0, "minutes": 0}])
}

fn add(
    store: &dyn Store,
    kind: RecordKind,
    start: i64,
    minutes: i64,
    segments: &[Segment],
) -> RecordId {
    let id = store
        .create_record(NewRecord {
            kind,
            title: None,
            started_at_unix_ms: start,
            source_app: None,
            audio_dir: None,
        })
        .unwrap();
    store.append_segments(&id, segments).unwrap();
    store.finish_record(&id, start + minutes * MINUTE).unwrap();
    id
}

fn line(channel: Channel, start_ms: u64, end_ms: u64, text: &str) -> Segment {
    Segment {
        channel,
        start_ms,
        end_ms,
        text: text.into(),
        speaker: None,
    }
}

#[test]
fn stats_get_counts_the_library_on_the_users_calendar() {
    let dir = TempDir::new("stats-get");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    let now = core.shared().clock.unix_ms();

    // A new library: everything is zero, and nothing is reached.
    let empty = ask(
        &core,
        &events,
        json!({"cmd": "stats.get", "utc_offsets": utc(), "week_start": 1}),
        "empty",
    );
    assert_eq!(empty["type"], "stats.counted");
    assert_eq!(empty["typing_wpm"], 40);
    assert_eq!(empty["dictation"]["words_all"], 0);
    assert_eq!(empty["dictation"]["streak_days"], 0);
    assert!(
        empty["dictation"].get("wpm_week").is_none(),
        "no figure yet"
    );
    assert_eq!(empty["meetings_all"]["meetings"], 0);
    assert_eq!(empty["promises_all"]["made"], 0);
    assert!(
        empty["milestones"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["reached"] == false)
    );

    // Two dictations today: 240 words in 2 minutes held.
    let words120 = "word ".repeat(120);
    for start in [now - 2 * MINUTE, now - 10 * MINUTE] {
        add(
            store.as_ref(),
            RecordKind::Dictation,
            start,
            1,
            &[line(Channel::Mic, 0, 60_000, &words120)],
        );
    }
    // A meeting: the user 30 s with a question, the far end 20 s.
    let call = add(
        store.as_ref(),
        RecordKind::Meeting,
        now - 60 * MINUTE,
        30,
        &[
            line(Channel::Mic, 0, 30_000, "shall we start?"),
            line(Channel::Far, 31_000, 51_000, "yes"),
        ],
    );
    let promise = |text: &str, done_due: Option<i64>| NewCommitment {
        text: text.into(),
        owner: None,
        recipient: None,
        due: None,
        due_at_unix_ms: done_due,
        provenance: vec![],
    };
    let ids = store
        .add_commitments(&call, &[promise("send it", None), promise("call", None)])
        .unwrap();
    store.set_commitment_done(&ids[0], true).unwrap();

    let stats = ask(
        &core,
        &events,
        json!({"cmd": "stats.get", "utc_offsets": utc(), "week_start": 1}),
        "full",
    );
    let d = &stats["dictation"];
    assert_eq!(d["words_today"], 240);
    assert_eq!(d["words_all"], 240);
    assert_eq!(d["dictations_all"], 2);
    assert_eq!(d["wpm_week"], 120);
    // 240 words typed at 40 wpm is 6 minutes; 2 were spoken.
    assert_eq!(d["saved_ms_all"], 4 * MINUTE);
    assert_eq!(d["streak_days"], 1);
    assert_eq!(d["heatmap_words"].as_array().unwrap().last().unwrap(), 240);
    let m = &stats["meetings_all"];
    assert_eq!(m["meetings"], 1);
    assert_eq!(m["recorded_ms"], 30 * MINUTE);
    assert_eq!(m["you_ms"], 30_000);
    assert_eq!(m["them_ms"], 20_000);
    assert_eq!(m["questions"], 1);
    assert_eq!(m["longest_monologue_ms"], 30_000);
    assert_eq!(
        stats["promises_all"],
        json!({"made": 2, "kept": 1, "open": 1, "overdue": 0})
    );

    // The typing speed is the user's setting (`setting.value` answers by key, not by id).
    core.command(
        &json!({"cmd": "setting.set", "key": "stats.typing_wpm", "value": "60"}).to_string(),
    )
    .unwrap();
    events
        .wait_for(WAIT, |v| {
            v["type"] == "setting.value" && v["key"] == "stats.typing_wpm" && v["value"] == "60"
        })
        .unwrap();
    let faster = ask(
        &core,
        &events,
        json!({"cmd": "stats.get", "utc_offsets": utc(), "week_start": 1}),
        "faster",
    );
    assert_eq!(faster["typing_wpm"], 60);
    // 240 words at 60 wpm is 4 minutes; 2 were spoken.
    assert_eq!(faster["dictation"]["saved_ms_all"], 2 * MINUTE);

    core.shutdown();
    events.assert_valid();
}

/// A command the core cannot read is refused where it is sent, with why, as every other is.
#[test]
fn a_bad_stats_get_or_typing_speed_is_refused() {
    let dir = TempDir::new("stats-bad");
    let (core, events) = core(&dir);
    let refused = core
        .command(&json!({"cmd": "stats.get", "utc_offsets": [], "week_start": 1}).to_string())
        .unwrap_err();
    assert!(refused.contains("utc_offsets"), "{refused}");
    for slow in ["9", "201", "forty", "40.5"] {
        let refused = core
            .command(
                &json!({"cmd": "setting.set", "key": "stats.typing_wpm", "value": slow})
                    .to_string(),
            )
            .unwrap_err();
        assert!(refused.contains("stats.typing_wpm"), "{refused}");
    }
    core.shutdown();
    events.assert_valid();
}

fn dictate(store: &dyn Store, start: i64, words: usize) {
    add(
        store,
        RecordKind::Dictation,
        start,
        1,
        &[line(Channel::Mic, 0, 60_000, &"word ".repeat(words))],
    );
}

fn check(core: &Core, events: &Recorder, id: &str) -> Vec<String> {
    let answer = ask(
        core,
        events,
        json!({"cmd": "milestones.check", "utc_offsets": utc(), "week_start": 1}),
        id,
    );
    assert_eq!(answer["type"], "milestones.reached", "{answer}");
    answer["milestones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Each milestone is celebrated once: the check that first finds it reached says so, and no
/// later one does. A library's first check only takes note of what was already reached (an
/// upgrade, an import): nothing old is celebrated as new.
#[test]
fn a_milestone_is_celebrated_once() {
    let dir = TempDir::new("milestones-once");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    let now = core.shared().clock.unix_ms();

    dictate(store.as_ref(), now - 10 * MINUTE, 1_200);
    assert_eq!(
        check(&core, &events, "first"),
        Vec::<String>::new(),
        "already reached"
    );
    assert_eq!(check(&core, &events, "again"), Vec::<String>::new());

    dictate(store.as_ref(), now - 5 * MINUTE, 9_000);
    assert_eq!(check(&core, &events, "ten"), ["words_10000"]);
    assert_eq!(
        check(&core, &events, "ten-again"),
        Vec::<String>::new(),
        "once"
    );
    core.shutdown();
    events.assert_valid();
}

/// With celebrations off, a milestone reached is noted and never celebrated, then or after
/// celebrations are turned back on.
#[test]
fn celebrations_turned_off_celebrate_nothing_later_either() {
    let dir = TempDir::new("milestones-off");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    let now = core.shared().clock.unix_ms();
    assert_eq!(check(&core, &events, "first"), Vec::<String>::new());

    core.command(
        &json!({"cmd": "setting.set", "key": "stats.celebrate", "value": "off"}).to_string(),
    )
    .unwrap();
    events
        .wait_for(WAIT, |v| {
            v["type"] == "setting.value" && v["key"] == "stats.celebrate"
        })
        .unwrap();
    dictate(store.as_ref(), now - 10 * MINUTE, 1_000);
    assert_eq!(check(&core, &events, "off"), Vec::<String>::new());

    core.command(
        &json!({"cmd": "setting.set", "key": "stats.celebrate", "value": "on"}).to_string(),
    )
    .unwrap();
    events
        .wait_for(WAIT, |v| {
            v["type"] == "setting.value" && v["key"] == "stats.celebrate" && v["value"] == "on"
        })
        .unwrap();
    assert_eq!(
        check(&core, &events, "on"),
        Vec::<String>::new(),
        "not late either"
    );
    dictate(store.as_ref(), now - 5 * MINUTE, 9_000);
    assert_eq!(check(&core, &events, "ten"), ["words_10000"]);
    core.shutdown();
    events.assert_valid();
}
