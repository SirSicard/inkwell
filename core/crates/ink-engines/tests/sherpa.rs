//! Parakeet TDT v3 int8 on sherpa-onnx (`engine-sherpa`, Windows). Every test needs the libraries
//! (`SHERPA_ONNX_DIR`, see `src/sherpa.rs`) and the model, so all are `#[ignore]` and run locally:
//!
//! ```text
//! SHERPA_ONNX_DIR=<unpacked no-tts archive> INK_PARAKEET_DIR=<model dir> INK_BENCH_DIR=<bench> \
//!     cargo test -p ink-engines --release --features engine-sherpa --test sherpa -- --ignored
//! ```
//!
//! `INK_PARAKEET_DIR` holds `encoder.int8.onnx`, `decoder.int8.onnx`, `joiner.int8.onnx` and
//! `tokens.txt` from `csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8` at
//! `2bda32ec70b097a55adaa07d9a7173915b43cc78`, each checked here against the registry row's size
//! and hash. The speeds printed are for comparison only; time them on a quiet machine.

#![cfg(feature = "engine-sherpa")]

mod bench;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ink_core::{
    AsrEvent, CancelToken, Channel, EngineError, EngineInfo, EventSink, OfflineEngine,
    StreamingEngine, TranscribeOptions, Transcript,
};
use ink_engines::sherpa::{MAX_SECONDS, SherpaParakeet};
use ink_engines::{TrailingWindow, parakeet_tdt_v3_int8};
use sha2::{Digest, Sha256};

const RATE: usize = 16_000;

/// `INK_PARAKEET_DIR`, after checking each of its files is the registry row's, by size and SHA-256.
fn model_dir() -> PathBuf {
    let dir =
        PathBuf::from(std::env::var_os("INK_PARAKEET_DIR").expect("INK_PARAKEET_DIR is not set"));
    for file in parakeet_tdt_v3_int8().files {
        let bytes =
            std::fs::read(dir.join(&file.name)).unwrap_or_else(|e| panic!("{}: {e}", file.name));
        assert_eq!(bytes.len() as u64, file.size, "{}'s size", file.name);
        let sha: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(sha, file.sha256, "{} is not the pinned file", file.name);
    }
    dir
}

fn info() -> EngineInfo {
    parakeet_tdt_v3_int8().info()
}

fn options() -> TranscribeOptions {
    TranscribeOptions {
        channel: Channel::Mic,
        context: None,
        cancel: CancelToken::new(),
    }
}

/// Transcribes every clip of `set` whole and returns the corpus edits, printing speeds.
fn corpus(engine: &SherpaParakeet, set: &str) -> bench::Edits {
    let bench = bench::bench_dir();
    let rows = bench::read_ami_tsv(&bench.join(format!("{set}.tsv"))).unwrap();
    let mut corpus = bench::Edits::default();
    let (mut audio_s, mut busy_s) = (0.0, 0.0);
    for clip in &rows {
        let (rate, audio) = bench::read_wav(&bench.join(set).join(&clip.wav)).unwrap();
        assert_eq!(rate as usize, RATE);
        let seconds = audio.len() as f64 / RATE as f64;
        let started = Instant::now();
        let transcript = engine.transcribe(&audio, &options()).unwrap();
        let elapsed = started.elapsed().as_secs_f64();
        let edits = bench::score(&clip.reference, &transcript.text());
        println!(
            "{:18} {seconds:5.1} s audio  {elapsed:5.2} s  {:5.1}x  {edits}",
            clip.wav,
            seconds / elapsed
        );
        corpus = corpus + edits;
        audio_s += seconds;
        busy_s += elapsed;
    }
    println!(
        "{set} corpus  {corpus}   {:.1}x real time",
        audio_s / busy_s
    );
    corpus
}

