//! Residency, with a load-counting loader and a mock clock: no sleeps, no timers, no models.

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use common::{Gate, row};
use ink_core::mock::{MockClock, MockEngine};
use ink_core::{
    CancelToken, Channel, Clock, EngineError, Job, OfflineEngine, TimedText, TranscribeOptions,
    Transcript,
};
use ink_engines::{EngineRow, IDLE_UNLOAD, Lease, Loader, Residency};

const MINUTE_NS: u64 = 60_000_000_000;
const IDLE_NS: u64 = IDLE_UNLOAD.as_secs() * 1_000_000_000;

#[derive(Default)]
struct Counts {
    loads: AtomicUsize,
    unloads: AtomicUsize,
    /// Copies alive right now, and the most ever alive at once, per model id.
    live: Mutex<HashMap<String, (usize, usize)>>,
}

impl Counts {
    fn loads(&self) -> usize {
        self.loads.load(Ordering::SeqCst)
    }
    fn unloads(&self) -> usize {
        self.unloads.load(Ordering::SeqCst)
    }
    fn max_live(&self, id: &str) -> usize {
        self.live.lock().unwrap().get(id).map_or(0, |&(_, max)| max)
    }
}

/// A loaded model: a mock engine that counts itself alive.
struct Model {
    id: String,
    engine: MockEngine,
    counts: Arc<Counts>,
}

impl Model {
    fn new(id: &str, counts: &Arc<Counts>) -> Self {
        let mut live = counts.live.lock().unwrap();
        let entry = live.entry(id.to_string()).or_default();
        entry.0 += 1;
        entry.1 = entry.1.max(entry.0);
        Self {
            id: id.into(),
            engine: MockEngine::new(id, &[Job::DictationFinal, Job::MeetingFinal]),
            counts: Arc::clone(counts),
        }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        self.counts.unloads.fetch_add(1, Ordering::SeqCst);
        if let Some(entry) = self.counts.live.lock().unwrap().get_mut(&self.id) {
            entry.0 -= 1;
        }
    }
}

#[derive(Default)]
struct CountingLoader {
    counts: Arc<Counts>,
    /// Loads block until this opens, when set; `entered` hears about each one first.
    gate: Mutex<Option<(Arc<Gate>, mpsc::Sender<String>)>>,
    /// Ids whose load fails.
    failing: Mutex<Vec<String>>,
    /// Ids whose load panics.
    panicking: Mutex<Vec<String>>,
}

impl Loader<Model> for CountingLoader {
    fn load(&self, row: &EngineRow) -> Result<Model, EngineError> {
        self.counts.loads.fetch_add(1, Ordering::SeqCst);
        let gate = self.gate.lock().unwrap().clone();
        if let Some((gate, entered)) = gate {
            let _ = entered.send(row.id.clone());
            gate.wait();
        }
        if self.panicking.lock().unwrap().contains(&row.id) {
            panic!("synthetic loader panic");
        }
        if self.failing.lock().unwrap().contains(&row.id) {
            return Err(EngineError::ModelMissing(row.id.clone()));
        }
        Ok(Model::new(&row.id, &self.counts))
    }
}

struct Setup {
    clock: Arc<MockClock>,
    loader: Arc<CountingLoader>,
    res: Arc<Residency<Model>>,
}

fn setup() -> Setup {
    let clock = Arc::new(MockClock::new(1_000, 1_700_000_000_000));
    let loader = Arc::new(CountingLoader::default());
    let res = Arc::new(Residency::new(
        loader.clone() as Arc<dyn Loader<Model>>,
        clock.clone() as Arc<dyn Clock>,
    ));
    Setup { clock, loader, res }
}

impl Setup {
    fn counts(&self) -> &Counts {
        &self.loader.counts
    }
    fn advance_min(&self, minutes: u64) {
        self.clock.advance_ns(minutes * MINUTE_NS);
    }
}

/// Receives with a bound, so a regression fails the test instead of hanging it.
fn recv<T>(rx: &mpsc::Receiver<T>) -> T {
    rx.recv_timeout(Duration::from_secs(20))
        .expect("timed out waiting for another thread")
}

