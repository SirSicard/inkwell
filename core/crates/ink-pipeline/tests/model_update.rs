//! Carry-over 5: a model is unloaded through residency before its files are replaced (Windows
//! refuses to replace a file that is open, and a loaded model holds its files open).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use ink_core::mock::{MockClock, MockEngine};
use ink_core::{CancelToken, EngineError, EventSink, Job};
use ink_engines::{
    DownloadError, DownloadProgress, EngineRow, JobScore, Loader, ModelFile, Os, Residency,
    Runtime, Unloaded,
};
use ink_pipeline::update::{ModelInstaller, ModelResidency, UpdateError, update_model};

fn no_progress() -> EventSink<DownloadProgress> {
    Arc::new(|_| {})
}

fn row(id: &str, revision_digit: char) -> EngineRow {
    EngineRow {
        id: id.into(),
        scores: vec![JobScore {
            job: Job::DictationFinal,
            wer: 5.0,
        }],
        files: vec![ModelFile {
            name: "model.gguf".into(),
            url: format!(
                "https://example.com/{id}/resolve/{}/model.gguf",
                revision_digit.to_string().repeat(40)
            ),
            sha256: "0".repeat(64),
            size: 1,
        }],
        revision: revision_digit.to_string().repeat(40),
        licence: "Apache-2.0".into(),
        oses: vec![Os::MacOs, Os::Windows],
        runtime: Runtime::LlamaCpp,
        kind: ink_engines::RowKind::Speech,
    }
}

/// A residency double that holds one mock engine per loaded model and journals every call.
#[derive(Default)]
struct Journal(Mutex<Vec<String>>);

