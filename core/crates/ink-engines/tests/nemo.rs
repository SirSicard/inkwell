//! Nemotron-3-Diarization on NeMo-Speech.cpp (`engine-nemo`).
//!
//! Building needs `NEMO_SPEECH_DIR` (see `native/build-nemo-speech.sh`). The `#[ignore]` tests
//! also need `INK_BENCH_DIR` holding
//! `models/nemotron-3-diarization/Nemotron-3-Diarization.q8_0.gguf` (hash-checked here) and
//! `INK_DIAR_SET`: a directory with `<meeting>.wav` (16 kHz mono) and `<meeting>.rttm` for the AMI
//! test meetings EN2002a, b and c, the 15-minute single-distant-mic stretches the diarizer was
//! chosen on:
//!
//! ```text
//! NEMO_SPEECH_DIR=<prefix> INK_BENCH_DIR=<bench> INK_DIAR_SET=<set> \
//!     cargo test -p ink-engines --features engine-nemo --release -- --ignored --test-threads 1
//! ```
//!
//! Scored with `der/` (pyannote.metrics semantics, proved in `tests/der_scorer.rs`). Setting
//! `INK_DIAR_OUT` also writes each hypothesis there as RTTM.

#![cfg(feature = "engine-nemo")]

mod der;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ink_core::mock::MockClock;
use ink_core::{
    CancelToken, Diarizer, EngineError, EngineInfo, EventSink, Job, SliceWindows, SpeakerTurn,
};
use ink_engines::{
    ModelDir, NemoDevice, NemoDiarizer, NemoLoader, Residency, nemotron_3_diarization,
};
use sha2::{Digest, Sha256};

use der::{Turn, clusters, der, min_speaker_recall, to_rttm};

/// Each meeting's DER at ±0.25 s from the diarization gate, with the final pass's preset and the
/// live one, and the substantial clusters the final pass found (as many as there are people).
const MEETINGS: [(&str, f64, f64, usize); 3] = [
    ("EN2002a", 15.3, 16.5, 4),
    ("EN2002b", 22.6, 22.7, 4),
    ("EN2002c", 22.8, 25.0, 3),
];

/// How far a reproduced DER may sit from the gate's (which is printed to one decimal).
const DER_TOLERANCE: f64 = 0.3;

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")))
}

fn model_path() -> PathBuf {
    let row = nemotron_3_diarization();
    env_dir("INK_BENCH_DIR")
        .join("models/nemotron-3-diarization")
        .join(&row.files[0].name)
}

/// The diarizer on the GPU, after checking the model file is the row's.
fn diarizer() -> NemoDiarizer {
    let row = nemotron_3_diarization();
    let path = model_path();
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(bytes.len() as u64, row.files[0].size, "model size");
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(sha, row.files[0].sha256, "model hash");
    NemoDiarizer::new(&path, row.info(), NemoDevice::Gpu(0)).unwrap()
}

fn read_wav(path: &Path) -> Vec<f32> {
    let mut reader =
        hound::WavReader::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let spec = reader.spec();
    assert_eq!(
        (spec.sample_rate, spec.channels),
        (16_000, 1),
        "{}",
        path.display()
    );
    match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32_768.0)
            .collect(),
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
    }
}

fn meeting(name: &str) -> (Vec<f32>, Vec<Turn>) {
    let set = env_dir("INK_DIAR_SET");
    let reference = der::parse_rttm(&fs::read_to_string(set.join(format!("{name}.rttm"))).unwrap());
    (read_wav(&set.join(format!("{name}.wav"))), reference)
}

fn as_turns(turns: &[SpeakerTurn]) -> Vec<Turn> {
    turns
        .iter()
        .map(|t| Turn::new(t.start_ms as f64 / 1e3, t.end_ms as f64 / 1e3, &t.speaker.0))
        .collect()
}

fn save(name: &str, suffix: &str, turns: &[Turn]) {
    if let Some(out) = std::env::var_os("INK_DIAR_OUT") {
        let out = PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        fs::write(
            out.join(format!("{name}.{suffix}.rttm")),
            to_rttm(name, turns),
        )
        .unwrap();
    }
}

/// Scores `hypothesis` and checks it against the gate's DER.
fn check_against_gate(name: &str, reference: &[Turn], hypothesis: &[Turn], gate: f64) -> f64 {
    let (d0, d25) = (
        der(reference, hypothesis, 0.0),
        der(reference, hypothesis, 0.25),
    );
    let (n, substantial) = clusters(hypothesis);
    println!(
        "{name}: DER {:.2} (c0), {:.4} (±0.25; gate {gate}); miss {:.2} / FA {:.2} / conf {:.2}; \
         clusters {n} ({substantial} >= 2 %); min speaker recall {:.1} %",
        d0.der,
        d25.der,
        d25.miss,
        d25.false_alarm,
        d25.confusion,
        min_speaker_recall(reference, hypothesis)
    );
    assert!(
        (d25.der - gate).abs() <= DER_TOLERANCE,
        "{name}: DER {:.2} against the gate's {gate}",
        d25.der
    );
    d25.der
}

