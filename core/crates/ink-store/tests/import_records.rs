//! `SqliteStore::import_records`: whole records, with everything that hangs off them, written in one
//! transaction behind a marker. Every text here is invented.

mod common;

use std::path::PathBuf;

use common::{TempDb, meeting, seg};
use ink_core::store::MAX_TIME_MS;
use ink_core::*;
use ink_store::import::{ImportError, ImportedCommitment, RecordImport};

const MARKER: &str = "import.example-source";

fn span(channel: Channel, start_ms: u64, end_ms: u64) -> Span {
    Span {
        channel,
        start_ms,
        end_ms,
    }
}

fn line(
    channel: Channel,
    start_ms: u64,
    end_ms: u64,
    text: &str,
    speaker: Option<&str>,
) -> Segment {
    Segment {
        channel,
        start_ms,
        end_ms,
        text: text.into(),
        speaker: speaker.map(|s| SpeakerId(s.into())),
    }
}

/// A meeting with every field set: a final pass (revision 3), both channels, a named speaker, a
/// summary and two commitments, one done and cited.
fn full(started: i64) -> RecordImport {
    RecordImport {
        record: NewRecord {
            kind: RecordKind::Meeting,
            title: Some("Quarterly widget review".into()),
            started_at_unix_ms: started,
            source_app: Some("com.example.meet".into()),
            audio_dir: Some(format!("meetings/{started}-imported")),
        },
        ended_at_unix_ms: Some(started + 1_800_000),
        revision: 3,
        segments: vec![
            line(
                Channel::Mic,
                0,
                2_000,
                "Morning, shall we start with the widgets?",
                None,
            ),
            line(
                Channel::Far,
                2_100,
                4_000,
                "Yes, the counts arrived overnight.",
                Some("spk0"),
            ),
            line(
                Channel::Far,
                4_100,
                6_000,
                "I will send the summary by Friday.",
                Some("spk1"),
            ),
            line(Channel::Mic, 6_000, 6_000, "Great.", None),
        ],
        summary: Some(Summary {
            items: Vec::new(),
            text: "Reviewed the widget counts.\n\nThe counts arrived.".into(),
            model: "local/example".into(),
            created_at_unix_ms: started + 1_900_000,
        }),
        speaker_names: vec![(SpeakerId("spk1".into()), "Robin Example".into())],
        commitments: vec![
            ImportedCommitment {
                commitment: NewCommitment {
                    recipient: None,
                    text: "Send the summary".into(),
                    owner: Some("Robin".into()),
                    due: Some("by Friday".into()),
                    due_at_unix_ms: Some(started + 3 * 86_400_000),
                    provenance: vec![span(Channel::Far, 4_100, 6_000)],
                },
                done: true,
            },
            ImportedCommitment {
                commitment: NewCommitment {
                    recipient: None,
                    text: "Book the room".into(),
                    owner: None,
                    due: None,
                    due_at_unix_ms: None,
                    provenance: vec![],
                },
                done: false,
            },
        ],
    }
}

/// A live-only dictation: revision 1, no end, nothing else.
fn bare(started: i64) -> RecordImport {
    RecordImport {
        record: NewRecord {
            kind: RecordKind::Dictation,
            title: None,
            started_at_unix_ms: started,
            source_app: None,
            audio_dir: None,
        },
        ended_at_unix_ms: None,
        revision: 1,
        segments: vec![line(Channel::Mic, 0, 1_500, "A short dictated note.", None)],
        summary: None,
        speaker_names: vec![],
        commitments: vec![],
    }
}

/// Every row of every table, in a comparable form.
fn dump(db: &TempDb) -> Vec<String> {
    let conn = db.raw();
    let mut out = Vec::new();
    for table in [
        "record",
        "segment",
        "note",
        "summary",
        "speaker",
        "commitment",
        "commitment_span",
        "removed_line",
        "setting",
    ] {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
            .unwrap();
        let n = stmt.column_count();
        let mut q = stmt.query([]).unwrap();
        while let Some(row) = q.next().unwrap() {
            let cells: Vec<String> = (0..n)
                .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                .collect();
            out.push(format!("{table}: {}", cells.join("|")));
        }
    }
    out
}

