//! I5 and carry-over 6, as behaviour: a dictation whose words are a known secret is run down
//! every path that logs or reports something, and neither the log nor any event, error or
//! warning (as the shell would print it) contains a word of it.

mod common;

use std::sync::{Arc, Mutex, OnceLock};

use common::{Rig, speech_48k};
use ink_core::mock::MemStore;
use ink_core::{
    CancelToken, Commitment, CommitmentId, Endpoint, Llm, LlmError, LlmInfo, LlmRequest,
    LlmResponse, NewCommitment, NewRecord, Note, NoteId, Permission, PermissionState, Record,
    RecordId, RecordQuery, SearchHit, Segment, SpeakerId, Store, StoreError, Summary,
};
use ink_pipeline::events::{DictationEvent, TakeFailure, Warning};

const SECRET: &str = "zebra marmalade quartz";

/// Every log line of the test binary, formatted as a logger would write it.
fn captured() -> &'static Mutex<Vec<String>> {
    static LINES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    static INSTALLED: OnceLock<()> = OnceLock::new();
    let lines = LINES.get_or_init(Mutex::default);
    INSTALLED.get_or_init(|| {
        struct Capture;
        impl log::Log for Capture {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                let line = format!("{} {}: {}", record.level(), record.target(), record.args());
                captured().lock().unwrap().push(line);
            }
            fn flush(&self) {}
        }
        static CAPTURE: Capture = Capture;
        log::set_logger(&CAPTURE).expect("no other logger in this binary");
        log::set_max_level(log::LevelFilter::Trace);
    });
    lines
}

fn assert_no_secret(what: &str, text: &str) {
    for word in SECRET.split(' ') {
        assert!(
            !text.to_lowercase().contains(word),
            "{what} contains a dictated word: {text}"
        );
    }
}

/// Everything the shell could print about a run: each event's `Debug`, and each error's
/// `Display` inside it.
fn printed(events: &[DictationEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| {
            let mut out = vec![format!("{e:?}")];
            match e {
                DictationEvent::Failed(TakeFailure::Transcription(err)) => {
                    out.push(err.to_string())
                }
                DictationEvent::Failed(TakeFailure::Insert(err)) => out.push(err.to_string()),
                DictationEvent::Warning(Warning::PolishFailed(err)) => out.push(err.to_string()),
                DictationEvent::Warning(Warning::SaveFailed(err)) => out.push(err.to_string()),
                DictationEvent::Warning(Warning::VadFailed(err)) => out.push(err.to_string()),
                _ => {}
            }
            out
        })
        .collect()
}

/// A store that creates records and then fails to write their text.
struct BrokenStore(MemStore);