// --- The final pass. ----------------------------------------------------------------------------

#[test]
#[ignore = "needs the Nemotron model and the AMI meetings (INK_BENCH_DIR, INK_DIAR_SET)"]
fn the_final_pass_reproduces_the_gate_der() {
    let diarizer = diarizer();
    let mut sum = 0.0;
    for (name, gate, _, people) in MEETINGS {
        let (audio, reference) = meeting(name);
        let started = Instant::now();
        let turns = diarizer
            .diarize(&mut SliceWindows::new(&audio), &CancelToken::new())
            .unwrap();
        let took = started.elapsed();
        let hypothesis = as_turns(&turns);
        save(name, "offline", &hypothesis);
        println!(
            "{name}: {} turns in {took:.2?} ({:.0}x real time, model resident after the first)",
            turns.len(),
            audio.len() as f64 / 16_000.0 / took.as_secs_f64()
        );
        sum += check_against_gate(name, &reference, &hypothesis, gate);
        // Architecture rule 5 on the pick: one substantial cluster per person, no fragments.
        assert_eq!(clusters(&hypothesis).1, people, "{name}");
        assert!(
            turns.windows(2).all(|w| w[0].start_ms <= w[1].start_ms),
            "{name}: order"
        );
    }
    let row = nemotron_3_diarization().wer(Job::Diarization).unwrap();
    println!("mean DER {:.2}; the registry row carries {row}", sum / 3.0);
    assert!((sum / 3.0 - f64::from(row)).abs() < 0.1);
}

#[test]
#[ignore = "needs the Nemotron model and the AMI meetings (INK_BENCH_DIR, INK_DIAR_SET)"]
fn the_final_pass_is_deterministic() {
    let diarizer = diarizer();
    let (audio, _) = meeting("EN2002c");
    let audio = &audio[..3 * 60 * 16_000];
    let a = diarizer
        .diarize(&mut SliceWindows::new(audio), &CancelToken::new())
        .unwrap();
    let b = diarizer
        .diarize(&mut SliceWindows::new(audio), &CancelToken::new())
        .unwrap();
    assert!(!a.is_empty());
    assert_eq!(a, b);
}

