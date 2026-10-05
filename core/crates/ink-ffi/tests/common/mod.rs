//! What the core's integration tests share: a temp directory, a real-time clock, an event
//! recorder that checks every event against the schema, a registry row installed on disk, and
//! scriptable models, loaders and installers.

#![allow(dead_code)] // each test binary uses a different subset

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ink_audio::bands_channel;
use ink_core::{
    CancelToken, Clock, EngineError, EngineInfo, EventSink, Job, OfflineEngine, TimedText,
    TranscribeOptions, Transcript,
};
use ink_engines::{
    DownloadError, DownloadProgress, EngineRow, JobScore, Loader, ModelDir, ModelFile, Os,
    Registry, Runtime,
};
use ink_ffi::hub::EventOut;
use ink_ffi::runtime::{Core, Model, Parts};
use ink_ffi::schema::{EVENTS_SCHEMA, Schema};
use ink_pipeline::update::ModelInstaller;
use serde_json::Value;

/// A temp directory removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ink-ffi-test-{}-{label}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Real time on a monotonic clock, starting at 1 s.
pub struct TestClock(Instant);

impl Clock for TestClock {
    fn now_ns(&self) -> u64 {
        1_000_000_000 + self.0.elapsed().as_nanos() as u64
    }

    fn unix_ms(&self) -> i64 {
        1_790_146_800_000 + self.0.elapsed().as_millis() as i64
    }
}

pub fn clock() -> Arc<TestClock> {
    Arc::new(TestClock(Instant::now()))
}

/// A monotonic sequence shared by a test's observers, to order what happened on different
/// threads.
pub static SEQ: AtomicU64 = AtomicU64::new(0);

pub fn seq() -> u64 {
    SEQ.fetch_add(1, Ordering::SeqCst)
}

/// Every event the shell would get, each checked against the schema as it arrives.
pub struct Recorder {
    events: Mutex<Vec<(u64, Value)>>,
    arrived: Condvar,
    invalid: Mutex<Vec<String>>,
    threads: Mutex<Vec<Option<String>>>,
    schema: Schema,
}

impl Recorder {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::default(),
            arrived: Condvar::new(),
            invalid: Mutex::default(),
            threads: Mutex::default(),
            schema: Schema::parse(EVENTS_SCHEMA).unwrap(),
        })
    }

    pub fn out(self: &Arc<Self>) -> EventOut {
        let me = self.clone();
        Box::new(move |json| me.push(json))
    }

    pub fn push(&self, json: &str) {
        let v: Value = serde_json::from_str(json).unwrap();
        if let Err(e) = self.schema.validate(&v) {
            self.invalid.lock().unwrap().push(format!("{v}: {e}"));
        }
        self.threads
            .lock()
            .unwrap()
            .push(std::thread::current().name().map(str::to_owned));
        self.events.lock().unwrap().push((seq(), v));
        self.arrived.notify_all();
    }

    /// Waits for an event matching `pred`, for up to `timeout`.
    pub fn wait_for(&self, timeout: Duration, pred: impl Fn(&Value) -> bool) -> Option<Value> {
        let until = Instant::now() + timeout;
        let mut events = self.events.lock().unwrap();
        loop {
            if let Some((_, v)) = events.iter().find(|(_, v)| pred(v)) {
                return Some(v.clone());
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            events = self.arrived.wait_timeout(events, left).unwrap().0;
        }
    }

    /// Waits until `n` events of type `ty` have arrived.
    pub fn wait_count(&self, ty: &str, n: usize, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        let mut events = self.events.lock().unwrap();
        loop {
            if events.iter().filter(|(_, v)| v["type"] == ty).count() >= n {
                return true;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            events = self.arrived.wait_timeout(events, left).unwrap().0;
        }
    }

    /// Waits for an event of type `ty`.
    pub fn wait_type(&self, ty: &str, timeout: Duration) -> Value {
        self.wait_for(timeout, |v| v["type"] == ty)
            .unwrap_or_else(|| panic!("no {ty} within {timeout:?}; got {:?}", self.types()))
    }

    pub fn all(&self) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// The sequence number at which the first event of type `ty` arrived.
    pub fn seq_of(&self, ty: &str) -> Option<u64> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .find(|(_, v)| v["type"] == ty)
            .map(|(s, _)| *s)
    }

    pub fn types(&self) -> Vec<String> {
        self.all()
            .iter()
            .map(|v| v["type"].as_str().unwrap_or("?").to_owned())
            .collect()
    }

    pub fn count(&self, ty: &str) -> usize {
        self.types().iter().filter(|t| *t == ty).count()
    }

    /// Every event matched the schema, and every one arrived on the event thread.
    pub fn assert_valid(&self) {
        let invalid = self.invalid.lock().unwrap();
        assert!(
            invalid.is_empty(),
            "events outside the schema: {invalid:#?}"
        );
        let threads = self.threads.lock().unwrap();
        assert!(
            threads.iter().all(|t| t.as_deref() == Some("ink-events")),
            "events delivered off the event thread: {threads:?}"
        );
    }
}