/// Parakeet v3 was chosen for live partials on the Mac's FluidAudio build: 23.4 on AMI IHM and
/// 45.3 on AMI SDM (fp16 on the Neural Engine). This is the int8 ONNX conversion on the CPU, a
/// different build of the same weights: AMI IHM reproduces the registry row's rate (27.93, measured
/// here), and AMI SDM, printed for comparison, is held only to a wide band around the Mac's.
#[test]
#[ignore = "needs sherpa-onnx, the Parakeet model and AMI under $INK_BENCH_DIR; run locally"]
fn parakeet_transcribes_ami_near_its_measured_wer() {
    let engine = SherpaParakeet::load(&model_dir(), info()).unwrap();
    let ihm = corpus(&engine, "ami-ihm");
    assert_eq!(ihm.reference, 709, "the reference set changed");
    let row = parakeet_tdt_v3_int8()
        .wer(ink_core::Job::LivePartials)
        .unwrap();
    assert!(
        (ihm.wer() - f64::from(row)).abs() <= 0.3,
        "AMI IHM WER {:.2} against the row's {row}",
        ihm.wer()
    );
    let sdm = corpus(&engine, "ami-sdm");
    assert_eq!(sdm.reference, 671, "the reference set changed");
    assert!(
        (sdm.wer() - 45.3).abs() <= 5.0,
        "AMI SDM WER {:.2}",
        sdm.wer()
    );
}

/// The gate's level normalisation: each utterance scaled to −23 dBFS RMS, or less where that would
/// put its peak above −1 dBFS. The app's own gain stage differs in detail; this is the rule the
/// other engines' "level-normalised" rates were measured with.
fn normalised(audio: &[f32]) -> Vec<f32> {
    let rms =
        (audio.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / audio.len() as f64).sqrt();
    let peak = audio.iter().fold(0.0f32, |p, s| p.max(s.abs()));
    if rms == 0.0 || peak == 0.0 {
        return audio.to_vec();
    }
    let gain = (10f64.powf(-23.0 / 20.0) / rms).min(10f64.powf(-1.0 / 20.0) / f64::from(peak));
    audio
        .iter()
        .map(|&s| (f64::from(s) * gain) as f32)
        .collect()
}

/// The dictation set the router's dictation rates are measured on: FLEURS English dev (394
/// utterances, `fleurs/en_us.dev.tsv` beside `fleurs/en_us/dev/`; its columns 1 and 3 are the WAV
/// and the normalised reference, as in the AMI sets), each utterance whole. Scored as published
/// (no gain, as the registry's rates are) and level-normalised ([`normalised`]). Prints each corpus
/// WER, the utterances that came back empty, and the median decode time of the utterances between
/// 4.5 and 5.5 s (the dictation-latency set), warm.
#[test]
#[ignore = "needs sherpa-onnx, the Parakeet model and FLEURS under $INK_BENCH_DIR; run locally"]
fn parakeet_on_the_dictation_set() {
    let engine = SherpaParakeet::load(&model_dir(), info()).unwrap();
    let bench = bench::bench_dir();
    let rows = bench::read_ami_tsv(&bench.join("fleurs/en_us.dev.tsv")).unwrap();
    assert_eq!(rows.len(), 394, "the reference set changed");
    let mut corpus = [bench::Edits::default(); 2];
    let (mut empty, mut short) = ([0; 2], Vec::new());
    for clip in &rows {
        let path = bench.join("fleurs/en_us/dev").join(&clip.wav);
        let (rate, audio) = bench::read_wav(&path).unwrap();
        assert_eq!(rate as usize, RATE);
        let seconds = audio.len() as f64 / RATE as f64;
        for (i, audio) in [audio.clone(), normalised(&audio)].iter().enumerate() {
            let started = Instant::now();
            let text = engine.transcribe(audio, &options()).unwrap().text();
            let elapsed = started.elapsed().as_secs_f64();
            if text.is_empty() {
                empty[i] += 1;
            }
            if i == 0 && (4.5..=5.5).contains(&seconds) {
                short.push(elapsed);
            }
            corpus[i] = corpus[i] + bench::score(&clip.reference, &text);
        }
    }
    assert_eq!(corpus[0].reference, 8_323, "the reference set changed");
    let row = parakeet_tdt_v3_int8()
        .wer(ink_core::Job::DictationFinal)
        .unwrap();
    assert!(
        (corpus[0].wer() - f64::from(row)).abs() <= 0.3,
        "FLEURS WER {:.2} against the row's {row}",
        corpus[0].wer()
    );
    short.sort_by(f64::total_cmp);
    println!("fleurs-en       {}   empty {}", corpus[0], empty[0]);
    println!("fleurs-en-norm  {}   empty {}", corpus[1], empty[1]);
    println!(
        "4.5-5.5 s decode median {:.0} ms (n={}, min {:.0}, max {:.0})",
        short[short.len() / 2] * 1000.0,
        short.len(),
        short[0] * 1000.0,
        short[short.len() - 1] * 1000.0,
    );
}

