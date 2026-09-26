//! The Inkwell 0.2 importer, on synthetic 0.2 data directories built here from 0.2's own DDL and
//! file formats (`src-tauri/src/history.rs`, `dictionary.rs`, `snippets.rs`, `modes.rs`,
//! `settings.rs`). Every text in them is invented.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use common::TempDb;
use ink_core::*;
use ink_store::SqliteStore;
use ink_store::import::{
    Counts, DICTIONARY_KEY, ImportError, Inkwell02, KeyProbe, KeyProbeError, MARKER_KEY, MODES_KEY,
    NoKeychain, SETTINGS_PREFIX, SNIPPETS_KEY,
};
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------------------------
// A synthetic Inkwell 0.2 data directory
// ---------------------------------------------------------------------------------------------

/// `transcripts.db` exactly as 0.2's `TranscriptDb::open` creates it (history.rs).
const DDL_0_2: &str = "CREATE TABLE IF NOT EXISTS transcripts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                raw_text TEXT NOT NULL,
                style TEXT NOT NULL DEFAULT 'formal',
                model TEXT NOT NULL DEFAULT 'unknown',
                audio_duration_ms INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_created ON transcripts(created_at DESC);";

/// Midday UTC on 15 January 2026, far from any daylight-saving change in any zone, so the local
/// time 0.2 wrote converts back to exactly this instant wherever the test runs.
const T0: i64 = 1_768_478_400;

/// One 0.2 transcript row.
struct Row {
    text: &'static str,
    raw_text: &'static str,
    model: &'static str,
    duration_ms: i64,
    /// When 0.2 saved it, Unix seconds; stored as local time text, as 0.2 did.
    saved_at: i64,
}

fn rows() -> Vec<Row> {
    let texts = [
        "Ship the synthetic fixture by Friday.",
        "Remind me to water the test plants.",
        "The quarterly widget count is forty two.",
        "Draft a note about the imaginary offsite.",
        "Pick up the placeholder parcel at noon.",
    ];
    (0..texts.len())
        .map(|i| Row {
            text: texts[i],
            // Two rows were cleaned up (the raw text differs), three were pasted as heard.
            raw_text: if i % 2 == 0 {
                "um so ship the synthetic fixture uh by friday"
            } else {
                texts[i]
            },
            model: if i < 3 { "parakeet" } else { "qwen3" },
            duration_ms: 1_500 + 250 * i as i64,
            saved_at: T0 + 3_600 * i as i64,
        })
        .collect()
}

/// A 0.2 data directory under the temp dir, removed on drop.
struct Legacy {
    dir: PathBuf,
}

impl Legacy {
    fn empty(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ink-store-import-{}-{name}-{}",
            std::process::id(),
            unique()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    /// Every kind present: the five transcripts (one more written and deleted, as the history's
    /// delete button does, so ids have a gap), three dictionary entries, three snippets, two
    /// modes and a full settings file.
    fn full(name: &str) -> Self {
        let legacy = Self::empty(name);
        legacy.transcripts(&rows(), true);
        legacy.write_json("dictionary.json", &dictionary());
        legacy.write_json("snippets.json", &snippets());
        legacy.write_json("modes.json", &modes());
        legacy.write_json("settings.json", &settings());
        legacy
    }

    fn path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    fn transcripts(&self, rows: &[Row], with_deleted_row: bool) {
        let conn = rusqlite::Connection::open(self.path("transcripts.db")).unwrap();
        conn.execute_batch(DDL_0_2).unwrap();
        for (i, row) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO transcripts (text, raw_text, style, model, audio_duration_ms, created_at)
                 VALUES (?1, ?2, 'formal', ?3, ?4, datetime(?5, 'unixepoch', 'localtime'))",
                params![row.text, row.raw_text, row.model, row.duration_ms, row.saved_at],
            )
            .unwrap();
            if with_deleted_row && i == 1 {
                conn.execute(
                    "INSERT INTO transcripts (text, raw_text) VALUES ('deleted later', 'deleted later')",
                    [],
                )
                .unwrap();
                conn.execute("DELETE FROM transcripts WHERE text = 'deleted later'", [])
                    .unwrap();
            }
        }
    }

    fn write_json(&self, file: &str, value: &Value) {
        // 0.2 saved every file with `serde_json::to_string_pretty`.
        std::fs::write(
            self.path(file),
            serde_json::to_string_pretty(value).unwrap(),
        )
        .unwrap();
    }

    fn write(&self, file: &str, bytes: &[u8]) {
        std::fs::write(self.path(file), bytes).unwrap();
    }

    /// Every file in the directory by name, with its sha256.
    fn snapshot(&self) -> BTreeMap<String, String> {
        std::fs::read_dir(&self.dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let bytes = std::fs::read(entry.path()).unwrap();
                let digest: String = Sha256::digest(&bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                (entry.file_name().to_string_lossy().into_owned(), digest)
            })
            .collect()
    }

    fn read(&self) -> Result<Inkwell02, ImportError> {
        Inkwell02::read(&self.dir, &NoKeychain)
    }
}

