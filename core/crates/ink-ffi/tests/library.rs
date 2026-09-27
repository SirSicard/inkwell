//! The library as the screens read it (`records.list`, `records.search`, `record.open`,
//! `library.stats`): each answered by one event that matches the schema and echoes the command's id
//! as `ref`, on the screens' thread; a failure is `command.failed` with that id.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use common::*;
use std::sync::atomic::Ordering;

use ink_core::{
    Channel, Commitment, CommitmentId, NewCommitment, NewRecord, Note, NoteId, Record, RecordQuery,
    SearchHit, Segment, Span, SpeakerId, Store, StoreError, Summary, SupersedeWith,
};
use ink_core::{RecordId, RecordKind};
use ink_ffi::runtime::{Core, Parts};
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(10);
/// 2026-09-24 12:00 UTC.
const NOON: i64 = 1_790_251_200_000;
const MINUTE: i64 = 60_000;

fn core(dir: &TempDir) -> (Core, Arc<Recorder>) {
    let loader = MockLoader::new(Behaviour::Say("from the final pass".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    start(dir, &[test_row(ROW_ID)], loader, installer)
}

fn record(store: &dyn Store, kind: RecordKind, title: Option<&str>, start: i64) -> RecordId {
    let id = store
        .create_record(NewRecord {
            kind,
            title: title.map(str::to_owned),
            started_at_unix_ms: start,
            source_app: None,
            audio_dir: None,
        })
        .unwrap();
    store.finish_record(&id, start + 30 * MINUTE).unwrap();
    id
}

fn seg(channel: Channel, start_ms: u64, text: &str) -> Segment {
    Segment {
        channel,
        start_ms,
        end_ms: start_ms + 1_000,
        text: text.into(),
        speaker: None,
    }
}

/// Sends `command` with id `id` and waits for the answer carrying it.
fn ask(core: &Core, events: &Recorder, command: Value, id: &str) -> Value {
    let mut command = command;
    command["id"] = id.into();
    core.command(&command.to_string()).unwrap();
    events
        .wait_for(WAIT, |v| v["ref"] == id || v["id"] == id)
        .unwrap_or_else(|| panic!("no answer to {id}: {:?}", events.types()))
}

fn ids(answer: &Value) -> Vec<String> {
    answer["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["record"].as_str().unwrap().to_owned())
        .collect()
}

/// The step's sort-order check, at the source: records come back newest first by start time,
/// whatever order they were written in (the earlier app's "latest record" sorted by something
/// else). Pages continue from a cursor without repeating or skipping.
#[test]
fn records_list_newest_first_by_start_and_pages_without_gaps() {
    let dir = TempDir::new("library-sort");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    // Written out of order: the newest start last-but-one, two sharing a start.
    let b = record(
        store.as_ref(),
        RecordKind::Meeting,
        Some("B"),
        NOON - 60 * MINUTE,
    );
    let d = record(
        store.as_ref(),
        RecordKind::Meeting,
        Some("D"),
        NOON + 90 * MINUTE,
    );
    let a = record(
        store.as_ref(),
        RecordKind::Meeting,
        Some("A"),
        NOON - 120 * MINUTE,
    );
    let e = record(
        store.as_ref(),
        RecordKind::Meeting,
        Some("E"),
        NOON + 120 * MINUTE,
    );
    let c = record(
        store.as_ref(),
        RecordKind::Meeting,
        Some("C"),
        NOON - 60 * MINUTE,
    );
    let dictation = record(
        store.as_ref(),
        RecordKind::Dictation,
        None,
        NOON + 200 * MINUTE,
    );
    store
        .append_segments(
            &dictation,
            &[seg(Channel::Mic, 0, "remember to water the plants")],
        )
        .unwrap();

    let all = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "records.list"}),
        "all",
    );
    assert_eq!(all["type"], "library.records");
    assert_eq!(all["more"], false);
    let listed = ids(&all);
    // B and C share a start: the id breaks the tie, descending.
    let (first_tie, second_tie) = if b.0 > c.0 { (&b, &c) } else { (&c, &b) };
    let expected: Vec<String> = [&dictation, &e, &d, first_tie, second_tie, &a]
        .iter()
        .map(|r| r.0.clone())
        .collect();
    assert_eq!(listed, expected);
    let starts: Vec<i64> = all["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["started_at_unix_ms"].as_i64().unwrap())
        .collect();
    assert!(starts.windows(2).all(|w| w[0] >= w[1]), "{starts:?}");
    // An untitled record carries its first words; a titled one does not.
    assert_eq!(
        all["records"][0]["preview"], "remember to water the plants",
        "{all}"
    );
    assert!(all["records"][1].get("preview").is_none());
    assert_eq!(all["records"][1]["title"], "E");
    assert_eq!(all["records"][1]["has_audio"], false);

    // Meetings only, two per page, following the cursor.
    let mut paged = Vec::new();
    let mut before: Option<Value> = None;
    for page in 0..5 {
        let mut command = serde_json::json!({"cmd": "records.list", "kind": "meeting", "limit": 2});
        if let Some(b) = &before {
            command["before"] = b.clone();
        }
        let answer = ask(&core, &events, command, &format!("page{page}"));
        assert_eq!(answer["kind"], "meeting");
        let rows = answer["records"].as_array().unwrap().clone();
        paged.extend(ids(&answer));
        if answer["more"] == false {
            break;
        }
        let last = rows.last().unwrap();
        before = Some(serde_json::json!({
            "started_at_unix_ms": last["started_at_unix_ms"],
            "id": last["record"],
        }));
    }
    assert_eq!(paged, expected[1..].to_vec());
    core.shutdown();
    events.assert_valid();
}

/// A record opens whole: transcript, notes, summary, commitments and named speakers. A missing
/// record is a `command.failed` carrying the command's id.
#[test]
fn record_open_carries_the_whole_record_and_a_missing_one_fails_by_id() {
    let dir = TempDir::new("library-open");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    let r = record(store.as_ref(), RecordKind::Meeting, Some("Plan"), NOON);
    store
        .append_segments(
            &r,
            &[
                seg(Channel::Mic, 1_000, "I will send the plan"),
                Segment {
                    speaker: Some(SpeakerId("spk0".into())),
                    ..seg(Channel::Far, 3_000, "thanks")
                },
            ],
        )
        .unwrap();
    store.add_note(&r, 2_500, "send plan").unwrap();
    store
        .save_summary(
            &r,
            &Summary {
                items: Vec::new(),
                text: "We agreed a plan.\n\n## Actions\n- Send the plan (You)".into(),
                model: "test".into(),
                created_at_unix_ms: NOON + MINUTE,
            },
        )
        .unwrap();
    store
        .add_commitments(
            &r,
            &[NewCommitment {
                recipient: None,
                text: "Send the plan".into(),
                owner: Some("You".into()),
                due: Some("Friday".into()),
                due_at_unix_ms: Some(NOON + 2 * 24 * 60 * MINUTE),
                provenance: vec![Span {
                    channel: Channel::Mic,
                    start_ms: 1_000,
                    end_ms: 2_000,
                }],
            }],
        )
        .unwrap();
    store
        .set_speaker_name(&r, &SpeakerId("spk0".into()), "Alex")
        .unwrap();

    let open = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "record.open", "record": r.0}),
        "open",
    );
    assert_eq!(open["type"], "library.record");
    assert_eq!(open["record"]["title"], "Plan");
    assert_eq!(open["segments"].as_array().unwrap().len(), 2);
    assert_eq!(open["segments"][1]["speaker"], "spk0");
    assert_eq!(open["notes"][0]["at_ms"], 2_500);
    assert!(
        open["summary"]["text"]
            .as_str()
            .unwrap()
            .starts_with("We agreed")
    );
    assert_eq!(open["commitments"][0]["provenance"][0]["start_ms"], 1_000);
    assert_eq!(open["speakers"][0]["name"], "Alex");
    assert!(open.get("audio").is_none(), "no audio directory: no player");

    let missing = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "record.open", "record": "no-such-record"}),
        "gone",
    );
    assert_eq!(missing["type"], "command.failed");
    assert_eq!(missing["command"], "record.open");
    assert!(
        missing["message"]
            .as_str()
            .unwrap()
            .contains("no-such-record")
    );
    core.shutdown();
    events.assert_valid();
}