fn dictation() -> EngineRow {
    row("synthetic-dictation", &[(Job::DictationFinal, 5.0)])
}

fn meeting() -> EngineRow {
    row("synthetic-meeting", &[(Job::MeetingFinal, 9.0)])
}

#[test]
fn the_idle_limit_is_five_minutes() {
    assert_eq!(IDLE_UNLOAD, Duration::from_secs(300));
}

#[test]
fn the_dictation_model_stays_warm() {
    let s = setup();
    s.res.set_warm(Some(&dictation())).unwrap();
    assert_eq!(s.counts().loads(), 1, "warming loads it at once");
    assert_eq!(s.res.warm().as_deref(), Some("synthetic-dictation"));

    for _ in 0..60 {
        s.advance_min(1);
        assert!(s.res.tick().is_empty());
    }
    // Used after an hour: still the one copy.
    let lease = s.res.acquire(&dictation()).unwrap();
    assert_eq!(lease.id(), "synthetic-dictation");
    drop(lease);
    s.advance_min(60);
    assert!(s.res.tick().is_empty());
    assert_eq!((s.counts().loads(), s.counts().unloads()), (1, 0));
    assert_eq!(s.res.resident(), vec!["synthetic-dictation".to_string()]);
}

#[test]
fn the_meeting_model_loads_at_meeting_end_and_unloads_after_five_idle_minutes() {
    let s = setup();
    s.res.set_warm(Some(&dictation())).unwrap();

    // During the meeting nothing loads the final-pass model.
    s.advance_min(45);
    s.res.tick();
    assert_eq!(s.res.resident(), vec!["synthetic-dictation".to_string()]);
    assert_eq!(s.counts().loads(), 1);

    // Meeting end: the final pass asks for it.
    let lease = s.res.acquire(&meeting()).unwrap();
    assert_eq!(s.counts().loads(), 2);
    drop(lease);

    s.clock.advance_ns(IDLE_NS - 1);
    assert!(s.res.tick().is_empty(), "one nanosecond short of idle");
    s.clock.advance_ns(1);
    assert_eq!(s.res.tick(), vec!["synthetic-meeting".to_string()]);
    assert_eq!(s.counts().unloads(), 1);
    assert_eq!(s.res.resident(), vec!["synthetic-dictation".to_string()]);

    // The next meeting loads it again.
    drop(s.res.acquire(&meeting()).unwrap());
    assert_eq!(s.counts().loads(), 3);
}

#[test]
fn idle_time_starts_when_the_last_lease_is_dropped() {
    let s = setup();
    let lease = s.res.acquire(&meeting()).unwrap();
    // A final pass that takes twenty minutes.
    s.advance_min(20);
    assert!(s.res.tick().is_empty(), "a held model is never unloaded");
    drop(lease);
    s.advance_min(4);
    assert!(s.res.tick().is_empty(), "idle for four minutes only");
    s.advance_min(1);
    assert_eq!(s.res.tick(), vec!["synthetic-meeting".to_string()]);
}

#[test]
fn a_model_held_by_any_lease_is_never_unloaded() {
    let s = setup();
    let a = s.res.acquire(&meeting()).unwrap();
    let b = s.res.acquire(&meeting()).unwrap();
    assert_eq!(s.counts().loads(), 1);
    drop(a);
    s.advance_min(30);
    assert!(s.res.tick().is_empty(), "b still holds it");
    drop(b);
    s.advance_min(5);
    assert_eq!(s.res.tick().len(), 1);
    assert_eq!(s.counts().unloads(), 1);
}

#[test]
fn one_model_serving_both_jobs_is_loaded_once() {
    // One resident model serves dictation and the meeting final pass (architecture rule 10).
    let s = setup();
    let both = row(
        "synthetic-both",
        &[(Job::DictationFinal, 5.0), (Job::MeetingFinal, 9.0)],
    );
    s.res.set_warm(Some(&both)).unwrap();
    let lease = s.res.acquire(&both).unwrap();
    drop(lease);
    s.advance_min(30);
    assert!(s.res.tick().is_empty());
    assert_eq!(s.counts().loads(), 1);
    assert_eq!(s.counts().max_live("synthetic-both"), 1);
}

