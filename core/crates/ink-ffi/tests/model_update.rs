//! A model update holds its model exclusively: between unloading it and installing the new files,
//! a job that needs it is refused (with `model.refused`), never served from files being replaced.
//! An update to itself is how a model that is not installed yet gets installed.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::*;
use ink_core::mock::MockPlatform;
use ink_core::{CancelToken, EventSink, HotkeyEvent};
use ink_engines::{
    DownloadError, DownloadProgress, Downloader, EngineRow, Fetch, FetchError, Fetched, ModelDir,
    ModelFile, Registry, Runtime,
};
use ink_ffi::dictation::Block;
use ink_ffi::dictation::DictationInbox;
use ink_ffi::runtime::{Core, DictationParts, Parts};
use ink_pipeline::chain::DictationSettings;
use ink_pipeline::events::VadUnavailable;
use ink_pipeline::gain_stage::Vad;
use ink_pipeline::update::ModelInstaller;

const BLOCK: usize = 160;
const BLOCK_NS: u64 = 10_000_000;

fn dictation(core: &Core, platform: &Arc<MockPlatform>) -> DictationInbox {
    core.start_dictation(DictationParts {
        inserter: platform.clone(),
        focus: platform.clone(),
        llm: None,
        settings: DictationSettings::default(),
        vad: Vad::Unavailable(VadUnavailable::ModelMissing),
    })
    .unwrap()
}

/// One take as the pump and the hotkey deliver it, stamped from now on the core's clock: room, a
/// press, 1.5 s of speech, a release, then quiet so the tail ends.
fn take(core: &Core, inbox: &DictationInbox, seed: u64) {
    let mut t = core.shared().clock.now_ns();
    let hotkey = inbox.hotkey_sink();
    let audio = |samples: &[f32], t: &mut u64| {
        for block in samples.chunks(BLOCK) {
            inbox.push_audio(Block {
                samples: block.to_vec(),
                host_time_ns: *t,
                dropped_frames: 0,
            });
            *t += BLOCK_NS;
        }
    };
    audio(&[0.0; 8_000], &mut t);
    hotkey(HotkeyEvent::Pressed { at_ns: t });
    audio(&ink_audio::synth::speech_like(1.5, -30.0, seed), &mut t);
    hotkey(HotkeyEvent::Released { at_ns: t });
    audio(&[0.0; 9_600], &mut t);
}

#[test]
fn a_dictation_during_an_update_is_refused_and_never_loads_the_model() {
    let dir = TempDir::new("update");
    let loader = MockLoader::new(Behaviour::Say("dictated words".into()));
    let gate = Arc::new(Gate::default());
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: Some(gate.clone()),
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader.clone(), installer.clone());
    core.command(r#"{"cmd":"model.warm","job":"dictation_final"}"#)
        .unwrap();
    events.wait_type("model.warmed", Duration::from_secs(5));
    assert_eq!(*loader.journal.loads.lock().unwrap(), [1]);
    let platform = Arc::new(MockPlatform::new());
    let inbox = dictation(&core, &platform);

    // The update unloads the model and stops mid-install, the files half replaced.
    core.command(&format!(
        r#"{{"cmd":"model.update","model":"{ROW_ID}","next":"{ROW_ID}"}}"#
    ))
    .unwrap();
    events.wait_type("model.update_started", Duration::from_secs(5));
    assert!(
        gate.until_waiting(Duration::from_secs(5)),
        "the install began"
    );
    assert!(core.shared().gate.is_held(ROW_ID));
    assert!(
        core.shared().residency.resident().is_empty(),
        "unloaded first"
    );
    assert_eq!(
        loader.generation.load(Ordering::SeqCst),
        0,
        "files being replaced"
    );

    // A dictation now is refused, and nothing is loaded.
    take(&core, &inbox, 11);
    let refused = events.wait_type("model.refused", Duration::from_secs(10));
    assert_eq!(refused["id"], ROW_ID);
    assert_eq!(refused["job"], "dictation_final");
    assert_eq!(refused["reason"], "updating");
    let failed = events.wait_type("dictation.failed", Duration::from_secs(5));
    assert_eq!(failed["stage"], "transcription");
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .contains("being updated"),
        "{failed}"
    );
    assert_eq!(
        *loader.journal.loads.lock().unwrap(),
        [1],
        "no load while the files were being replaced"
    );
    assert!(platform.inserted().is_empty());
    // Commands run one at a time: a warm asked for now waits for the update to finish.
    core.command(r#"{"cmd":"model.warm","job":"dictation_final"}"#)
        .unwrap();

    // The install finishes, the new model is warmed under the hold, and the hold is released.
    gate.open();
    let finished = events.wait_type("model.update_finished", Duration::from_secs(5));
    assert_eq!(finished["ok"], true, "{finished}");
    assert_eq!(finished["no_model_warm"], false);
    assert!(!core.shared().gate.is_held(ROW_ID));
    assert!(events.wait_count("model.warmed", 2, Duration::from_secs(5)));
    let types = events.types();
    let finished_at = types
        .iter()
        .position(|t| t == "model.update_finished")
        .unwrap();
    let last_warm = types.iter().rposition(|t| t == "model.warmed").unwrap();
    assert!(
        last_warm > finished_at,
        "the queued warm ran after the update: {types:?}"
    );
    assert_eq!(
        events.count("model.refused"),
        1,
        "only the take was refused"
    );
    assert_eq!(
        loader.journal.loads.lock().unwrap().first(),
        Some(&1),
        "{:?}",
        loader.journal.loads
    );
    assert!(
        loader.journal.loads.lock().unwrap()[1..]
            .iter()
            .all(|g| *g == 2),
        "every later load read the new files: {:?}",
        loader.journal.loads
    );

    // The next dictation is served by the new model.
    take(&core, &inbox, 12);
    let inserted = events.wait_type("dictation.inserted", Duration::from_secs(10));
    // The default mode writes it as a sentence.
    assert_eq!(inserted["text"], "Dictated words.");
    assert_eq!(
        platform.inserted().len(),
        1,
        "inserted once, by the second take only"
    );
    assert_eq!(installer.installs.load(Ordering::SeqCst), 1);
    core.shutdown();
    events.assert_valid();
}