impl Journal {
    fn push(&self, entry: String) {
        self.0.lock().unwrap().push(entry);
    }
    fn entries(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

struct FakeResidency {
    journal: Arc<Journal>,
    loaded: Mutex<Vec<(String, Arc<MockEngine>)>>,
    warm: Mutex<Option<String>>,
    /// A lease the caller cannot see holds the model: unloading fails.
    pinned: bool,
    /// Loading any model fails.
    broken_loader: bool,
    /// Unloading reports "not loaded" whatever is loaded: residency and the caller disagree about
    /// which model this is.
    claims_not_loaded: bool,
}

impl FakeResidency {
    fn new(journal: &Arc<Journal>, warm: &EngineRow, pinned: bool) -> (Self, Weak<MockEngine>) {
        let engine = Arc::new(MockEngine::new(&warm.id, &[Job::DictationFinal]));
        let weak = Arc::downgrade(&engine);
        (
            Self {
                journal: journal.clone(),
                loaded: Mutex::new(vec![(warm.id.clone(), engine)]),
                warm: Mutex::new(Some(warm.id.clone())),
                pinned,
                broken_loader: false,
                claims_not_loaded: false,
            },
            weak,
        )
    }
}

impl ModelResidency for FakeResidency {
    fn warm(&self) -> Option<String> {
        self.warm.lock().unwrap().clone()
    }

    fn set_warm(&self, row: Option<&EngineRow>) -> Result<(), EngineError> {
        self.journal
            .push(format!("warm {}", row.map_or("none", |r| r.id.as_str())));
        if row.is_some() && self.broken_loader {
            return Err(EngineError::ModelMissing("scripted".into()));
        }
        if let Some(r) = row {
            let mut loaded = self.loaded.lock().unwrap();
            if !loaded.iter().any(|(id, _)| *id == r.id) {
                loaded.push((r.id.clone(), Arc::new(MockEngine::new(&r.id, &[]))));
            }
        }
        *self.warm.lock().unwrap() = row.map(|r| r.id.clone());
        Ok(())
    }

    fn is_resident(&self, id: &str) -> bool {
        self.loaded.lock().unwrap().iter().any(|(i, _)| i == id)
    }

    fn unload(&self, id: &str) -> Result<Unloaded, EngineError> {
        self.journal.push(format!("unload {id}"));
        if self.pinned {
            return Err(EngineError::Failed("a lease still holds the model".into()));
        }
        if self.claims_not_loaded {
            return Ok(Unloaded::NotLoaded);
        }
        let mut loaded = self.loaded.lock().unwrap();
        let before = loaded.len();
        loaded.retain(|(i, _)| i != id);
        if loaded.len() == before {
            return Ok(Unloaded::NotLoaded);
        }
        let mut warm = self.warm.lock().unwrap();
        if warm.as_deref() == Some(id) {
            *warm = None;
        }
        Ok(Unloaded::WasLoaded)
    }
}

/// An installer that checks, when it runs, that no engine for the row is alive.
struct CheckingInstaller {
    journal: Arc<Journal>,
    must_be_gone: Weak<MockEngine>,
    fail: bool,
}

impl ModelInstaller for CheckingInstaller {
    fn install(
        &self,
        row: &EngineRow,
        _: &CancelToken,
        _: EventSink<DownloadProgress>,
    ) -> Result<(), DownloadError> {
        let gone = self.must_be_gone.upgrade().is_none();
        self.journal
            .push(format!("install {} (old model gone: {gone})", row.id));
        if self.fail {
            return Err(DownloadError::Cancelled);
        }
        Ok(())
    }
}

#[test]
fn a_model_is_unloaded_before_its_files_are_replaced() {
    let journal = Arc::new(Journal::default());
    let (old, new) = (row("asr", 'a'), row("asr", 'b'));
    let (residency, weak) = FakeResidency::new(&journal, &old, false);
    let installer = CheckingInstaller {
        journal: journal.clone(),
        must_be_gone: weak,
        fail: false,
    };
    update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    )
    .unwrap();
    assert_eq!(
        journal.entries(),
        vec![
            "unload asr",
            "install asr (old model gone: true)",
            "warm asr",
        ]
    );
}

#[test]
fn an_update_is_refused_while_the_model_stays_loaded() {
    let journal = Arc::new(Journal::default());
    let (old, new) = (row("asr", 'a'), row("asr", 'b'));
    let (residency, weak) = FakeResidency::new(&journal, &old, true);
    let installer = CheckingInstaller {
        journal: journal.clone(),
        must_be_gone: weak,
        fail: false,
    };
    let result = update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    );
    assert!(
        matches!(
            result,
            Err(UpdateError::StillLoaded {
                rewarm_failed: None,
                ..
            })
        ),
        "{result:?}"
    );
    assert!(
        !journal.entries().iter().any(|e| e.starts_with("install")),
        "nothing was written under a loaded model: {:?}",
        journal.entries()
    );
    assert_eq!(residency.warm().as_deref(), Some("asr"), "still warm");
}

#[test]
fn a_failed_install_brings_the_old_model_back() {
    let journal = Arc::new(Journal::default());
    let (old, new) = (row("asr", 'a'), row("asr-next", 'b'));
    let (residency, weak) = FakeResidency::new(&journal, &old, false);
    let installer = CheckingInstaller {
        journal: journal.clone(),
        must_be_gone: weak,
        fail: true,
    };
    let result = update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    );
    assert!(
        matches!(
            result,
            Err(UpdateError::Install {
                error: DownloadError::Cancelled,
                rewarm_failed: None
            })
        ),
        "{result:?}"
    );
    assert_eq!(
        journal.entries().last().map(String::as_str),
        Some("warm asr")
    );
    assert_eq!(residency.warm().as_deref(), Some("asr"));
}

#[test]
fn a_model_reported_not_loaded_although_it_is_is_refused() {
    // A wrong id must not let an update write under a live model.
    let journal = Arc::new(Journal::default());
    let (old, new) = (row("asr", 'a'), row("asr", 'b'));
    let (mut residency, weak) = FakeResidency::new(&journal, &old, false);
    residency.claims_not_loaded = true;
    let installer = CheckingInstaller {
        journal: journal.clone(),
        must_be_gone: weak,
        fail: false,
    };
    let result = update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    );
    match &result {
        Err(e @ UpdateError::Mismatch { id, .. }) => {
            assert_eq!(id, "asr");
            assert!(e.to_string().contains("asr"), "{e}");
        }
        other => panic!("expected a mismatch, got {other:?}"),
    }
    assert!(
        !journal.entries().iter().any(|e| e.starts_with("install")),
        "{:?}",
        journal.entries()
    );
}