/// A replayed meeting's record opens with its audio: every chunk the pump wrote, placed on the
/// timeline from the start the meeting wrote beside them, so a chunk's place and the segments'
/// stamps share one zero.
#[test]
fn a_replayed_meeting_opens_with_its_chunks_on_the_recorded_timeline() {
    let dir = TempDir::new("library-audio");
    let (core, events) = core(&dir);
    let (mic, far) = (dir.path().join("mic.wav"), dir.path().join("far.wav"));
    speech_wav(&mic, 3.0, 1);
    speech_wav(&far, 2.0, 2);
    core.command(&format!(
        r#"{{"cmd":"replay_meeting","mic":{:?},"far":{:?},"pacing":"fast"}}"#,
        mic.to_str().unwrap(),
        far.to_str().unwrap()
    ))
    .unwrap();
    let finished = events.wait_type("meeting.finished", Duration::from_secs(60));
    let record = finished["record"].as_str().unwrap().to_owned();

    let open = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "record.open", "record": record}),
        "open",
    );
    let audio = &open["audio"];
    assert_eq!(audio["timeline"], "recorded", "{audio}");
    let chunks = audio["chunks"].as_array().unwrap();
    for channel in ["mic", "far"] {
        let first = chunks
            .iter()
            .find(|c| c["channel"] == channel)
            .unwrap_or_else(|| panic!("no {channel} chunk: {audio}"));
        // Capture starts right after the meeting does: its first frame is within a moment of 0.
        let start = first["start_ms"].as_i64().unwrap();
        assert!((0..500).contains(&start), "{channel} starts at {start} ms");
        assert_eq!(first["data_offset"], 64);
        assert_eq!(first["sample_rate"], 16_000);
        assert_eq!(first["channels"], 1);
        assert!(first["frames"].as_u64().unwrap() > 0);
        let path = std::path::Path::new(first["path"].as_str().unwrap());
        assert!(path.is_absolute() && path.is_file(), "{path:?}");
    }
    core.shutdown();
    events.assert_valid();
}