pub const ROW_ID: &str = "test-asr";
pub const ROW_V2: &str = "test-asr-v2";

/// A registry row for a model that fills both finals, with one 4-byte file.
pub fn test_row(id: &str) -> EngineRow {
    EngineRow {
        id: id.into(),
        scores: vec![
            JobScore {
                job: Job::DictationFinal,
                wer: 5.0,
            },
            JobScore {
                job: Job::MeetingFinal,
                wer: 8.0,
            },
        ],
        files: vec![ModelFile {
            name: "model.bin".into(),
            url: format!(
                "https://example.com/{id}/resolve/{}/model.bin",
                "a".repeat(40)
            ),
            sha256: "0".repeat(64),
            size: 4,
        }],
        revision: "a".repeat(40),
        licence: "Apache-2.0".into(),
        oses: vec![Os::MacOs, Os::Windows],
        runtime: Runtime::LlamaCpp,
    }
}

/// Lays `row` out as the downloader would, so the router sees it installed.
pub fn install(dir: &ModelDir, row: &EngineRow) {
    for f in &row.files {
        let path = dir.file_path(row, f);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [0u8; 4]).unwrap();
    }
    std::fs::write(dir.marker_path(row), &row.revision).unwrap();
    assert!(dir.is_installed(row));
}

/// What a [`MockModel`] does per call.
#[derive(Clone)]
pub enum Behaviour {
    /// Answers with this text.
    Say(String),
    /// Panics.
    Panic,
    /// Waits for the gate to open, then answers.
    WaitThen(Arc<Gate>, String),
    /// Keeps the audio of each call, then answers with this text.
    Keep(Arc<Mutex<Vec<Vec<f32>>>>, String),
}

/// A one-shot gate a test opens.
#[derive(Default)]
pub struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
    pub waiting: AtomicUsize,
}

impl Gate {
    pub fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }

    pub fn wait(&self) {
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
    }

    /// Waits until someone is waiting at the gate.
    pub fn until_waiting(&self, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        while self.waiting.load(Ordering::SeqCst) == 0 {
            if Instant::now() > until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }
}

/// What models loaded by one [`MockLoader`] report.
#[derive(Default)]
pub struct Journal {
    /// Loads, each with the file generation it read.
    pub loads: Mutex<Vec<u64>>,
    /// Models dropped, with the sequence number at which each dropped.
    pub drops: Mutex<Vec<u64>>,
    /// Threads the models were called on.
    pub threads: Mutex<Vec<Option<String>>>,
}

pub struct MockModel {
    id: String,
    behaviour: Behaviour,
    journal: Arc<Journal>,
}

impl OfflineEngine for MockModel {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: self.id.clone(),
            jobs: vec![Job::DictationFinal, Job::MeetingFinal],
            licence: "Apache-2.0".into(),
        }
    }

    fn transcribe(&self, audio: &[f32], _: &TranscribeOptions) -> Result<Transcript, EngineError> {
        self.journal
            .threads
            .lock()
            .unwrap()
            .push(std::thread::current().name().map(str::to_owned));
        let text = match &self.behaviour {
            Behaviour::Say(t) => t.clone(),
            Behaviour::Panic => panic!("scripted engine panic"),
            Behaviour::WaitThen(gate, t) => {
                gate.wait();
                t.clone()
            }
            Behaviour::Keep(heard, t) => {
                heard.lock().unwrap().push(audio.to_vec());
                t.clone()
            }
        };
        Ok(Transcript {
            segments: vec![TimedText {
                start_ms: 0,
                end_ms: (audio.len() / 16) as u64,
                text,
            }],
        })
    }
}

impl Drop for MockModel {
    fn drop(&mut self) {
        self.journal.drops.lock().unwrap().push(seq());
    }
}