impl Drop for Legacy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn unique() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn dictionary() -> Value {
    json!({ "entries": [
        { "find": "ink well", "replace": "Inkwell" },
        { "find": "widgit", "replace": "widget" },
        { "find": "exampel", "replace": "example" },
    ]})
}

fn snippets() -> Value {
    json!({ "snippets": [
        { "id": "s1", "trigger": "my sig", "expansion": "Best, A. Tester", "category": "email", "enabled": true },
        { "id": "s2", "trigger": "the addr", "expansion": "1 Example Road", "category": "", "enabled": false },
        // Written by an early build: `category` and `enabled` fall back to their serde defaults.
        { "id": "s3", "trigger": "today", "expansion": "{date}" },
    ]})
}

fn modes() -> Value {
    json!({
        "default_id": "default",
        "modes": [
            {
                "id": "default", "name": "Default", "style": "formal", "model": "",
                "polish_prompt": "", "polish_enabled": false, "apps": [], "remove_fillers": true
            },
            // Only the required fields: the rest take 0.2's serde defaults.
            { "id": "chat", "name": "Chat", "style": "casual", "apps": ["com.example.chat"] },
        ]
    })
}

/// A settings file as 0.2's `Settings::save` wrote it: every field.
fn settings() -> Value {
    json!({
        "style": "casual",
        "model": "parakeet",
        "hotkey": "super+shift+space",
        "recording_mode": "ptt",
        "start_on_boot": false,
        "show_overlay": true,
        "theme": "dark",
        "overlay_position": "top-center",
        "advanced_mode": true,
        "mic_device": "auto",
        "vad_threshold": 0.45,
        "polish_enabled": true,
        "polish_prompt": "Tidy this synthetic dictation.",
        "sound_dictation": true,
        "remove_fillers": true,
        "debug_save_audio": false,
        "edit_hotkey": "super+shift+e",
        "mic_idle_release_mins": 3,
        "append_space": true,
        "show_partials": false
    })
}

/// A keychain answering from a fixed list, and recording what it was asked.
struct Keys {
    present: &'static [&'static str],
    asked: Mutex<Vec<String>>,
}

impl Keys {
    fn with(present: &'static [&'static str]) -> Self {
        Self {
            present,
            asked: Mutex::new(Vec::new()),
        }
    }
}

impl KeyProbe for Keys {
    fn has_key(&self, account: &str) -> Result<bool, KeyProbeError> {
        self.asked.lock().unwrap().push(account.to_string());
        Ok(self.present.contains(&account))
    }
}

// ---------------------------------------------------------------------------------------------
// Looking at the 1.0 store independently of the importer
// ---------------------------------------------------------------------------------------------

fn all_records(store: &SqliteStore) -> Vec<Record> {
    store
        .records(&RecordQuery {
            kind: None,
            before: None,
            limit: usize::MAX,
        })
        .unwrap()
}

fn document(store: &SqliteStore, key: &str) -> Option<Value> {
    store
        .setting(key)
        .unwrap()
        .map(|text| serde_json::from_str(&text).unwrap())
}