#[test]
fn unwarming_starts_the_idle_clock() {
    let s = setup();
    s.res.set_warm(Some(&dictation())).unwrap();
    s.advance_min(30);
    // The user picked another dictation model.
    let other = row("synthetic-dictation-2", &[(Job::DictationFinal, 4.0)]);
    s.res.set_warm(Some(&other)).unwrap();
    assert_eq!(s.res.warm().as_deref(), Some("synthetic-dictation-2"));
    s.advance_min(4);
    assert!(s.res.tick().is_empty(), "the old one gets its five minutes");
    s.advance_min(1);
    assert_eq!(s.res.tick(), vec!["synthetic-dictation".to_string()]);

    s.res.set_warm(None).unwrap();
    assert_eq!(s.res.warm(), None);
    s.advance_min(5);
    assert_eq!(s.res.tick(), vec!["synthetic-dictation-2".to_string()]);
    assert!(s.res.resident().is_empty());
    assert_eq!((s.counts().loads(), s.counts().unloads()), (2, 2));
}

#[test]
fn a_lease_calls_the_loaded_engine() {
    let s = setup();
    let lease: Lease<Model> = s.res.acquire(&dictation()).unwrap();
    let audio = [0.1f32, 0.2, 0.3];
    let expected = Transcript {
        segments: vec![TimedText {
            start_ms: 0,
            end_ms: 500,
            text: "synthetic".into(),
        }],
    };
    lease.engine.add_fixture(&audio, expected.clone());
    let options = TranscribeOptions {
        channel: Channel::Mic,
        context: None,
        cancel: CancelToken::new(),
    };
    assert_eq!(lease.engine.transcribe(&audio, &options).unwrap(), expected);
}

#[test]
fn a_failed_load_leaves_nothing_resident_and_can_be_retried() {
    let s = setup();
    s.loader
        .failing
        .lock()
        .unwrap()
        .push("synthetic-meeting".into());
    assert_eq!(
        s.res.acquire(&meeting()).err(),
        Some(EngineError::ModelMissing("synthetic-meeting".into()))
    );
    assert!(s.res.resident().is_empty());

    s.loader.failing.lock().unwrap().clear();
    drop(s.res.acquire(&meeting()).unwrap());
    assert_eq!(s.res.resident(), vec!["synthetic-meeting".to_string()]);
    assert_eq!(s.counts().loads(), 2);
}

#[test]
fn a_panicking_loader_does_not_wedge_other_threads() {
    let s = setup();
    s.loader
        .panicking
        .lock()
        .unwrap()
        .push("synthetic-meeting".into());
    let res = Arc::clone(&s.res);
    let crashed = std::thread::spawn(move || res.acquire(&meeting()).map(|_| ())).join();
    assert!(crashed.is_err(), "the loader panicked");

    s.loader.panicking.lock().unwrap().clear();
    let (tx, rx) = mpsc::channel();
    let res = Arc::clone(&s.res);
    std::thread::spawn(move || {
        let _ = tx.send(res.acquire(&meeting()).map(|l| l.id().to_string()));
    });
    let got = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("acquire hung behind a crashed load");
    assert_eq!(got.unwrap(), "synthetic-meeting");
}

