//! Carry-over 5: a model is unloaded through residency before its files are replaced (Windows
//! refuses to replace a file that is open, and a loaded model holds its files open).

use std::sync::{Arc, Mutex, Weak};

use ink_core::mock::{MockClock, MockEngine};
use ink_core::{CancelToken, EngineError, Job};
use ink_engines::{DownloadError, EngineRow, JobScore, Loader, ModelFile, Os, Residency, Runtime};
use ink_pipeline::update::{ModelInstaller, ModelResidency, UpdateError, update_model};

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

    fn unload(&self, id: &str) -> Result<(), EngineError> {
        self.journal.push(format!("unload {id}"));
        if self.pinned {
            return Err(EngineError::Failed("a lease still holds the model".into()));
        }
        self.loaded.lock().unwrap().retain(|(i, _)| i != id);
        Ok(())
    }
}

/// An installer that checks, when it runs, that no engine for the row is alive.
struct CheckingInstaller {
    journal: Arc<Journal>,
    must_be_gone: Weak<MockEngine>,
    fail: bool,
}

impl ModelInstaller for CheckingInstaller {
    fn install(&self, row: &EngineRow, _: &CancelToken) -> Result<(), DownloadError> {
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
    update_model(&residency, &installer, &old, &new, &CancelToken::new()).unwrap();
    assert_eq!(
        journal.entries(),
        vec![
            "warm none",
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
    let result = update_model(&residency, &installer, &old, &new, &CancelToken::new());
    assert!(
        matches!(result, Err(UpdateError::StillLoaded(_))),
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
    let result = update_model(&residency, &installer, &old, &new, &CancelToken::new());
    assert!(
        matches!(result, Err(UpdateError::Install(DownloadError::Cancelled))),
        "{result:?}"
    );
    assert_eq!(
        journal.entries().last().map(String::as_str),
        Some("warm asr")
    );
    assert_eq!(residency.warm().as_deref(), Some("asr"));
}

/// Loads a mock engine per row.
struct MockLoader;

impl Loader<MockEngine> for MockLoader {
    fn load(&self, row: &EngineRow) -> Result<MockEngine, EngineError> {
        Ok(MockEngine::new(&row.id, &[Job::DictationFinal]))
    }
}

#[test]
fn todays_residency_cannot_unload_on_demand_so_the_update_fails_closed() {
    let clock = Arc::new(MockClock::new(0, 0));
    let residency = Residency::new(Arc::new(MockLoader), clock);
    let (old, new) = (row("asr", 'a'), row("asr", 'b'));
    residency.set_warm(Some(&old)).unwrap();
    let journal = Arc::new(Journal::default());
    let installer = CheckingInstaller {
        journal: journal.clone(),
        must_be_gone: Weak::new(),
        fail: false,
    };
    let result = update_model(&residency, &installer, &old, &new, &CancelToken::new());
    assert!(
        matches!(
            result,
            Err(UpdateError::StillLoaded(EngineError::Unsupported(_)))
        ),
        "{result:?}"
    );
    assert!(journal.entries().is_empty(), "the installer never ran");
    assert_eq!(
        ModelResidency::warm(&residency).as_deref(),
        Some("asr"),
        "warm again"
    );
    assert!(residency.is_resident("asr"));
}
