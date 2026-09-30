//! What only the SQLite store has to prove: the file, the schema, the transaction, the index.

mod common;

use std::sync::Arc;

use common::{TempDb, meeting, seg};
use ink_core::*;
use ink_store::{SCHEMA_VERSION, SqliteStore};

fn user_version(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

/// Every row of the tables a supersede touches, in a comparable form.
fn dump(conn: &rusqlite::Connection) -> Vec<String> {
    let mut rows = Vec::new();
    for sql in [
        "SELECT seq, record_id, revision, channel, start_ms, end_ms, text, speaker FROM segment ORDER BY seq",
        "SELECT id, revision FROM record ORDER BY id",
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        let n = stmt.column_count();
        let mut q = stmt.query([]).unwrap();
        while let Some(row) = q.next().unwrap() {
            let cells: Vec<String> = (0..n)
                .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                .collect();
            rows.push(cells.join("|"));
        }
    }
    rows
}

fn fts_is_consistent(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT INTO segment_fts (segment_fts, rank) VALUES ('integrity-check', 1)",
        [],
    )
    .expect("the FTS index matches the segment table");
}

// --- The build ---------------------------------------------------------------------------------

#[test]
fn fts5_is_compiled_in() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE VIRTUAL TABLE probe USING fts5(text)")
        .unwrap();
    conn.execute("INSERT INTO probe (text) VALUES ('fts five works')", [])
        .unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM probe WHERE probe MATCH 'five'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

// --- Migrations ------------------------------------------------------------------------------

#[test]
fn migrations_from_empty_reach_the_current_version() {
    let db = TempDb::new("migrations");
    drop(db.open());
    let raw = db.raw();
    assert_eq!(user_version(&raw), SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 4);

    let mut stmt = raw
        .prepare(
            "SELECT name, strict FROM pragma_table_list \
             WHERE schema = 'main' AND type = 'table' AND name NOT LIKE 'sqlite_%' \
             AND name NOT LIKE 'segment_fts_%' ORDER BY name",
        )
        .unwrap();
    let tables: Vec<(String, bool)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let names: Vec<&str> = tables.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "commitment",
            "commitment_done_evidence",
            "commitment_span",
            "note",
            "record",
            "removed_line",
            "segment",
            "setting",
            "speaker",
            "summary",
            "summary_item",
        ]
    );
    assert!(
        tables.iter().all(|(_, strict)| *strict),
        "every table is STRICT: {tables:?}"
    );
    let fts: String = raw
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'segment_fts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(fts.contains("fts5"), "{fts}");
    let mode: String = raw
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn a_database_from_before_removed_lines_is_brought_up_to_date() {
    let db = TempDb::new("migrate-v1");
    let id = {
        let store = db.open();
        let id = meeting(&store, 1);
        store
            .append_segments(&id, &[seg(Channel::Mic, 0, "kept through the upgrade")])
            .unwrap();
        id
    };
    // As a build of schema 1 left it.
    {
        let raw = db.raw();
        raw.execute_batch(
            "DROP TABLE removed_line; DROP TABLE summary_item;
             DROP TABLE commitment_done_evidence; ALTER TABLE commitment DROP COLUMN recipient;
             ALTER TABLE record DROP COLUMN imported; PRAGMA user_version = 1;",
        )
        .unwrap();
    }
    let store = db.open();
    assert_eq!(user_version(&db.raw()), SCHEMA_VERSION);
    assert_eq!(store.segments(&id).unwrap().len(), 1);
    store
        .save_removed(&id, &[seg(Channel::Mic, 5, "an echo line")])
        .unwrap();
    assert_eq!(store.removed(&id).unwrap().len(), 1);
}

