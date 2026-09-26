//! The `Store` contract, run against `MemStore` and `SqliteStore` alike.
//!
//! The first seven scenarios restate the store section of `ink-core`'s contract tests; the rest
//! pin behaviour both stores must share that those tests leave implicit. Each scenario runs three
//! times: on the in-memory mock, on SQLite in memory, and on SQLite in a WAL file. Where the two
//! stores could disagree, the `ink-core` docs decide.

mod common;

use common::{meeting, seg};
use ink_core::*;

fn assert_send_sync<T: ?Sized + Send + Sync>() {}

#[test]
fn both_stores_are_send_sync_store_objects() {
    assert_send_sync::<dyn Store>();
    assert_send_sync::<ink_store::SqliteStore>();
    assert_send_sync::<ink_core::mock::MemStore>();
}

// --- Restated from ink-core/tests/contracts.rs -------------------------------------------------

fn supersede_refuses_empty_and_collapsed_revisions_and_keeps_the_live_one(store: &dyn Store) {
    let id = meeting(store, 1);
    let live = [
        seg(Channel::Mic, 0, "one two three four"),
        seg(Channel::Far, 1_000, "five six seven eight"),
    ];
    store.append_segments(&id, &live).unwrap();

    assert_eq!(store.supersede(&id, &[]), Err(StoreError::EmptySupersede));
    assert_eq!(
        store.supersede(&id, &[seg(Channel::Mic, 0, "   ")]),
        Err(StoreError::EmptySupersede)
    );
    assert_eq!(
        store.supersede(&id, &[seg(Channel::Mic, 0, "one two three")]),
        Err(StoreError::SuspiciousSupersede {
            channel: Channel::Far,
            previous_words: 4,
            new_words: 0
        })
    );
    assert_eq!(store.segments(&id).unwrap(), live.to_vec());
    assert_eq!(store.record(&id).unwrap().unwrap().revision, 1);

    let offline = [
        seg(Channel::Mic, 0, "one two three four"),
        seg(Channel::Far, 1_000, "five six"),
    ];
    assert_eq!(store.supersede(&id, &offline), Ok(2));
    assert_eq!(store.segments(&id).unwrap(), offline.to_vec());
    assert_eq!(store.record(&id).unwrap().unwrap().revision, 2);
}

fn one_channel_collapsing_is_refused_even_when_the_total_passes(store: &dyn Store) {
    let id = meeting(store, 1);
    let ten = "w w w w w w w w w w";
    store
        .append_segments(
            &id,
            &[seg(Channel::Mic, 0, ten), seg(Channel::Far, 1_000, ten)],
        )
        .unwrap();

    let far_only = [seg(Channel::Far, 1_000, "w w w w w w w w w w w")];
    assert_eq!(
        store.supersede(&id, &far_only),
        Err(StoreError::SuspiciousSupersede {
            channel: Channel::Mic,
            previous_words: 10,
            new_words: 0
        })
    );
    assert_eq!(store.record(&id).unwrap().unwrap().revision, 1);

    let other = meeting(store, 2);
    store
        .append_segments(&other, &[seg(Channel::Mic, 0, ten)])
        .unwrap();
    assert_eq!(
        store.supersede(
            &other,
            &[seg(Channel::Mic, 0, ten), seg(Channel::Far, 9, "hi")]
        ),
        Ok(2)
    );
}

fn unknown_records_are_not_found(store: &dyn Store) {
    let ghost = RecordId("nope".into());
    assert_eq!(store.record(&ghost), Ok(None));
    assert_eq!(store.segments(&ghost), Err(StoreError::NotFound));
    assert_eq!(
        store.supersede(&ghost, &[seg(Channel::Mic, 0, "x")]),
        Err(StoreError::NotFound)
    );
    assert_eq!(store.delete_record(&ghost), Err(StoreError::NotFound));
    assert_eq!(store.set_title(&ghost, "x"), Err(StoreError::NotFound));
    assert_eq!(store.add_note(&ghost, 0, "x"), Err(StoreError::NotFound));
}