fn invalid(err: &ImportError) -> bool {
    matches!(err, ImportError::Store(StoreError::Invalid(_)))
}

#[test]
fn every_field_round_trips() {
    let db = TempDb::new("records-round-trip");
    let store = db.open();
    let want = full(1_700_000_000_000);
    let ids = store
        .import_records(MARKER, "{}", std::slice::from_ref(&want))
        .unwrap();
    let id = &ids[0];

    let record = store.record(id).unwrap().unwrap();
    assert_eq!(
        record,
        Record {
            id: id.clone(),
            kind: want.record.kind,
            title: want.record.title.clone(),
            started_at_unix_ms: want.record.started_at_unix_ms,
            ended_at_unix_ms: want.ended_at_unix_ms,
            source_app: want.record.source_app.clone(),
            audio_dir: want.record.audio_dir.clone(),
            // Set directly: no pass had to run.
            revision: 3,
            imported: true,
            stuck: false,
        }
    );
    assert_eq!(store.segments(id).unwrap(), want.segments);
    assert_eq!(store.summary(id).unwrap(), want.summary);
    assert_eq!(store.speaker_names(id).unwrap(), want.speaker_names);
    let got = store.commitments(id).unwrap();
    assert_eq!(got.len(), 2);
    for (got, want) in got.iter().zip(&want.commitments) {
        assert_eq!(got.record, *id);
        assert_eq!(got.text, want.commitment.text);
        assert_eq!(got.owner, want.commitment.owner);
        assert_eq!(got.due, want.commitment.due);
        assert_eq!(got.due_at_unix_ms, want.commitment.due_at_unix_ms);
        assert_eq!(got.provenance, want.commitment.provenance);
        assert_eq!(got.done, want.done);
        assert_eq!(got.merged_into, None);
    }
    // The summary is of the revision it was imported with.
    let summary_revision: u32 = db
        .raw()
        .query_row(
            "SELECT transcript_revision FROM summary WHERE record_id = ?1",
            [&id.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(summary_revision, 3);
    // Only the open commitment is owed.
    let open = store.open_commitments(10).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].text, "Book the room");
}

