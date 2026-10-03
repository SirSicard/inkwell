//! Inkwell 0.2's import from the screens (`import.check`, `import.run`), through the core, on
//! synthetic 0.2 data directories in temp dirs: never a real one. Every text in them is invented.
//! ink-store's own tests cover the importer on full 0.2 folders (the dictations among them); these
//! cover what the core adds: where it looks, the dry run, the keychain asked only by an import,
//! the marker, the answers' refs and the words of a failure.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use ink_core::{NewRecord, RecordKind, RecordQuery, Store};
use ink_engines::{ModelDir, Registry};
use ink_ffi::import02::Import02;
use ink_ffi::runtime::{Core, Parts};
use ink_llm::{ApiKey, KeyStore, KeyStoreError};
use ink_store::SqliteStore;
use ink_store::import::MARKER_KEY;
use serde_json::Value;

const WAIT: Duration = Duration::from_secs(10);

/// 0.2's settings with a key combination and separate presses: the import leaves a key note.
const SETTINGS_0_2: &str = r#"{"hotkey":"super+shift+space","recording_mode":"toggle","polish_enabled":true,"theme":"dark"}"#;
const SNIPPETS_0_2: &str =
    r#"{"snippets":[{"id":"s1","trigger":"my sig","expansion":"Best, A. Tester"}]}"#;
const DICTIONARY_0_2: &str =
    r#"{"entries":[{"find":"ink well","replace":"Inkwell"},{"find":"widgit","replace":"widget"}]}"#;

/// A keychain that holds a key for `present` and counts what it is asked.
struct Keys {
    present: &'static [&'static str],
    asked: AtomicUsize,
}

impl KeyStore for Keys {
    fn has_key(&self, provider: &str) -> Result<bool, KeyStoreError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Ok(self.present.contains(&provider))
    }

    fn read_key(&self, _: &str) -> Result<ApiKey, KeyStoreError> {
        panic!("an import never reads a key")
    }

    fn save_key(&self, _: &str, _: &str) -> Result<(), KeyStoreError> {
        panic!("an import never writes a key")
    }

    fn delete_key(&self, _: &str) -> Result<(), KeyStoreError> {
        panic!("an import never deletes a key")
    }
}

/// A synthetic 0.2 data directory holding `files`.
fn legacy(label: &str, files: &[(&str, &[u8])]) -> TempDir {
    let dir = TempDir::new(label);
    for (name, bytes) in files {
        std::fs::write(dir.path().join(name), bytes).unwrap();
    }
    dir
}

/// Every file in `dir` with its bytes: what must be the same after a check or an import.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

struct Rig {
    core: Core,
    events: Arc<Recorder>,
    store: Arc<SqliteStore>,
    keys: Arc<Keys>,
    next: Mutex<u32>,
    _dir: TempDir,
}