fn setting_rows(db: &TempDb) -> BTreeMap<String, String> {
    let conn = db.raw();
    let mut stmt = conn.prepare("SELECT key, value FROM setting").unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Every row of every table the import writes, in a comparable form.
fn dump(db: &TempDb) -> Vec<String> {
    let conn = db.raw();
    let mut out = Vec::new();
    for sql in [
        "SELECT * FROM record ORDER BY id",
        "SELECT * FROM segment ORDER BY seq",
        "SELECT * FROM setting ORDER BY key",
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        let n = stmt.column_count();
        let mut q = stmt.query([]).unwrap();
        while let Some(row) = q.next().unwrap() {
            let cells: Vec<String> = (0..n)
                .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                .collect();
            out.push(cells.join("|"));
        }
    }
    out
}

/// Every byte the store has on disk: the database and its WAL.
fn store_bytes(db: &TempDb) -> Vec<u8> {
    let mut bytes = std::fs::read(db.path()).unwrap();
    let mut wal = db.path().into_os_string();
    wal.push("-wal");
    if let Ok(more) = std::fs::read(PathBuf::from(wal)) {
        bytes.extend(more);
    }
    bytes
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

// ---------------------------------------------------------------------------------------------
// Counts, content and the dry run
// ---------------------------------------------------------------------------------------------

#[test]
fn output_counts_equal_source_counts_per_kind() {
    let legacy = Legacy::full("counts");
    let keys = Keys::with(&["openai", "groq"]);
    let source = Inkwell02::read(&legacy.dir, &keys).unwrap();
    let expected = Counts {
        dictations: 5,
        dictionary_entries: 3,
        snippets: 3,
        modes: 2,
        settings: 20,
        linked_keys: 2,
    };
    assert_eq!(source.counts(), expected, "the dry run's counts");

    let db = TempDb::new("import-counts");
    let store = db.open();
    let written = store.import_inkwell02(&source).unwrap();
    assert_eq!(written, expected, "what the import says it wrote");

    // Counted again from the store, without the importer's help.
    let records = all_records(&store);
    assert_eq!(records.len(), 5);
    assert!(records.iter().all(|r| r.kind == RecordKind::Dictation));
    let entries = document(&store, DICTIONARY_KEY).unwrap();
    assert_eq!(entries.as_array().unwrap().len(), 3);
    let snippets = document(&store, SNIPPETS_KEY).unwrap();
    assert_eq!(snippets.as_array().unwrap().len(), 3);
    let modes = document(&store, MODES_KEY).unwrap();
    assert_eq!(modes["modes"].as_array().unwrap().len(), 2);
    let settings = setting_rows(&db)
        .into_keys()
        .filter(|k| k.starts_with(SETTINGS_PREFIX))
        .count();
    assert_eq!(settings, 20);
    let marker = document(&store, MARKER_KEY).unwrap();
    assert_eq!(marker["linked_keys"], json!(["openai", "groq"]));
}

#[test]
fn dictations_keep_their_text_times_and_order() {
    let legacy = Legacy::full("content");
    if std::env::var_os(TZ_CHILD).is_some() {
        // Run by `times_convert_from_local_time_in_a_zone_far_from_utc`: prove the zone took.
        let conn = rusqlite::Connection::open(legacy.path("transcripts.db")).unwrap();
        let first: String = conn
            .query_row("SELECT created_at FROM transcripts WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            first, "2026-01-15 17:30:00",
            "0.2 wrote local time, UTC+5:30"
        );
    }
    let source = legacy.read().unwrap();
    let db = TempDb::new("import-content");
    let store = db.open();
    store.import_inkwell02(&source).unwrap();

    // Newest first, as the Library lists them; the source's oldest row is last.
    let records = all_records(&store);
    let mut expected = rows();
    expected.reverse();
    assert_eq!(records.len(), expected.len());
    for (record, row) in records.iter().zip(&expected) {
        // 0.2 stamped a transcript when it was saved, just after the key came up: the record
        // ends then and starts the recording's length before.
        let saved_ms = row.saved_at * 1_000;
        assert_eq!(record.ended_at_unix_ms, Some(saved_ms));
        assert_eq!(record.started_at_unix_ms, saved_ms - row.duration_ms);
        assert_eq!(record.revision, 1);
        assert_eq!(record.title, None);
        assert_eq!(record.source_app, None);
        assert_eq!(record.audio_dir, None, "0.2 kept no audio");

        let segments = store.segments(&record.id).unwrap();
        assert_eq!(
            segments,
            vec![Segment {
                channel: Channel::Mic,
                start_ms: 0,
                end_ms: row.duration_ms as u64,
                text: row.text.to_string(),
                speaker: None,
            }],
            "the text that was pasted, as one mic segment"
        );
    }
}

const TZ_CHILD: &str = "INK_STORE_IMPORT_TZ_CHILD";

/// The time test again, in a child process set to a zone far from UTC (+5:30, no daylight
/// saving). On a machine set to UTC, as CI runners are, local time and UTC agree and a missing
/// conversion would pass unseen.
#[cfg(unix)]
#[test]
fn times_convert_from_local_time_in_a_zone_far_from_utc() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "dictations_keep_their_text_times_and_order"])
        .env("TZ", "Asia/Kolkata")
        .env(TZ_CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("1 passed"), "{stdout}");
}