/// Loads [`MockModel`]s; the generation of the files on disk is shared with the installer.
pub struct MockLoader {
    pub behaviour: Mutex<Behaviour>,
    pub journal: Arc<Journal>,
    pub generation: Arc<AtomicU64>,
    /// Loading panics while set: a bug in an adapter.
    pub panic_on_load: std::sync::atomic::AtomicBool,
}

impl MockLoader {
    pub fn new(behaviour: Behaviour) -> Arc<Self> {
        Arc::new(Self {
            behaviour: Mutex::new(behaviour),
            journal: Arc::default(),
            generation: Arc::new(AtomicU64::new(1)),
            panic_on_load: std::sync::atomic::AtomicBool::new(false),
        })
    }
}

impl Loader<Model> for MockLoader {
    fn load(&self, row: &EngineRow) -> Result<Model, EngineError> {
        if self.panic_on_load.load(Ordering::SeqCst) {
            panic!("scripted adapter panic");
        }
        self.journal
            .loads
            .lock()
            .unwrap()
            .push(self.generation.load(Ordering::SeqCst));
        Ok(Box::new(MockModel {
            id: row.id.clone(),
            behaviour: self.behaviour.lock().unwrap().clone(),
            journal: self.journal.clone(),
        }))
    }
}

/// An installer that marks the files as being replaced (generation 0) while it runs, waits for
/// `gate` if one is set, then leaves generation `n + 1`.
pub struct MockInstaller {
    pub generation: Arc<AtomicU64>,
    pub gate: Option<Arc<Gate>>,
    pub installs: AtomicUsize,
}

impl ModelInstaller for MockInstaller {
    fn install(
        &self,
        _: &EngineRow,
        _: &CancelToken,
        _: EventSink<DownloadProgress>,
    ) -> Result<(), DownloadError> {
        self.installs.fetch_add(1, Ordering::SeqCst);
        let before = self.generation.swap(0, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.wait();
        }
        self.generation.store(before + 1, Ordering::SeqCst);
        Ok(())
    }
}

/// A core over an in-memory library, `rows` installed in a temp model directory, `loader` and
/// `installer`.
pub fn start(
    dir: &TempDir,
    rows: &[EngineRow],
    loader: Arc<dyn Loader<Model>>,
    installer: Arc<dyn ModelInstaller>,
) -> (Core, Arc<Recorder>) {
    let models = ModelDir::new(dir.path().join("models"));
    for row in rows {
        install(&models, row);
    }
    let parts = Parts {
        store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
        clock: clock(),
        registry: Registry::new(rows.to_vec()).unwrap(),
        models,
        loader,
        installer,
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: Default::default(),
    };
    start_parts(parts)
}

/// A core over `parts`, started, with the bands writer lent and `core.ready` seen.
pub fn start_parts(parts: Parts) -> (Core, Arc<Recorder>) {
    let recorder = Recorder::new();
    let (writer, _) = bands_channel();
    let core = Core::start(parts, recorder.out()).unwrap();
    core.lend_bands(writer);
    recorder.wait_type("core.ready", Duration::from_secs(5));
    (core, recorder)
}

/// A 16-bit mono WAV of speech-like audio at 16 kHz.
pub fn speech_wav(path: &Path, seconds: f64, seed: u64) {
    let samples = ink_audio::synth::speech_like(seconds, -25.0, seed);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for s in samples {
        w.write_sample((s * 32_767.0).clamp(-32_768.0, 32_767.0) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
}

/// A store that fails the methods a test names (a disk that refuses a write, a locked database),
/// and otherwise is `inner`.
pub struct FailingStore {
    pub inner: Arc<dyn ink_core::Store>,
    failing: Mutex<Vec<&'static str>>,
    /// Run once after the named method next succeeds (see [`after`](Self::after)).
    after: Mutex<Vec<(&'static str, Then)>>,
}

/// What [`FailingStore::after`] runs.
type Then = Box<dyn FnOnce() + Send>;

impl FailingStore {
    pub fn new(inner: Arc<dyn ink_core::Store>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            failing: Mutex::default(),
            after: Mutex::default(),
        })
    }

    /// Runs `then` right after `method` next succeeds, on the thread that called it: to put
    /// something else exactly into the moment after a store call.
    pub fn after(&self, method: &'static str, then: impl FnOnce() + Send + 'static) {
        self.after.lock().unwrap().push((method, Box::new(then)));
    }

    fn succeeded(&self, method: &'static str) {
        let due: Vec<_> = {
            let mut after = self.after.lock().unwrap();
            let (due, rest) = std::mem::take(&mut *after)
                .into_iter()
                .partition(|(m, _)| *m == method);
            *after = rest;
            due
        };
        for (_, then) in due {
            then();
        }
    }

    /// From now on, the methods named fail.
    pub fn fail(&self, methods: &[&'static str]) {
        self.failing.lock().unwrap().extend_from_slice(methods);
    }

    /// From now on, nothing fails.
    pub fn heal(&self) {
        self.failing.lock().unwrap().clear();
    }

    fn check(&self, method: &'static str) -> Result<(), ink_core::StoreError> {
        match self.failing.lock().unwrap().contains(&method) {
            true => Err(ink_core::StoreError::Backend(format!(
                "scripted {method} failure"
            ))),
            false => Ok(()),
        }
    }
}

/// Each method: checked, then passed on.
macro_rules! failing {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $out:ty;)*) => {
        $(fn $name(&self, $($arg: $ty),*) -> Result<$out, ink_core::StoreError> {
            self.check(stringify!($name))?;
            let out = self.inner.$name($($arg),*);
            if out.is_ok() {
                self.succeeded(stringify!($name));
            }
            out
        })*
    };
}