/// A long dictation take: AMI IHM's three clips (231 s) joined into one call, which the adapter
/// cuts into windows of at most 90 s (see `src/sherpa.rs`). Its words keep their order on the take's
/// timeline, and it reproduces its measured rate, 29.06: near the clips decoded one by one (27.93),
/// where no cut falls inside a clip.
#[test]
#[ignore = "needs sherpa-onnx, the Parakeet model and AMI under $INK_BENCH_DIR; run locally"]
fn a_take_longer_than_one_pass_is_decoded_in_windows() {
    let engine = SherpaParakeet::load(&model_dir(), info()).unwrap();
    let bench = bench::bench_dir();
    let rows = bench::read_ami_tsv(&bench.join("ami-ihm.tsv")).unwrap();
    let mut take = Vec::new();
    for clip in &rows {
        take.extend(
            bench::read_wav(&bench.join("ami-ihm").join(&clip.wav))
                .unwrap()
                .1,
        );
    }
    let seconds = take.len() / RATE;
    assert!(
        seconds > 2 * MAX_SECONDS as usize,
        "{seconds} s: three windows or more"
    );
    let transcript = engine.transcribe(&take, &options()).unwrap();
    let starts: Vec<u64> = transcript.segments.iter().map(|w| w.start_ms).collect();
    assert!(starts.windows(2).all(|p| p[0] <= p[1]), "words in order");
    let last = transcript.segments.last().unwrap();
    assert!(last.end_ms <= take.len() as u64 * 1000 / RATE as u64);
    assert!(
        last.start_ms > 2 * u64::from(MAX_SECONDS) * 1000,
        "the last window's words"
    );
    let reference: Vec<&str> = rows.iter().map(|r| r.reference.as_str()).collect();
    let edits = bench::score(&reference.join(" "), &transcript.text());
    println!("ami-ihm as one {seconds} s take  {edits}");
    assert!(
        (edits.wer() - 29.06).abs() <= 0.3,
        "one long take's WER {:.2} against its measured 29.06",
        edits.wer()
    );
}

/// An engine that counts its decodes, so a stream's work shows.
struct Counting {
    engine: Arc<SherpaParakeet>,
    decodes: AtomicUsize,
}

impl OfflineEngine for Counting {
    fn info(&self) -> EngineInfo {
        self.engine.info()
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        self.decodes.fetch_add(1, Ordering::SeqCst);
        self.engine.transcribe(audio, options)
    }
}

