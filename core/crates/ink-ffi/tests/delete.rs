//! `record.delete`: the user deletes one record, whole, the way the retention sweep deletes one:
//! its transcript, notes, summary, commitments, speaker names and search entries through the
//! store's secure delete, then its audio. A record still being recorded or finished is refused,
//! as is one that is not there, and nothing else in the library is touched.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use common::*;
use ink_core::{
    Channel, Clock, NewCommitment, NewRecord, RecordId, RecordKind, Segment, Span, SpeakerId,
    Store, Summary,
};
use ink_engines::{ModelDir, Registry};
use ink_ffi::runtime::{Core, MeetingPlatform, Parts};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(10);
const MINUTE: i64 = 60_000;

struct Rig {
    core: Core,
    events: Arc<Recorder>,
    store: Arc<ink_store::SqliteStore>,
    dir: TempDir,
}

/// A core over a library in a file (so its words can be looked for on disk afterwards).
fn rig(label: &str) -> Rig {
    let dir = TempDir::new(label);
    let store = Arc::new(ink_store::SqliteStore::open(dir.path().join("library.sqlite")).unwrap());
    let parts = Parts {
        store: store.clone(),
        clock: clock(),
        registry: Registry::new(Vec::new()).unwrap(),
        models: ModelDir::new(dir.path().join("models")),
        loader: MockLoader::new(Behaviour::Say("x".into())),
        installer: Arc::new(MockInstaller {
            generation: Arc::default(),
            gate: None,
            installs: AtomicUsize::new(0),
        }),
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        local: Default::default(),
        meetings: MeetingPlatform::default(),
    };
    let (core, events) = start_parts(parts);
    Rig {
        core,
        events,
        store,
        dir,
    }
}

impl Rig {
    /// A record of `kind` with `word` in its title and transcript, ended unless `live`, with
    /// audio under `audio` when given.
    fn record(&self, kind: RecordKind, word: &str, live: bool, audio: Option<&str>) -> RecordId {
        let start = clock().unix_ms() - 60 * MINUTE;
        let id = self
            .store
            .create_record(NewRecord {
                kind,
                title: Some(format!("{word} title")),
                started_at_unix_ms: start,
                source_app: None,
                audio_dir: audio.map(Into::into),
            })
            .unwrap();
        self.store
            .append_segments(
                &id,
                &[
                    Segment {
                        channel: Channel::Mic,
                        start_ms: 0,
                        end_ms: 1_000,
                        text: format!("the {word} was said here"),
                        speaker: None,
                    },
                    Segment {
                        channel: Channel::Far,
                        start_ms: 1_000,
                        end_ms: 2_000,
                        text: format!("and {word} again"),
                        speaker: Some(SpeakerId("spk0".into())),
                    },
                ],
            )
            .unwrap();
        if !live {
            self.store.finish_record(&id, start + 30 * MINUTE).unwrap();
        }
        if let Some(audio) = audio {
            let dir = self.dir.path().join(audio);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("mic-000000-16000x1.pcm"), [0u8; 64]).unwrap();
        }
        id
    }

    /// Everything a record can hold besides its transcript, each with `word` in it.
    fn fill(&self, id: &RecordId, word: &str) {
        self.store
            .add_note(id, 500, &format!("{word} note"))
            .unwrap();
        self.store
            .save_summary(
                id,
                &Summary {
                    items: Vec::new(),
                    text: format!("{word} summary"),
                    model: "test".into(),
                    created_at_unix_ms: 1,
                },
            )
            .unwrap();
        self.store
            .add_commitments(
                id,
                &[NewCommitment {
                    recipient: None,
                    text: format!("send the {word} plan"),
                    owner: Some("You".into()),
                    due: None,
                    due_at_unix_ms: None,
                    provenance: vec![Span {
                        channel: Channel::Mic,
                        start_ms: 0,
                        end_ms: 1_000,
                    }],
                }],
            )
            .unwrap();
        self.store
            .set_speaker_name(id, &SpeakerId("spk0".into()), &format!("{word} Person"))
            .unwrap();
    }

    /// Sends `record.delete` for `record` with id `id`, and returns its answer.
    fn delete(&self, record: &str, id: &str) -> Value {
        self.core
            .command(&json!({"cmd": "record.delete", "record": record, "id": id}).to_string())
            .unwrap();
        self.events
            .wait_for(WAIT, |v| v["ref"] == id || v["id"] == id)
            .unwrap_or_else(|| panic!("no answer to {id}: {:?}", self.events.types()))
    }

    /// How many times `word` is in the library's files.
    fn on_disk(&self, word: &str) -> usize {
        ["library.sqlite", "library.sqlite-wal"]
            .iter()
            .filter_map(|f| std::fs::read(self.dir.path().join(f)).ok())
            .map(|bytes| {
                bytes
                    .windows(word.len())
                    .filter(|w| *w == word.as_bytes())
                    .count()
            })
            .sum()
    }
}