impl ink_core::Store for FailingStore {
    failing! {
        create_record(record: ink_core::NewRecord) -> ink_core::RecordId;
        record(id: &ink_core::RecordId) -> Option<ink_core::Record>;
        records(query: &ink_core::RecordQuery) -> Vec<ink_core::Record>;
        set_title(id: &ink_core::RecordId, title: &str) -> ();
        finish_record(id: &ink_core::RecordId, ended_at_unix_ms: i64) -> ();
        mark_stuck(id: &ink_core::RecordId) -> ();
        delete_record(id: &ink_core::RecordId) -> ();
        append_segments(id: &ink_core::RecordId, segments: &[ink_core::Segment]) -> ();
        segments(id: &ink_core::RecordId) -> Vec<ink_core::Segment>;
        save_removed(id: &ink_core::RecordId, lines: &[ink_core::Segment]) -> ();
        removed(id: &ink_core::RecordId) -> Vec<ink_core::Segment>;
        search(query: &str, limit: usize) -> Vec<ink_core::SearchHit>;
        add_note(id: &ink_core::RecordId, at_ms: u64, text: &str) -> ink_core::NoteId;
        update_note(id: &ink_core::NoteId, text: &str) -> ();
        delete_note(id: &ink_core::NoteId) -> ();
        notes(id: &ink_core::RecordId) -> Vec<ink_core::Note>;
        save_summary(id: &ink_core::RecordId, summary: &ink_core::Summary) -> ();
        summary(id: &ink_core::RecordId) -> Option<ink_core::Summary>;
        set_speaker_name(id: &ink_core::RecordId, speaker: &ink_core::SpeakerId, name: &str) -> ();
        clear_speaker_name(id: &ink_core::RecordId, speaker: &ink_core::SpeakerId) -> ();
        speaker_names(id: &ink_core::RecordId) -> Vec<(ink_core::SpeakerId, String)>;
        add_commitments(id: &ink_core::RecordId, items: &[ink_core::NewCommitment]) -> Vec<ink_core::CommitmentId>;
        add_commitments_merged(id: &ink_core::RecordId, items: &[ink_core::NewCommitment], merges: &[(usize, usize)]) -> Vec<ink_core::CommitmentId>;
        commitments(id: &ink_core::RecordId) -> Vec<ink_core::Commitment>;
        open_commitments(limit: usize) -> Vec<ink_core::Commitment>;
        set_commitment_done(id: &ink_core::CommitmentId, done: bool) -> ();
        set_done_evidence(id: &ink_core::CommitmentId, evidence: Option<&ink_core::DoneEvidence>) -> ();
        merge_commitment(id: &ink_core::CommitmentId, into: &ink_core::CommitmentId) -> ();
        setting(key: &str) -> Option<String>;
        set_setting(key: &str, value: &str) -> ();
        set_settings(settings: &[(&str, &str)]) -> ();
    }

    fn supersede_with(
        &self,
        id: &ink_core::RecordId,
        segments: &[ink_core::Segment],
        with: ink_core::SupersedeWith<'_>,
    ) -> Result<u32, ink_core::StoreError> {
        self.check("supersede_with")?;
        self.inner.supersede_with(id, segments, with)
    }

    fn unscrubbed(&self) -> bool {
        self.inner.unscrubbed()
    }

    fn scrub_change(&self) -> Option<bool> {
        self.inner.scrub_change()
    }
}