/// S2.8's migration: a schema-2 library keeps its commitments and summaries, which read back
/// with no recipient, no items and no suggestion, and takes the new ones.
#[test]
fn a_database_from_before_summary_items_is_brought_up_to_date() {
    let db = TempDb::new("migrate-v2");
    let (id, commitment) = {
        let store = db.open();
        let id = meeting(&store, 1);
        let ids = store
            .add_commitments(
                &id,
                &[NewCommitment {
                    text: "kept through the upgrade".into(),
                    owner: None,
                    recipient: None,
                    due: None,
                    due_at_unix_ms: None,
                    provenance: vec![],
                }],
            )
            .unwrap();
        store
            .save_summary(
                &id,
                &Summary {
                    text: "A summary.".into(),
                    model: "m".into(),
                    created_at_unix_ms: 1,
                    items: vec![],
                },
            )
            .unwrap();
        (id, ids[0].clone())
    };
    // As a build of schema 2 left it.
    {
        let raw = db.raw();
        raw.execute_batch(
            "DROP TABLE summary_item; DROP TABLE commitment_done_evidence;
             ALTER TABLE commitment DROP COLUMN recipient; ALTER TABLE record DROP COLUMN imported;
             PRAGMA user_version = 2;",
        )
        .unwrap();
    }
    let store = db.open();
    assert_eq!(user_version(&db.raw()), SCHEMA_VERSION);
    let kept = store.commitments(&id).unwrap();
    assert_eq!(kept[0].text, "kept through the upgrade");
    assert_eq!(kept[0].recipient, None);
    assert_eq!(kept[0].looks_done, None);
    assert!(store.summary(&id).unwrap().unwrap().items.is_empty());
    let evidence = DoneEvidence {
        record: id.clone(),
        span: Span {
            channel: Channel::Mic,
            start_ms: 0,
            end_ms: 1,
        },
    };
    store
        .set_done_evidence(&commitment, Some(&evidence))
        .unwrap();
    assert_eq!(
        store.commitments(&id).unwrap()[0].looks_done,
        Some(evidence)
    );
}

/// Retention never sweeps an import, so records carry whether an import wrote them. A schema-3
/// library's records were all written before that was known: after the migration they read as
/// made here, not imported, whether the store or the build before it wrote them.
#[test]
fn a_database_from_before_imports_were_marked_is_brought_up_to_date() {
    let db = TempDb::new("migrate-v3");
    let id = {
        let store = db.open();
        let id = meeting(&store, 1);
        store
            .append_segments(&id, &[seg(Channel::Mic, 0, "kept through the upgrade")])
            .unwrap();
        id
    };
    // As a build of schema 3 left it, with a row that build wrote.
    {
        let raw = db.raw();
        raw.execute_batch(
            "ALTER TABLE record DROP COLUMN imported; PRAGMA user_version = 3;
             INSERT INTO record (id, kind, started_at_unix_ms) VALUES ('from-v3', 'dictation', 2);",
        )
        .unwrap();
    }
    let store = db.open();
    let raw = db.raw();
    assert_eq!(user_version(&raw), SCHEMA_VERSION);
    let column: (bool, String) = raw
        .query_row(
            "SELECT \"notnull\", dflt_value FROM pragma_table_info('record') WHERE name = 'imported'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(column, (true, "0".to_string()), "NOT NULL DEFAULT 0");
    let record = store.record(&id).unwrap().unwrap();
    assert!(!record.imported);
    assert_eq!(store.segments(&id).unwrap().len(), 1);
    let listed = store
        .records(&RecordQuery {
            kind: None,
            before: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|r| !r.imported), "{listed:?}");
}

#[test]
fn reopening_is_idempotent() {
    let db = TempDb::new("reopen");
    let id = {
        let store = db.open();
        let id = meeting(&store, 1);
        store
            .append_segments(&id, &[seg(Channel::Mic, 10, "kept across opens")])
            .unwrap();
        id
    };
    let schema = |conn: &rusqlite::Connection| -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT type || ' ' || name FROM sqlite_schema ORDER BY type, name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let before = schema(&db.raw());

    for _ in 0..2 {
        let store = db.open();
        assert_eq!(store.segments(&id).unwrap().len(), 1);
        assert_eq!(store.search("kept", 10).unwrap().len(), 1);
    }
    let raw = db.raw();
    assert_eq!(user_version(&raw), SCHEMA_VERSION);
    assert_eq!(schema(&raw), before);
    fts_is_consistent(&raw);
}

#[test]
fn a_database_from_a_newer_build_is_refused_and_left_alone() {
    let db = TempDb::new("newer");
    drop(db.open());
    db.raw()
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    match SqliteStore::open(db.path()) {
        Err(StoreError::Backend(msg)) => assert!(msg.contains("newer"), "{msg}"),
        Err(other) => panic!("expected Backend, got {other:?}"),
        Ok(_) => panic!("opened a database from a newer build"),
    }
    assert_eq!(user_version(&db.raw()), SCHEMA_VERSION + 1);
}

// --- The file ----------------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn database_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let side = |db: &TempDb, suffix: &str| {
        let mut p = db.path().into_os_string();
        p.push(suffix);
        std::path::PathBuf::from(p)
    };

    let db = TempDb::new("perms");
    let store = db.open();
    let id = meeting(&store, 1);
    store
        .append_segments(&id, &[seg(Channel::Mic, 0, "private words")])
        .unwrap();
    assert_eq!(mode(&db.path()), 0o600);
    for suffix in ["-wal", "-shm"] {
        let p = side(&db, suffix);
        assert!(p.exists(), "{suffix} exists while the store is open");
        assert_eq!(mode(&p), 0o600, "{suffix}");
    }
    drop(store);

    // A file an older build or another tool left readable is tightened on open.
    std::fs::set_permissions(db.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
    drop(db.open());
    assert_eq!(mode(&db.path()), 0o600);
}