impl Store for BrokenStore {
    fn create_record(&self, record: NewRecord) -> Result<RecordId, StoreError> {
        self.0.create_record(record)
    }
    fn record(&self, id: &RecordId) -> Result<Option<Record>, StoreError> {
        self.0.record(id)
    }
    fn records(&self, query: &RecordQuery) -> Result<Vec<Record>, StoreError> {
        self.0.records(query)
    }
    fn set_title(&self, id: &RecordId, title: &str) -> Result<(), StoreError> {
        self.0.set_title(id, title)
    }
    fn finish_record(&self, id: &RecordId, at: i64) -> Result<(), StoreError> {
        self.0.finish_record(id, at)
    }
    fn delete_record(&self, id: &RecordId) -> Result<(), StoreError> {
        self.0.delete_record(id)
    }
    fn append_segments(&self, _: &RecordId, _: &[Segment]) -> Result<(), StoreError> {
        Err(StoreError::Backend("disk full".into()))
    }
    fn segments(&self, id: &RecordId) -> Result<Vec<Segment>, StoreError> {
        self.0.segments(id)
    }
    fn supersede(&self, id: &RecordId, s: &[Segment]) -> Result<u32, StoreError> {
        self.0.supersede(id, s)
    }
    fn search(&self, q: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError> {
        self.0.search(q, limit)
    }
    fn add_note(&self, id: &RecordId, at: u64, text: &str) -> Result<NoteId, StoreError> {
        self.0.add_note(id, at, text)
    }
    fn update_note(&self, id: &NoteId, text: &str) -> Result<(), StoreError> {
        self.0.update_note(id, text)
    }
    fn delete_note(&self, id: &NoteId) -> Result<(), StoreError> {
        self.0.delete_note(id)
    }
    fn notes(&self, id: &RecordId) -> Result<Vec<Note>, StoreError> {
        self.0.notes(id)
    }
    fn save_summary(&self, id: &RecordId, s: &Summary) -> Result<(), StoreError> {
        self.0.save_summary(id, s)
    }
    fn summary(&self, id: &RecordId) -> Result<Option<Summary>, StoreError> {
        self.0.summary(id)
    }
    fn set_speaker_name(&self, id: &RecordId, s: &SpeakerId, n: &str) -> Result<(), StoreError> {
        self.0.set_speaker_name(id, s, n)
    }
    fn speaker_names(&self, id: &RecordId) -> Result<Vec<(SpeakerId, String)>, StoreError> {
        self.0.speaker_names(id)
    }
    fn add_commitments(
        &self,
        id: &RecordId,
        items: &[NewCommitment],
    ) -> Result<Vec<CommitmentId>, StoreError> {
        self.0.add_commitments(id, items)
    }
    fn commitments(&self, id: &RecordId) -> Result<Vec<Commitment>, StoreError> {
        self.0.commitments(id)
    }
    fn open_commitments(&self, limit: usize) -> Result<Vec<Commitment>, StoreError> {
        self.0.open_commitments(limit)
    }
    fn set_commitment_done(&self, id: &CommitmentId, done: bool) -> Result<(), StoreError> {
        self.0.set_commitment_done(id, done)
    }
    fn merge_commitment(&self, id: &CommitmentId, into: &CommitmentId) -> Result<(), StoreError> {
        self.0.merge_commitment(id, into)
    }
    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.0.setting(key)
    }
    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.0.set_setting(key, value)
    }
}

/// A model that answers with nothing, which the polish task refuses.
struct EmptyLlm;

impl Llm for EmptyLlm {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "empty".into(),
            model: "empty".into(),
            endpoint: Endpoint::InProcess,
        }
    }
    fn complete(&self, _: &LlmRequest, _: &CancelToken) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse { text: "  ".into() })
    }
}

#[test]
fn no_dictated_word_reaches_a_log_an_event_or_an_error() {
    let lines = captured();
    // Every failure the chain reports after transcription, in one take: polish refuses the
    // answer, the store fails half way, and insertion is not permitted.
    let rig = Rig::builder()
        .settings(|s| s.modes.modes[0].polish_enabled = true)
        .llm(Arc::new(EmptyLlm))
        .store(Arc::new(BrokenStore(MemStore::new())))
        .build();
    rig.platform
        .set_permission(Permission::Accessibility, PermissionState::Denied);
    rig.dictate_fixture(SECRET, 2.0, -30.0);

    // And a clean dictation, a voice command and a take the engine does not know.
    let command = Rig::builder()
        .settings(|s| s.commands.enabled = true)
        .build();
    command.dictate_fixture(SECRET, 2.5, -30.0);
    assert_eq!(command.inserted().len(), 1, "the clean dictation went in");
    command.dictate_fixture(&format!("inkwell scratch that {SECRET}"), 2.0, -30.0);
    command.dictate(&speech_48k(1.5, -30.0, 99));

    let events = [rig.events(), command.events()].concat();
    // The run went down the paths it was meant to.
    let debug = format!("{events:?}");
    for path in [
        "PolishFailed",
        "SaveFailed",
        "Insert(PermissionDenied",
        "Command(Undo)",
        "Transcription",
    ] {
        assert!(debug.contains(path), "{path} not exercised: {debug}");
    }
    for line in printed(&events) {
        assert_no_secret("an event", &line);
    }
    let logged = lines.lock().unwrap().clone();
    assert!(
        logged.iter().any(|l| l.contains("dictation take")),
        "the chain logged nothing, so this proves nothing: {logged:?}"
    );
    for line in &logged {
        assert_no_secret("a log line", line);
    }
}
