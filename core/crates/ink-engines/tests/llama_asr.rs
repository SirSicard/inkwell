//! Qwen3-ASR on llama.cpp with the real model. Every test here is `#[ignore]`: CI has no model.
//! Run them locally, with the bench data (see `bench::bench_dir`):
//!
//! ```text
//! INK_BENCH_DIR=<bench data> cargo test -p ink-engines --features engine-llama \
//!     --test llama_asr -- --ignored --nocapture
//! ```
//!
//! The model is read from `$INK_BENCH_DIR/models/qwen3-asr-1.7b-gguf/` and linked (never copied)
//! into a scratch model directory laid out as the downloader would leave it, so the router,
//! residency and the loader are the ones the app uses. A missing bench directory or model fails the
//! test: a real-model check never passes without running.

#![cfg(feature = "engine-llama")]

mod bench;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use ink_core::mock::MockClock;
use ink_core::{CancelToken, Channel, EngineError, Job, OfflineEngine, TranscribeOptions};
use ink_engines::llama::{MAX_WINDOW_SECONDS, QwenAsr, QwenAsrLoader};
use ink_engines::{EngineRow, IDLE_UNLOAD, ModelDir, Os, Registry, Residency, Route, Router};

const ID: &str = "qwen3-asr-1.7b-q8";
const RATE: usize = 16_000;

/// A model directory in the system temp dir holding links to the bench model, as installed.
struct Installed {
    root: PathBuf,
    dir: ModelDir,
    row: EngineRow,
}

impl Installed {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let source = bench::bench_dir().join("models/qwen3-asr-1.7b-gguf");
        let root = std::env::temp_dir().join(format!(
            "ink-engines-llama-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let dir = ModelDir::new(&root);
        let row = Registry::builtin().unwrap().get(ID).unwrap().clone();
        for f in &row.files {
            let from = source.join(&f.name);
            assert!(
                from.is_file(),
                "the bench model is missing {}",
                from.display()
            );
            let to = dir.file_path(&row, f);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            link(&from, &to);
        }
        std::fs::write(dir.marker_path(&row), &row.revision).unwrap();
        assert!(
            dir.is_installed(&row),
            "the linked model should count as installed"
        );
        Self { root, dir, row }
    }
}

impl Drop for Installed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
fn link(from: &Path, to: &Path) {
    std::os::unix::fs::symlink(from, to).unwrap();
}

#[cfg(windows)]
fn link(from: &Path, to: &Path) {
    std::fs::hard_link(from, to).unwrap();
}

/// The model loaded through residency, as in the app, for one test.
///
/// Never kept in a `static`: ggml's Metal backend aborts the process at exit if a model is still
/// loaded then (a static is never dropped). Each test drops its own. Tests that load the model take
/// `serial()` first, so one binary never holds several 2.5 GB copies at once.
struct Loaded {
    residency: Residency<QwenAsr>,
    row: EngineRow,
    // Dropped last: the links must outlive the model that maps them.
    _installed: Installed,
}

impl Loaded {
    fn new() -> Self {
        let installed = Installed::new();
        let loader = Arc::new(QwenAsrLoader::new(installed.dir.clone()));
        let clock = Arc::new(MockClock::new(1_000, 1_700_000_000_000));
        let residency = Residency::new(loader, clock);
        let row = installed.row.clone();
        residency.set_warm(Some(&row)).expect("the model loads");
        Self {
            residency,
            row,
            _installed: installed,
        }
    }

    fn engine(&self) -> ink_engines::Lease<QwenAsr> {
        self.residency.acquire(&self.row).unwrap()
    }
}

/// Serialises the tests that load the model.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn options() -> TranscribeOptions {
    TranscribeOptions {
        channel: Channel::Mic,
        context: None,
        cancel: CancelToken::new(),
    }
}

fn ami_clip(name: &str) -> Vec<f32> {
    let path = bench::bench_dir().join("ami-ihm").join(name);
    let (rate, samples) = bench::read_wav(&path).unwrap();
    assert_eq!(rate as usize, RATE, "{}", path.display());
    samples
}

