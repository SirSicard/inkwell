//! Managing models from the screens: `model.cancel` stops a download, running or queued, without
//! waiting behind it on the command thread, and keeps its part file for a resume; a download that
//! cannot fit is refused before a byte is fetched (`not_enough_space`); `model.remove` deletes a
//! model's files, never while a job or a call holds it (`model_in_use`). Synthetic models only.

mod common;

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use ink_core::CancelToken;
use ink_engines::{Downloader, EngineRow, Fetch, FetchError, Fetched, ModelDir, Registry};
use ink_ffi::runtime::{Core, Parts};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(10);

/// The synthetic file's size: forty 64 KiB chunks.
const SIZE: usize = 64 * 1024 * 40;

/// SHA-256 of [`data`].
const DATA_SHA256: &str = "e7ef5df249549b67da4bba7fb285f225c45843e57c799cd3af6387f6be994973";

fn data() -> Vec<u8> {
    (0..SIZE).map(|i| ((i * 31 + 7) % 251) as u8).collect()
}

/// A speech row whose one file is [`data`], not installed.
fn big_row(id: &str) -> EngineRow {
    let mut row = test_row(id);
    row.files[0].sha256 = DATA_SHA256.into();
    row.files[0].size = SIZE as u64;
    row
}

/// Serves [`data`], honouring ranges; slowly (a chunk every 20 ms) while `slow` is set. Records
/// the offset of every request.
struct Served {
    slow: Arc<AtomicBool>,
    offsets: Mutex<Vec<u64>>,
}