/// Windows' live partials for a meeting's two sides: the trailing-window scheme over one engine
/// (one recognizer, as in the app, where both sides route to one loaded copy), AMI IHM's first clip
/// on the mic side and its second on the far side, each fed in 20 ms blocks at real time from its
/// own thread. Every push returns within its 20 ms, partials keep coming on both sides, and each
/// side's finals, joined, score near its clip's WER in one pass. Prints each side's finals' WER, the
/// partials and stalls sent, the decodes it took (still windows take none), and its slowest push.
#[test]
#[ignore = "needs sherpa-onnx, the Parakeet model and AMI under $INK_BENCH_DIR; about 90 s"]
fn live_partials_settle_a_meetings_two_sides_into_finals_at_real_time() {
    let engine = Arc::new(SherpaParakeet::load(&model_dir(), info()).unwrap());
    let bench = bench::bench_dir();
    let clips = bench::read_ami_tsv(&bench.join("ami-ihm.tsv")).unwrap();
    let sides: Vec<_> = [(Channel::Mic, &clips[0]), (Channel::Far, &clips[1])]
        .into_iter()
        .map(|(channel, clip)| {
            let (_, audio) = bench::read_wav(&bench.join("ami-ihm").join(&clip.wav)).unwrap();
            let counting = Arc::new(Counting {
                engine: engine.clone(),
                decodes: AtomicUsize::new(0),
            });
            let live = TrailingWindow::new(counting.clone(), info());
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink: EventSink<AsrEvent> = {
                let events = events.clone();
                Arc::new(move |e| events.lock().unwrap().push(e))
            };
            let mut stream = live.open_stream(channel, sink).unwrap();
            let feeder = std::thread::spawn(move || {
                let started = Instant::now();
                let mut slowest = Duration::ZERO;
                for (i, block) in audio.chunks(RATE / 50).enumerate() {
                    if let Some(wait) =
                        Duration::from_millis(20 * i as u64).checked_sub(started.elapsed())
                    {
                        std::thread::sleep(wait);
                    }
                    let pushed = Instant::now();
                    stream.push(block).unwrap();
                    slowest = slowest.max(pushed.elapsed());
                }
                stream.finish().unwrap();
                slowest
            });
            (channel, clip, counting, events, feeder)
        })
        .collect();
    for (channel, clip, counting, events, feeder) in sides {
        let slowest = feeder.join().unwrap();
        let events = events.lock().unwrap();
        let finals: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                AsrEvent::Final(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        let partials = events
            .iter()
            .filter(|e| matches!(e, AsrEvent::Partial { .. }))
            .count();
        let stalls = events
            .iter()
            .filter(|e| matches!(e, AsrEvent::Stalled { .. }))
            .count();
        let edits = bench::score(&clip.reference, &finals.join(" "));
        println!(
            "{channel:?} {}  live finals {edits}  ({} finals, {partials} partials, {stalls} stalls, \
             {} decodes, slowest push {slowest:?})",
            clip.wav,
            finals.len(),
            counting.decodes.load(Ordering::SeqCst)
        );
        assert!(
            slowest < Duration::from_millis(20),
            "{channel:?}: a push waited {slowest:?}"
        );
        assert!(
            partials > 20 && finals.len() > 3,
            "{channel:?}: {partials} partials, {} finals",
            finals.len()
        );
        assert!(
            edits.wer() < 40.0,
            "{channel:?}: live finals' WER {:.2}",
            edits.wer()
        );
    }
}

#[test]
#[ignore = "needs sherpa-onnx and the Parakeet model; run locally"]
fn odd_input_is_refused_or_empty_never_a_crash() {
    let engine = SherpaParakeet::load(&model_dir(), info()).unwrap();
    assert!(
        engine
            .transcribe(&[], &options())
            .unwrap()
            .segments
            .is_empty()
    );
    let mut bad = vec![0.0f32; RATE];
    bad[7] = f32::NAN;
    match engine.transcribe(&bad, &options()) {
        Err(EngineError::Failed(msg)) => assert!(msg.contains("sample 7"), "{msg}"),
        other => panic!("expected a failure, got {other:?}"),
    }
    // Longer than one pass: decoded in windows, and silence is no words.
    let long = vec![0.0f32; (MAX_SECONDS as usize + 1) * RATE];
    assert_eq!(
        engine.transcribe(&long, &options()),
        Ok(Transcript::default())
    );
    let cancelled = options();
    cancelled.cancel.cancel();
    assert_eq!(
        engine.transcribe(&[0.0; RATE], &cancelled),
        Err(EngineError::Cancelled)
    );
}

#[test]
#[ignore = "needs sherpa-onnx; run locally"]
fn a_missing_model_file_is_model_missing() {
    let dir = std::env::temp_dir().join("ink-engines-no-parakeet");
    let err = SherpaParakeet::load(&dir, info()).err().unwrap();
    assert!(matches!(err, EngineError::ModelMissing(_)), "{err}");
}