#[test]
fn concurrent_acquires_load_one_copy() {
    const THREADS: usize = 16;
    let s = setup();
    let gate = Arc::new(Gate::default());
    let (entered_tx, entered_rx) = mpsc::channel();
    *s.loader.gate.lock().unwrap() = Some((Arc::clone(&gate), entered_tx));

    let start = Arc::new(Barrier::new(THREADS + 1));
    let release = Arc::new(Gate::default());
    let (held_tx, held_rx) = mpsc::channel();
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let (res, start, release, held) = (
                Arc::clone(&s.res),
                Arc::clone(&start),
                Arc::clone(&release),
                held_tx.clone(),
            );
            std::thread::spawn(move || {
                start.wait();
                let lease = res.acquire(&meeting()).unwrap();
                held.send(()).unwrap();
                // Hold every lease at once.
                release.wait();
                drop(lease);
            })
        })
        .collect();
    start.wait();
    // One thread is inside the load; the rest are queued behind it or not yet in `acquire`.
    assert_eq!(recv(&entered_rx), "synthetic-meeting");
    gate.open();
    for _ in 0..THREADS {
        recv(&held_rx);
    }
    release.open();
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(s.counts().loads(), 1);
    assert_eq!(s.counts().max_live("synthetic-meeting"), 1);
    assert!(entered_rx.try_recv().is_err(), "no second load started");
}