/// Search, and the stats Today shows.
#[test]
fn search_and_stats_answer_from_the_store() {
    let dir = TempDir::new("library-owed");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    let heard = record(
        store.as_ref(),
        RecordKind::Meeting,
        Some("Budget review"),
        NOON - 3 * 24 * 60 * MINUTE,
    );
    store
        .append_segments(
            &heard,
            &[
                seg(Channel::Mic, 0, "the budget is tight this quarter"),
                seg(Channel::Far, 2_000, "agreed on the budget"),
            ],
        )
        .unwrap();
    // The two newest meetings kept only the mic.
    let deaf1 = record(
        store.as_ref(),
        RecordKind::Meeting,
        Some("One"),
        NOON - 60 * MINUTE,
    );
    let deaf2 = record(store.as_ref(), RecordKind::Meeting, Some("Two"), NOON);
    for r in [&deaf1, &deaf2] {
        store
            .append_segments(r, &[seg(Channel::Mic, 0, "hello is anyone there")])
            .unwrap();
    }
    let dictated = record(store.as_ref(), RecordKind::Dictation, None, NOON + MINUTE);
    store
        .append_segments(&dictated, &[seg(Channel::Mic, 0, "three words here")])
        .unwrap();
    let found = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "records.search", "query": "budg"}),
        "search",
    );
    assert_eq!(found["type"], "library.search");
    assert_eq!(found["query"], "budg");
    let hits = found["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h["record"] == heard.0.as_str()));
    assert_eq!(hits[0]["title"], "Budget review");

    let stats = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "library.stats", "since_unix_ms": NOON - 2 * 60 * MINUTE}),
        "stats",
    );
    assert_eq!(stats["type"], "library.stats");
    let kind = |k: &str| {
        stats["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["kind"] == k)
            .unwrap()
            .clone()
    };
    assert_eq!(kind("dictation")["records"], 1);
    assert_eq!(kind("dictation")["words"], 3);
    assert_eq!(kind("dictation")["duration_ms"], 30 * MINUTE);
    // The budget review started before the moment: not counted.
    assert_eq!(kind("meeting")["records"], 2);
    assert_eq!(kind("meeting")["duration_ms"], 60 * MINUTE);
    assert_eq!(kind("file_import")["records"], 0);
    assert_eq!(stats["far_silent_meetings"], 2);
    assert_eq!(stats["far_silent_since_unix_ms"], NOON - 60 * MINUTE);
    core.shutdown();
    events.assert_valid();
}