#[test]
fn ids_come_back_in_order_and_the_marker_is_written() {
    let db = TempDb::new("records-order");
    let store = db.open();
    let batch = [full(3_000), bare(1_000), full(2_000)];
    let ids = store.import_records(MARKER, r#"{"n":3}"#, &batch).unwrap();
    assert_eq!(ids.len(), 3);
    for (id, want) in ids.iter().zip(&batch) {
        let record = store.record(id).unwrap().unwrap();
        assert_eq!(record.started_at_unix_ms, want.record.started_at_unix_ms);
        assert_eq!(record.kind, want.record.kind);
    }
    let bare = store.record(&ids[1]).unwrap().unwrap();
    assert_eq!((bare.revision, bare.ended_at_unix_ms), (1, None));
    assert_eq!(
        store.setting(MARKER).unwrap().as_deref(),
        Some(r#"{"n":3}"#)
    );
    // Nothing to import still writes the marker, so the same source is not imported twice.
    let db = TempDb::new("records-empty");
    let store = db.open();
    assert!(store.import_records(MARKER, "{}", &[]).unwrap().is_empty());
    assert!(store.setting(MARKER).unwrap().is_some());
}

#[test]
fn an_import_leaves_the_records_already_there_untouched() {
    let db = TempDb::new("records-existing");
    let store = db.open();
    let kept = meeting(&store, 10);
    store
        .append_segments(&kept, &[seg(Channel::Mic, 0, "an existing line")])
        .unwrap();
    store
        .supersede(&kept, &[seg(Channel::Mic, 0, "an existing final line")])
        .unwrap();
    store.add_note(&kept, 0, "an existing note").unwrap();
    store
        .add_commitments(
            &kept,
            &[NewCommitment {
                recipient: None,
                text: "an existing promise".into(),
                owner: None,
                due: None,
                due_at_unix_ms: None,
                provenance: vec![],
            }],
        )
        .unwrap();
    store.set_setting("some.setting", "kept").unwrap();
    let before = dump(&db);

    store
        .import_records(MARKER, "{}", &[full(20), bare(30)])
        .unwrap();
    let after = dump(&db);
    for row in &before {
        assert!(after.contains(row), "changed or gone: {row}");
    }
    assert!(after.len() > before.len());
    assert_eq!(
        store.segments(&kept).unwrap()[0].text,
        "an existing final line"
    );
    assert_eq!(store.record(&kept).unwrap().unwrap().revision, 2);
}

/// Retention never deletes what an import brought in: each record the import writes is marked as
/// imported, and a record made here, before the import or after it, is not.
#[test]
fn the_records_an_import_writes_are_marked_and_nothing_else() {
    let db = TempDb::new("records-marked");
    let store = db.open();
    let before = meeting(&store, 10);
    let mut imported = store
        .import_records(MARKER, "{}", &[full(20), bare(30)])
        .unwrap();
    let after = meeting(&store, 40);
    for id in &imported {
        assert!(store.record(id).unwrap().unwrap().imported);
    }
    for id in [&before, &after] {
        assert!(!store.record(id).unwrap().unwrap().imported);
    }
    let listed = store
        .records(&RecordQuery {
            kind: None,
            before: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(listed.len(), 4);
    let mut marked: Vec<RecordId> = listed
        .into_iter()
        .filter(|r| r.imported)
        .map(|r| r.id)
        .collect();
    marked.sort();
    imported.sort();
    assert_eq!(marked, imported);
}

#[test]
fn a_marker_already_present_refuses_and_writes_nothing() {
    let db = TempDb::new("records-marker");
    let store = db.open();
    store.import_records(MARKER, "first", &[bare(1)]).unwrap();
    let before = dump(&db);
    let err = store
        .import_records(MARKER, "second", &[full(2)])
        .unwrap_err();
    assert!(matches!(err, ImportError::MarkerPresent), "{err:?}");
    assert_eq!(dump(&db), before);
    // Another source's marker does not stand in the way.
    store
        .import_records("import.another", "{}", &[bare(3)])
        .unwrap();
}

#[test]
fn a_failure_midway_rolls_every_row_back() {
    let db = TempDb::new("records-midway");
    let store = db.open();
    let kept = meeting(&store, 1);
    store
        .append_segments(&kept, &[seg(Channel::Far, 0, "kept")])
        .unwrap();
    let before = dump(&db);
    // Fails on the last commitment span: records, lines, summaries and names are written by then.
    db.raw()
        .execute_batch(
            "CREATE TRIGGER fail_late AFTER INSERT ON commitment_span
             WHEN (SELECT count(*) FROM commitment_span) >= 2
             BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
        )
        .unwrap();
    let err = store
        .import_records(MARKER, "{}", &[full(10), bare(20), full(30)])
        .unwrap_err();
    assert!(
        matches!(err, ImportError::Store(StoreError::Backend(_))),
        "{err:?}"
    );
    assert_eq!(
        dump(&db),
        before,
        "one transaction: nothing stays, the marker neither"
    );
    // With the fault gone the same import goes through: the failure left no marker.
    db.raw().execute_batch("DROP TRIGGER fail_late").unwrap();
    assert_eq!(
        store
            .import_records(MARKER, "{}", &[full(10), bare(20), full(30)])
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn imported_text_is_searchable_and_the_index_stays_consistent() {
    let db = TempDb::new("records-search");
    let store = db.open();
    let ids = store
        .import_records(MARKER, "{}", &[full(5), bare(6)])
        .unwrap();
    let hits = store.search("widgets", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record, ids[0]);
    assert_eq!(hits[0].title.as_deref(), Some("Quarterly widget review"));
    assert_eq!(store.search("dictated", 10).unwrap()[0].record, ids[1]);
    db.raw()
        .execute(
            "INSERT INTO segment_fts (segment_fts, rank) VALUES ('integrity-check', 1)",
            [],
        )
        .expect("the FTS index matches the segment table");
}

#[test]
fn invalid_input_is_refused_before_anything_is_written() {
    let db = TempDb::new("records-invalid");
    let store = db.open();
    let before = dump(&db);
    let mut cases: Vec<(&str, RecordImport)> = Vec::new();
    let mut r = full(1);
    r.revision = 0;
    cases.push(("revision 0", r));
    let mut r = full(1);
    r.segments[1].end_ms = r.segments[1].start_ms - 1;
    cases.push(("a line that ends before it starts", r));
    let mut r = full(1);
    r.segments[0].start_ms = MAX_TIME_MS + 1;
    r.segments[0].end_ms = MAX_TIME_MS + 1;
    cases.push(("a time beyond the store's range", r));
    let mut r = full(1);
    r.commitments[0].commitment.provenance = vec![span(Channel::Far, 10, 5)];
    cases.push(("a span that ends before it starts", r));
    let mut r = full(1_000);
    r.ended_at_unix_ms = Some(999);
    cases.push(("an end before the start", r));
    let mut r = full(1);
    r.speaker_names
        .push((SpeakerId("spk1".into()), "A second name".into()));
    cases.push(("one speaker named twice", r));
    for (what, r) in cases {
        // The bad record comes second: nothing of the first may be written either.
        let err = store
            .import_records(MARKER, "{}", &[bare(0), r])
            .unwrap_err();
        assert!(invalid(&err), "{what}: {err:?}");
        assert!(!err.to_string().contains("widget"), "{what}: {err}");
        assert_eq!(dump(&db), before, "{what}");
    }
    let err = store.import_records("", "{}", &[bare(0)]).unwrap_err();
    assert!(invalid(&err), "an empty marker key: {err:?}");
    assert_eq!(dump(&db), before);
}

// --- The scrub rules ---------------------------------------------------------------------------
//
// A successful import only adds rows: the marker must be absent, every record is new, and a
// speaker named twice is refused, so no row is replaced or deleted and there is nothing to scrub.
// A failed one is rolled back, but pages SQLite spilled to the log before the failure still hold
// the text it was writing, so a failure is followed by the same checkpoint a delete gets.

fn on_disk(db: &TempDb, marker: &str) -> usize {
    ["", "-wal"]
        .into_iter()
        .filter_map(|suffix| {
            let mut path = db.path().into_os_string();
            path.push(suffix);
            std::fs::read(PathBuf::from(path)).ok()
        })
        .map(|bytes| {
            bytes
                .windows(marker.len())
                .filter(|w| *w == marker.as_bytes())
                .count()
        })
        .sum()
}

/// Enough text that SQLite spills pages to the log before the transaction ends.
fn large(started: i64, marker: &str) -> RecordImport {
    let mut r = bare(started);
    r.segments = (0..20_000u64)
        .map(|n| {
            line(
                Channel::Mic,
                n * 1_000,
                n * 1_000 + 900,
                &format!(
                    "line {n} carries {marker} and enough ordinary words to fill a page quickly"
                ),
                None,
            )
        })
        .collect();
    r
}

#[test]
fn a_successful_import_replaces_nothing_and_needs_no_scrub() {
    let db = TempDb::new("records-no-scrub");
    let store = db.open();
    store
        .import_records(MARKER, "{}", &[full(1), bare(2)])
        .unwrap();
    assert!(!store.unscrubbed());
    assert_eq!(store.scrub_change(), None);
    // Every imported row is still there: nothing was written twice and dropped.
    let segments: i64 = db
        .raw()
        .query_row("SELECT count(*) FROM segment", [], |r| r.get(0))
        .unwrap();
    assert_eq!(segments, 5);
}

#[test]
fn a_failed_import_leaves_none_of_its_text_in_the_log() {
    let db = TempDb::new("records-failed-scrub");
    let store = db.open();
    meeting(&store, 1);
    db.raw()
        .execute_batch(
            "CREATE TRIGGER fail_at_marker BEFORE INSERT ON setting
             BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
        )
        .unwrap();
    let err = store
        .import_records(MARKER, "{}", &[large(10, "zqxspilledmarker")])
        .unwrap_err();
    assert!(matches!(err, ImportError::Store(_)), "{err:?}");
    assert_eq!(on_disk(&db, "zqxspilledmarker"), 0);
    assert!(!store.unscrubbed());
}