#[test]
#[ignore = "needs the Nemotron model and the AMI meetings (INK_BENCH_DIR, INK_DIAR_SET)"]
fn cancelling_stops_the_final_pass() {
    let diarizer = diarizer();
    let (audio, _) = meeting("EN2002a");
    // Already cancelled: nothing runs.
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        diarizer.diarize(&mut SliceWindows::new(&audio), &cancel),
        Err(EngineError::Cancelled)
    );
    // Cancelled 100 ms into a pass that takes seconds: it stops at the next push.
    diarizer
        .diarize(
            &mut SliceWindows::new(&audio[..16_000]),
            &CancelToken::new(),
        )
        .unwrap(); // load the model first
    let cancel = CancelToken::new();
    let remote = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        remote.cancel();
    });
    let started = Instant::now();
    let result = diarizer.diarize(&mut SliceWindows::new(&audio), &cancel);
    canceller.join().unwrap();
    assert_eq!(result, Err(EngineError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

/// The final pass pulls its audio a window at a time (ink-core's `DiarizeInput`). How the windows
/// are cut changes nothing: 60 s windows and 7.3 s windows give the same turns, and the DER
/// matches the gate. With `INK_DIAR_BASELINE` pointing at RTTMs written (through `INK_DIAR_OUT`)
/// by the earlier one-buffer `diarize`, the turns are compared with those too, byte for byte.
#[test]
#[ignore = "needs the Nemotron model and the AMI meetings (INK_BENCH_DIR, INK_DIAR_SET)"]
fn the_windowed_feed_leaves_the_final_pass_unchanged() {
    let diarizer = diarizer();
    let baseline = std::env::var_os("INK_DIAR_BASELINE").map(PathBuf::from);
    for (name, gate, _, _) in MEETINGS {
        let (audio, reference) = meeting(name);
        let whole = diarizer
            .diarize(&mut SliceWindows::new(&audio), &CancelToken::new())
            .unwrap();
        let cut = diarizer
            .diarize(
                &mut SliceWindows::with_window(&audio, 116_800),
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(whole, cut, "{name}: the windows changed the turns");
        let hypothesis = as_turns(&whole);
        check_against_gate(name, &reference, &hypothesis, gate);
        if let Some(dir) = &baseline {
            let before = fs::read_to_string(dir.join(format!("{name}.offline.rttm"))).unwrap();
            assert_eq!(
                to_rttm(name, &hypothesis),
                before,
                "{name}: differs from the one-buffer baseline"
            );
            println!(
                "{name}: identical to the one-buffer baseline ({} turns)",
                whole.len()
            );
        }
    }
}

// --- Live labels. -------------------------------------------------------------------------------

/// Everything a live stream reported, with how much audio had been pushed when each turn came.
type Reported = Arc<Mutex<Vec<(SpeakerTurn, u64)>>>;

#[test]
#[ignore = "needs the Nemotron model and the AMI meetings (INK_BENCH_DIR, INK_DIAR_SET)"]
fn live_labels_reproduce_the_gate_der_and_arrive_while_the_meeting_runs() {
    // Pushed 20 ms at a time, as the pump would. Every turn is reported once it is settled; the
    // union is the whole meeting's diarization, scored against the gate's streaming run.
    const PUSH: usize = 320;
    let diarizer = diarizer();
    for (name, _, gate, _) in MEETINGS {
        let (audio, reference) = meeting(name);
        let reported: Reported = Arc::default();
        let pushed = Arc::new(Mutex::new(0u64));
        let sink: EventSink<SpeakerTurn> = {
            let (reported, pushed) = (Arc::clone(&reported), Arc::clone(&pushed));
            Arc::new(move |t| {
                let at = *pushed.lock().unwrap();
                reported.lock().unwrap().push((t, at));
            })
        };
        let mut stream = diarizer.open_stream(sink).unwrap();
        for chunk in audio.chunks(PUSH) {
            stream.push(chunk).unwrap();
            *pushed.lock().unwrap() += (chunk.len() / 16) as u64;
        }
        let before_finish = reported.lock().unwrap().len();
        stream.finish().unwrap();
        let reported = reported.lock().unwrap().clone();
        let turns: Vec<SpeakerTurn> = reported.iter().map(|(t, _)| t.clone()).collect();
        let hypothesis = as_turns(&turns);
        save(name, "live", &hypothesis);
        // How long after a turn ended it was reported (audio time), for turns reported live.
        let mut delays: Vec<u64> = reported[..before_finish]
            .iter()
            .map(|(t, at)| at.saturating_sub(t.end_ms))
            .collect();
        delays.sort_unstable();
        let median = delays.get(delays.len() / 2).copied().unwrap_or(0);
        let worst = delays.last().copied().unwrap_or(0);
        println!(
            "{name}: {} turns, {before_finish} reported live; reported {median} ms after they \
             ended (median), {worst} ms at worst",
            turns.len()
        );
        check_against_gate(name, &reference, &hypothesis, gate);
        assert!(
            before_finish * 10 >= turns.len() * 9,
            "{name}: most turns came at the end"
        );
        assert!(worst <= 5_000, "{name}: a turn came {worst} ms late");
    }
}

// --- Loading. -----------------------------------------------------------------------------------

fn info() -> EngineInfo {
    nemotron_3_diarization().info()
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ink-engines-nemo-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_missing_model_is_model_missing() {
    let err = NemoDiarizer::new(Path::new("no-such-dir/model.gguf"), info(), NemoDevice::Cpu)
        .unwrap_err();
    assert!(matches!(err, EngineError::ModelMissing(_)), "{err:?}");
}

#[test]
fn a_corrupt_model_fails_on_first_use_with_the_librarys_reason() {
    let dir = temp_dir("corrupt");
    let path = dir.join("model.gguf");
    fs::write(&path, b"not a gguf file").unwrap();
    let diarizer = NemoDiarizer::new(&path, info(), NemoDevice::Cpu).unwrap();
    // No audio, nothing to load.
    assert_eq!(
        diarizer.diarize(&mut SliceWindows::new(&[]), &CancelToken::new()),
        Ok(vec![])
    );
    let second = [0.0; 16_000];
    let err = diarizer
        .diarize(&mut SliceWindows::new(&second), &CancelToken::new())
        .unwrap_err();
    let _ = fs::remove_dir_all(&dir);
    assert!(
        matches!(&err, EngineError::Failed(m) if m.contains("loading the model")),
        "{err:?}"
    );
}

#[test]
#[ignore = "needs the Nemotron model and the AMI meetings (INK_BENCH_DIR, INK_DIAR_SET)"]
fn the_loader_loads_the_installed_row_through_residency() {
    let root = temp_dir("installed");
    let dir = ModelDir::new(&root);
    let row = nemotron_3_diarization();
    let path = dir.file_path(&row, &row.files[0]);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::copy(model_path(), &path).unwrap();
    let residency = Residency::new(
        Arc::new(NemoLoader::new(dir, NemoDevice::Gpu(0))),
        Arc::new(MockClock::new(0, 0)),
    );
    let lease = residency.acquire(&row).unwrap();
    assert_eq!(lease.info().id, row.id);
    let (audio, _) = meeting("EN2002c");
    let turns = lease
        .diarize(
            &mut SliceWindows::new(&audio[..60 * 16_000]),
            &CancelToken::new(),
        )
        .unwrap();
    drop(lease);
    let _ = fs::remove_dir_all(&root);
    assert!(!turns.is_empty());
}
