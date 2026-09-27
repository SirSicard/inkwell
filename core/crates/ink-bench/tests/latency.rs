//! The latency harness on its stand-in engine: it runs, times each take from the key-up, and
//! writes its rows. Proves the harness; measures nothing real (the real run is local, with
//! Qwen3-ASR: scripts/dictation-latency.sh).

use std::path::Path;
use std::time::Duration;

use ink_bench::latency::{Config, EngineChoice, Pace, VadChoice, percentile, run};

fn write_clip(path: &Path, seconds: f64, seed: u64) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for s in ink_audio::synth::speech_like(seconds, -25.0, seed) {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
}

fn temp(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ink-bench-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_harness_times_each_take_from_the_key_up_and_writes_its_rows() {
    let dir = temp("latency");
    write_clip(&dir.join("a.wav"), 1.3, 1);
    write_clip(&dir.join("b.wav"), 1.4, 2);
    // Too short for a 1 s utterance: left out.
    write_clip(&dir.join("c.wav"), 0.6, 3);
    let out = dir.join("out/latency.tsv");
    let config = Config {
        clips: dir.clone(),
        runs: 3,
        seconds: 1.0,
        idle: Duration::ZERO,
        warmup: true,
        pace: Pace::Fast,
        engine: EngineChoice::Mock {
            delay: Duration::from_millis(40),
        },
        vad: VadChoice::None,
        out: Some(out.clone()),
    };
    let report = run(&config).unwrap();
    assert_eq!(report.rows.len(), 3);
    for row in &report.rows {
        assert!(row.decode_ms >= 40.0, "{row:?}");
        assert!(row.total_ms >= row.decode_ms, "{row:?}");
        assert!(row.words > 0);
        assert!(row.clip == "a.wav" || row.clip == "b.wav", "{row:?}");
    }
    let tsv = std::fs::read_to_string(&out).unwrap();
    assert_eq!(tsv.lines().count(), 4, "{tsv}");
    assert!(report.summary().contains("p50"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn percentiles_are_nearest_rank() {
    let v = [5.0, 1.0, 4.0, 2.0, 3.0];
    assert_eq!(percentile(&v, 50.0), 3.0);
    assert_eq!(percentile(&v, 95.0), 5.0);
    assert_eq!(percentile(&v, 0.0), 1.0);
    assert!(percentile(&[], 50.0).is_nan());
}

#[test]
fn the_command_line_reads_its_options_and_refuses_the_rest() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let c = Config::from_args(&args(&[
        "--clips",
        "/c",
        "--runs",
        "5",
        "--seconds",
        "5",
        "--idle",
        "240",
        "--warmup",
        "off",
        "--pace",
        "fast",
        "--engine",
        "mock",
        "--vad",
        "none",
        "--no-out",
    ]))
    .unwrap();
    assert_eq!(c.runs, 5);
    assert_eq!(c.idle, Duration::from_secs(240));
    assert!(!c.warmup);
    assert_eq!(c.pace, Pace::Fast);
    assert_eq!(c.vad, VadChoice::None);
    assert!(c.out.is_none());
    for bad in [
        &["--runs", "0"][..],
        &["--warmup", "maybe"],
        &["--seconds", "0.1"],
        &["--engine", "whisper"],
        &["--colour", "blue"],
        &["--runs"],
    ] {
        assert!(Config::from_args(&args(bad)).is_err(), "{bad:?}");
    }
}