#[test]
fn imported_dictations_are_searchable() {
    let legacy = Legacy::full("search");
    let db = TempDb::new("import-search");
    let store = db.open();
    store.import_inkwell02(&legacy.read().unwrap()).unwrap();
    let hits = store.search("widget", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].snippet, "The quarterly widget count is forty two.");
}

#[test]
fn settings_documents_carry_0_2_values_with_its_defaults_filled_in() {
    let legacy = Legacy::full("documents");
    let db = TempDb::new("import-documents");
    let store = db.open();
    store.import_inkwell02(&legacy.read().unwrap()).unwrap();

    assert_eq!(
        document(&store, DICTIONARY_KEY).unwrap(),
        dictionary()["entries"]
    );
    let snippets = document(&store, SNIPPETS_KEY).unwrap();
    assert_eq!(
        snippets[2],
        json!({ "id": "s3", "trigger": "today", "expansion": "{date}", "category": "", "enabled": true })
    );
    let modes = document(&store, MODES_KEY).unwrap();
    assert_eq!(modes["default_id"], "default");
    assert_eq!(
        modes["modes"][1],
        json!({
            "id": "chat", "name": "Chat", "style": "casual", "model": "", "polish_prompt": "",
            "polish_enabled": false, "apps": ["com.example.chat"], "remove_fillers": true
        })
    );
    // Each settings field is its own row holding the JSON value.
    let rows = setting_rows(&db);
    assert_eq!(rows[&format!("{SETTINGS_PREFIX}theme")], "\"dark\"");
    assert_eq!(rows[&format!("{SETTINGS_PREFIX}advanced_mode")], "true");
    assert_eq!(
        rows[&format!("{SETTINGS_PREFIX}mic_idle_release_mins")],
        "3"
    );
    assert_eq!(rows[&format!("{SETTINGS_PREFIX}vad_threshold")], "0.45");
}

#[test]
fn a_dry_run_reads_everything_and_writes_nothing() {
    let legacy = Legacy::full("dry-run");
    let before = legacy.snapshot();
    let source = legacy.read().unwrap();
    assert_eq!(source.counts().dictations, 5);
    // The dry run is `read` plus `counts`: it is never handed a store, so it cannot write one,
    // and it leaves the source directory as it found it.
    assert_eq!(legacy.snapshot(), before);
    let report = source.report();
    assert_eq!(
        report.raw_text_differs, 3,
        "rows whose raw text is not carried over"
    );
    assert_eq!(report.keys_unchecked, 5, "no keychain was asked");
}

#[test]
fn missing_optional_files_import_as_nothing() {
    let legacy = Legacy::empty("only-db");
    legacy.transcripts(&rows()[..2], false);
    let source = legacy.read().unwrap();
    assert_eq!(
        source.counts(),
        Counts {
            dictations: 2,
            ..Counts::default()
        }
    );
    let db = TempDb::new("import-only-db");
    let store = db.open();
    store.import_inkwell02(&source).unwrap();
    assert_eq!(all_records(&store).len(), 2);
    let rows = setting_rows(&db);
    assert_eq!(rows.keys().collect::<Vec<_>>(), vec![MARKER_KEY]);

    let settings_only = Legacy::empty("only-settings");
    settings_only.write_json("settings.json", &json!({ "theme": "light" }));
    assert_eq!(
        settings_only.read().unwrap().counts(),
        Counts {
            settings: 1,
            ..Counts::default()
        }
    );
}

