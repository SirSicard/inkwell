//! A replayed meeting runs the real path: replay → capture ring → pump → chunks on disk and the
//! bounded queue → the meeting worker → final pass → a record in the library.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use common::*;
use ink_core::RecordId;

#[test]
fn a_replayed_meeting_becomes_a_record_through_the_pump_and_the_worker() {
    let dir = TempDir::new("replay");
    let loader = MockLoader::new(Behaviour::Say("from the final pass".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader.clone(), installer);
    let (mic, far) = (dir.path().join("mic.wav"), dir.path().join("far.wav"));
    speech_wav(&mic, 3.0, 1);
    speech_wav(&far, 2.0, 2);
    core.command(&format!(
        r#"{{"cmd":"replay_meeting","mic":{:?},"far":{:?},"title":"Replay","pacing":"fast"}}"#,
        mic.to_str().unwrap(),
        far.to_str().unwrap()
    ))
    .unwrap();

    let finished = events.wait_type("meeting.finished", Duration::from_secs(60));
    assert_eq!(
        finished["revision"], 2,
        "the final pass superseded the live one"
    );
    let record = RecordId(finished["record"].as_str().unwrap().to_owned());
    assert_eq!(
        events.wait_type("meeting.started", Duration::ZERO)["record"],
        finished["record"]
    );
    let store = &core.shared().store;
    let stored = store.record(&record).unwrap().expect("the record exists");
    assert_eq!(stored.title.as_deref(), Some("Replay"));
    assert_eq!(stored.revision, 2);
    let segments = store.segments(&record).unwrap();
    assert!(
        segments.iter().any(|s| s.text == "from the final pass"),
        "{segments:?}"
    );

    // The final pass ran on the meeting worker, through residency, and read chunks from disk.
    let threads = loader.journal.threads.lock().unwrap().clone();
    assert!(!threads.is_empty());
    assert!(
        threads.iter().all(|t| t.as_deref() == Some("ink-meeting")),
        "{threads:?}"
    );
    let passes: Vec<_> = events
        .all()
        .into_iter()
        .filter(|v| v["type"] == "meeting.transcribed")
        .collect();
    assert_eq!(passes.len(), 2, "one pass per side");
    for p in &passes {
        assert!(p["pass"]["chunks"].as_u64().unwrap() >= 1, "{p}");
        assert!(p["pass"]["chunks_written"].as_u64().unwrap() >= 1, "{p}");
    }
    assert_eq!(
        events.count("audio.dropped"),
        0,
        "nothing dropped at this pace"
    );

    // A second meeting may start once the first is done.
    core.command(&format!(
        r#"{{"cmd":"replay_meeting","mic":{:?},"pacing":"fast","id":"second"}}"#,
        mic.to_str().unwrap()
    ))
    .unwrap();
    events
        .wait_for(Duration::from_secs(60), |v| {
            v["type"] == "meeting.finished" && v["record"] != finished["record"]
        })
        .expect("a second meeting");
    core.shutdown();
    events.assert_valid();
}

#[test]
fn a_bad_replay_fails_as_an_event_and_starts_nothing() {
    let dir = TempDir::new("replay-bad");
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader, installer);
    let missing = dir.path().join("missing.wav");
    core.command(&format!(
        r#"{{"cmd":"replay_meeting","mic":{:?},"id":"m1"}}"#,
        missing.to_str().unwrap()
    ))
    .unwrap();
    let failed = events.wait_type("command.failed", Duration::from_secs(5));
    assert_eq!(failed["command"], "replay_meeting");
    assert_eq!(failed["id"], "m1");
    assert!(
        failed["message"].as_str().unwrap().contains("missing.wav"),
        "{failed}"
    );
    assert!(core.command(r#"{"cmd":"replay_meeting"}"#).is_err());
    core.shutdown();
    assert_eq!(events.count("meeting.started"), 0);
    events.assert_valid();
}

/// Replay determinism (carried into S2.8 from S2.2): two replays of the same files hand the final
/// pass the same audio, sample for sample. The replay's host times start at the meeting's start,
/// not whenever its thread happened to start, so the final pass (which places audio by host time
/// against the meeting's start) reads the same samples however the threads are scheduled.
#[test]
fn two_replays_of_the_same_files_feed_identical_audio_to_the_final_pass() {
    let dir = TempDir::new("replay-determinism");
    let (mic, far) = (dir.path().join("mic.wav"), dir.path().join("far.wav"));
    speech_wav(&mic, 4.0, 11);
    speech_wav(&far, 3.0, 12);
    let mut runs = Vec::new();
    for run in 0..2 {
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let loader = MockLoader::new(Behaviour::Keep(heard.clone(), "words".into()));
        let installer = Arc::new(MockInstaller {
            generation: loader.generation.clone(),
            gate: None,
            installs: AtomicUsize::new(0),
        });
        let (core, events) = start(&dir, &[test_row(ROW_ID)], loader, installer);
        core.command(&format!(
            r#"{{"cmd":"replay_meeting","mic":{:?},"far":{:?},"pacing":"fast","id":"r{run}"}}"#,
            mic.to_str().unwrap(),
            far.to_str().unwrap()
        ))
        .unwrap();
        events.wait_type("meeting.finished", Duration::from_secs(60));
        core.shutdown();
        let calls = std::mem::take(&mut *heard.lock().unwrap());
        assert!(
            !calls.is_empty(),
            "run {run}: the final pass called the engine"
        );
        runs.push(calls);
    }
    assert_eq!(runs[0].len(), runs[1].len(), "the same regions");
    for (a, b) in runs[0].iter().zip(&runs[1]) {
        assert_eq!(a.len(), b.len(), "the same region lengths");
        assert!(
            a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()),
            "the same samples"
        );
    }
}
