//! `ink-bench latency`: dictation latency in replay, from the simulated key-up to the insertion
//! request. The plan's target: p50 ≤ 350 ms and p95 ≤ 700 ms on 5 s utterances, on the M5 Pro.
//!
//! Each run is half a second of room, a press, the utterance (a clip cut to `--seconds`), the
//! key-up at its end, then room, in 10 ms blocks through the real dictation chain: the gain stage
//! (with Silero when the build has it), the adaptive tail, the engine, the text stages and the
//! store, to an inserter that notes the moment it is asked to insert. After the key-up the audio is
//! paced in real time, so the tail waits as long as it would on the Mac; the hold is paced too
//! (`--pace realtime`, the default), so the engine's warm-up at the take's start has the time it
//! has in use. One priming take runs first and is not counted (it loads what is loaded once).
//!
//! **Left out:** capture (the device callback and the pump, a few ms), the event tap, polish (off:
//! Foundation Models belongs to the shell), and the paste itself (the insertion request is the
//! end). The app adds those; this measures the core.
//!
//! **Idle:** `--idle SECS` waits that long before every counted run: "the first dictation after N
//! minutes idle". `--warmup off` leaves out the warm-up at the take's start, to measure what it
//! saves. Measure the shipped configuration: the Mac shell runs with ggml's Metal residency off
//! (`GGML_METAL_NO_RESIDENCY=1`, which `scripts/dictation-latency.sh` sets).
//!
//! **Output:** a summary on stdout and one row per run (TSV) in `--out`, by default
//! `$INK_BENCH_DIR/out/s2.7/latency-<unix time>.tsv`. Rows hold timings and a word count, never
//! the words.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ink_core::{
    Clock, EngineError, EngineInfo, FocusInfo, FocusReader, InsertOutcome, Job, OfflineEngine,
    PlatformError, TextInserter, TimedText, TranscribeOptions, Transcript,
};
use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
use ink_pipeline::events::{DictationEvent, VadUnavailable};
use ink_pipeline::gain_stage::Vad;
use ink_pipeline::warm::{EngineWarmer, WARM_AFTER_IDLE, WARM_AUDIO};

/// 10 ms at 16 kHz.
const BLOCK: usize = 160;
const BLOCK_NS: u64 = 10_000_000;
/// Room before each press.
const ROOM_BEFORE: usize = 8_000;
/// The longest a run waits for its insertion after the key-up.
const GIVE_UP: Duration = Duration::from_secs(10);

/// How the hold is fed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// In real time: 10 ms of audio every 10 ms.
    Realtime,
    /// As fast as the chain takes it (the key-up's tail is always real time).
    Fast,
}

/// The engine to measure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineChoice {
    /// A stand-in that answers after a fixed delay: proves the harness, measures nothing real.
    Mock {
        /// How long each decode takes.
        delay: Duration,
    },
    /// Qwen3-ASR 1.7B Q8_0, the shipped dictation engine (needs the `engine-llama` feature), from
    /// this directory.
    Qwen {
        /// The directory holding the row's two GGUF files.
        dir: PathBuf,
    },
}

/// The voice-activity detector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VadChoice {
    /// Silero from this ONNX file (needs the `engine-silero` feature), as shipped.
    Silero(PathBuf),
    /// None: the gain stage's fallback.
    None,
}

/// What to run.
#[derive(Clone, Debug)]
pub struct Config {
    /// A directory of 16 kHz mono WAV clips (FLEURS English, say), used in name order.
    pub clips: PathBuf,
    /// Counted runs.
    pub runs: usize,
    /// Each utterance's length.
    pub seconds: f64,
    /// Idle before each counted run.
    pub idle: Duration,
    /// Whether a take's start warms the engine (as shipped).
    pub warmup: bool,
    /// How the hold is fed.
    pub pace: Pace,
    /// The engine.
    pub engine: EngineChoice,
    /// The VAD.
    pub vad: VadChoice,
    /// Where the rows go.
    pub out: Option<PathBuf>,
}