#[test]
fn a_deleted_record_goes_whole_with_its_audio_and_its_words_leave_the_files() {
    let r = rig("delete-whole");
    let doomed = r.record(
        RecordKind::Meeting,
        "zebrafinch",
        false,
        Some("meetings/doomed"),
    );
    r.fill(&doomed, "zebrafinch");
    let kept = r.record(
        RecordKind::Meeting,
        "marmosetfox",
        false,
        Some("meetings/kept"),
    );
    r.fill(&kept, "marmosetfox");
    assert!(r.on_disk("zebrafinch") > 0, "written before");

    let deleted = r.delete(&doomed.0, "del-1");
    assert_eq!(deleted["type"], "record.deleted", "{deleted}");
    assert_eq!(deleted["record"], doomed.0.as_str());
    assert_eq!(deleted["kind"], "meeting");
    assert_eq!(deleted["scrubbed"], true);
    assert_eq!(deleted["audio_left"], false);

    assert_eq!(r.store.record(&doomed).unwrap(), None);
    assert!(r.store.search("zebrafinch", 10).unwrap().is_empty());
    assert!(
        !r.dir.path().join("meetings/doomed").exists(),
        "its audio went too"
    );
    let owed = r.store.open_commitments(100).unwrap();
    assert_eq!(owed.len(), 1, "only the kept record's");
    assert_eq!(owed[0].record, kept);
    assert_eq!(
        r.on_disk("zebrafinch"),
        0,
        "no copy of its words in any file"
    );

    // Nothing else touched.
    assert!(r.store.record(&kept).unwrap().is_some());
    assert_eq!(r.store.segments(&kept).unwrap().len(), 2);
    assert_eq!(r.store.notes(&kept).unwrap().len(), 1);
    assert!(r.store.summary(&kept).unwrap().is_some());
    assert_eq!(r.store.speaker_names(&kept).unwrap().len(), 1);
    assert_eq!(r.store.search("marmosetfox", 10).unwrap().len(), 2);
    assert!(
        r.dir
            .path()
            .join("meetings/kept/mic-000000-16000x1.pcm")
            .is_file()
    );
    assert!(
        r.on_disk("marmosetfox") > 0,
        "a control: kept words are there"
    );

    // Any kind the user made or brought in: a dictation, an imported file, a record another app's
    // import wrote without an end.
    let dictation = r.record(RecordKind::Dictation, "okapiwren", false, None);
    let file = r.record(RecordKind::FileImport, "narwhalbee", false, None);
    let imported = r
        .store
        .import_records(
            "import.example-source",
            "{}",
            &[ink_store::import::RecordImport {
                record: NewRecord {
                    kind: RecordKind::Meeting,
                    title: None,
                    started_at_unix_ms: 1_000,
                    source_app: None,
                    audio_dir: None,
                },
                ended_at_unix_ms: None,
                revision: 2,
                segments: vec![Segment {
                    channel: Channel::Mic,
                    start_ms: 0,
                    end_ms: 1_000,
                    text: "an imported line".into(),
                    speaker: None,
                }],
                summary: None,
                speaker_names: vec![],
                commitments: vec![],
            }],
        )
        .unwrap()
        .remove(0);
    for (id, kind, n) in [
        (&dictation, "dictation", 2),
        (&file, "file_import", 3),
        (&imported, "meeting", 4),
    ] {
        let deleted = r.delete(&id.0, &format!("del-{n}"));
        assert_eq!(deleted["type"], "record.deleted", "{kind}: {deleted}");
        assert_eq!(deleted["kind"], kind);
        assert_eq!(r.store.record(id).unwrap(), None, "{kind}");
    }
    assert!(r.store.record(&kept).unwrap().is_some());
    r.core.shutdown();
    r.events.assert_valid();
}