fn records_segments_search_speakers_and_settings(store: &dyn Store) {
    let older = meeting(store, 100);
    let newer = meeting(store, 200);
    let dictation = store
        .create_record(NewRecord {
            kind: RecordKind::Dictation,
            title: None,
            started_at_unix_ms: 300,
            source_app: None,
            audio_dir: Some("audio/imported-1".into()),
        })
        .unwrap();
    let ids = |q: &RecordQuery| -> Vec<RecordId> {
        store
            .records(q)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    };
    let all = RecordQuery {
        kind: None,
        started_before_unix_ms: None,
        limit: 10,
    };
    assert_eq!(
        ids(&all),
        vec![dictation.clone(), newer.clone(), older.clone()]
    );
    assert_eq!(
        ids(&RecordQuery {
            limit: 1,
            ..all.clone()
        }),
        vec![dictation.clone()]
    );
    assert_eq!(
        ids(&RecordQuery {
            kind: Some(RecordKind::Meeting),
            ..all.clone()
        }),
        vec![newer.clone(), older.clone()]
    );
    assert_eq!(
        ids(&RecordQuery {
            started_before_unix_ms: Some(200),
            ..all.clone()
        }),
        vec![older.clone()],
        "the cursor is exclusive, so paging never repeats a record"
    );
    assert_eq!(
        store
            .record(&dictation)
            .unwrap()
            .unwrap()
            .audio_dir
            .as_deref(),
        Some("audio/imported-1")
    );

    assert_eq!(store.record(&older).unwrap().unwrap().title, None);
    store.set_title(&older, "Draft review").unwrap();
    assert_eq!(
        store.record(&older).unwrap().unwrap().title.as_deref(),
        Some("Draft review")
    );

    store
        .append_segments(
            &older,
            &[
                seg(Channel::Far, 5_000, "Ship the Draft"),
                seg(Channel::Mic, 1_000, "hello"),
            ],
        )
        .unwrap();
    let starts: Vec<u64> = store
        .segments(&older)
        .unwrap()
        .iter()
        .map(|s| s.start_ms)
        .collect();
    assert_eq!(starts, vec![1_000, 5_000]);

    let hits = store.search("ship", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        (hits[0].record.clone(), hits[0].start_ms),
        (older.clone(), 5_000)
    );
    assert_eq!(hits[0].title.as_deref(), Some("Draft review"));
    assert_eq!(hits[0].started_at_unix_ms, 100);
    assert!(store.search("  ", 10).unwrap().is_empty());

    store.finish_record(&older, 900).unwrap();
    assert_eq!(
        store.record(&older).unwrap().unwrap().ended_at_unix_ms,
        Some(900)
    );

    let spk = SpeakerId("spk0".into());
    store.set_speaker_name(&older, &spk, "Guest").unwrap();
    assert_eq!(
        store.speaker_names(&older).unwrap(),
        vec![(spk, "Guest".to_string())]
    );

    let summary = Summary {
        text: "short".into(),
        model: "mock".into(),
        created_at_unix_ms: 5,
    };
    store.save_summary(&older, &summary).unwrap();
    assert_eq!(store.summary(&older).unwrap(), Some(summary));
    assert_eq!(store.summary(&newer).unwrap(), None);

    assert_eq!(store.setting("hotkey").unwrap(), None);
    store.set_setting("hotkey", "fn").unwrap();
    assert_eq!(store.setting("hotkey").unwrap().as_deref(), Some("fn"));
}