/// A library query never runs on the command thread: a model update stuck there does not hold
/// the library's answers up.
#[test]
fn library_queries_answer_while_the_command_thread_is_busy() {
    let dir = TempDir::new("library-busy");
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let gate = Arc::new(Gate::default());
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: Some(gate.clone()),
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader, installer);

    // The update waits at the gate, on the command thread, until the end of the test.
    core.command(&format!(
        r#"{{"cmd":"model.update","model":"{ROW_ID}","next":"{ROW_ID}"}}"#
    ))
    .unwrap();
    assert!(gate.until_waiting(WAIT));

    let listed = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "records.list"}),
        "list",
    );
    assert_eq!(listed["type"], "library.records");
    assert_eq!(listed["records"], serde_json::json!([]));
    let stats = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "library.stats", "since_unix_ms": core.shared().clock.unix_ms() - 86_400_000}),
        "stats",
    );
    assert_eq!(stats["type"], "library.stats");
    assert_eq!(events.count("model.update_finished"), 0, "still busy");

    gate.open();
    events.wait_type("model.update_finished", WAIT);
    core.shutdown();
    events.assert_valid();
}

/// A store that counts `segments()` calls, over SQLite.
struct Counting {
    inner: ink_store::SqliteStore,
    segment_reads: AtomicUsize,
}

impl Store for Counting {
    fn create_record(&self, r: NewRecord) -> Result<RecordId, StoreError> {
        self.inner.create_record(r)
    }
    fn record(&self, id: &RecordId) -> Result<Option<Record>, StoreError> {
        self.inner.record(id)
    }
    fn records(&self, q: &RecordQuery) -> Result<Vec<Record>, StoreError> {
        self.inner.records(q)
    }
    fn set_title(&self, id: &RecordId, t: &str) -> Result<(), StoreError> {
        self.inner.set_title(id, t)
    }
    fn finish_record(&self, id: &RecordId, at: i64) -> Result<(), StoreError> {
        self.inner.finish_record(id, at)
    }
    fn delete_record(&self, id: &RecordId) -> Result<(), StoreError> {
        self.inner.delete_record(id)
    }
    fn append_segments(&self, id: &RecordId, s: &[Segment]) -> Result<(), StoreError> {
        self.inner.append_segments(id, s)
    }
    fn segments(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError> {
        self.segment_reads.fetch_add(1, Ordering::SeqCst);
        self.inner.segments(id)
    }
    fn supersede_with(
        &self,
        id: &RecordId,
        s: &[Segment],
        w: SupersedeWith<'_>,
    ) -> Result<u32, StoreError> {
        self.inner.supersede_with(id, s, w)
    }
    fn save_removed(&self, id: &RecordId, l: &[Segment]) -> Result<(), StoreError> {
        self.inner.save_removed(id, l)
    }
    fn removed(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError> {
        self.inner.removed(id)
    }
    fn search(&self, q: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError> {
        self.inner.search(q, limit)
    }
    fn add_note(&self, id: &RecordId, at: u64, t: &str) -> Result<NoteId, StoreError> {
        self.inner.add_note(id, at, t)
    }
    fn update_note(&self, id: &NoteId, t: &str) -> Result<(), StoreError> {
        self.inner.update_note(id, t)
    }
    fn delete_note(&self, id: &NoteId) -> Result<(), StoreError> {
        self.inner.delete_note(id)
    }
    fn notes(&self, id: &RecordId) -> Result<Vec<Note>, StoreError> {
        self.inner.notes(id)
    }
    fn save_summary(&self, id: &RecordId, s: &Summary) -> Result<(), StoreError> {
        self.inner.save_summary(id, s)
    }
    fn summary(&self, id: &RecordId) -> Result<Option<Summary>, StoreError> {
        self.inner.summary(id)
    }
    fn set_speaker_name(&self, id: &RecordId, s: &SpeakerId, n: &str) -> Result<(), StoreError> {
        self.inner.set_speaker_name(id, s, n)
    }
    fn speaker_names(&self, id: &RecordId) -> Result<Vec<(SpeakerId, String)>, StoreError> {
        self.inner.speaker_names(id)
    }
    fn add_commitments(
        &self,
        id: &RecordId,
        i: &[NewCommitment],
    ) -> Result<Vec<CommitmentId>, StoreError> {
        self.inner.add_commitments(id, i)
    }
    fn commitments(&self, id: &RecordId) -> Result<Vec<Commitment>, StoreError> {
        self.inner.commitments(id)
    }
    fn open_commitments(&self, limit: usize) -> Result<Vec<Commitment>, StoreError> {
        self.inner.open_commitments(limit)
    }
    fn set_commitment_done(&self, id: &CommitmentId, d: bool) -> Result<(), StoreError> {
        self.inner.set_commitment_done(id, d)
    }
    fn set_done_evidence(
        &self,
        id: &CommitmentId,
        evidence: Option<&ink_core::DoneEvidence>,
    ) -> Result<(), StoreError> {
        self.inner.set_done_evidence(id, evidence)
    }
    fn merge_commitment(&self, id: &CommitmentId, into: &CommitmentId) -> Result<(), StoreError> {
        self.inner.merge_commitment(id, into)
    }
    fn setting(&self, k: &str) -> Result<Option<String>, StoreError> {
        self.inner.setting(k)
    }
    fn set_setting(&self, k: &str, v: &str) -> Result<(), StoreError> {
        self.inner.set_setting(k, v)
    }
}

/// Review fix: `record.open` read an untitled record's transcript twice (once for the record,
/// once more for its row's preview). It reads it once, and the preview is still there.
#[test]
fn opening_an_untitled_record_reads_its_transcript_once() {
    let dir = TempDir::new("library-once");
    let store = Arc::new(Counting {
        inner: ink_store::SqliteStore::open_in_memory().unwrap(),
        segment_reads: AtomicUsize::new(0),
    });
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start_parts(Parts {
        store: store.clone(),
        clock: clock(),
        registry: ink_engines::Registry::new(Vec::new()).unwrap(),
        models: ink_engines::ModelDir::new(dir.path().join("models")),
        loader,
        installer,
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
    });
    let untitled = record(store.as_ref(), RecordKind::Dictation, None, NOON);
    store
        .append_segments(&untitled, &[seg(Channel::Mic, 0, "a note to self")])
        .unwrap();
    let before = store.segment_reads.load(Ordering::SeqCst);
    let open = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "record.open", "record": untitled.0}),
        "open",
    );
    assert_eq!(open["record"]["preview"], "a note to self");
    assert_eq!(open["segments"][0]["text"], "a note to self");
    assert_eq!(store.segment_reads.load(Ordering::SeqCst) - before, 1);
    core.shutdown();
    events.assert_valid();
}