fn bench_dir() -> Option<PathBuf> {
    std::env::var_os("INK_BENCH_DIR").map(PathBuf::from)
}

impl Config {
    /// Reads the command line (after `latency`). Defaults: the FLEURS English dev clips under
    /// `$INK_BENCH_DIR`, 20 runs of 5 s, no idle, warm-up on, real-time pace, Qwen3-ASR and Silero
    /// when the build has them (else the mock engine and no VAD), rows under
    /// `$INK_BENCH_DIR/out/s2.7/`.
    pub fn from_args(args: &[String]) -> Result<Self, String> {
        let bench = bench_dir();
        let mut config = Self {
            clips: bench
                .as_ref()
                .map(|b| b.join("fleurs/en_us/dev"))
                .unwrap_or_default(),
            runs: 20,
            seconds: 5.0,
            idle: Duration::ZERO,
            warmup: true,
            pace: Pace::Realtime,
            engine: if cfg!(feature = "engine-llama") {
                EngineChoice::Qwen {
                    dir: bench
                        .as_ref()
                        .map(|b| b.join("models/qwen3-asr-1.7b-gguf"))
                        .unwrap_or_default(),
                }
            } else {
                EngineChoice::Mock {
                    delay: Duration::from_millis(200),
                }
            },
            vad: if cfg!(feature = "engine-silero") {
                VadChoice::Silero(
                    bench
                        .as_ref()
                        .map(|b| b.join("models/silero-vad/silero_vad_16k_op15.onnx"))
                        .unwrap_or_default(),
                )
            } else {
                VadChoice::None
            },
            out: bench.as_ref().map(|b| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                b.join(format!("out/s2.7/latency-{now}.tsv"))
            }),
        };
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let mut value = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| format!("{flag} needs a value"))
            };
            match flag.as_str() {
                "--clips" => config.clips = value()?.into(),
                "--runs" => config.runs = number(flag, &value()?)?,
                "--seconds" => {
                    config.seconds = value()?
                        .parse()
                        .ok()
                        .filter(|s: &f64| (0.5..=60.0).contains(s))
                        .ok_or("--seconds is 0.5 to 60")?;
                }
                "--idle" => config.idle = Duration::from_secs(number(flag, &value()?)? as u64),
                "--warmup" => config.warmup = on_off(flag, &value()?)?,
                "--pace" => {
                    config.pace = match value()?.as_str() {
                        "realtime" => Pace::Realtime,
                        "fast" => Pace::Fast,
                        _ => return Err("--pace is realtime or fast".into()),
                    }
                }
                "--engine" => {
                    config.engine = match value()?.as_str() {
                        "mock" => EngineChoice::Mock {
                            delay: Duration::from_millis(200),
                        },
                        "qwen" => EngineChoice::Qwen {
                            dir: bench
                                .as_ref()
                                .map(|b| b.join("models/qwen3-asr-1.7b-gguf"))
                                .unwrap_or_default(),
                        },
                        _ => return Err("--engine is mock or qwen".into()),
                    }
                }
                "--model" => {
                    config.engine = EngineChoice::Qwen {
                        dir: value()?.into(),
                    }
                }
                "--mock-delay-ms" => {
                    config.engine = EngineChoice::Mock {
                        delay: Duration::from_millis(number(flag, &value()?)? as u64),
                    }
                }
                "--vad" => {
                    config.vad = match value()?.as_str() {
                        "none" => VadChoice::None,
                        "silero" => VadChoice::Silero(
                            bench
                                .as_ref()
                                .map(|b| b.join("models/silero-vad/silero_vad_16k_op15.onnx"))
                                .unwrap_or_default(),
                        ),
                        path => VadChoice::Silero(path.into()),
                    }
                }
                "--out" => config.out = Some(value()?.into()),
                "--no-out" => config.out = None,
                other => return Err(format!("unknown option {other}")),
            }
        }
        if config.runs == 0 {
            return Err("--runs must be at least 1".into());
        }
        Ok(config)
    }
}