#[test]
fn a_directory_without_0_2_data_is_refused() {
    let legacy = Legacy::empty("nothing");
    legacy.write("notes.txt", b"not an Inkwell file");
    assert!(matches!(legacy.read(), Err(ImportError::NoSource)));
    let gone = legacy.dir.join("no-such-dir");
    assert!(matches!(
        Inkwell02::read(&gone, &NoKeychain),
        Err(ImportError::NoSource)
    ));
}

// ---------------------------------------------------------------------------------------------
// The sources are read-only
// ---------------------------------------------------------------------------------------------

#[test]
fn sources_are_byte_identical_after_the_import() {
    let legacy = Legacy::full("identical");
    let before = legacy.snapshot();
    assert!(before.contains_key("transcripts.db"));
    let db = TempDb::new("import-identical");
    let store = db.open();
    store.import_inkwell02(&legacy.read().unwrap()).unwrap();
    drop(store);
    // Same files, same bytes: no journal, WAL or shared-memory file appeared beside the
    // database, and nothing was rewritten.
    assert_eq!(legacy.snapshot(), before);
}

#[test]
fn a_second_import_is_refused_and_changes_nothing() {
    let legacy = Legacy::full("twice");
    let db = TempDb::new("import-twice");
    let store = db.open();
    let source = legacy.read().unwrap();
    store.import_inkwell02(&source).unwrap();
    let after_first = dump(&db);
    assert!(matches!(
        store.import_inkwell02(&source),
        Err(ImportError::AlreadyImported)
    ));
    // A fresh read of the same directory is refused just the same.
    assert!(matches!(
        store.import_inkwell02(&legacy.read().unwrap()),
        Err(ImportError::AlreadyImported)
    ));
    assert_eq!(dump(&db), after_first);
    assert_eq!(all_records(&store).len(), 5);
}

#[test]
fn an_import_into_a_store_in_use_keeps_what_was_there() {
    let legacy = Legacy::full("in-use");
    let db = TempDb::new("import-in-use");
    let store = db.open();
    let own = store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: Some("A 1.0 meeting".into()),
            started_at_unix_ms: T0 * 1_000,
            source_app: None,
            audio_dir: None,
        })
        .unwrap();
    store.set_setting("ui.theme", "light").unwrap();
    store.import_inkwell02(&legacy.read().unwrap()).unwrap();
    assert_eq!(all_records(&store).len(), 6);
    assert!(store.record(&own).unwrap().is_some());
    assert_eq!(store.setting("ui.theme").unwrap().as_deref(), Some("light"));
}

// ---------------------------------------------------------------------------------------------
// Malformed and partial sources: a clear error, and the store unchanged
// ---------------------------------------------------------------------------------------------

/// Imports a full fixture after `spoil` has damaged it; the import must fail before or during
/// the write and leave the store exactly as it was (it already holds one 1.0 record).
fn refused_after(name: &str, spoil: impl FnOnce(&Legacy)) -> ImportError {
    let legacy = Legacy::full(name);
    spoil(&legacy);
    let before = legacy.snapshot();
    let db = TempDb::new(&format!("import-{name}"));
    let store = db.open();
    store
        .create_record(NewRecord {
            kind: RecordKind::Dictation,
            title: None,
            started_at_unix_ms: 1,
            source_app: None,
            audio_dir: None,
        })
        .unwrap();
    let untouched = dump(&db);
    let err = match legacy.read() {
        Err(e) => e,
        Ok(source) => store
            .import_inkwell02(&source)
            .expect_err("a spoiled source must not import"),
    };
    assert_eq!(dump(&db), untouched, "{name}: the store changed");
    assert_eq!(legacy.snapshot(), before, "{name}: the source changed");
    // Errors name the file and what is wrong, never what the user said or typed.
    let message = err.to_string();
    for canary in ["synthetic", "widgit", "Tester", "Example Road", "canary"] {
        assert!(!message.contains(canary), "{name}: {message}");
    }
    err
}