struct Body {
    data: Arc<Vec<u8>>,
    at: usize,
    slow: Arc<AtomicBool>,
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.slow.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let n = buf.len().min(64 * 1024).min(self.data.len() - self.at);
        buf[..n].copy_from_slice(&self.data[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

impl Fetch for Served {
    fn get(&self, _: &str, offset: u64) -> Result<Fetched, FetchError> {
        self.offsets.lock().unwrap().push(offset);
        Ok(Fetched {
            start: offset,
            total: Some(SIZE as u64),
            body: Box::new(Body {
                data: Arc::new(data()),
                at: usize::try_from(offset).unwrap(),
                slow: self.slow.clone(),
            }),
        })
    }
}

struct Rig {
    core: Option<Core>,
    events: Arc<Recorder>,
    served: Arc<Served>,
    models: ModelDir,
    local: Arc<MockLocalLoader>,
    _dir: TempDir,
}

impl Rig {
    /// A core over `rows` (with `installed` installed), downloading through [`Served`], with
    /// `system` for the free space.
    fn new(
        label: &str,
        rows: Vec<EngineRow>,
        installed: &[EngineRow],
        system: Arc<FakeSystem>,
    ) -> Self {
        let dir = TempDir::new(label);
        let models = ModelDir::new(dir.path().join("models"));
        for row in installed {
            install(&models, row);
        }
        let served = Arc::new(Served {
            slow: Arc::new(AtomicBool::new(true)),
            offsets: Mutex::default(),
        });
        let local = MockLocalLoader::new("Hello, world!");
        let loader = MockLoader::new(Behaviour::Say("hello world".into()));
        let (core, events) = start_parts(Parts {
            store: Arc::new(ink_store::SqliteStore::open_in_memory().unwrap()),
            clock: clock(),
            registry: Registry::new(rows).unwrap(),
            models: models.clone(),
            loader,
            installer: Arc::new(Downloader::new(served.clone(), models.clone())),
            data_dir: dir.path().to_owned(),
            permissions: Arc::new(ink_ffi::queries::NoPermissionProbe),
            local: local.parts(Some(system)),
            meetings: Default::default(),
        });
        Self {
            core: Some(core),
            events,
            served,
            models,
            local,
            _dir: dir,
        }
    }

    fn core(&self) -> &Core {
        self.core.as_ref().unwrap()
    }

    fn send(&self, cmd: Value) {
        self.core().command(&cmd.to_string()).unwrap();
    }

    fn ask(&self, mut cmd: Value, id: &str) -> Value {
        cmd["id"] = id.into();
        self.send(cmd);
        self.events
            .wait_for(WAIT, |v| {
                v["ref"] == id || (v["type"] == "command.failed" && v["id"] == id)
            })
            .unwrap_or_else(|| panic!("no answer to {id}: {:?}", self.events.types()))
    }

    fn finished(&self, next: &str) -> Value {
        self.events
            .wait_for(WAIT, |v| {
                v["type"] == "model.update_finished" && v["next"] == next
            })
            .unwrap_or_else(|| panic!("{next} never finished: {:?}", self.events.types()))
    }

    fn finish(mut self) {
        self.core.take().unwrap().shutdown();
        self.events.assert_valid();
    }
}

fn roomy() -> Arc<FakeSystem> {
    FakeSystem::new(Some(u64::MAX / 2))
}

#[test]
fn a_cancel_stops_a_running_download_at_once_and_the_next_one_resumes_it() {
    let row = big_row("test-big");
    let rig = Rig::new("cancel-running", vec![row.clone()], &[], roomy());
    rig.send(json!({"cmd": "model.update", "model": "test-big", "next": "test-big"}));
    rig.events.wait_type("model.update_started", WAIT);
    // A first mebibyte on disk.
    rig.events
        .wait_for(WAIT, |v| {
            v["type"] == "model.update_progress" && v["done_bytes"].as_u64() > Some(0)
        })
        .expect("under way");

    // Answered on the queries thread while the download holds the command thread.
    let asked = Instant::now();
    rig.send(json!({"cmd": "model.cancel", "model": "test-big", "id": "x1"}));
    let finished = rig.finished("test-big");
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "stopped at its next chunk, not at the end of the download"
    );
    assert_eq!(finished["ok"], false, "{finished}");
    assert_eq!(finished["cancelled"], true, "{finished}");
    assert!(!rig.models.is_installed(&row));
    let part = rig.models.part_path(&row, &row.files[0]);
    let kept = std::fs::metadata(&part)
        .expect("the part file is kept")
        .len();
    assert!(kept > 0 && kept < SIZE as u64, "{kept}");

    // The next download resumes from the part file and installs it.
    rig.served.slow.store(false, Ordering::SeqCst);
    rig.send(json!({"cmd": "model.update", "model": "test-big", "next": "test-big"}));
    let again = rig
        .events
        .wait_for(WAIT, |v| {
            v["type"] == "model.update_finished" && v["ok"] == true
        })
        .expect("installed");
    assert_eq!(again["cancelled"], false);
    assert!(rig.models.is_installed(&row));
    let offsets = rig.served.offsets.lock().unwrap().clone();
    assert_eq!(
        offsets.last(),
        Some(&kept),
        "resumed from the part: {offsets:?}"
    );
    rig.finish();
}

#[test]
fn a_cancel_of_a_queued_download_ends_it_before_it_starts() {
    let first = big_row("test-big");
    let second = big_row("test-big-2");
    let rig = Rig::new(
        "cancel-queued",
        vec![first.clone(), second.clone()],
        &[],
        roomy(),
    );
    rig.send(json!({"cmd": "model.update", "model": "test-big", "next": "test-big"}));
    rig.events.wait_type("model.update_started", WAIT);
    rig.send(json!({"cmd": "model.update", "model": "test-big-2", "next": "test-big-2"}));

    rig.send(json!({"cmd": "model.cancel", "model": "test-big-2"}));
    let queued = rig.finished("test-big-2");
    assert_eq!(
        (&queued["ok"], &queued["cancelled"]),
        (&json!(false), &json!(true)),
        "{queued}"
    );
    assert_eq!(
        rig.events.count("model.update_finished"),
        1,
        "the running one goes on"
    );

    rig.send(json!({"cmd": "model.cancel", "model": "test-big"}));
    assert_eq!(rig.finished("test-big")["cancelled"], true);
    // The queued one, reached on the command thread, does nothing more.
    rig.send(json!({"cmd": "models.list", "id": "l1"}));
    rig.events.wait_for(WAIT, |v| v["ref"] == "l1").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let started: Vec<Value> = rig
        .events
        .all()
        .into_iter()
        .filter(|e| e["type"] == "model.update_started")
        .collect();
    assert_eq!(
        started.len(),
        1,
        "the queued one never started: {started:?}"
    );
    assert_eq!(rig.events.count("model.update_finished"), 2);
    assert!(
        !rig.models.root().join("test-big-2").exists(),
        "nothing written"
    );
    rig.finish();
}

#[test]
fn a_cancel_with_no_download_says_so() {
    let rig = Rig::new("cancel-none", vec![big_row("test-big")], &[], roomy());
    let failed = rig.ask(json!({"cmd": "model.cancel", "model": "test-big"}), "x1");
    assert_eq!(failed["type"], "command.failed");
    assert_eq!(failed["code"], "not_downloading");
    rig.finish();
}

#[test]
fn a_download_that_cannot_fit_fetches_nothing_and_changes_nothing() {
    let row = big_row("test-big");
    let system = FakeSystem::new(Some(100));
    let rig = Rig::new("no-space", vec![row.clone()], &[], system.clone());
    let failed = rig.ask(
        json!({"cmd": "model.update", "model": "test-big", "next": "test-big"}),
        "u1",
    );
    assert_eq!(failed["type"], "command.failed", "{failed}");
    assert_eq!(failed["command"], "model.update");
    assert_eq!(failed["code"], "not_enough_space");
    assert_eq!(
        failed["needed_bytes"],
        SIZE as u64 + ink_ffi::models::SPACE_MARGIN
    );
    assert_eq!(failed["free_bytes"], 100);
    assert!(
        rig.served.offsets.lock().unwrap().is_empty(),
        "nothing fetched"
    );
    assert_eq!(rig.events.count("model.update_started"), 0);
    assert!(
        !rig.models.root().join("test-big").exists(),
        "nothing written"
    );

    // With the room, it goes ahead: what is already on disk counts.
    *system.free.lock().unwrap() = Some(SIZE as u64 + ink_ffi::models::SPACE_MARGIN);
    rig.served.slow.store(false, Ordering::SeqCst);
    rig.send(json!({"cmd": "model.update", "model": "test-big", "next": "test-big"}));
    assert_eq!(rig.finished("test-big")["ok"], true);
    rig.finish();
}

#[test]
fn free_space_the_os_cannot_say_never_blocks_a_download() {
    let row = big_row("test-big");
    let rig = Rig::new("space-unknown", vec![row], &[], FakeSystem::new(None));
    rig.served.slow.store(false, Ordering::SeqCst);
    rig.send(json!({"cmd": "model.update", "model": "test-big", "next": "test-big"}));
    assert_eq!(rig.finished("test-big")["ok"], true);
    rig.finish();
}

#[test]
fn removing_a_model_deletes_its_files_and_answers_with_the_catalogue() {
    let row = test_row(ROW_ID);
    let rig = Rig::new(
        "remove",
        vec![row.clone()],
        std::slice::from_ref(&row),
        roomy(),
    );
    // The dictation model, warm: unloaded first.
    rig.send(json!({"cmd": "model.warm", "job": "dictation_final"}));
    rig.events.wait_type("model.warmed", WAIT);
    let listed = rig.ask(json!({"cmd": "model.remove", "model": ROW_ID}), "r1");
    assert_eq!(listed["type"], "models.listed", "{listed}");
    assert_eq!(listed["models"][0]["installed"], false);
    assert!(listed["free_bytes"].is_u64());
    assert!(!rig.models.root().join(ROW_ID).exists());
    assert!(rig.core().shared().residency.resident().is_empty());

    let unknown = rig.ask(json!({"cmd": "model.remove", "model": "nope"}), "r2");
    assert_eq!(unknown["type"], "command.failed");
    assert!(unknown.get("code").is_none(), "{unknown}");
    // Nothing there: removing it again is no failure.
    let again = rig.ask(json!({"cmd": "model.remove", "model": ROW_ID}), "r3");
    assert_eq!(again["type"], "models.listed");
    rig.finish();
}

#[test]
fn a_model_a_job_holds_is_never_removed() {
    let row = test_row(ROW_ID);
    let rig = Rig::new(
        "remove-held",
        vec![row.clone()],
        std::slice::from_ref(&row),
        roomy(),
    );
    let held = rig.core().shared().gate.enter(ROW_ID).unwrap();
    let refused = rig.ask(json!({"cmd": "model.remove", "model": ROW_ID}), "r1");
    assert_eq!(refused["type"], "command.failed");
    assert_eq!(refused["code"], "model_in_use");
    assert!(rig.models.is_installed(&row), "nothing deleted");
    drop(held);
    let listed = rig.ask(json!({"cmd": "model.remove", "model": ROW_ID}), "r2");
    assert_eq!(listed["type"], "models.listed");
    rig.finish();
}

#[test]
fn the_language_model_in_use_is_removed_only_between_calls_and_then_none_is_used() {
    let chat = language_row("test-chat", "Test Chat");
    let rig = Rig::new(
        "remove-local",
        vec![chat.clone()],
        std::slice::from_ref(&chat),
        roomy(),
    );
    // A mode pinned to it.
    let saved = rig.ask(
        json!({"cmd": "modes.save", "mode": {"id": "default", "polish_model": "engine:local"}}),
        "m1",
    );
    assert_eq!(saved["modes"][0]["polish_model_state"], "ready", "{saved}");

    // A call in flight holds it.
    let gate = Arc::new(Gate::default());
    *rig.local.gate.lock().unwrap() = Some(gate.clone());
    let llm = ink_ffi::engines::llm(rig.core().shared()).unwrap();
    let call = std::thread::spawn(move || {
        llm.complete(
            &ink_core::LlmRequest {
                system: String::new(),
                user: "synthetic".into(),
                max_tokens: 8,
                temperature: 0.0,
                json_schema: None,
            },
            &CancelToken::new(),
        )
    });
    let until = Instant::now() + WAIT;
    while rig.local.calls() == 0 {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    let refused = rig.ask(json!({"cmd": "model.remove", "model": "test-chat"}), "r1");
    assert_eq!(refused["code"], "model_in_use", "{refused}");
    assert!(rig.models.is_installed(&chat));
    gate.open();
    call.join().unwrap().unwrap();

    let listed = rig.ask(json!({"cmd": "model.remove", "model": "test-chat"}), "r2");
    assert_eq!(listed["type"], "models.listed", "{listed}");
    assert!(!rig.models.is_installed(&chat));
    assert!(rig.core().shared().local.resident().is_empty(), "unloaded");
    let modes = rig.ask(json!({"cmd": "modes.list"}), "m2");
    assert!(modes.get("setting_polish_model").is_none(), "{modes}");
    assert_eq!(modes["modes"][0]["polish_model_state"], "missing");
    assert!(rig.core().shared().llms.pick().is_none(), "no model at all");
    assert_eq!(rig.local.loads(), 1);
    rig.finish();
}