impl Rig {
    /// A core over an in-memory library, with 0.2's data at `source` (when given the import at
    /// all) and a keychain holding an OpenAI key.
    fn new(label: &str, source: Option<Option<PathBuf>>) -> Self {
        let dir = TempDir::new(label);
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let loader = MockLoader::new(Behaviour::Say("x".into()));
        let (core, events) = start_parts(Parts {
            store: store.clone(),
            clock: clock(),
            registry: Registry::new(Vec::new()).unwrap(),
            models: ModelDir::new(dir.path().join("models")),
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
        let keys = Arc::new(Keys {
            present: &["openai"],
            asked: AtomicUsize::new(0),
        });
        if let Some(source) = source {
            core.set_import02(Import02 {
                library: store.clone(),
                source,
                keys: Some(keys.clone()),
            });
        }
        Self {
            core,
            events,
            store,
            keys,
            next: Mutex::new(0),
            _dir: dir,
        }
    }

    /// Sends `cmd` with a fresh id and waits for the event that carries it (as `ref`, or as a
    /// `command.failed`'s `id`).
    fn ask(&self, cmd: &str) -> Value {
        let id = {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            format!("q{next}")
        };
        self.core
            .command(&format!(r#"{{"cmd":"{cmd}","id":"{id}"}}"#))
            .unwrap();
        let answer = self
            .events
            .wait_for(WAIT, |v| v["ref"] == id.as_str() || v["id"] == id.as_str())
            .unwrap_or_else(|| panic!("no answer to {cmd}: {:?}", self.events.types()));
        if answer["type"] == "command.failed" {
            assert_eq!(answer["command"], cmd);
        }
        answer
    }

    fn marker(&self) -> Option<String> {
        ink_core::Store::setting(self.store.as_ref(), MARKER_KEY).unwrap()
    }
}

#[test]
fn found_data_is_counted_without_writing_anything_or_asking_the_keychain() {
    let source = legacy(
        "check-found",
        &[
            ("settings.json", SETTINGS_0_2.as_bytes()),
            ("snippets.json", SNIPPETS_0_2.as_bytes()),
            ("dictionary.json", DICTIONARY_0_2.as_bytes()),
        ],
    );
    let before = snapshot(source.path());
    let rig = Rig::new("check-found", Some(Some(source.path().to_owned())));

    let checked = rig.ask("import.check");
    assert_eq!(checked["type"], "import.checked", "{checked}");
    assert_eq!(checked["state"], "found");
    let counts = &checked["counts"];
    assert_eq!(counts["dictations"], 0);
    assert_eq!(counts["snippets"], 1);
    assert_eq!(counts["dictionary_entries"], 2);
    assert_eq!(counts["settings"], 4);
    assert_eq!(counts["linked_keys"], 0, "the keychain is not asked");
    assert!(checked.get("message").is_none());

    assert_eq!(rig.keys.asked.load(Ordering::SeqCst), 0);
    assert_eq!(rig.marker(), None, "a check writes nothing");
    assert_eq!(snapshot(source.path()), before, "the source is untouched");
    rig.events.assert_valid();
    rig.core.shutdown();
}

#[test]
fn an_import_links_keys_brings_the_key_note_and_is_done_once() {
    let source = legacy(
        "run",
        &[
            ("settings.json", SETTINGS_0_2.as_bytes()),
            ("snippets.json", SNIPPETS_0_2.as_bytes()),
        ],
    );
    let before = snapshot(source.path());
    let rig = Rig::new("run", Some(Some(source.path().to_owned())));

    let finished = rig.ask("import.run");
    assert_eq!(finished["type"], "import.finished", "{finished}");
    assert_eq!(finished["counts"]["snippets"], 1);
    assert_eq!(finished["counts"]["settings"], 4);
    assert_eq!(finished["counts"]["linked_keys"], 1, "the OpenAI key");
    // Existence only, once per provider 0.2 knew.
    assert_eq!(
        rig.keys.asked.load(Ordering::SeqCst),
        ink_store::import::PROVIDERS.len()
    );
    assert!(rig.marker().is_some());
    assert_eq!(snapshot(source.path()), before, "the source is untouched");

    // It went into the library the core runs on: the screens read what it brought.
    let notes = rig.ask("import.notes");
    assert_eq!(notes["key"]["hotkey"], "super+shift+space", "{notes}");
    assert_eq!(notes["key"]["outcome"], "combination");
    assert_eq!(notes["key"]["toggle"], true);
    let snippets = rig.ask("snippets.list");
    assert_eq!(snippets["from_import"], true, "{snippets}");
    assert_eq!(snippets["snippets"][0]["trigger"], "my sig");

    // Once imported, a check says so without opening 0.2's data, and a second import is refused
    // before the keychain is asked again.
    std::fs::remove_file(source.path().join("snippets.json")).unwrap();
    let asked = rig.keys.asked.load(Ordering::SeqCst);
    assert_eq!(rig.ask("import.check")["state"], "imported");
    let again = rig.ask("import.run");
    assert_eq!(again["type"], "command.failed", "{again}");
    assert!(
        again["message"].as_str().unwrap().contains("already"),
        "{again}"
    );
    assert_eq!(rig.keys.asked.load(Ordering::SeqCst), asked);
    rig.events.assert_valid();
    rig.core.shutdown();
}

#[test]
fn no_data_is_absent_and_an_import_of_nothing_says_so() {
    let empty = TempDir::new("absent-empty");
    let gone = empty.path().join("not-there");
    for source in [None, Some(gone), Some(empty.path().to_owned())] {
        let rig = Rig::new("absent", Some(source));
        let checked = rig.ask("import.check");
        assert_eq!(checked["state"], "absent", "{checked}");
        assert!(checked.get("counts").is_none());
        let failed = rig.ask("import.run");
        assert_eq!(failed["type"], "command.failed", "{failed}");
        assert!(
            failed["message"]
                .as_str()
                .unwrap()
                .contains("no Inkwell 0.2 data"),
            "{failed}"
        );
        assert_eq!(rig.marker(), None);
        assert_eq!(rig.keys.asked.load(Ordering::SeqCst), 0);
        rig.events.assert_valid();
        rig.core.shutdown();
    }
}

/// 0.2 in the middle of a save (its rollback journal has content): the words say to quit it, and
/// nothing is written or asked.
#[test]
fn a_save_in_progress_is_unreadable_and_says_to_quit_0_2() {
    let source = legacy(
        "in-use",
        &[
            // Only the journal's presence is read before the refusal: these bytes are never
            // parsed as a database.
            (
                "transcripts.db",
                b"SQLite format 3\0 synthetic, never opened".as_slice(),
            ),
            ("transcripts.db-journal", b"a write in progress".as_slice()),
            ("snippets.json", SNIPPETS_0_2.as_bytes()),
        ],
    );
    let before = snapshot(source.path());
    let rig = Rig::new("in-use", Some(Some(source.path().to_owned())));

    let checked = rig.ask("import.check");
    assert_eq!(checked["state"], "unreadable", "{checked}");
    let message = checked["message"].as_str().unwrap();
    assert!(message.contains("Quit Inkwell 0.2"), "{message}");
    assert!(
        !message.contains(source.path().to_str().unwrap()),
        "no path"
    );
    let failed = rig.ask("import.run");
    assert_eq!(failed["type"], "command.failed", "{failed}");
    assert_eq!(failed["message"], message, "the same words");

    assert_eq!(rig.marker(), None);
    assert_eq!(
        rig.keys.asked.load(Ordering::SeqCst),
        0,
        "asked last, after the files"
    );
    assert_eq!(snapshot(source.path()), before);
    rig.events.assert_valid();
    rig.core.shutdown();
}

#[test]
fn a_file_0_2_could_not_have_written_is_named_and_nothing_is_imported() {
    let source = legacy(
        "malformed",
        &[
            ("snippets.json", SNIPPETS_0_2.as_bytes()),
            ("modes.json", b"{ this is not json".as_slice()),
        ],
    );
    let rig = Rig::new("malformed", Some(Some(source.path().to_owned())));
    let checked = rig.ask("import.check");
    assert_eq!(checked["state"], "unreadable", "{checked}");
    let message = checked["message"].as_str().unwrap();
    assert!(message.starts_with("modes.json"), "{checked}");
    assert_eq!(rig.ask("import.run")["type"], "command.failed");
    assert_eq!(rig.marker(), None);
    assert_eq!(rig.ask("snippets.list")["from_import"], false);
    rig.events.assert_valid();
    rig.core.shutdown();
}

/// A file in 0.2's folder that is a link (a synced dotfile, say): named, without advice the app
/// cannot follow, and nothing is imported. Unix only: making a link needs no rights there.
#[cfg(unix)]
#[test]
fn a_linked_file_is_named_and_nothing_is_imported() {
    let elsewhere = legacy(
        "linked-target",
        &[("settings.json", SETTINGS_0_2.as_bytes())],
    );
    let source = legacy("linked", &[("snippets.json", SNIPPETS_0_2.as_bytes())]);
    std::os::unix::fs::symlink(
        elsewhere.path().join("settings.json"),
        source.path().join("settings.json"),
    )
    .unwrap();
    let rig = Rig::new("linked", Some(Some(source.path().to_owned())));

    let checked = rig.ask("import.check");
    assert_eq!(checked["state"], "unreadable", "{checked}");
    let message = checked["message"].as_str().unwrap();
    assert_eq!(
        message,
        "settings.json is a link or a folder, not a file, and Inkwell imports only files"
    );
    let failed = rig.ask("import.run");
    assert_eq!(failed["type"], "command.failed", "{failed}");
    assert_eq!(failed["message"], message, "the same words");
    assert_eq!(rig.marker(), None);
    assert_eq!(rig.keys.asked.load(Ordering::SeqCst), 0);
    rig.events.assert_valid();
    rig.core.shutdown();
}

/// Midday UTC on 15 January 2026, Unix seconds.
const JAN_15_2026: i64 = 1_768_478_400;

/// `transcripts.db` as Inkwell 0.2 created it (its `history.rs`), with `texts` saved an hour
/// apart from [`JAN_15_2026`], in local time as 0.2 wrote them.
fn transcripts_0_2(dir: &Path, texts: &[&str]) {
    let conn = rusqlite::Connection::open(dir.join("transcripts.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS transcripts (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             text TEXT NOT NULL,
             raw_text TEXT NOT NULL,
             style TEXT NOT NULL DEFAULT 'formal',
             model TEXT NOT NULL DEFAULT 'unknown',
             audio_duration_ms INTEGER NOT NULL DEFAULT 0,
             created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
         );
         CREATE INDEX IF NOT EXISTS idx_created ON transcripts(created_at DESC);",
    )
    .unwrap();
    for (i, text) in (0_i64..).zip(texts) {
        conn.execute(
            "INSERT INTO transcripts (text, raw_text, audio_duration_ms, created_at)
             VALUES (?1, ?1, 2000, datetime(?2, 'unixepoch', 'localtime'))",
            rusqlite::params![text, JAN_15_2026 + 3_600 * i],
        )
        .unwrap();
    }
}

/// Retention never sweeps an import: it is deliberate, and the library may hold the only copy.
/// Three 0.2 dictations from January are imported, then the library is set to keep 30 days: the
/// sweep deletes a dictation made here that is as old, and keeps the three (0.2's marker refuses
/// a second import, so a swept import could never be brought back).
#[test]
fn imported_dictations_outlive_a_retention_change() {
    let source = legacy("retention", &[]);
    transcripts_0_2(
        source.path(),
        &[
            "An invented first note.",
            "An invented second note.",
            "An invented third note.",
        ],
    );
    let rig = Rig::new("retention", Some(Some(source.path().to_owned())));
    // Made here and as old as the imports: what the sweep must still delete.
    let started = JAN_15_2026 * 1_000;
    let made_here = rig
        .store
        .create_record(NewRecord {
            kind: RecordKind::Dictation,
            title: None,
            started_at_unix_ms: started,
            source_app: None,
            audio_dir: None,
        })
        .unwrap();
    rig.store
        .finish_record(&made_here, started + 2_000)
        .unwrap();

    let finished = rig.ask("import.run");
    assert_eq!(finished["type"], "import.finished", "{finished}");
    assert_eq!(finished["counts"]["dictations"], 3);
    rig.core
        .command(r#"{"cmd":"setting.set","key":"retention.days","value":"30"}"#)
        .unwrap();
    let swept = rig.events.wait_type("library.swept", WAIT);
    assert_eq!(swept["deleted"], 1, "only the dictation made here: {swept}");
    assert_eq!(swept["failed"], 0);
    assert_eq!(rig.store.record(&made_here).unwrap(), None);
    let kept = rig
        .store
        .records(&RecordQuery {
            kind: None,
            before: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(kept.len(), 3, "the imported dictations: {kept:?}");
    assert!(kept.iter().all(|r| r.kind == RecordKind::Dictation));
    for record in &kept {
        assert_eq!(rig.store.segments(&record.id).unwrap().len(), 1);
    }
    rig.events.assert_valid();
    rig.core.shutdown();
}

#[test]
fn without_the_import_the_commands_fail_and_a_path_is_never_taken() {
    let rig = Rig::new("not-given", None);
    let failed = rig.ask("import.check");
    assert_eq!(failed["type"], "command.failed", "{failed}");
    assert_eq!(rig.ask("import.run")["type"], "command.failed");
    // The shell cannot say where to look.
    assert!(
        rig.core
            .command(r#"{"cmd":"import.run","dir":"/tmp/elsewhere"}"#)
            .is_err()
    );
    rig.events.assert_valid();
    rig.core.shutdown();
}

/// What an import brings is history, not a milestone reached now: the next check notes it
/// silently, however many milestones the imported words pass at once.
#[test]
fn an_imports_milestones_are_noted_not_celebrated() {
    let source = legacy("milestones", &[]);
    let long = "word ".repeat(600);
    transcripts_0_2(source.path(), &[&long, &long]);
    let rig = Rig::new("milestones", Some(Some(source.path().to_owned())));
    let check = |id: &str| {
        rig.core
            .command(&format!(
                r#"{{"cmd":"milestones.check","utc_offsets":[{{"from_unix_ms":0,"minutes":0}}],"week_start":1,"id":"{id}"}}"#
            ))
            .unwrap();
        rig.events
            .wait_for(WAIT, |v| v["ref"] == id)
            .unwrap_or_else(|| panic!("no answer to {id}"))
    };
    // The library's first check, before the import: nothing reached.
    assert_eq!(check("before")["milestones"], serde_json::json!([]));
    let finished = rig.ask("import.run");
    assert_eq!(finished["type"], "import.finished", "{finished}");
    // 1,200 words came over: words_1000 is noted, not reported.
    let after = check("after");
    assert_eq!(after["type"], "milestones.reached");
    assert_eq!(after["milestones"], serde_json::json!([]));
    let noted = Store::setting(rig.store.as_ref(), ink_ffi::stats::MILESTONES_KEY)
        .unwrap()
        .unwrap();
    assert_eq!(noted, r#"["words_1000"]"#);
    rig.events.assert_valid();
    rig.core.shutdown();
}