fn number(flag: &str, v: &str) -> Result<usize, String> {
    v.parse()
        .map_err(|_| format!("{flag} takes a whole number, not {v}"))
}

fn on_off(flag: &str, v: &str) -> Result<bool, String> {
    match v {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{flag} is on or off")),
    }
}

/// One counted run.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Its number, from 1.
    pub run: usize,
    /// The clip's file name.
    pub clip: String,
    /// Key-up to the insertion request.
    pub total_ms: f64,
    /// Key-up to the engine's call: the tail, then the gain stage and the VAD.
    pub before_decode_ms: f64,
    /// The engine's call.
    pub decode_ms: f64,
    /// The engine's answer to the insertion request: the text stages and the store.
    pub after_decode_ms: f64,
    /// Whether a warm-up ran during this take's hold.
    pub warmed: bool,
    /// Words inserted (never the words).
    pub words: usize,
}

/// Every counted run, and what they ran on.
#[derive(Clone, Debug)]
pub struct Report {
    /// The runs.
    pub rows: Vec<Row>,
    /// The engine's id.
    pub engine: String,
    /// Silero or the fallback.
    pub vad: String,
    /// The configuration's idle, warm-up and length, for the summary.
    pub idle: Duration,
    /// Warm-up on.
    pub warmup: bool,
    /// Utterance length.
    pub seconds: f64,
    /// Where the rows were written.
    pub out: Option<PathBuf>,
}

/// The `p`-th percentile (0 to 100) of `values`, nearest rank; `NaN` for none.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * v.len() as f64).ceil().max(1.0) as usize;
    v[rank.min(v.len()) - 1]
}

impl Report {
    fn column(&self, f: impl Fn(&Row) -> f64) -> Vec<f64> {
        self.rows.iter().map(f).collect()
    }

    /// The rows as TSV, with a header.
    pub fn tsv(&self) -> String {
        let mut out = String::from(
            "run\tclip\tengine\tvad\tidle_s\twarmup\twarmed\ttotal_ms\tbefore_decode_ms\tdecode_ms\tafter_decode_ms\twords\n",
        );
        for r in &self.rows {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{}\n",
                r.run,
                r.clip,
                self.engine,
                self.vad,
                self.idle.as_secs(),
                if self.warmup { "on" } else { "off" },
                r.warmed,
                r.total_ms,
                r.before_decode_ms,
                r.decode_ms,
                r.after_decode_ms,
                r.words
            ));
        }
        out
    }

    /// The summary printed at the end.
    pub fn summary(&self) -> String {
        let total = self.column(|r| r.total_ms);
        let p = |v: &[f64], q| percentile(v, q);
        format!(
            "dictation latency, key-up to the insertion request ({} runs of {:.1} s; engine {}; VAD {}; warm-up {}; idle {} s before each)\n  \
             total          p50 {:.0} ms  p95 {:.0} ms  min {:.0}  max {:.0}   (target p50 <= 350, p95 <= 700)\n  \
             before decode  p50 {:.0} ms   decode p50 {:.0} ms   after decode p50 {:.0} ms   warmed {}/{}\n  \
             rows: {}",
            self.rows.len(),
            self.seconds,
            self.engine,
            self.vad,
            if self.warmup { "on" } else { "off" },
            self.idle.as_secs(),
            p(&total, 50.0),
            p(&total, 95.0),
            total.iter().copied().fold(f64::INFINITY, f64::min),
            total.iter().copied().fold(0.0, f64::max),
            p(&self.column(|r| r.before_decode_ms), 50.0),
            p(&self.column(|r| r.decode_ms), 50.0),
            p(&self.column(|r| r.after_decode_ms), 50.0),
            self.rows.iter().filter(|r| r.warmed).count(),
            self.rows.len(),
            self.out
                .as_ref()
                .map_or_else(|| "not written".into(), |p| p.display().to_string()),
        )
    }
}