fn malformed_file(err: &ImportError) -> &'static str {
    match err {
        ImportError::Malformed { file, .. } => file,
        other => panic!("expected Malformed, got {other:?}"),
    }
}

#[test]
fn a_json_file_that_does_not_parse_is_refused() {
    let err = refused_after("bad-json", |l| {
        l.write(
            "dictionary.json",
            b"{ \"entries\": [ { \"find\": \"canary\", ",
        );
    });
    assert_eq!(malformed_file(&err), "dictionary.json");
}

#[test]
fn a_json_file_with_the_wrong_shape_is_refused() {
    let err = refused_after("snippet-shape", |l| {
        l.write_json(
            "snippets.json",
            &json!({ "snippets": [ { "id": "x", "expansion": "canary" } ] }),
        );
    });
    assert_eq!(malformed_file(&err), "snippets.json");

    let err = refused_after("mode-shape", |l| {
        l.write_json(
            "modes.json",
            &json!({ "default_id": "d", "modes": [ { "id": "d", "name": "D", "style": "formal", "apps": [7] } ] }),
        );
    });
    assert_eq!(malformed_file(&err), "modes.json");

    let err = refused_after("dictionary-shape", |l| {
        l.write_json("dictionary.json", &json!([{ "find": "canary" }]));
    });
    assert_eq!(malformed_file(&err), "dictionary.json");
}

#[test]
fn a_settings_field_of_the_wrong_type_is_refused() {
    // 0.2 would have thrown the whole file away and run on defaults, so importing the rest of it
    // would import settings the user never had.
    let err = refused_after("settings-type", |l| {
        let mut s = settings();
        s["show_overlay"] = json!("canary");
        l.write_json("settings.json", &s);
    });
    assert_eq!(malformed_file(&err), "settings.json");
    assert!(err.to_string().contains("show_overlay"), "{err}");
}

#[test]
fn a_truncated_transcripts_db_is_refused() {
    let err = refused_after("truncated", |l| {
        let path = l.path("transcripts.db");
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    });
    assert_eq!(malformed_file(&err), "transcripts.db");
}

#[test]
fn a_damaged_index_fails_the_integrity_check() {
    // The import reads the table in id order and never touches this index, so only the
    // integrity check sees the damage: a damaged file is refused whole, not half read.
    let err = refused_after("bad-index", |l| {
        let path = l.path("transcripts.db");
        let (page_size, root): (u32, u32) = {
            let conn = rusqlite::Connection::open(&path).unwrap();
            (
                conn.query_row("PRAGMA page_size", [], |r| r.get(0))
                    .unwrap(),
                conn.query_row(
                    "SELECT rootpage FROM sqlite_master WHERE name = 'idx_created'",
                    [],
                    |r| r.get(0),
                )
                .unwrap(),
            )
        };
        let mut bytes = std::fs::read(&path).unwrap();
        let (page_size, root) = (page_size as usize, root as usize);
        let page = page_size * (root - 1)..page_size * root;
        bytes[page].fill(0xA5);
        std::fs::write(&path, bytes).unwrap();
    });
    assert_eq!(malformed_file(&err), "transcripts.db");
}

#[test]
fn a_file_that_is_not_a_database_is_refused() {
    let err = refused_after("not-a-db", |l| {
        l.write("transcripts.db", b"canary text, not SQLite");
    });
    assert_eq!(malformed_file(&err), "transcripts.db");
}

#[test]
fn a_database_without_the_transcripts_table_is_refused() {
    let err = refused_after("no-table", |l| {
        std::fs::remove_file(l.path("transcripts.db")).unwrap();
        let conn = rusqlite::Connection::open(l.path("transcripts.db")).unwrap();
        conn.execute_batch("CREATE TABLE other (x TEXT)").unwrap();
    });
    assert_eq!(malformed_file(&err), "transcripts.db");
}

