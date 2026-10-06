//! Review (S2.8): a looks-done suggestion whose evidence cannot be read still shows in Owed, and
//! the failed read is logged (it was dropped without a word). Its own test binary: the logger is
//! process-wide.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use ink_core::{Channel, DoneEvidence, NewCommitment, NewRecord, RecordKind, Segment, Span, Store};
use ink_ffi::logging::{self, Line};

#[test]
fn a_looks_done_line_that_cannot_be_read_is_logged_and_the_suggestion_still_shows() {
    logging::install(log::LevelFilter::Info, false).unwrap();
    let lines: Arc<Mutex<Vec<Line>>> = Arc::default();
    let sink = lines.clone();
    logging::set_sink(Some(Arc::new(move |l: &Line| {
        sink.lock().unwrap().push(l.clone())
    })));

    let store = FailingStore::new(Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()));
    let record = |title: &str| {
        store
            .create_record(NewRecord {
                kind: RecordKind::Meeting,
                title: Some(title.into()),
                started_at_unix_ms: 1_790_000_000_000,
                source_app: None,
                audio_dir: None,
            })
            .unwrap()
    };
    let promised = record("Earlier");
    let said = record("Later");
    let span = Span {
        channel: Channel::Mic,
        start_ms: 0,
        end_ms: 1_000,
    };
    store
        .append_segments(
            &said,
            &[Segment {
                channel: Channel::Mic,
                start_ms: 0,
                end_ms: 1_000,
                text: "I already sent the deck".into(),
                speaker: None,
            }],
        )
        .unwrap();
    let ids = store
        .add_commitments(
            &promised,
            &[NewCommitment {
                text: "Send the deck".into(),
                owner: None,
                recipient: None,
                due: None,
                due_at_unix_ms: None,
                provenance: vec![span],
            }],
        )
        .unwrap();
    store
        .set_done_evidence(
            &ids[0],
            Some(&DoneEvidence {
                record: said.clone(),
                span,
            }),
        )
        .unwrap();
    store.fail(&["segments"]);

    let dir = TempDir::new("owed-logging");
    let (core, events) = start_parts(ink_ffi::runtime::Parts {
        store: store.clone(),
        clock: clock(),
        registry: ink_engines::Registry::new(Vec::new()).unwrap(),
        models: ink_engines::ModelDir::new(dir.path().join("models")),
        loader: MockLoader::new(Behaviour::Say("x".into())),
        installer: Arc::new(MockInstaller {
            generation: Arc::default(),
            gate: None,
            installs: std::sync::atomic::AtomicUsize::new(0),
        }),
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        local: Default::default(),
        meetings: Default::default(),
    });
    core.command(r#"{"cmd":"commitments.list","id":"owed"}"#)
        .unwrap();
    let listed = events.wait_type("commitments.listed", Duration::from_secs(10));
    core.shutdown();
    logging::set_sink(None);

    let item = &listed["items"][0];
    assert_eq!(item["text"], "Send the deck");
    let looks_done = &item["looks_done"];
    assert_eq!(
        looks_done["record"],
        said.0.as_str(),
        "the suggestion still shows"
    );
    assert_eq!(looks_done["record_title"], "Later");
    assert!(
        looks_done.get("text").is_none(),
        "its line could not be read"
    );
    let lines = lines.lock().unwrap();
    let all: String = lines
        .iter()
        .map(|l| format!("{} {}\n", l.level, l.message))
        .collect();
    assert!(
        all.contains("warn owed: the line a looks-done suggestion cites could not be read"),
        "{all}"
    );
    assert!(
        !all.contains("already sent the deck"),
        "no words in the log:\n{all}"
    );
    events.assert_valid();
}