#[test]
fn opening_in_a_missing_directory_fails_without_creating_it() {
    let db = TempDb::new("missing-dir");
    let path = db.dir().join("not-there").join("inkwell.sqlite");
    assert!(matches!(
        SqliteStore::open(&path),
        Err(StoreError::Backend(_))
    ));
    assert!(!db.dir().join("not-there").exists());
}

// --- Supersede ---------------------------------------------------------------------------------

#[test]
fn supersede_refuses_an_empty_result() {
    let store = SqliteStore::open_in_memory().unwrap();
    let id = meeting(&store, 1);
    store
        .append_segments(&id, &[seg(Channel::Mic, 0, "one two")])
        .unwrap();
    assert_eq!(store.supersede(&id, &[]), Err(StoreError::EmptySupersede));
    assert_eq!(
        store.supersede(&id, &[seg(Channel::Far, 0, " \t ")]),
        Err(StoreError::EmptySupersede)
    );
}

#[test]
fn supersede_refuses_a_channel_under_half() {
    let store = SqliteStore::open_in_memory().unwrap();
    let id = meeting(&store, 1);
    store
        .append_segments(
            &id,
            &[
                seg(Channel::Mic, 0, "a b c d e f g h i j"),
                seg(Channel::Far, 0, "k l m n"),
            ],
        )
        .unwrap();
    assert_eq!(
        store.supersede(
            &id,
            &[
                seg(Channel::Mic, 0, "a b c d"),
                seg(Channel::Far, 0, "k l m n o p q")
            ]
        ),
        Err(StoreError::SuspiciousSupersede {
            channel: Channel::Mic,
            previous_words: 10,
            new_words: 4
        })
    );
    // Exactly half is kept.
    assert_eq!(
        store.supersede(
            &id,
            &[
                seg(Channel::Mic, 0, "a b c d e"),
                seg(Channel::Far, 0, "k l")
            ]
        ),
        Ok(2)
    );
}

#[test]
fn a_refused_supersede_changes_nothing() {
    let db = TempDb::new("refused");
    let store = db.open();
    let id = meeting(&store, 1);
    store
        .append_segments(
            &id,
            &[
                seg(Channel::Mic, 1_200, "we ship on friday"),
                seg(Channel::Far, 3_400, "friday works for us"),
            ],
        )
        .unwrap();
    let before = dump(&db.raw());

    assert_eq!(store.supersede(&id, &[]), Err(StoreError::EmptySupersede));
    assert!(matches!(
        store.supersede(&id, &[seg(Channel::Far, 0, "friday works for us")]),
        Err(StoreError::SuspiciousSupersede {
            channel: Channel::Mic,
            ..
        })
    ));

    let raw = db.raw();
    assert_eq!(dump(&raw), before);
    fts_is_consistent(&raw);
    assert_eq!(store.record(&id).unwrap().unwrap().revision, 1);
    assert_eq!(store.search("ship", 10).unwrap().len(), 1);
}