#[test]
fn a_row_that_0_2_could_not_have_written_is_refused() {
    for (name, sql) in [
        (
            "bad-date",
            "UPDATE transcripts SET created_at = 'canary' WHERE id = 4",
        ),
        // SQLite would read a bare number as a Julian day and import a date nobody saved.
        (
            "numeric-date",
            "UPDATE transcripts SET created_at = '2461055.5' WHERE id = 4",
        ),
        (
            "negative-duration",
            "UPDATE transcripts SET audio_duration_ms = -5 WHERE id = 4",
        ),
        (
            "blob-text",
            "UPDATE transcripts SET text = x'00ff' WHERE id = 4",
        ),
    ] {
        let err = refused_after(name, |l| {
            let conn = rusqlite::Connection::open(l.path("transcripts.db")).unwrap();
            // Ids 1, 2, 4, 5 and 6: 3 was deleted.
            assert_eq!(conn.execute(sql, []).unwrap(), 1);
        });
        assert_eq!(malformed_file(&err), "transcripts.db", "{name}");
        assert!(err.to_string().contains("row 4"), "{name}: {err}");
    }
}

#[test]
fn a_leftover_rollback_journal_is_refused() {
    // A journal beside the database means 0.2 is writing, or crashed mid-write. Opening would
    // have to roll it back, which a read-only import must not do.
    let err = refused_after("hot-journal", |l| {
        l.write("transcripts.db-journal", b"canary journal bytes");
    });
    assert!(
        matches!(
            err,
            ImportError::InUse {
                file: "transcripts.db"
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_wal_mode_database_is_refused_without_touching_it() {
    // 0.2 never used WAL. Reading one read-only would either miss what is still in its log
    // (`immutable=1`) or create and write shared-memory files beside it, so it is refused.
    let err = refused_after("wal", |l| {
        let conn = rusqlite::Connection::open(l.path("transcripts.db")).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    });
    assert_eq!(malformed_file(&err), "transcripts.db");
    assert!(err.to_string().contains("WAL mode"), "{err}");

    let err = refused_after("live-wal", |l| {
        l.write("transcripts.db-wal", b"canary wal bytes");
    });
    assert!(
        matches!(
            err,
            ImportError::InUse {
                file: "transcripts.db"
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_failure_midway_through_the_write_rolls_back_every_row() {
    let legacy = Legacy::full("midway");
    let db = TempDb::new("import-midway");
    let store = db.open();
    let untouched = dump(&db);
    // A trigger that fails the third segment: the first two records are already written when
    // it fires.
    db.raw()
        .execute_batch(
            "CREATE TRIGGER fail_third AFTER INSERT ON segment
             WHEN (SELECT count(*) FROM segment) >= 3
             BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
        )
        .unwrap();
    let err = store.import_inkwell02(&legacy.read().unwrap()).unwrap_err();
    assert!(
        matches!(err, ImportError::Store(StoreError::Backend(_))),
        "{err:?}"
    );
    assert_eq!(dump(&db), untouched, "one transaction: nothing stays");
    // With the fault gone, the same store takes the import: the refusal left no marker.
    db.raw().execute_batch("DROP TRIGGER fail_third").unwrap();
    store.import_inkwell02(&legacy.read().unwrap()).unwrap();
    assert_eq!(all_records(&store).len(), 5);
}

// ---------------------------------------------------------------------------------------------
// Secrets and keychain references
// ---------------------------------------------------------------------------------------------

#[test]
fn plaintext_keys_in_settings_json_are_never_imported() {
    // Builds before 0.2's secret stripper wrote keys into settings.json; a copy of the folder
    // taken from such an install still has them.
    let legacy = Legacy::full("plaintext");
    let mut s = settings();
    s["groq_key"] = json!("sk-canary-groq");
    s["api_key"] = json!("sk-canary-api");
    s["agent_token"] = json!("tok-canary");
    s["configured_providers"] = json!(["groq"]);
    legacy.write_json("settings.json", &s);

    let source = legacy.read().unwrap();
    assert_eq!(source.counts().settings, 20, "only 0.2's own fields");
    let report = source.report();
    assert_eq!(
        report.secret_fields_skipped,
        vec!["agent_token", "api_key", "groq_key"]
    );
    assert_eq!(report.unknown_fields_skipped, 1);

    let db = TempDb::new("import-plaintext");
    let store = db.open();
    store.import_inkwell02(&source).unwrap();
    drop(store);
    let bytes = store_bytes(&db);
    for canary in ["sk-canary-groq", "sk-canary-api", "tok-canary"] {
        assert!(!contains(&bytes, canary), "{canary} reached the store");
    }
    assert!(
        !setting_rows(&db)
            .keys()
            .any(|k| k.ends_with("_key") || k.ends_with("token"))
    );
}

#[test]
fn the_keychain_is_asked_about_every_0_2_provider_and_nothing_else() {
    let legacy = Legacy::full("asked");
    let keys = Keys::with(&["anthropic"]);
    let source = Inkwell02::read(&legacy.dir, &keys).unwrap();
    assert_eq!(
        *keys.asked.lock().unwrap(),
        vec!["openai", "groq", "anthropic", "openrouter", "custom"]
    );
    assert_eq!(source.counts().linked_keys, 1);
    assert_eq!(source.report().keys_unchecked, 0);
}

#[test]
fn a_keychain_that_cannot_answer_leaves_keys_unchecked_not_the_import_failed() {
    let legacy = Legacy::full("locked");
    let source = Inkwell02::read(&legacy.dir, &NoKeychain).unwrap();
    assert_eq!(source.counts().linked_keys, 0);
    assert_eq!(source.report().keys_unchecked, 5);
    let db = TempDb::new("import-locked");
    let store = db.open();
    store.import_inkwell02(&source).unwrap();
    let marker = document(&store, MARKER_KEY).unwrap();
    assert_eq!(marker["linked_keys"], json!([]));
    assert_eq!(marker["unchecked_keys"], json!(5));
}

/// The re-link, end to end: a key stored the way 0.2 stored it is found by ink-llm's own lookup,
/// with nothing written anywhere by the import but the provider's name.
mod relink {
    use super::*;
    use ink_llm::keys::ExistenceCheck;
    use ink_llm::provider::{Provider, configured_providers};
    use ink_llm::{KeyStore, OsKeyStore};

    /// The importer's view of ink-llm's key store: existence only.
    struct LlmKeys<'a>(&'a OsKeyStore);

    impl KeyProbe for LlmKeys<'_> {
        fn has_key(&self, account: &str) -> Result<bool, KeyProbeError> {
            self.0.has_key(account).map_err(|_| KeyProbeError)
        }
    }

    #[test]
    fn the_legacy_keychain_reference_is_the_one_ink_llm_looks_up() {
        assert_eq!(ink_store::import::KEYRING_SERVICE, ink_llm::KEYRING_SERVICE);
        let ids: Vec<&str> = Provider::ALL.iter().map(|p| p.id()).collect();
        assert_eq!(ink_store::import::PROVIDERS.to_vec(), ids);
    }

    #[test]
    fn a_0_2_key_is_linked_by_reference_and_never_copied() {
        let mock: std::sync::Arc<keyring_core::CredentialStore> =
            keyring_core::mock::Store::new().unwrap();
        // What 0.2's `save_api_key` did: keyring `Entry::new("inkwell", provider)`.
        mock.build("inkwell", "groq", None)
            .unwrap()
            .set_password("sk-canary-secret-groq")
            .unwrap();
        let keys = OsKeyStore::with_store(mock, ExistenceCheck::Attributes);

        let legacy = Legacy::full("relink");
        let source = Inkwell02::read(&legacy.dir, &LlmKeys(&keys)).unwrap();
        assert_eq!(source.counts().linked_keys, 1);
        let db = TempDb::new("import-relink");
        let store = db.open();
        store.import_inkwell02(&source).unwrap();

        assert_eq!(configured_providers(&keys), vec![Provider::Groq]);
        assert_eq!(
            keys.read_key("groq").unwrap().expose(),
            "sk-canary-secret-groq"
        );
        drop(store);
        assert!(!contains(&store_bytes(&db), "sk-canary-secret"));
    }
}

#[test]
fn errors_name_files_not_paths() {
    let legacy = Legacy::full("paths");
    legacy.write("modes.json", b"[");
    let message = legacy.read().unwrap_err().to_string();
    assert!(message.starts_with("modes.json"), "{message}");
    let dir = legacy.dir.to_string_lossy().into_owned();
    assert!(!message.contains(&dir), "{message}");
    assert!(!message.contains(Path::new(&dir).file_name().unwrap().to_str().unwrap()));
}