// ---------------------------------------------------------------------------------------------
// With ink-engines' own residency
// ---------------------------------------------------------------------------------------------

/// Live copies of each model, counted by the loader and by each copy's drop.
#[derive(Default)]
struct Live(Mutex<HashMap<String, usize>>);

impl Live {
    fn of(&self, id: &str) -> usize {
        self.0.lock().unwrap().get(id).copied().unwrap_or(0)
    }
}

/// A loaded model: a mock engine that counts itself alive.
struct Model {
    id: String,
    live: Arc<Live>,
}

impl Drop for Model {
    fn drop(&mut self) {
        *self
            .live
            .0
            .lock()
            .unwrap()
            .entry(self.id.clone())
            .or_default() -= 1;
    }
}

struct CountingLoader(Arc<Live>);

impl Loader<Model> for CountingLoader {
    fn load(&self, row: &EngineRow) -> Result<Model, EngineError> {
        *self.0.0.lock().unwrap().entry(row.id.clone()).or_default() += 1;
        Ok(Model {
            id: row.id.clone(),
            live: self.0.clone(),
        })
    }
}

/// An installer that records, when it runs, how many copies of each named model are alive.
struct LiveCheckingInstaller {
    live: Arc<Live>,
    watch: Vec<&'static str>,
    ran: Mutex<Vec<String>>,
}

impl ModelInstaller for LiveCheckingInstaller {
    fn install(
        &self,
        row: &EngineRow,
        _: &CancelToken,
        _: EventSink<DownloadProgress>,
    ) -> Result<(), DownloadError> {
        let alive: Vec<String> = self
            .watch
            .iter()
            .map(|id| format!("{id}={}", self.live.of(id)))
            .collect();
        self.ran
            .lock()
            .unwrap()
            .push(format!("install {} with {}", row.id, alive.join(" ")));
        Ok(())
    }
}

fn residency(live: &Arc<Live>) -> Residency<Model> {
    Residency::new(
        Arc::new(CountingLoader(live.clone())),
        Arc::new(MockClock::new(0, 0)),
    )
}

#[test]
fn residency_unloads_confirms_installs_and_warms_in_that_order() {
    let live = Arc::new(Live::default());
    let residency = residency(&live);
    let (old, new) = (row("asr", 'a'), row("asr", 'b'));
    residency.set_warm(Some(&old)).unwrap();
    assert_eq!(live.of("asr"), 1);
    let installer = LiveCheckingInstaller {
        live: live.clone(),
        watch: vec!["asr"],
        ran: Mutex::default(),
    };
    update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    )
    .unwrap();
    assert_eq!(
        *installer.ran.lock().unwrap(),
        vec!["install asr with asr=0"],
        "no copy of the model was alive while its files were written"
    );
    assert_eq!(ModelResidency::warm(&residency).as_deref(), Some("asr"));
    assert_eq!(live.of("asr"), 1, "the new model is loaded and warm");
}

#[test]
fn a_model_in_use_is_never_written_under() {
    let live = Arc::new(Live::default());
    let residency = residency(&live);
    let (old, new) = (row("asr", 'a'), row("asr", 'b'));
    residency.set_warm(Some(&old)).unwrap();
    let lease = residency.acquire(&old).unwrap();
    let installer = LiveCheckingInstaller {
        live: live.clone(),
        watch: vec!["asr"],
        ran: Mutex::default(),
    };
    let result = update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    );
    assert!(
        matches!(
            result,
            Err(UpdateError::StillLoaded {
                rewarm_failed: None,
                ..
            })
        ),
        "{result:?}"
    );
    assert!(
        installer.ran.lock().unwrap().is_empty(),
        "the installer never ran"
    );
    assert_eq!(
        ModelResidency::warm(&residency).as_deref(),
        Some("asr"),
        "still warm"
    );
    drop(lease);
}

