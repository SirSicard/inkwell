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

const DAY: i64 = 24 * 60 * MINUTE;

/// The ISO weekday of a UTC day (1970-01-01 was a Thursday).
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7) + 1
}

fn date(day: i64) -> String {
    ink_ffi::stats::Calendar::date(day)
}

fn set(core: &Core, events: &Recorder, key: &str, value: &str) {
    core.command(&json!({"cmd": "setting.set", "key": key, "value": value}).to_string())
        .unwrap();
    events
        .wait_for(WAIT, |v| {
            v["type"] == "setting.value" && v["key"] == key && v["value"] == value
        })
        .unwrap_or_else(|| panic!("{key} = {value} not echoed"));
}

fn get(core: &Core, events: &Recorder, week_start: i64, id: &str) -> Value {
    let answer = ask(
        core,
        events,
        json!({"cmd": "stats.get", "utc_offsets": utc(), "week_start": week_start}),
        id,
    );
    assert_eq!(answer["type"], "stats.counted", "{answer}");
    answer
}

/// The streak's and the share card's settings are the core's to judge: each takes its values in
/// one spelling, and anything else is refused where it is sent. The pauses and the bests noted
/// are the core's own.
#[test]
fn the_streak_and_share_settings_are_judged_by_the_core() {
    let dir = TempDir::new("stats-settings");
    let (core, events) = core(&dir);
    for (key, value) in [
        ("stats.rest_days", "6,7"),
        ("stats.rest_days", "3"),
        ("stats.rest_days", "none"),
        ("stats.streak", "hidden"),
        ("stats.streak", "shown"),
        ("stats.share_heatmap", "on"),
        ("stats.share_heatmap", "off"),
        ("stats.review_dismissed", "2026-09-28"),
    ] {
        set(&core, &events, key, value);
    }
    for (key, value) in [
        ("stats.rest_days", "7,6"),
        ("stats.rest_days", "1,2,3,4,5,6,7"),
        ("stats.rest_days", ""),
        ("stats.rest_days", "sat"),
        ("stats.streak", "off"),
        ("stats.share_heatmap", "yes"),
        ("stats.review_dismissed", "2026-02-30"),
        ("stats.review_dismissed", "last week"),
        ("stats.streak_pauses", "[]"),
        ("stats.bests", "{}"),
    ] {
        let refused = core
            .command(&json!({"cmd": "setting.set", "key": key, "value": value}).to_string())
            .unwrap_err();
        assert!(refused.contains(key), "{key} = {value}: {refused}");
    }
    core.shutdown();
    events.assert_valid();
}

/// Rest days and a pause carry a streak over days without a dictation. The pause starts and
/// ends from the Stats screen, answered with the numbers it changes.
#[test]
fn rest_days_and_a_pause_keep_the_streak_going() {
    let dir = TempDir::new("stats-streak");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    let now = core.shared().clock.unix_ms();
    let today = now.div_euclid(DAY);

    // Today and three days ago: the two days missed between end a streak.
    dictate(store.as_ref(), now - 3 * DAY, 10);
    dictate(store.as_ref(), now, 10);
    assert_eq!(
        get(&core, &events, 1, "plain")["dictation"]["streak_days"],
        1
    );

    // Those two days' weekdays as rest days: one streak of two.
    let mut rest = [weekday(today - 1), weekday(today - 2)];
    rest.sort_unstable();
    set(
        &core,
        &events,
        "stats.rest_days",
        &format!("{},{}", rest[0], rest[1]),
    );
    let rested = get(&core, &events, 1, "rested");
    assert_eq!(rested["dictation"]["streak_days"], 2);
    assert_eq!(rested["dictation"]["rest_days"], json!(rest));
    set(&core, &events, "stats.rest_days", "none");
    assert_eq!(
        get(&core, &events, 1, "unrested")["dictation"]["streak_days"],
        1
    );

    // A pause from today: the answer is the summary, saying since when.
    let paused = ask(
        &core,
        &events,
        json!({"cmd": "streak.pause", "utc_offsets": utc(), "week_start": 1}),
        "pause",
    );
    assert_eq!(paused["type"], "stats.counted", "{paused}");
    assert_eq!(paused["dictation"]["streak_paused_since"], date(today));
    assert_eq!(
        get(&core, &events, 1, "still-paused")["dictation"]["streak_paused_since"],
        date(today)
    );
    // Resumed the day it began: as if it never was.
    let resumed = ask(
        &core,
        &events,
        json!({"cmd": "streak.resume", "utc_offsets": utc(), "week_start": 1}),
        "resume",
    );
    assert!(resumed["dictation"].get("streak_paused_since").is_none());

    // A pause over the two missed days (as one ended yesterday is kept): one streak again.
    store
        .set_setting(
            "stats.streak_pauses",
            &json!([{"from": date(today - 2), "until": date(today - 1)}]).to_string(),
        )
        .unwrap();
    assert_eq!(
        get(&core, &events, 1, "paused")["dictation"]["streak_days"],
        2
    );

    // A pause command reads the calendar as stats.get does, and refuses what stats.get refuses.
    let refused = core
        .command(&json!({"cmd": "streak.pause", "utc_offsets": [], "week_start": 1}).to_string())
        .unwrap_err();
    assert!(refused.contains("utc_offsets"), "{refused}");
    core.shutdown();
    events.assert_valid();
}