/// Wall time as the chain's clock.
struct RealClock(Instant);

impl Clock for RealClock {
    fn now_ns(&self) -> u64 {
        1_000_000_000 + u64::try_from(self.0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn unix_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// No app in front, nothing selected: the default mode.
struct NoFocus;

impl FocusReader for NoFocus {
    fn focus(&self) -> Result<FocusInfo, PlatformError> {
        Ok(FocusInfo::default())
    }

    fn selected_text(&self) -> Result<Option<String>, PlatformError> {
        Ok(None)
    }
}

/// Notes when it is asked to insert, and how many words; keeps no text.
#[derive(Default)]
struct Stopwatch {
    inserts: Mutex<Vec<(Instant, usize)>>,
}

impl TextInserter for Stopwatch {
    fn insert(&self, text: &str) -> Result<InsertOutcome, PlatformError> {
        let at = Instant::now();
        self.inserts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((at, text.split_whitespace().count()));
        Ok(InsertOutcome::Pasted)
    }
}

/// A decode: when it started and ended, and whether it was a warm-up.
#[derive(Clone, Copy, Debug)]
struct Call {
    start: Instant,
    end: Instant,
    warm_up: bool,
}

/// The engine, timed.
struct Timed {
    inner: Arc<dyn OfflineEngine>,
    calls: Mutex<Vec<Call>>,
}

impl OfflineEngine for Timed {
    fn info(&self) -> EngineInfo {
        self.inner.info()
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let warm_up = audio.len() == WARM_AUDIO && audio.iter().all(|&s| s == 0.0);
        let start = Instant::now();
        let result = self.inner.transcribe(audio, options);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                start,
                end: Instant::now(),
                warm_up,
            });
        result
    }
}

/// The stand-in engine: answers after its delay.
struct Delay(Duration);

impl OfflineEngine for Delay {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: format!("mock-{}ms", self.0.as_millis()),
            jobs: vec![Job::DictationFinal],
            licence: "MIT".into(),
        }
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        let until = Instant::now() + self.0;
        while Instant::now() < until {
            if options.cancel.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(Transcript {
            segments: vec![TimedText {
                start_ms: 0,
                end_ms: (audio.len() / 16) as u64,
                text: "a stand-in for what was said".into(),
            }],
        })
    }
}

fn engine(choice: &EngineChoice) -> Result<Arc<dyn OfflineEngine>, String> {
    match choice {
        EngineChoice::Mock { delay } => Ok(Arc::new(Delay(*delay))),
        EngineChoice::Qwen { dir } => qwen(dir),
    }
}

#[cfg(feature = "engine-llama")]
fn qwen(dir: &Path) -> Result<Arc<dyn OfflineEngine>, String> {
    let registry = ink_engines::Registry::builtin().map_err(|e| e.to_string())?;
    let row = registry
        .get("qwen3-asr-1.7b-q8")
        .ok_or("the built-in registry has no Qwen3-ASR row")?;
    let (model, mmproj) = (&row.files[0].name, &row.files[1].name);
    ink_engines::llama::QwenAsr::load(&dir.join(model), &dir.join(mmproj), row.info())
        .map(|m| Arc::new(m) as Arc<dyn OfflineEngine>)
        .map_err(|e| format!("Qwen3-ASR from {}: {e}", dir.display()))
}

#[cfg(not(feature = "engine-llama"))]
fn qwen(_: &Path) -> Result<Arc<dyn OfflineEngine>, String> {
    Err("this build has no Qwen3-ASR: build with --features engine-llama".into())
}

fn vad(choice: &VadChoice) -> Result<(Vad, String), String> {
    match choice {
        VadChoice::None => Ok((
            Vad::Unavailable(VadUnavailable::ModelMissing),
            "none (fallback)".into(),
        )),
        VadChoice::Silero(path) => silero(path),
    }
}