/// Review fix: a record whose audio directory points out of the library (`..`, an absolute
/// path) is refused, loudly, never read.
#[test]
fn a_record_whose_audio_is_outside_the_library_is_refused() {
    let dir = TempDir::new("library-escape");
    let (core, events) = core(&dir);
    let store = core.shared().store.clone();
    for (n, escape) in ["../outside", "/tmp"].iter().enumerate() {
        let id = store
            .create_record(NewRecord {
                kind: RecordKind::Meeting,
                title: Some("Escape".into()),
                started_at_unix_ms: NOON,
                source_app: None,
                audio_dir: Some((*escape).into()),
            })
            .unwrap();
        let failed = ask(
            &core,
            &events,
            serde_json::json!({"cmd": "record.open", "record": id.0}),
            &format!("escape{n}"),
        );
        assert_eq!(failed["type"], "command.failed", "{escape}");
        assert!(
            failed["message"]
                .as_str()
                .unwrap()
                .contains("outside the library"),
            "{failed}"
        );
    }
    core.shutdown();
    events.assert_valid();
}

/// Review fix: `library.stats` reads every transcript in its window, so the window is bounded in
/// the core: a moment more than 31 days back is refused with a clear error, not cut short.
#[test]
fn library_stats_refuses_a_window_longer_than_a_month() {
    let dir = TempDir::new("library-window");
    let (core, events) = core(&dir);
    let now = core.shared().clock.unix_ms();
    let ok = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "library.stats", "since_unix_ms": now - 7 * 24 * 3_600_000}),
        "week",
    );
    assert_eq!(ok["type"], "library.stats");
    let refused = ask(
        &core,
        &events,
        serde_json::json!({"cmd": "library.stats", "since_unix_ms": now - 32 * 24 * 3_600_000}),
        "year",
    );
    assert_eq!(refused["type"], "command.failed");
    assert!(
        refused["message"].as_str().unwrap().contains("31 days"),
        "{refused}"
    );
    core.shutdown();
    events.assert_valid();
}