/// A take that sets a best says so once, in the milestone check sent after it: which best, the
/// old value and the new. The library's first check only takes note, and one note a day is all.
#[test]
fn a_best_is_news_after_the_take_that_set_it() {
    let dir = TempDir::new("stats-bests");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    let now = core.shared().clock.unix_ms();
    let take = |start: i64, held_ms: u64| {
        add(
            store.as_ref(),
            RecordKind::Dictation,
            start,
            1,
            &[line(Channel::Mic, 0, held_ms, &"word ".repeat(50))],
        )
    };
    let check_best = |id: &str| {
        ask(
            &core,
            &events,
            json!({"cmd": "milestones.check", "utc_offsets": utc(), "week_start": 1}),
            id,
        )
    };
    assert!(check_best("first").get("best").is_none());
    // Six takes of 20 s: the shelf's first values, noted without news.
    // Seconds apart, so the test does not straddle midnight but in its first few seconds.
    for i in 0..6 {
        take(now - 20_000 + i * 1_000, 20_000);
    }
    assert!(check_best("baseline").get("best").is_none());

    // A longer one: news.
    let longer = take(now - 10_000, 40_000);
    let news = check_best("longer");
    assert_eq!(news["type"], "milestones.reached");
    let best = &news["best"];
    assert_eq!(best["id"], "longest_dictation", "{news}");
    assert_eq!(best["unit"], "ms");
    assert_eq!(best["old"], 20_000);
    assert_eq!(best["new"], 40_000);
    assert_eq!(best["record"], longer.0.as_str());
    assert_eq!(best["date"], date((now - 10_000).div_euclid(DAY)));
    assert!(check_best("again").get("best").is_none(), "once");

    // Longer still, the same day: on the shelf, without a second note.
    take(now - 5_000, 50_000);
    assert!(check_best("same-day").get("best").is_none());
    let shelf = get(&core, &events, 1, "shelf");
    assert_eq!(shelf["bests"][0]["id"], "longest_dictation");
    assert_eq!(shelf["bests"][0]["value"], 50_000);
    core.shutdown();
    events.assert_valid();
}

/// Last week's review is offered until the user dismisses it, and stays dismissed for that week.
#[test]
fn the_week_review_stays_dismissed_for_its_week() {
    let dir = TempDir::new("stats-review");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    let now = core.shared().clock.unix_ms();
    let today = now.div_euclid(DAY);
    // Weeks start on today's weekday, so last week is the seven days before today.
    let week_start = weekday(today);
    dictate(store.as_ref(), now - 3 * DAY, 120);

    let review = get(&core, &events, week_start, "review");
    assert_eq!(review["week_review"]["week"], date(today - 7), "{review}");
    assert_eq!(review["week_review"]["words"], 120);
    set(&core, &events, "stats.review_dismissed", &date(today - 7));
    assert!(
        get(&core, &events, week_start, "dismissed")
            .get("week_review")
            .is_none()
    );
    core.shutdown();
    events.assert_valid();
}