#[cfg(feature = "engine-silero")]
fn silero(path: &Path) -> Result<(Vad, String), String> {
    let vad = ink_engines::SileroModel::load(path)
        .and_then(|m| m.vad())
        .map_err(|e| format!("Silero from {}: {e}", path.display()))?;
    Ok((Vad::Installed(Box::new(vad)), "silero".into()))
}

#[cfg(not(feature = "engine-silero"))]
fn silero(_: &Path) -> Result<(Vad, String), String> {
    Err("this build has no Silero: build with --features engine-silero, or pass --vad none".into())
}

/// The clips: 16 kHz mono WAVs at least `seconds` long, in name order, each cut to `seconds`.
fn clips(dir: &Path, seconds: f64) -> Result<Vec<(String, Vec<f32>)>, String> {
    let want = (seconds * 16_000.0) as usize;
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    names.sort();
    let mut out = Vec::new();
    for path in names {
        let Ok(mut reader) = hound::WavReader::open(&path) else {
            continue;
        };
        let spec = reader.spec();
        if (spec.sample_rate, spec.channels) != (16_000, 1) {
            continue;
        }
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().filter_map(Result::ok).collect(),
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .filter_map(Result::ok)
                .map(|s| f32::from(s) / 32_768.0)
                .collect(),
        };
        if samples.len() >= want {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push((name, samples[..want].to_vec()));
        }
    }
    if out.is_empty() {
        return Err(format!(
            "no 16 kHz mono WAV of at least {seconds} s in {}",
            dir.display()
        ));
    }
    Ok(out)
}

/// Feeds the chain one take and returns its row (`run` 0 for the priming take).
struct Take<'a> {
    chain: &'a mut DictationChain,
    host_ns: &'a mut u64,
    pace: Pace,
}

impl Take<'_> {
    fn push(&mut self, block: &[f32], due: &mut Instant, paced: bool) {
        if paced {
            let now = Instant::now();
            if *due > now {
                std::thread::sleep(*due - now);
            }
            *due += Duration::from_nanos(BLOCK_NS);
        }
        self.chain.push_audio(block, *self.host_ns, 0);
        *self.host_ns += BLOCK_NS;
    }

    /// Returns the key-up's moment.
    fn hold(&mut self, utterance: &[f32]) -> Instant {
        let mut due = Instant::now();
        for block in [0.0f32; ROOM_BEFORE].chunks(BLOCK) {
            self.push(block, &mut due, false);
        }
        self.chain.hotkey(ink_core::HotkeyEvent::Pressed {
            at_ns: *self.host_ns,
        });
        let mut due = Instant::now();
        for block in utterance.chunks(BLOCK) {
            self.push(block, &mut due, self.pace == Pace::Realtime);
        }
        let up = Instant::now();
        self.chain.hotkey(ink_core::HotkeyEvent::Released {
            at_ns: *self.host_ns,
        });
        up
    }

    /// Room at real time after the key-up until `inserted` says the take went in.
    fn tail(&mut self, inserted: impl Fn() -> bool) -> bool {
        let quiet = [0.0f32; BLOCK];
        let mut due = Instant::now();
        let until = Instant::now() + GIVE_UP;
        while !inserted() {
            if Instant::now() > until {
                return false;
            }
            self.push(&quiet, &mut due, true);
        }
        true
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1_000.0
}

