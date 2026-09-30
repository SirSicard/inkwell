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
//! `2bda32ec70b097a55adaa07d9a7173915b43cc78` (the ONNX files are hash-checked here). The speeds
//! printed are for comparison only; time them on a quiet machine.

#![cfg(feature = "engine-sherpa")]

mod bench;

use std::path::PathBuf;
use std::time::Instant;

use ink_core::TranscribeOptions;
use ink_core::{CancelToken, Channel, EngineError, EngineInfo, Job, OfflineEngine};
use ink_engines::sherpa::{MAX_SECONDS, SherpaParakeet};
use sha2::{Digest, Sha256};

/// The ONNX files' SHA-256s at the pinned revision (Hugging Face's LFS object ids).
const MODEL: [(&str, &str); 3] = [
    (
        "encoder.int8.onnx",
        "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247",
    ),
    (
        "decoder.int8.onnx",
        "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e",
    ),
    (
        "joiner.int8.onnx",
        "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3",
    ),
];

const RATE: usize = 16_000;

fn model_dir() -> PathBuf {
    let dir =
        PathBuf::from(std::env::var_os("INK_PARAKEET_DIR").expect("INK_PARAKEET_DIR is not set"));
    for (name, pinned) in MODEL {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let sha: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(sha, pinned, "{name} is not the pinned file");
    }
    dir
}

fn info() -> EngineInfo {
    EngineInfo {
        id: "parakeet-tdt-0.6b-v3-int8".into(),
        jobs: vec![Job::LivePartials],
        licence: "CC-BY-4.0".into(),
    }
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
/// different build of the same weights, so the numbers are printed for comparison and held only
/// to a wide band: a broken decode (empty or garbled text) lands far outside it.
#[test]
#[ignore = "needs sherpa-onnx, the Parakeet model and AMI under $INK_BENCH_DIR; run locally"]
fn parakeet_transcribes_ami_near_its_measured_wer() {
    let engine = SherpaParakeet::load(&model_dir(), info()).unwrap();
    let ihm = corpus(&engine, "ami-ihm");
    assert_eq!(ihm.reference, 709, "the reference set changed");
    assert!(
        (ihm.wer() - 23.4).abs() <= 5.0,
        "AMI IHM WER {:.2}",
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
    let long = vec![0.0f32; (MAX_SECONDS as usize + 1) * RATE];
    assert!(matches!(
        engine.transcribe(&long, &options()),
        Err(EngineError::Failed(_))
    ));
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