#[test]
fn a_model_loaded_under_the_new_id_is_unloaded_too() {
    let live = Arc::new(Live::default());
    let residency = residency(&live);
    let (old, new) = (row("asr", 'a'), row("asr-next", 'b'));
    residency.set_warm(Some(&old)).unwrap();
    drop(residency.acquire(&new).unwrap()); // loaded, idle
    assert_eq!(live.of("asr-next"), 1);
    let installer = LiveCheckingInstaller {
        live: live.clone(),
        watch: vec!["asr", "asr-next"],
        ran: Mutex::default(),
    };
    update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    )
    .unwrap();
    assert_eq!(
        *installer.ran.lock().unwrap(),
        vec!["install asr-next with asr=0 asr-next=0"]
    );
    assert_eq!(
        ModelResidency::warm(&residency).as_deref(),
        Some("asr-next")
    );
}

#[test]
fn a_model_that_is_not_loaded_is_updated_without_warming_anything() {
    let live = Arc::new(Live::default());
    let residency = residency(&live);
    let (old, new) = (row("asr", 'a'), row("asr", 'b'));
    let installer = LiveCheckingInstaller {
        live: live.clone(),
        watch: vec!["asr"],
        ran: Mutex::default(),
    };
    update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    )
    .unwrap();
    assert_eq!(
        *installer.ran.lock().unwrap(),
        vec!["install asr with asr=0"]
    );
    assert_eq!(ModelResidency::warm(&residency), None);
    assert_eq!(live.of("asr"), 0, "nothing was loaded");
}

#[test]
fn a_previous_model_that_will_not_load_again_is_reported_not_just_logged() {
    let journal = Arc::new(Journal::default());
    let (old, new) = (row("asr", 'a'), row("asr-next", 'b'));
    let (mut residency, weak) = FakeResidency::new(&journal, &old, false);
    residency.broken_loader = true;
    let installer = CheckingInstaller {
        journal: journal.clone(),
        must_be_gone: weak,
        fail: true,
    };
    let result = update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    );
    match &result {
        Err(
            e @ UpdateError::Install {
                rewarm_failed: Some(EngineError::ModelMissing(_)),
                ..
            },
        ) => assert!(e.no_model_warm(), "{e}"),
        other => panic!("expected the failed re-warm in the error, got {other:?}"),
    }
}

#[test]
fn a_new_model_that_will_not_load_is_named() {
    let journal = Arc::new(Journal::default());
    let (old, new) = (row("asr", 'a'), row("asr-next", 'b'));
    let (mut residency, weak) = FakeResidency::new(&journal, &old, false);
    residency.broken_loader = true;
    let installer = CheckingInstaller {
        journal: journal.clone(),
        must_be_gone: weak,
        fail: false,
    };
    let result = update_model(
        &residency,
        &installer,
        &old,
        &new,
        &CancelToken::new(),
        no_progress(),
    );
    match &result {
        Err(e @ UpdateError::Warm { id, .. }) => {
            assert_eq!(id, "asr-next");
            assert!(e.no_model_warm());
            assert!(e.to_string().contains("asr-next"), "{e}");
        }
        other => panic!("expected a warm failure naming the model, got {other:?}"),
    }
}

/// An installer that reports two steps of progress.
struct ReportingInstaller;

impl ModelInstaller for ReportingInstaller {
    fn install(
        &self,
        row: &EngineRow,
        _: &CancelToken,
        progress: EventSink<DownloadProgress>,
    ) -> Result<(), DownloadError> {
        for done in [0, row.total_size()] {
            progress(DownloadProgress {
                id: row.id.clone(),
                done,
                total: row.total_size(),
            });
        }
        Ok(())
    }
}

#[test]
fn the_installs_progress_reaches_the_caller() {
    let live = Arc::new(Live::default());
    let residency = residency(&live);
    let row = row("asr", 'a');
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let seen = seen.clone();
        Arc::new(move |p: DownloadProgress| seen.lock().unwrap().push((p.id, p.done, p.total)))
    };
    update_model(
        &residency,
        &ReportingInstaller,
        &row,
        &row,
        &CancelToken::new(),
        sink,
    )
    .unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [("asr".to_string(), 0, 1), ("asr".to_string(), 1, 1)]
    );
}