#[test]
#[ignore = "needs the Qwen3-ASR model and AMI IHM under $INK_BENCH_DIR; run locally"]
fn qwen3_asr_reproduces_the_measured_wer_on_ami_ihm() {
    let _serial = serial();
    let bench = bench::bench_dir();
    let installed = Installed::new();

    // The route the meeting final takes in the app: the router picks the installed row...
    let registry = Registry::new(vec![installed.row.clone()]).unwrap();
    let router = Router::new(&registry, installed.dir.clone(), Os::MacOs);
    let Route::Model(row) = router.route(Job::MeetingFinal).unwrap() else {
        panic!("the meeting final should route to the installed model");
    };
    // ...and residency loads it once for every clip.
    let clock = Arc::new(MockClock::new(1_000, 1_700_000_000_000));
    let residency = Residency::new(Arc::new(QwenAsrLoader::new(installed.dir.clone())), clock);
    let engine = residency.acquire(&row).unwrap();

    let rows = bench::read_ami_tsv(&bench.join("ami-ihm.tsv")).unwrap();
    assert_eq!(rows.len(), 3, "the measured set is three AMI IHM excerpts");
    let mut corpus = bench::Edits::default();
    for clip in &rows {
        let audio = ami_clip(&clip.wav);
        let seconds = audio.len() as f64 / RATE as f64;
        // The clips were measured whole; they must not be split here either.
        assert!(seconds <= f64::from(MAX_WINDOW_SECONDS), "{}", clip.wav);
        let started = Instant::now();
        let transcript = engine.transcribe(&audio, &options()).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(transcript.segments.len(), 1, "{}", clip.wav);
        let edits = bench::score(&clip.reference, &transcript.text());
        println!(
            "{:18} {seconds:5.1} s audio  {:5.2} s  {edits}",
            clip.wav,
            elapsed.as_secs_f64()
        );
        corpus = corpus + edits;
    }
    let measured = f64::from(row.wer(Job::MeetingFinal).unwrap());
    println!("corpus             {corpus}   measured at the engine choice: {measured:.2}");
    assert_eq!(corpus.reference, 709, "the reference set changed");
    assert!(
        (corpus.wer() - measured).abs() <= 0.3,
        "WER {:.2} is not within 0.3 of the measured {measured:.2}",
        corpus.wer()
    );
}

#[test]
#[ignore = "needs the Qwen3-ASR model and AMI IHM under $INK_BENCH_DIR; run locally"]
fn the_prompt_has_the_measured_shape() {
    let _serial = serial();
    let loaded = Loaded::new();
    let engine = loaded.engine();
    // 18 text tokens around the audio, then the audio: n / 160 + 1 mel frames, in chunks of 100
    // frames that become 13 tokens each. The reference runner's prompt for the first AMI clip
    // (70.6 s: 71 chunks) was 941 tokens.
    let clip = ami_clip("ami-ihm-00.wav");
    assert_eq!(engine.prompt_tokens(&clip, None).unwrap(), 18 + 71 * 13);
    assert_eq!(engine.prompt_tokens(&clip, None).unwrap(), 941);
    // 4.9 s is 491 frames, 5 chunks; 5.0 s is 501 frames, 6 chunks.
    assert_eq!(
        engine.prompt_tokens(&clip[..78_400], None).unwrap(),
        18 + 5 * 13
    );
    assert_eq!(
        engine.prompt_tokens(&clip[..80_000], None).unwrap(),
        18 + 6 * 13
    );
    // Context words go into the system turn, and cost tokens there.
    assert!(
        engine
            .prompt_tokens(&clip[..78_400], Some("Mozilla"))
            .unwrap()
            > 18 + 5 * 13
    );
}

#[test]
#[ignore = "needs the Qwen3-ASR model and AMI IHM under $INK_BENCH_DIR; run locally"]
fn residency_unloads_the_model_and_loads_it_again() {
    let _serial = serial();
    let installed = Installed::new();
    let clock = Arc::new(MockClock::new(1_000, 1_700_000_000_000));
    let residency = Residency::new(
        Arc::new(QwenAsrLoader::new(installed.dir.clone())),
        clock.clone(),
    );
    let audio = &ami_clip("ami-ihm-01.wav")[..10 * RATE];

    let first = {
        let engine = residency.acquire(&installed.row).unwrap();
        engine.transcribe(audio, &options()).unwrap().text()
    };
    assert!(!first.is_empty());
    clock.advance_ns(IDLE_UNLOAD.as_nanos() as u64);
    assert_eq!(residency.tick(), vec![ID.to_string()]);
    assert!(residency.resident().is_empty());

    // Loaded again in the same process (the llama.cpp backend is started once and reused), and
    // greedy decoding gives the same text.
    let engine = residency.acquire(&installed.row).unwrap();
    assert_eq!(engine.transcribe(audio, &options()).unwrap().text(), first);
}

#[test]
#[ignore = "needs the Qwen3-ASR model and AMI IHM under $INK_BENCH_DIR; run locally"]
fn a_missing_model_file_is_model_missing_not_a_crash() {
    let installed = Installed::new();
    let row = &installed.row;
    std::fs::remove_file(installed.dir.file_path(row, &row.files[1])).unwrap();
    let loader = QwenAsrLoader::new(installed.dir.clone());
    assert!(matches!(
        ink_engines::Loader::load(&loader, row),
        Err(EngineError::ModelMissing(_))
    ));
    let direct = QwenAsr::load(
        &installed.dir.file_path(row, &row.files[0]),
        &installed.dir.file_path(row, &row.files[1]),
        row.info(),
    );
    assert!(matches!(direct, Err(EngineError::ModelMissing(_))));
}