#[test]
fn a_live_or_unfinished_or_unknown_record_is_refused_and_kept() {
    let r = rig("delete-refused");
    // Still being recorded: no end.
    let live = r.record(
        RecordKind::Meeting,
        "quokkabird",
        true,
        Some("meetings/live"),
    );
    let refused = r.delete(&live.0, "live");
    assert_eq!(refused["type"], "command.failed", "{refused}");
    assert_eq!(refused["command"], "record.delete");
    assert!(
        refused["message"].as_str().unwrap().contains(&live.0),
        "{refused}"
    );
    assert!(r.store.record(&live).unwrap().is_some());
    assert!(r.dir.path().join("meetings/live").exists());

    // An imported file still being written: no end yet.
    let importing = r.record(RecordKind::FileImport, "lemurgnat", true, None);
    assert_eq!(
        r.delete(&importing.0, "importing")["type"],
        "command.failed"
    );
    assert!(r.store.record(&importing).unwrap().is_some());

    // Ended, but its final pass has not finished: a crash-recovery marker beside its audio.
    let blotting = r.record(
        RecordKind::Meeting,
        "tapirmoth",
        false,
        Some("meetings/blot"),
    );
    std::fs::write(
        r.dir
            .path()
            .join("meetings/blot")
            .join(ink_ffi::recovery::LIVE_FILE),
        b"{}",
    )
    .unwrap();
    assert_eq!(r.delete(&blotting.0, "blot")["type"], "command.failed");
    assert!(r.store.record(&blotting).unwrap().is_some());

    // Ended, and this process is finishing it (its final pass holds it).
    let finishing = r.record(RecordKind::Meeting, "ibisvole", false, None);
    let shared = r.core.shared().clone();
    let hold = shared.hold_from_sweep(&finishing).unwrap();
    assert_eq!(r.delete(&finishing.0, "held")["type"], "command.failed");
    assert!(r.store.record(&finishing).unwrap().is_some());
    drop(hold);
    assert_eq!(
        r.delete(&finishing.0, "held-done")["type"],
        "record.deleted",
        "once its pass is over"
    );

    // Not there.
    let missing = r.delete("no-such-record", "missing");
    assert_eq!(missing["type"], "command.failed");
    assert!(
        missing["message"]
            .as_str()
            .unwrap()
            .contains("no-such-record")
    );

    // Its fields, read as every command's.
    for bad in [
        json!({"cmd": "record.delete"}),
        json!({"cmd": "record.delete", "record": 7}),
        json!({"cmd": "record.delete", "record": "r", "audio": false}),
    ] {
        let refused = r.core.command(&bad.to_string()).unwrap_err();
        assert!(refused.starts_with("record.delete: "), "{refused}");
    }
    let left = r
        .store
        .records(&ink_core::RecordQuery {
            kind: None,
            before: None,
            limit: 100,
        })
        .unwrap();
    assert_eq!(
        left.len(),
        3,
        "the live one, the import being written, the unfinished one"
    );
    r.core.shutdown();
    r.events.assert_valid();
}