#[test]
fn never_two_copies_under_concurrent_use_and_unloading() {
    let s = setup();
    let rows = [dictation(), meeting()];
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // A thread that keeps moving time and unloading idle models while workers acquire.
    let ticker = {
        let (res, clock, stop) = (Arc::clone(&s.res), Arc::clone(&s.clock), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                clock.advance_ns(IDLE_NS);
                res.tick();
            }
        })
    };
    let workers: Vec<_> = (0..8)
        .map(|t| {
            let (res, rows) = (Arc::clone(&s.res), rows.clone());
            std::thread::spawn(move || {
                for i in 0..300 {
                    let lease = res.acquire(&rows[(t + i) % 2]).unwrap();
                    assert_eq!(lease.id(), rows[(t + i) % 2].id);
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    ticker.join().unwrap();

    for r in &rows {
        assert_eq!(s.counts().max_live(&r.id), 1, "{} had two copies", r.id);
    }
    // The ticker really did unload and the workers reload, or this proved nothing.
    assert!(s.counts().unloads() > 0, "nothing was ever unloaded");
    // Every load is matched by an unload or a model still resident.
    assert_eq!(
        s.counts().loads(),
        s.counts().unloads() + s.res.resident().len()
    );
}

#[test]
fn no_lock_is_held_across_a_load() {
    let s = setup();
    let gate = Arc::new(Gate::default());
    let (entered_tx, entered_rx) = mpsc::channel();
    *s.loader.gate.lock().unwrap() = Some((Arc::clone(&gate), entered_tx));

    // A slow load of the meeting model.
    let res = Arc::clone(&s.res);
    let slow = std::thread::spawn(move || res.acquire(&meeting()).map(|l| l.id().to_string()));
    assert_eq!(recv(&entered_rx), "synthetic-meeting");
    *s.loader.gate.lock().unwrap() = None;

    // Meanwhile: another model loads, and tick, resident and warm answer.
    let (tx, rx) = mpsc::channel();
    let res = Arc::clone(&s.res);
    std::thread::spawn(move || {
        let lease = res.acquire(&dictation()).map(|l| l.id().to_string());
        let _ = tx.send((lease, res.tick(), res.resident(), res.warm()));
    });
    let (lease, ticked, resident, warm) = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("blocked behind another model's load: a lock is held across it");
    assert_eq!(lease.unwrap(), "synthetic-dictation");
    assert!(ticked.is_empty());
    assert_eq!(resident, vec!["synthetic-dictation".to_string()]);
    assert_eq!(warm, None);

    gate.open();
    assert_eq!(slow.join().unwrap().unwrap(), "synthetic-meeting");
}

#[test]
fn residency_and_leases_cross_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send<T: Send>() {}
    assert_send_sync::<Residency<Model>>();
    assert_send::<Lease<Model>>();
}

// On-demand unload, for an update that replaces a model's files.

#[test]
fn unload_drops_an_idle_model_at_once_and_clears_its_warm_state() {
    let s = setup();
    s.res.set_warm(Some(&dictation())).unwrap();
    drop(s.res.acquire(&meeting()).unwrap());

    // No five-minute wait, warm or not.
    s.res.unload("synthetic-dictation").unwrap();
    assert_eq!(s.res.warm(), None, "no longer kept warm");
    s.res.unload("synthetic-meeting").unwrap();
    assert!(s.res.resident().is_empty());
    assert_eq!(s.counts().unloads(), 2);

    // The next use loads it again (from the new files, after an update).
    drop(s.res.acquire(&dictation()).unwrap());
    assert_eq!(s.counts().loads(), 3);
    assert_eq!(s.counts().max_live("synthetic-dictation"), 1);
}

#[test]
fn unload_is_refused_while_a_lease_holds_the_model() {
    let s = setup();
    s.res.set_warm(Some(&dictation())).unwrap();
    let lease = s.res.acquire(&dictation()).unwrap();

    match s.res.unload("synthetic-dictation") {
        Err(EngineError::Failed(msg)) => {
            assert!(
                msg.contains("synthetic-dictation") && msg.contains("in use"),
                "{msg}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    // A refusal changes nothing.
    assert_eq!(s.res.resident(), vec!["synthetic-dictation".to_string()]);
    assert_eq!(s.res.warm().as_deref(), Some("synthetic-dictation"));
    assert_eq!(s.counts().unloads(), 0);

    drop(lease);
    s.res.unload("synthetic-dictation").unwrap();
    assert!(s.res.resident().is_empty());
}

#[test]
fn unloading_an_unknown_id_is_ok_and_changes_nothing() {
    let s = setup();
    s.res.set_warm(Some(&dictation())).unwrap();
    s.res.unload("never-loaded").unwrap();
    assert_eq!(s.res.resident(), vec!["synthetic-dictation".to_string()]);
    assert_eq!(s.res.warm().as_deref(), Some("synthetic-dictation"));
    assert_eq!(s.counts().unloads(), 0);
}

/// A model whose drop calls back into the residency that holds it.
struct Reentrant {
    residency: Arc<std::sync::OnceLock<std::sync::Weak<Residency<Reentrant>>>>,
    seen: mpsc::Sender<(Vec<String>, Option<String>)>,
}

impl Drop for Reentrant {
    fn drop(&mut self) {
        if let Some(res) = self.residency.get().and_then(std::sync::Weak::upgrade) {
            let _ = self.seen.send((res.resident(), res.warm()));
        }
    }
}

struct ReentrantLoader {
    residency: Arc<std::sync::OnceLock<std::sync::Weak<Residency<Reentrant>>>>,
    seen: mpsc::Sender<(Vec<String>, Option<String>)>,
}

impl Loader<Reentrant> for ReentrantLoader {
    fn load(&self, _row: &EngineRow) -> Result<Reentrant, EngineError> {
        Ok(Reentrant {
            residency: Arc::clone(&self.residency),
            seen: self.seen.clone(),
        })
    }
}

#[test]
fn unload_drops_the_model_outside_the_lock() {
    let handle = Arc::new(std::sync::OnceLock::new());
    let (seen_tx, seen_rx) = mpsc::channel();
    let res = Arc::new(Residency::new(
        Arc::new(ReentrantLoader {
            residency: Arc::clone(&handle),
            seen: seen_tx,
        }) as Arc<dyn Loader<Reentrant>>,
        Arc::new(MockClock::new(1_000, 1_700_000_000_000)) as Arc<dyn Clock>,
    ));
    let _ = handle.set(Arc::downgrade(&res));
    res.set_warm(Some(&dictation())).unwrap();

    let (done_tx, done_rx) = mpsc::channel();
    let worker = Arc::clone(&res);
    std::thread::spawn(move || {
        let _ = done_tx.send(worker.unload("synthetic-dictation"));
    });
    // A drop run under the lock would deadlock here on its own call into residency.
    recv(&done_rx).unwrap();
    let (resident, warm) = recv(&seen_rx);
    assert!(
        resident.is_empty(),
        "mid-unload it is no longer listed as resident"
    );
    assert_eq!(warm, None, "warm was cleared before the drop");
    assert!(res.resident().is_empty());
}