#[test]
fn an_update_is_refused_while_a_job_is_using_the_model() {
    let dir = TempDir::new("update-busy");
    let busy = Arc::new(Gate::default());
    let loader = MockLoader::new(Behaviour::WaitThen(busy.clone(), "slow words".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader.clone(), installer.clone());
    let platform = Arc::new(MockPlatform::new());
    let inbox = dictation(&core, &platform);
    take(&core, &inbox, 21);
    assert!(
        busy.until_waiting(Duration::from_secs(10)),
        "the take reached the model"
    );

    core.command(&format!(
        r#"{{"cmd":"model.update","model":"{ROW_ID}","next":"{ROW_ID}","id":"u1"}}"#
    ))
    .unwrap();
    let failed = events.wait_type("command.failed", Duration::from_secs(5));
    assert!(
        failed["message"].as_str().unwrap().contains("in use"),
        "{failed}"
    );
    assert_eq!(failed["id"], "u1");
    assert_eq!(events.count("model.update_started"), 0);
    assert_eq!(
        installer.installs.load(Ordering::SeqCst),
        0,
        "nothing was replaced"
    );

    busy.open();
    events.wait_type("dictation.inserted", Duration::from_secs(10));
    core.shutdown();
    events.assert_valid();
}

/// Files served from memory by URL, from the offset asked for: no network.
struct Served(HashMap<String, Vec<u8>>);

impl Fetch for Served {
    fn get(&self, url: &str, offset: u64) -> Result<Fetched, FetchError> {
        let bytes = self.0.get(url).ok_or(FetchError::Http { status: 404 })?;
        Ok(Fetched {
            start: offset,
            total: Some(bytes.len() as u64),
            body: Box::new(std::io::Cursor::new(bytes[offset as usize..].to_vec())),
        })
    }
}

/// SHA-256 of `abc` (FIPS 180-2's published vector): the served model's three bytes.
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn an_update_to_itself_installs_a_model_that_is_not_installed_and_changes_nothing_else() {
    let dir = TempDir::new("first-install");
    let models = ModelDir::new(dir.path().join("models"));
    let warm = test_row(ROW_ID);
    install(&models, &warm);
    // Not installed; its bytes are served in memory. A worse rate than the warm model's, so what
    // serves each job stays the same once it is installed.
    let mut fresh = test_row(ROW_V2);
    fresh.files[0].sha256 = ABC_SHA256.into();
    fresh.files[0].size = 3;
    for score in &mut fresh.scores {
        score.wer += 10.0;
    }
    let fetch = Served(HashMap::from([(
        fresh.files[0].url.clone(),
        b"abc".to_vec(),
    )]));
    let loader = MockLoader::new(Behaviour::Say("words".into()));
    let (core, events) = start_parts(Parts {
        store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
        clock: clock(),
        registry: Registry::new(vec![warm.clone(), fresh.clone()]).unwrap(),
        models: models.clone(),
        loader: loader.clone(),
        installer: Arc::new(Downloader::new(Arc::new(fetch), models.clone())),
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: Default::default(),
    });
    core.command(r#"{"cmd":"model.warm","job":"dictation_final"}"#)
        .unwrap();
    events.wait_type("model.warmed", Duration::from_secs(5));
    let warm_path = models.file_path(&warm, &warm.files[0]);
    let warm_bytes = std::fs::read(&warm_path).unwrap();
    assert!(!models.is_installed(&fresh));

    core.command(&format!(
        r#"{{"cmd":"model.update","model":"{ROW_V2}","next":"{ROW_V2}","id":"first"}}"#
    ))
    .unwrap();
    let started = events.wait_type("model.update_started", Duration::from_secs(5));
    assert_eq!(
        (&started["id"], &started["next"]),
        (&ROW_V2.into(), &ROW_V2.into())
    );
    let finished = events.wait_type("model.update_finished", Duration::from_secs(5));
    assert_eq!(finished["ok"], true, "{finished}");
    assert_eq!(finished["no_model_warm"], false);
    assert!(finished.get("message").is_none(), "{finished}");
    // Its progress, between the two: the start and the end of its three bytes.
    let progress: Vec<(u64, u64)> = events
        .all()
        .iter()
        .filter(|e| e["type"] == "model.update_progress")
        .inspect(|e| assert_eq!((&e["id"], &e["next"]), (&ROW_V2.into(), &ROW_V2.into())))
        .map(|e| {
            let n = |k: &str| e[k].as_u64().unwrap();
            (n("done_bytes"), n("total_bytes"))
        })
        .collect();
    assert_eq!(progress, [(0, 3), (3, 3)]);
    let types = events.types();
    let at = |ty: &str| types.iter().position(|t| t == ty).unwrap();
    let last_progress = types
        .iter()
        .rposition(|t| t == "model.update_progress")
        .unwrap();
    assert!(at("model.update_started") < at("model.update_progress"));
    assert!(last_progress < at("model.update_finished"));

    // Installed: verified, in place, marked.
    assert!(models.is_installed(&fresh));
    assert_eq!(
        std::fs::read(models.file_path(&fresh, &fresh.files[0])).unwrap(),
        b"abc"
    );
    assert!(!core.shared().gate.is_held(ROW_V2));
    // Nothing else changed: the warm model is still the one loaded and warm, loaded once, its
    // files untouched; the new one was neither loaded nor warmed; nothing failed.
    assert_eq!(core.shared().residency.warm().as_deref(), Some(ROW_ID));
    assert_eq!(core.shared().residency.resident(), [ROW_ID]);
    assert_eq!(*loader.journal.loads.lock().unwrap(), [1]);
    assert!(models.is_installed(&warm));
    assert_eq!(std::fs::read(&warm_path).unwrap(), warm_bytes);
    assert_eq!(events.count("model.warmed"), 1);
    assert_eq!(events.count("command.failed"), 0);
    core.command(r#"{"cmd":"engine.route","job":"dictation_final"}"#)
        .unwrap();
    let routed = events.wait_type("engine.routed", Duration::from_secs(5));
    assert_eq!(routed["id"], ROW_ID, "the better model still serves");
    core.shutdown();
    events.assert_valid();
}

/// An installer that reports progress as fast as it can, then the end twice (as the downloader can),
/// and keeps how long that took.
struct Flooding {
    took: std::sync::Mutex<Option<Duration>>,
}

impl ModelInstaller for Flooding {
    fn install(
        &self,
        row: &EngineRow,
        _: &CancelToken,
        progress: EventSink<DownloadProgress>,
    ) -> Result<(), DownloadError> {
        let total = 1_000_000;
        let start = Instant::now();
        let report = |done| {
            progress(DownloadProgress {
                id: row.id.clone(),
                done,
                total,
            })
        };
        for done in (0..total).step_by(10) {
            report(done);
        }
        report(total);
        report(total);
        *self.took.lock().unwrap() = Some(start.elapsed());
        Ok(())
    }
}

#[test]
fn an_updates_progress_reaches_the_shell_about_four_times_a_second_and_once_at_the_end() {
    let dir = TempDir::new("progress");
    let installer = Arc::new(Flooding {
        took: Default::default(),
    });
    let loader = MockLoader::new(Behaviour::Say("words".into()));
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader, installer.clone());
    core.command(&format!(
        r#"{{"cmd":"model.update","model":"{ROW_ID}","next":"{ROW_ID}"}}"#
    ))
    .unwrap();
    let finished = events.wait_type("model.update_finished", Duration::from_secs(10));
    assert_eq!(finished["ok"], true, "{finished}");
    let took = installer.took.lock().unwrap().unwrap();
    let done: Vec<u64> = events
        .all()
        .iter()
        .filter(|e| e["type"] == "model.update_progress")
        .map(|e| e["done_bytes"].as_u64().unwrap())
        .collect();
    // 100,001 reports: the first, one per quarter second of reporting, and the end once.
    let most = 2 + (took.as_millis() / 250) as usize;
    assert!(
        (2..=most).contains(&done.len()),
        "{} events in {took:?}: {done:?}",
        done.len()
    );
    assert_eq!(done[0], 0);
    assert_eq!(done.iter().filter(|&&d| d == 1_000_000).count(), 1);
    assert_eq!(done.last(), Some(&1_000_000));
    core.shutdown();
    events.assert_valid();
}

/// A model the shell runs (Core ML, the Mac's Parakeet) is installed the same way, files in
/// subdirectories and all, and the core never loads, warms or routes to it.
#[test]
fn an_update_to_itself_installs_a_model_the_shell_runs_without_loading_it() {
    let dir = TempDir::new("shell-model");
    let models = ModelDir::new(dir.path().join("models"));
    let warm = test_row(ROW_ID);
    install(&models, &warm);
    let mut shell_model = test_row("test-core-ml");
    shell_model.runtime = Runtime::CoreMl;
    shell_model.scores.clear();
    shell_model.files = ["v3/Encoder.mlmodelc/weights/weight.bin", "v3/vocab.json"]
        .iter()
        .map(|name| ModelFile {
            name: (*name).into(),
            url: format!(
                "https://example.com/test-core-ml/resolve/{}/{name}",
                "a".repeat(40)
            ),
            sha256: ABC_SHA256.into(),
            size: 3,
        })
        .collect();
    let fetch = Served(
        shell_model
            .files
            .iter()
            .map(|f| (f.url.clone(), b"abc".to_vec()))
            .collect(),
    );
    let loader = MockLoader::new(Behaviour::Say("words".into()));
    let (core, events) = start_parts(Parts {
        store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
        clock: clock(),
        registry: Registry::new(vec![warm.clone(), shell_model.clone()]).unwrap(),
        models: models.clone(),
        loader: loader.clone(),
        installer: Arc::new(Downloader::new(Arc::new(fetch), models.clone())),
        data_dir: dir.path().to_owned(),
        permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
        meetings: Default::default(),
    });
    core.command(r#"{"cmd":"model.warm","job":"dictation_final"}"#)
        .unwrap();
    events.wait_type("model.warmed", Duration::from_secs(5));

    core.command(r#"{"cmd":"model.update","model":"test-core-ml","next":"test-core-ml"}"#)
        .unwrap();
    let finished = events.wait_type("model.update_finished", Duration::from_secs(5));
    assert_eq!(finished["ok"], true, "{finished}");
    assert!(models.is_installed(&shell_model));
    for f in &shell_model.files {
        let path = models.file_path(&shell_model, f);
        assert!(path.starts_with(models.row_dir(&shell_model)), "{path:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
    }
    let progress = events.all();
    let last = progress
        .iter()
        .rfind(|e| e["type"] == "model.update_progress")
        .unwrap();
    assert_eq!(
        (&last["done_bytes"], &last["total_bytes"]),
        (&6.into(), &6.into())
    );

    // Never loaded or warmed, and no job routes to it.
    assert_eq!(*loader.journal.loads.lock().unwrap(), [1]);
    assert_eq!(core.shared().residency.resident(), [ROW_ID]);
    assert_eq!(core.shared().residency.warm().as_deref(), Some(ROW_ID));
    for job in ["dictation_final", "meeting_final", "live_partials"] {
        core.command(&format!(r#"{{"cmd":"engine.route","job":"{job}"}}"#))
            .unwrap();
    }
    assert!(events.wait_count("engine.routed", 3, Duration::from_secs(5)));
    for routed in events.all().iter().filter(|e| e["type"] == "engine.routed") {
        assert_ne!(routed["id"], "test-core-ml", "{routed}");
    }
    core.shutdown();
    events.assert_valid();
}