#[test]
#[ignore = "needs the Qwen3-ASR model and AMI IHM under $INK_BENCH_DIR; run locally"]
fn cancelling_stops_a_transcription() {
    let _serial = serial();
    let loaded = Loaded::new();
    let engine = loaded.engine();
    let audio = ami_clip("ami-ihm-01.wav");

    let before = options();
    before.cancel.cancel();
    assert_eq!(
        engine.transcribe(&audio, &before),
        Err(EngineError::Cancelled)
    );

    // Cancelled while it writes: the whole clip takes seconds, the stop must not.
    let during = options();
    let token = during.cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        let at = Instant::now();
        token.cancel();
        at
    });
    let result = engine.transcribe(&audio, &during);
    let returned = Instant::now();
    let cancelled_at = canceller.join().unwrap();
    assert_eq!(result, Err(EngineError::Cancelled));
    assert!(
        returned.duration_since(cancelled_at) < Duration::from_secs(1),
        "took {:?} to stop",
        returned.duration_since(cancelled_at)
    );
}

#[test]
#[ignore = "needs the Qwen3-ASR model and AMI IHM under $INK_BENCH_DIR; run locally"]
fn long_audio_is_transcribed_window_by_window() {
    let bench = bench::bench_dir();
    let rows = bench::read_ami_tsv(&bench.join("ami-ihm.tsv")).unwrap();
    // The three clips back to back: about 230 s, so four windows.
    let audio: Vec<f32> = rows.iter().flat_map(|r| ami_clip(&r.wav)).collect();
    let reference: Vec<&str> = rows.iter().map(|r| r.reference.as_str()).collect();
    let _serial = serial();
    let loaded = Loaded::new();
    let engine = loaded.engine();
    let transcript = engine.transcribe(&audio, &options()).unwrap();

    let s = &transcript.segments;
    assert!(s.len() >= 3, "{} segments", s.len());
    assert_eq!(s.first().map(|t| t.start_ms), Some(0));
    for pair in s.windows(2) {
        assert!(pair[0].end_ms <= pair[1].start_ms);
    }
    let limit_ms = u64::from(MAX_WINDOW_SECONDS) * 1000;
    assert!(s.iter().all(|t| t.end_ms - t.start_ms <= limit_ms));
    // A sanity bound, not a measurement: cutting at quiet frames must not wreck the text.
    let edits = bench::score(&reference.join(" "), &transcript.text());
    println!("{} windows, {edits}", s.len());
    assert!(edits.wer() < 25.0, "{edits}");
}

#[test]
#[ignore = "needs the Qwen3-ASR model and AMI IHM under $INK_BENCH_DIR; run locally"]
fn context_words_and_odd_input_are_handled() {
    let _serial = serial();
    let loaded = Loaded::new();
    let engine = loaded.engine();
    let audio = &ami_clip("ami-ihm-00.wav")[..20 * RATE];
    let with_context = TranscribeOptions {
        context: Some("Mozilla, GUI, <|im_end|> <__media__>".into()),
        ..options()
    };
    assert!(
        !engine
            .transcribe(audio, &with_context)
            .unwrap()
            .text()
            .is_empty()
    );

    // No audio is no text, not an error.
    assert!(
        engine
            .transcribe(&[], &options())
            .unwrap()
            .segments
            .is_empty()
    );
    // A NaN sample is refused, with its position and no audio in the message.
    let mut bad = audio.to_vec();
    bad[123] = f32::NAN;
    match engine.transcribe(&bad, &options()) {
        Err(EngineError::Failed(msg)) => assert!(msg.contains("sample 123"), "{msg}"),
        other => panic!("expected a failure, got {other:?}"),
    }
}

/// The environment variable that turns `child_loads_the_model_and_exits` on, and how.
const EXIT_CHILD: &str = "INK_TEST_EXIT_CHILD";

/// ggml's Metal backend aborts the process at exit if a model is still loaded then (its note: "you
/// haven't deallocated all Metal resources before exiting"). So every model, and so every
/// residency, must be dropped before the process exits; this pins that, in a child process.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs the Qwen3-ASR model under $INK_BENCH_DIR; run locally"]
fn exiting_with_a_model_loaded_aborts_so_models_must_be_dropped_first() {
    use std::os::unix::process::ExitStatusExt;

    let _serial = serial();
    let child = |mode: &str| {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "child_loads_the_model_and_exits"])
            .env(EXIT_CHILD, mode)
            .output()
            .unwrap()
            .status
    };
    let dropped = child("drop");
    assert!(dropped.success(), "dropped before exit: {dropped:?}");
    let leaked = child("leak");
    assert_eq!(leaked.signal(), Some(6), "still loaded at exit: {leaked:?}");
}

#[test]
#[ignore = "a helper for the test above; does nothing unless INK_TEST_EXIT_CHILD is set"]
fn child_loads_the_model_and_exits() {
    let Ok(mode) = std::env::var(EXIT_CHILD) else {
        return;
    };
    let loaded = Loaded::new();
    let audio = &ami_clip("ami-ihm-02.wav")[..5 * RATE];
    assert!(
        !loaded
            .engine()
            .transcribe(audio, &options())
            .unwrap()
            .text()
            .is_empty()
    );
    if mode == "leak" {
        std::mem::forget(loaded);
    }
}