fn commitments_merge_complete_and_go_with_their_record(store: &dyn Store) {
    let id = meeting(store, 1);
    let said = |text: &str, at: u64| NewCommitment {
        text: text.into(),
        owner: Some("Guest".into()),
        due: None,
        due_at_unix_ms: None,
        provenance: vec![Span {
            channel: Channel::Far,
            start_ms: at,
            end_ms: at + 500,
        }],
    };
    let ids = store
        .add_commitments(
            &id,
            &[
                said("send the deck", 1_000),
                said("send over the deck", 9_000),
            ],
        )
        .unwrap();
    assert_eq!(ids.len(), 2);

    store.merge_commitment(&ids[1], &ids[0]).unwrap();
    assert!(matches!(
        store.merge_commitment(&ids[0], &ids[0]),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(
        store.merge_commitment(&ids[0], &CommitmentId("gone".into())),
        Err(StoreError::NotFound)
    );
    store.set_commitment_done(&ids[0], true).unwrap();

    let all = store.commitments(&id).unwrap();
    assert_eq!(all.len(), 2, "merged commitments are kept for audit");
    assert!(all[0].done);
    assert_eq!(all[1].merged_into.as_ref(), Some(&ids[0]));
    assert_eq!(all[1].provenance[0].start_ms, 9_000);

    store.delete_record(&id).unwrap();
    assert_eq!(
        store.set_commitment_done(&ids[0], false),
        Err(StoreError::NotFound)
    );
}

fn open_commitments_span_records_and_skip_done_and_merged(store: &dyn Store) {
    let a = meeting(store, 1);
    let b = meeting(store, 2);
    let owe = |text: &str, due_at: Option<i64>| NewCommitment {
        text: text.into(),
        owner: Some("Guest".into()),
        due: due_at.map(|_| "as said".into()),
        due_at_unix_ms: due_at,
        provenance: vec![],
    };
    let in_a = store
        .add_commitments(
            &a,
            &[
                owe("undated", None),
                owe("later", Some(900)),
                owe("done", Some(1)),
            ],
        )
        .unwrap();
    let in_b = store
        .add_commitments(
            &b,
            &[owe("soonest", Some(100)), owe("repeat of later", Some(900))],
        )
        .unwrap();
    store.set_commitment_done(&in_a[2], true).unwrap();
    store.merge_commitment(&in_b[1], &in_a[1]).unwrap();

    let open: Vec<String> = store
        .open_commitments(10)
        .unwrap()
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(open, vec!["soonest", "later", "undated"]);
    assert_eq!(store.open_commitments(1).unwrap().len(), 1);
}

fn notes_are_kept_in_time_order_and_go_with_their_record(store: &dyn Store) {
    let id = meeting(store, 1);
    let late = store.add_note(&id, 9_000, "follow up on pricing").unwrap();
    let early = store.add_note(&id, 1_000, "agenda").unwrap();
    store
        .update_note(&late, "follow up on pricing, Friday")
        .unwrap();

    let notes = store.notes(&id).unwrap();
    assert_eq!(
        notes
            .iter()
            .map(|n| (n.id.clone(), n.at_ms))
            .collect::<Vec<_>>(),
        vec![(early.clone(), 1_000), (late.clone(), 9_000)]
    );
    assert_eq!(notes[1].text, "follow up on pricing, Friday");
    assert_eq!(notes[0].record, id);

    store.delete_note(&early).unwrap();
    assert_eq!(store.delete_note(&early), Err(StoreError::NotFound));
    assert_eq!(store.update_note(&early, "x"), Err(StoreError::NotFound));
    store.delete_record(&id).unwrap();
    assert_eq!(store.update_note(&late, "x"), Err(StoreError::NotFound));
}

// --- Shared behaviour the ink-core tests leave implicit ------------------------------------------

/// Every call scoped to one record reports an unknown record the same way; only `record` answers
/// `None`.
fn every_record_scoped_call_on_an_unknown_record_is_not_found(store: &dyn Store) {
    let ghost = RecordId("nope".into());
    let nf = Err(StoreError::NotFound);
    assert_eq!(store.finish_record(&ghost, 1), nf);
    assert_eq!(
        store.append_segments(&ghost, &[seg(Channel::Mic, 0, "x")]),
        nf
    );
    assert_eq!(store.append_segments(&ghost, &[]), nf);
    assert_eq!(store.notes(&ghost), Err(StoreError::NotFound));
    let summary = Summary {
        text: "s".into(),
        model: "m".into(),
        created_at_unix_ms: 1,
    };
    assert_eq!(store.save_summary(&ghost, &summary), nf);
    assert_eq!(store.summary(&ghost), Err(StoreError::NotFound));
    assert_eq!(
        store.set_speaker_name(&ghost, &SpeakerId("spk0".into()), "A"),
        nf
    );
    assert_eq!(store.speaker_names(&ghost), Err(StoreError::NotFound));
    assert_eq!(
        store.add_commitments(&ghost, &[]),
        Err(StoreError::NotFound)
    );
    assert_eq!(store.commitments(&ghost), Err(StoreError::NotFound));

    let gone = CommitmentId("gone".into());
    assert_eq!(store.set_commitment_done(&gone, true), nf);
    assert_eq!(
        store.merge_commitment(&gone, &CommitmentId("also gone".into())),
        nf
    );
    let real = meeting(store, 1);
    let ids = store
        .add_commitments(
            &real,
            &[NewCommitment {
                text: "t".into(),
                owner: None,
                due: None,
                due_at_unix_ms: None,
                provenance: vec![],
            }],
        )
        .unwrap();
    assert_eq!(store.merge_commitment(&gone, &ids[0]), nf);
    assert_eq!(store.update_note(&NoteId("gone".into()), "x"), nf);
    assert_eq!(store.delete_note(&NoteId("gone".into())), nf);
}

/// Records that started in the same millisecond still list in one stable order: id descending.
fn records_with_the_same_start_order_by_id_descending(store: &dyn Store) {
    let mut ids: Vec<RecordId> = (0..6).map(|_| meeting(store, 500)).collect();
    ids.sort();
    ids.reverse();
    let listed: Vec<RecordId> = store
        .records(&RecordQuery {
            kind: None,
            started_before_unix_ms: None,
            limit: 100,
        })
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(listed, ids);
    assert!(
        store
            .records(&RecordQuery {
                kind: None,
                started_before_unix_ms: None,
                limit: 0,
            })
            .unwrap()
            .is_empty()
    );
}

/// Ties in due time keep the order the commitments were added, whatever their ids sort as.
fn open_commitment_ties_keep_the_order_they_were_added(store: &dyn Store) {
    let a = meeting(store, 1);
    let b = meeting(store, 2);
    let owe = |text: String, due_at: Option<i64>| NewCommitment {
        text,
        owner: None,
        due: None,
        due_at_unix_ms: due_at,
        provenance: vec![],
    };
    let mut expected_dated = Vec::new();
    let mut expected_undated = Vec::new();
    for round in 0..10 {
        let record = if round % 2 == 0 { &a } else { &b };
        let dated = format!("dated {round}");
        let undated = format!("undated {round}");
        store
            .add_commitments(
                record,
                &[owe(undated.clone(), None), owe(dated.clone(), Some(7_000))],
            )
            .unwrap();
        expected_dated.push(dated);
        expected_undated.push(undated);
    }
    let expected: Vec<String> = expected_dated.into_iter().chain(expected_undated).collect();
    let open: Vec<String> = store
        .open_commitments(100)
        .unwrap()
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(open, expected);
    assert_eq!(
        store.open_commitments(3).unwrap().len(),
        3,
        "the limit applies after ordering"
    );

    let texts = |r: &RecordId| -> Vec<String> {
        store
            .commitments(r)
            .unwrap()
            .into_iter()
            .map(|c| c.text)
            .collect()
    };
    assert_eq!(
        texts(&a)[..4],
        ["undated 0", "dated 0", "undated 2", "dated 2"]
    );
}

/// Segments sharing a start time list the mic first, then in the order they were appended; notes
/// sharing a stamp keep the order they were added.
fn same_time_segments_and_notes_keep_a_stable_order(store: &dyn Store) {
    let id = meeting(store, 1);
    store
        .append_segments(
            &id,
            &[
                seg(Channel::Far, 2_000, "far first"),
                seg(Channel::Mic, 2_000, "mic first"),
                seg(Channel::Far, 2_000, "far second"),
                seg(Channel::Mic, 2_000, "mic second"),
                seg(Channel::Mic, 0, "opening"),
            ],
        )
        .unwrap();
    let texts: Vec<String> = store
        .segments(&id)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(
        texts,
        [
            "opening",
            "mic first",
            "mic second",
            "far first",
            "far second"
        ]
    );

    let notes: Vec<NoteId> = (0..6)
        .map(|n| store.add_note(&id, 4_000, &format!("note {n}")).unwrap())
        .collect();
    let listed: Vec<NoteId> = store
        .notes(&id)
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(listed, notes);
}

/// Setting a value twice keeps the second; so do speaker names and summaries.
fn upserts_replace_the_previous_value(store: &dyn Store) {
    let id = meeting(store, 1);
    store.set_setting("theme", "dark").unwrap();
    store.set_setting("theme", "light").unwrap();
    assert_eq!(store.setting("theme").unwrap().as_deref(), Some("light"));

    let s1 = SpeakerId("spk1".into());
    let s0 = SpeakerId("spk0".into());
    store.set_speaker_name(&id, &s1, "Host").unwrap();
    store.set_speaker_name(&id, &s0, "Guest").unwrap();
    store.set_speaker_name(&id, &s1, "Chair").unwrap();
    assert_eq!(
        store.speaker_names(&id).unwrap(),
        vec![(s0, "Guest".to_string()), (s1, "Chair".to_string())]
    );

    let first = Summary {
        text: "first".into(),
        model: "a".into(),
        created_at_unix_ms: 1,
    };
    let second = Summary {
        text: "second".into(),
        model: "b".into(),
        created_at_unix_ms: 2,
    };
    store.save_summary(&id, &first).unwrap();
    store.save_summary(&id, &second).unwrap();
    assert_eq!(store.summary(&id).unwrap(), Some(second));
}

/// Search sees the current revision only, and forgets a deleted record.
fn search_follows_supersede_and_delete(store: &dyn Store) {
    let id = meeting(store, 1);
    store
        .append_segments(&id, &[seg(Channel::Mic, 0, "alpha bravo charlie")])
        .unwrap();
    assert_eq!(store.search("alpha", 10).unwrap().len(), 1);

    store
        .supersede(&id, &[seg(Channel::Mic, 250, "delta echo foxtrot")])
        .unwrap();
    assert!(store.search("alpha", 10).unwrap().is_empty());
    let hits = store.search("echo", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].start_ms, 250);
    assert_eq!(hits[0].snippet, "delta echo foxtrot");

    assert!(store.search("echo", 0).unwrap().is_empty());
    assert!(store.search("", 10).unwrap().is_empty());
    assert!(store.search("\t\n ", 10).unwrap().is_empty());

    store.delete_record(&id).unwrap();
    assert!(store.search("echo", 10).unwrap().is_empty());
}

/// Every field of a record and a commitment survives the round trip.
fn fields_round_trip(store: &dyn Store) {
    let id = store
        .create_record(NewRecord {
            kind: RecordKind::FileImport,
            title: Some("Quarterly sync.m4a".into()),
            started_at_unix_ms: -5,
            source_app: Some("com.example.files".into()),
            audio_dir: Some("audio/x".into()),
        })
        .unwrap();
    store.finish_record(&id, 1_700_000_000_000).unwrap();
    let record = store.record(&id).unwrap().unwrap();
    assert_eq!(
        record,
        Record {
            id: id.clone(),
            kind: RecordKind::FileImport,
            title: Some("Quarterly sync.m4a".into()),
            started_at_unix_ms: -5,
            ended_at_unix_ms: Some(1_700_000_000_000),
            source_app: Some("com.example.files".into()),
            audio_dir: Some("audio/x".into()),
            revision: 1,
        }
    );

    let segment = Segment {
        channel: Channel::Far,
        start_ms: 61_250,
        end_ms: 64_900,
        text: "Café au lait, s'il vous plaît".into(),
        speaker: Some(SpeakerId("spk2".into())),
    };
    store
        .append_segments(&id, std::slice::from_ref(&segment))
        .unwrap();
    assert_eq!(store.segments(&id).unwrap(), vec![segment]);

    let provenance = vec![
        Span {
            channel: Channel::Mic,
            start_ms: 10,
            end_ms: 20,
        },
        Span {
            channel: Channel::Far,
            start_ms: 30,
            end_ms: 45,
        },
    ];
    let ids = store
        .add_commitments(
            &id,
            &[NewCommitment {
                text: "draft the plan".into(),
                owner: Some("Host".into()),
                due: Some("next Tuesday".into()),
                due_at_unix_ms: Some(1_700_100_000_000),
                provenance: provenance.clone(),
            }],
        )
        .unwrap();
    assert_eq!(
        store.commitments(&id).unwrap(),
        vec![Commitment {
            id: ids[0].clone(),
            record: id.clone(),
            text: "draft the plan".into(),
            owner: Some("Host".into()),
            due: Some("next Tuesday".into()),
            due_at_unix_ms: Some(1_700_100_000_000),
            provenance,
            merged_into: None,
            done: false,
        }]
    );
    assert_eq!(store.open_commitments(10).unwrap().len(), 1);
    store.set_commitment_done(&ids[0], true).unwrap();
    assert!(store.open_commitments(10).unwrap().is_empty());
    store.set_commitment_done(&ids[0], false).unwrap();
    assert_eq!(store.open_commitments(10).unwrap().len(), 1);
}

macro_rules! contract {
    ($($scenario:ident),* $(,)?) => {
        mod mem {
            $(#[test]
            fn $scenario() {
                super::$scenario(&ink_core::mock::MemStore::new());
            })*
        }
        mod sqlite_memory {
            $(#[test]
            fn $scenario() {
                super::$scenario(&ink_store::SqliteStore::open_in_memory().unwrap());
            })*
        }
        mod sqlite_file {
            $(#[test]
            fn $scenario() {
                let db = crate::common::TempDb::new(stringify!($scenario));
                super::$scenario(&db.open());
            })*
        }
    };
}

contract!(
    supersede_refuses_empty_and_collapsed_revisions_and_keeps_the_live_one,
    one_channel_collapsing_is_refused_even_when_the_total_passes,
    unknown_records_are_not_found,
    records_segments_search_speakers_and_settings,
    commitments_merge_complete_and_go_with_their_record,
    open_commitments_span_records_and_skip_done_and_merged,
    notes_are_kept_in_time_order_and_go_with_their_record,
    every_record_scoped_call_on_an_unknown_record_is_not_found,
    records_with_the_same_start_order_by_id_descending,
    open_commitment_ties_keep_the_order_they_were_added,
    same_time_segments_and_notes_keep_a_stable_order,
    upserts_replace_the_previous_value,
    search_follows_supersede_and_delete,
    fields_round_trip,
);