/// Runs the benchmark. Progress goes to stderr; the rows to `config.out`.
pub fn run(config: &Config) -> Result<Report, String> {
    let clips = clips(&config.clips, config.seconds)?;
    let inner = engine(&config.engine)?;
    let engine_id = inner.info().id;
    let (vad, vad_name) = vad(&config.vad)?;
    let timed = Arc::new(Timed {
        inner,
        calls: Mutex::default(),
    });
    let clock: Arc<dyn Clock> = Arc::new(RealClock(Instant::now()));
    let warmer = EngineWarmer::start(timed.clone(), clock.clone(), WARM_AFTER_IDLE)
        .map_err(|e| format!("the warm-up thread: {e}"))?;
    let stopwatch = Arc::new(Stopwatch::default());
    let events: Arc<Mutex<Vec<DictationEvent>>> = Arc::default();
    let sink = {
        let (events, warm, warmup) = (events.clone(), warmer.handle(), config.warmup);
        Arc::new(move |e: DictationEvent| {
            if warmup && matches!(e, DictationEvent::Started { .. }) {
                warm.key_down();
            }
            events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(e);
        })
    };
    let store = ink_store::SqliteStore::open_in_memory().map_err(|e| e.to_string())?;
    let mut chain = DictationChain::new(
        Services {
            engine: warmer.engine(),
            store: Arc::new(store),
            inserter: stopwatch.clone(),
            focus: Arc::new(NoFocus),
            clock: clock.clone(),
            llm: None,
        },
        DictationSettings::default(),
        vad,
        sink,
    );
    let mut host_ns = 1_000_000_000u64;
    let mut rows = Vec::new();
    for run in 0..=config.runs {
        if run > 0 && !config.idle.is_zero() {
            eprintln!(
                "run {run}/{}: idle {} s",
                config.runs,
                config.idle.as_secs()
            );
            std::thread::sleep(config.idle);
        }
        let (clip, audio) = &clips[run.saturating_sub(1) % clips.len()];
        let inserts_before = stopwatch
            .inserts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let calls_before = timed
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let mut take = Take {
            chain: &mut chain,
            host_ns: &mut host_ns,
            pace: config.pace,
        };
        let up = take.hold(audio);
        let inserted = || {
            stopwatch
                .inserts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len()
                > inserts_before
        };
        if !take.tail(inserted) {
            let why: Vec<String> = events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|e| {
                    matches!(
                        e,
                        DictationEvent::Discarded(_)
                            | DictationEvent::Failed(_)
                            | DictationEvent::Warning(_)
                    )
                })
                .map(|e| format!("{e:?}"))
                .collect();
            return Err(format!(
                "run {run} ({clip}) inserted nothing within {} s: {why:?}",
                GIVE_UP.as_secs()
            ));
        }
        let (at, words) = stopwatch
            .inserts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)[inserts_before];
        let calls: Vec<Call> =
            timed.calls.lock().unwrap_or_else(PoisonError::into_inner)[calls_before..].to_vec();
        let decode = calls
            .iter()
            .rev()
            .find(|c| !c.warm_up)
            .copied()
            .ok_or("the take reached no engine")?;
        let row = Row {
            run,
            clip: clip.clone(),
            total_ms: ms(at.saturating_duration_since(up)),
            before_decode_ms: ms(decode.start.saturating_duration_since(up)),
            decode_ms: ms(decode.end.saturating_duration_since(decode.start)),
            after_decode_ms: ms(at.saturating_duration_since(decode.end)),
            warmed: calls.iter().any(|c| c.warm_up),
            words,
        };
        if run == 0 {
            eprintln!("priming take: {:.0} ms (not counted)", row.total_ms);
            continue;
        }
        eprintln!(
            "run {run}/{}: {:.0} ms ({:.0} before decode, {:.0} decode){}",
            config.runs,
            row.total_ms,
            row.before_decode_ms,
            row.decode_ms,
            if row.warmed { ", warmed" } else { "" }
        );
        rows.push(row);
    }
    drop(chain);
    warmer.stop();
    let report = Report {
        rows,
        engine: engine_id,
        vad: vad_name,
        idle: config.idle,
        warmup: config.warmup,
        seconds: config.seconds,
        out: config.out.clone(),
    };
    if let Some(out) = &config.out {
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(out, report.tsv()).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    Ok(report)
}