#[test]
fn supersede_bumps_the_revision_and_replaces_every_row() {
    let db = TempDb::new("bump");
    let store = db.open();
    let id = meeting(&store, 1);
    let bystander = meeting(&store, 2);
    store
        .append_segments(&id, &[seg(Channel::Mic, 0, "rough live text")])
        .unwrap();
    store
        .append_segments(&bystander, &[seg(Channel::Mic, 0, "untouched")])
        .unwrap();

    assert_eq!(
        store.supersede(&id, &[seg(Channel::Mic, 0, "clean offline text")]),
        Ok(2)
    );
    assert_eq!(
        store.supersede(&id, &[seg(Channel::Mic, 0, "cleaner offline text")]),
        Ok(3)
    );
    assert_eq!(store.record(&id).unwrap().unwrap().revision, 3);
    assert_eq!(store.record(&bystander).unwrap().unwrap().revision, 1);

    let raw = db.raw();
    let revisions: Vec<i64> = raw
        .prepare("SELECT revision FROM segment WHERE record_id = ?1")
        .unwrap()
        .query_map([&id.0], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(revisions, [3], "old revisions are removed, not hidden");
    fts_is_consistent(&raw);
    assert!(store.search("rough", 10).unwrap().is_empty());
    assert_eq!(store.search("cleaner", 10).unwrap().len(), 1);

    // Live finals appended after a supersede join the current revision.
    store
        .append_segments(&id, &[seg(Channel::Far, 9_000, "late addition")])
        .unwrap();
    assert_eq!(store.segments(&id).unwrap().len(), 2);
}

// --- Live timestamps ---------------------------------------------------------------------------

#[test]
fn live_segments_keep_their_engine_timestamps() {
    let db = TempDb::new("timestamps");
    let store = db.open();
    let id = meeting(&store, 1);
    let live = [
        Segment {
            channel: Channel::Mic,
            start_ms: 12_345,
            end_ms: 13_900,
            text: "first final".into(),
            speaker: None,
        },
        Segment {
            channel: Channel::Far,
            start_ms: 3_600_000,
            end_ms: 3_604_250,
            text: "an hour in".into(),
            speaker: Some(SpeakerId("spk1".into())),
        },
    ];
    store.append_segments(&id, &live).unwrap();
    assert_eq!(store.segments(&id).unwrap(), live.to_vec());

    let stored: Vec<(i64, i64)> = db
        .raw()
        .prepare("SELECT start_ms, end_ms FROM segment ORDER BY seq")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(stored, [(12_345, 13_900), (3_600_000, 3_604_250)]);

    let hit = &store.search("hour", 10).unwrap()[0];
    assert_eq!(hit.start_ms, 3_600_000);
}

#[test]
fn the_schema_refuses_an_end_before_its_start() {
    let db = TempDb::new("reversed");
    let store = db.open();
    let id = meeting(&store, 1);
    let ids = store
        .add_commitments(
            &id,
            &[NewCommitment {
                recipient: None,
                text: "t".into(),
                owner: None,
                due: None,
                due_at_unix_ms: None,
                provenance: vec![],
            }],
        )
        .unwrap();
    let raw = db.raw();
    // Below the trait's own check: a row written by any other path is refused too.
    assert!(
        raw.execute(
            "INSERT INTO segment (record_id, revision, channel, start_ms, end_ms, text)
             VALUES (?1, 1, 'mic', 10, 9, 'x')",
            [&id.0],
        )
        .is_err()
    );
    assert!(
        raw.execute(
            "INSERT INTO commitment_span (commitment_id, ord, channel, start_ms, end_ms)
             VALUES (?1, 0, 'mic', 10, 9)",
            [&ids[0].0],
        )
        .is_err()
    );
    raw.execute(
        "INSERT INTO commitment_span (commitment_id, ord, channel, start_ms, end_ms)
         VALUES (?1, 0, 'mic', 10, 10)",
        [&ids[0].0],
    )
    .unwrap();
}

// --- Search ------------------------------------------------------------------------------------

#[test]
fn search_ranks_more_and_more_frequent_matches_first() {
    let store = SqliteStore::open_in_memory().unwrap();
    let add = |start: i64, text: &str| {
        let id = meeting(&store, start);
        store
            .append_segments(&id, &[seg(Channel::Mic, 0, text)])
            .unwrap();
        id
    };
    // Filler, so the query terms are rare enough for their weight to count.
    for n in 0..12 {
        add(
            1_000 + n,
            &format!("unrelated chatter about lunch number {n}"),
        );
    }
    let both_terms = add(10, "the budget forecast looks fine today");
    let one_term = add(20, "the budget looks fine today again");
    let repeated = add(30, "budget budget budget and more today");
    let once = add(40, "one budget line and more today");

    let order = |q: &str| -> Vec<RecordId> {
        store
            .search(q, 10)
            .unwrap()
            .into_iter()
            .map(|h| h.record)
            .collect()
    };
    let hits = order("budget forecast");
    assert_eq!(hits[0], both_terms, "more terms matched ranks first");
    assert!(hits.contains(&one_term), "any term matches");
    let pos = |id: &RecordId| hits.iter().position(|h| h == id).unwrap();
    assert!(pos(&both_terms) < pos(&one_term));

    let hits = order("budget");
    let pos = |id: &RecordId| hits.iter().position(|h| h == id).unwrap();
    assert!(
        pos(&repeated) < pos(&once),
        "more occurrences in a same-length segment rank first: {hits:?}"
    );
    assert_eq!(store.search("budget", 2).unwrap().len(), 2);
}

#[test]
fn search_hits_carry_the_record_title_and_start() {
    let store = SqliteStore::open_in_memory().unwrap();
    let id = meeting(&store, 1_700_000_000_000);
    store.set_title(&id, "Weekly planning").unwrap();
    store
        .append_segments(
            &id,
            &[
                seg(Channel::Mic, 0, "hello there"),
                seg(Channel::Far, 42_000, "the roadmap is ready"),
            ],
        )
        .unwrap();
    assert_eq!(
        store.search("roadmap", 10).unwrap(),
        vec![SearchHit {
            record: id,
            title: Some("Weekly planning".into()),
            started_at_unix_ms: 1_700_000_000_000,
            start_ms: 42_000,
            snippet: "the roadmap is ready".into(),
        }]
    );
}

#[test]
fn search_treats_query_syntax_as_plain_words() {
    let store = SqliteStore::open_in_memory().unwrap();
    let id = meeting(&store, 1);
    store
        .append_segments(
            &id,
            &[
                seg(Channel::Mic, 0, "budget and forecast"),
                seg(Channel::Far, 1_000, "near the e-mail thread"),
            ],
        )
        .unwrap();
    let count = |q: &str| -> usize {
        store
            .search(q, 10)
            .unwrap_or_else(|e| panic!("query {q:?} failed: {e}"))
            .len()
    };

    // Operators and punctuation are words or separators, never syntax: `-` does not negate,
    // `AND`/`NOT`/`NEAR` are words, quotes and brackets are dropped.
    assert_eq!(count("\"budget"), 1);
    assert_eq!(count("budget\""), 1);
    assert_eq!(count("\"budget forecast\""), 1);
    assert_eq!(count("budget*"), 1);
    assert_eq!(count("-budget"), 1);
    assert_eq!(count("budget -forecast"), 1);
    assert_eq!(count("(budget"), 1);
    assert_eq!(count("budget)"), 1);
    assert_eq!(count("(budget OR"), 1);
    assert_eq!(count("AND"), 1, "a plain word, found in the first segment");
    assert_eq!(
        count("NEAR"),
        1,
        "a plain word, found in the second segment"
    );
    assert_eq!(count("NOT budget"), 1);
    assert_eq!(count("^budget"), 1);
    assert_eq!(count("+budget"), 1);
    assert_eq!(count("{budget}"), 1);
    assert_eq!(count("bud\0get"), 1, "a control character separates words");
    // Punctuation inside a word keeps its parts together as a phrase.
    assert_eq!(count("e-mail"), 1);
    assert_eq!(
        count("text:budget"),
        0,
        "the phrase 'text budget', not a column filter"
    );
    assert_eq!(
        count("NEAR(budget, forecast)"),
        1,
        "only 'forecast' matches"
    );

    // Nothing searchable is an empty answer, not an error.
    for q in [
        "\"",
        "\"\"",
        "*",
        "-",
        "(",
        ")",
        "()",
        "^",
        ":",
        "\0",
        "* - ( ) \"",
        "NOT",
        "OR",
    ] {
        assert_eq!(count(q), 0, "{q:?}");
    }
}

#[test]
fn search_matches_word_prefixes_ignoring_case_and_diacritics() {
    let store = SqliteStore::open_in_memory().unwrap();
    let id = meeting(&store, 1);
    store
        .append_segments(
            &id,
            &[
                seg(Channel::Mic, 0, "Café meeting"),
                seg(Channel::Far, 1_000, "SHIPPING soon"),
            ],
        )
        .unwrap();
    assert_eq!(store.search("cafe", 10).unwrap().len(), 1);
    assert_eq!(store.search("CAFÉ", 10).unwrap().len(), 1);
    assert_eq!(store.search("ship", 10).unwrap().len(), 1);
    assert_eq!(
        store.search("hipping", 10).unwrap().len(),
        0,
        "prefixes, not infixes"
    );
}

// --- Delete ------------------------------------------------------------------------------------

#[test]
fn deleting_a_record_removes_everything_it_owns() {
    let db = TempDb::new("delete");
    let store = db.open();
    let doomed = meeting(&store, 1);
    let kept = meeting(&store, 2);
    for id in [&doomed, &kept] {
        store
            .append_segments(id, &[seg(Channel::Mic, 0, "shared words here")])
            .unwrap();
        store.add_note(id, 5, "a note").unwrap();
        store
            .save_summary(
                id,
                &Summary {
                    items: Vec::new(),
                    text: "sum".into(),
                    model: "m".into(),
                    created_at_unix_ms: 1,
                },
            )
            .unwrap();
        store
            .set_speaker_name(id, &SpeakerId("spk0".into()), "Guest")
            .unwrap();
        store
            .save_removed(id, &[seg(Channel::Mic, 0, "a removed line")])
            .unwrap();
    }
    let span = Span {
        channel: Channel::Mic,
        start_ms: 0,
        end_ms: 10,
    };
    let owe = |text: &str| NewCommitment {
        recipient: None,
        text: text.into(),
        owner: None,
        due: None,
        due_at_unix_ms: None,
        provenance: vec![span, span],
    };
    let doomed_c = store.add_commitments(&doomed, &[owe("original")]).unwrap();
    let kept_c = store.add_commitments(&kept, &[owe("repeat")]).unwrap();
    store.merge_commitment(&kept_c[0], &doomed_c[0]).unwrap();
    assert_eq!(store.open_commitments(10).unwrap().len(), 1);

    store.delete_record(&doomed).unwrap();

    let raw = db.raw();
    for (table, column) in [
        ("segment", "record_id"),
        ("removed_line", "record_id"),
        ("note", "record_id"),
        ("summary", "record_id"),
        ("speaker", "record_id"),
        ("commitment", "record_id"),
        ("record", "id"),
    ] {
        let count = |id: &RecordId| -> i64 {
            raw.query_row(
                &format!("SELECT count(*) FROM {table} WHERE {column} = ?1"),
                [&id.0],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(count(&doomed), 0, "{table} rows of the deleted record");
        assert_eq!(count(&kept), 1, "{table} rows of the other record");
    }
    let spans: i64 = raw
        .query_row("SELECT count(*) FROM commitment_span", [], |r| r.get(0))
        .unwrap();
    assert_eq!(spans, 2, "only the surviving commitment's spans remain");
    fts_is_consistent(&raw);
    assert_eq!(store.search("shared", 10).unwrap().len(), 1);

    // The commitment merged into a deleted one is still owed, so it comes back.
    let open = store.open_commitments(10).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, kept_c[0]);
    assert_eq!(open[0].merged_into, None);
}

// --- Deleted means deleted ---------------------------------------------------------------------
//
// Nothing here checkpoints for the store: text a call deletes or replaces must be gone from the
// database file **and** its write-ahead log when the call returns, by the store's own doing.

/// How often `marker` appears in the raw bytes of the database file and its WAL.
fn on_disk(db: &TempDb, marker: &str) -> usize {
    ["", "-wal"]
        .into_iter()
        .filter_map(|suffix| {
            let mut path = db.path().into_os_string();
            path.push(suffix);
            std::fs::read(path).ok()
        })
        .map(|bytes| {
            bytes
                .windows(marker.len())
                .filter(|w| *w == marker.as_bytes())
                .count()
        })
        .sum()
}

/// A record the deleted one shares pages with, so deletions happen inside live pages too.
fn neighbour(store: &SqliteStore) {
    let id = meeting(store, 5);
    let lines: Vec<Segment> = (0..40)
        .map(|n| {
            seg(
                Channel::Far,
                n * 1_000,
                &format!("ordinary words in line {n}"),
            )
        })
        .collect();
    store.append_segments(&id, &lines).unwrap();
    store.add_note(&id, 0, "a kept note").unwrap();
}

#[test]
fn a_deleted_record_leaves_no_text_on_disk() {
    let db = TempDb::new("scrub-delete");
    let store = db.open();
    neighbour(&store);
    let doomed = meeting(&store, 9);
    store.set_title(&doomed, "zqxtitlemarker").unwrap();
    store
        .append_segments(
            &doomed,
            &[seg(
                Channel::Mic,
                0,
                "the codename is zqxsegmentmarker today",
            )],
        )
        .unwrap();
    store.add_note(&doomed, 10, "zqxnotemarker").unwrap();
    store
        .save_removed(&doomed, &[seg(Channel::Mic, 20, "zqxremovedmarker")])
        .unwrap();
    store
        .save_summary(
            &doomed,
            &Summary {
                items: Vec::new(),
                text: "zqxsummarymarker".into(),
                model: "m".into(),
                created_at_unix_ms: 1,
            },
        )
        .unwrap();
    store
        .set_speaker_name(&doomed, &SpeakerId("spk0".into()), "zqxspeakermarker")
        .unwrap();
    store
        .add_commitments(
            &doomed,
            &[NewCommitment {
                recipient: None,
                text: "zqxcommitmentmarker".into(),
                owner: None,
                due: None,
                due_at_unix_ms: None,
                provenance: vec![],
            }],
        )
        .unwrap();
    let markers = [
        "zqxtitlemarker",
        "zqxsegmentmarker",
        "zqxnotemarker",
        "zqxremovedmarker",
        "zqxsummarymarker",
        "zqxspeakermarker",
        "zqxcommitmentmarker",
    ];
    // Written, and not checkpointed: in the log (or the file, if SQLite checkpointed on its own).
    for marker in markers {
        assert!(on_disk(&db, marker) >= 1, "{marker} is written before");
    }

    store.delete_record(&doomed).unwrap();
    assert!(!store.unscrubbed());
    for marker in markers {
        assert_eq!(on_disk(&db, marker), 0, "{marker} is overwritten");
    }
    assert_eq!(store.search("ordinary", 100).unwrap().len(), 40);
    fts_is_consistent(&db.raw());
}

#[test]
fn superseded_and_replaced_text_leaves_no_trace_on_disk() {
    let db = TempDb::new("scrub-supersede");
    let store = db.open();
    neighbour(&store);
    let id = meeting(&store, 9);
    let summary = |text: &str| Summary {
        items: Vec::new(),
        text: text.into(),
        model: "m".into(),
        created_at_unix_ms: 1,
    };
    // Each call that replaces or deletes user text, with the text it replaces.
    type Step = Box<dyn Fn(&SqliteStore, &RecordId)>;
    let steps: Vec<(&str, Step, Step)> = vec![
        (
            "zqxlivemarker",
            Box::new(|s, id| {
                s.append_segments(
                    id,
                    &[seg(Channel::Mic, 0, "alpha zqxlivemarker beta gamma")],
                )
                .unwrap()
            }),
            Box::new(|s, id| {
                s.supersede(id, &[seg(Channel::Mic, 0, "alpha beta gamma delta")])
                    .map(drop)
                    .unwrap()
            }),
        ),
        (
            "zqxoldtitlemarker",
            Box::new(|s, id| s.set_title(id, "zqxoldtitlemarker").unwrap()),
            Box::new(|s, id| s.set_title(id, "a new title").unwrap()),
        ),
        (
            "zqxoldsummarymarker",
            Box::new(move |s, id| s.save_summary(id, &summary("zqxoldsummarymarker")).unwrap()),
            Box::new(move |s, id| s.save_summary(id, &summary("new")).unwrap()),
        ),
        (
            "zqxoldspeakermarker",
            Box::new(|s, id| {
                s.set_speaker_name(id, &SpeakerId("spk0".into()), "zqxoldspeakermarker")
                    .unwrap()
            }),
            Box::new(|s, id| {
                s.set_speaker_name(id, &SpeakerId("spk0".into()), "Guest")
                    .unwrap()
            }),
        ),
        (
            "zqxoldremovedmarker",
            Box::new(|s, id| {
                s.save_removed(id, &[seg(Channel::Mic, 0, "zqxoldremovedmarker")])
                    .unwrap()
            }),
            Box::new(|s, id| {
                s.save_removed(id, &[seg(Channel::Mic, 0, "a later pass's line")])
                    .unwrap()
            }),
        ),
        (
            "zqxoldsettingmarker",
            Box::new(|s, _| s.set_setting("dictionary", "zqxoldsettingmarker").unwrap()),
            Box::new(|s, _| s.set_setting("dictionary", "new").unwrap()),
        ),
    ];
    for (marker, write, replace) in &steps {
        write(&store, &id);
        assert!(on_disk(&db, marker) >= 1, "{marker} is written before");
        replace(&store, &id);
        assert!(!store.unscrubbed());
        assert_eq!(on_disk(&db, marker), 0, "{marker} is overwritten");
    }
    // A note changed, then one deleted.
    let note = store.add_note(&id, 0, "zqxoldnotemarker").unwrap();
    store.update_note(&note, "new").unwrap();
    assert_eq!(on_disk(&db, "zqxoldnotemarker"), 0);
    let gone = store.add_note(&id, 5, "zqxdeletednotemarker").unwrap();
    assert!(on_disk(&db, "zqxdeletednotemarker") >= 1);
    store.delete_note(&gone).unwrap();
    assert_eq!(on_disk(&db, "zqxdeletednotemarker"), 0);

    assert_eq!(store.search("delta", 10).unwrap().len(), 1);
    fts_is_consistent(&db.raw());
}

/// Another process reading the database holds the log: the store tries a bounded number of
/// times, then says so (`unscrubbed`), and catches up on its next call once the reader is done.
#[test]
fn a_reader_that_holds_the_log_delays_the_scrub_and_it_is_reported() {
    let db = TempDb::new("scrub-busy");
    let store = db.open();
    neighbour(&store);
    let doomed = meeting(&store, 9);
    store
        .append_segments(&doomed, &[seg(Channel::Mic, 0, "zqxheldmarker")])
        .unwrap();

    let reader = db.raw();
    reader.execute_batch("BEGIN").unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM segment", [], |r| r.get(0))
        .unwrap();

    assert_eq!(store.scrub_change(), None, "nothing to report yet");
    store.delete_record(&doomed).unwrap();
    assert!(
        store.unscrubbed(),
        "the scrub could not finish, and says so"
    );
    assert!(
        on_disk(&db, "zqxheldmarker") >= 1,
        "still in the log while it is read"
    );
    // The change is reported once, however often it is asked.
    assert_eq!(store.scrub_change(), Some(true));
    assert_eq!(store.scrub_change(), None);

    reader.execute_batch("COMMIT").unwrap();
    drop(reader);
    // Any later call catches up.
    assert_eq!(store.search("ordinary", 100).unwrap().len(), 40);
    assert!(!store.unscrubbed());
    assert_eq!(on_disk(&db, "zqxheldmarker"), 0);
    assert_eq!(store.scrub_change(), Some(false), "the clear, once");
    assert_eq!(store.scrub_change(), None);
}

// --- Threads -----------------------------------------------------------------------------------

#[test]
fn one_store_serves_many_worker_threads() {
    let db = TempDb::new("threads");
    let store: Arc<dyn Store> = Arc::new(db.open());
    let id = meeting(store.as_ref(), 1);
    let handles: Vec<_> = (0..8u64)
        .map(|t| {
            let store = Arc::clone(&store);
            let id = id.clone();
            std::thread::spawn(move || {
                for n in 0..25u64 {
                    store
                        .append_segments(
                            &id,
                            &[seg(
                                Channel::Mic,
                                t * 1_000 + n,
                                &format!("thread {t} line {n}"),
                            )],
                        )
                        .unwrap();
                    store.search("line", 5).unwrap();
                    meeting(store.as_ref(), i64::try_from(t * 100 + n).unwrap() + 10);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(store.segments(&id).unwrap().len(), 200);
    assert_eq!(
        store
            .records(&RecordQuery {
                kind: None,
                before: None,
                limit: 1_000
            })
            .unwrap()
            .len(),
        201
    );
    fts_is_consistent(&db.raw());
}

/// `set_settings` is one transaction: when a later write fails, the earlier ones are rolled back
/// and neither key changes. The failure is SQLite's own: a trigger (created on a second
/// connection, so the store's code is untouched) that aborts any insert or update of key "b".
#[test]
fn set_settings_rolls_back_every_write_when_a_later_one_fails() {
    let db = TempDb::new("set-settings-rollback");
    let store = db.open();
    store.set_setting("a", "old").unwrap();
    store.set_setting("b", "kept").unwrap();
    db.raw()
        .execute_batch(
            "CREATE TRIGGER refuse_b_insert BEFORE INSERT ON setting WHEN NEW.key = 'b'
               BEGIN SELECT RAISE(ABORT, 'refused'); END;
             CREATE TRIGGER refuse_b_update BEFORE UPDATE ON setting WHEN NEW.key = 'b'
               BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
    // The trigger does refuse "b" on its own, so the pair below fails at its second write.
    assert!(store.set_setting("b", "changed").is_err());

    assert!(
        store
            .set_settings(&[("a", "new"), ("b", "changed")])
            .is_err(),
        "the second write fails"
    );
    assert_eq!(
        store.setting("a").unwrap().as_deref(),
        Some("old"),
        "the first write was rolled back"
    );
    assert_eq!(store.setting("b").unwrap().as_deref(), Some("kept"));

    // A key new to the store is not left behind either.
    assert!(
        store
            .set_settings(&[("c", "new"), ("b", "changed")])
            .is_err()
    );
    assert_eq!(store.setting("c").unwrap(), None);
}
