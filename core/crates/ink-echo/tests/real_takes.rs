//! Local only: replays S0.3's echo recordings through this crate and checks its numbers.
//!
//! Run with `INK_BENCH_DIR=<bench dir> cargo test -p ink-echo --release -- --ignored`. The takes
//! are private recordings and live only in the bench directory, under `out/s0.3/`:
//!
//! - `speakers/`: laptop speakers into the laptop mic, far end only (nobody read).
//! - `speakers2/`: the far end in earbuds while the passage was read; the earbuds' mic and the
//!   built-in mic, neither of which heard the far end.
//! - `mix-speakers-speakers2/`: double talk, the echo-only mic plus the reading on the same
//!   built-in mic, with the reading as the near-end truth.
//! - `synthetic/<variant>/`: S0.3's synthetic mixes (public corpus audio through a seeded room).
//!
//! Each goes through the product's path: both streams to 16 kHz by `ink-audio`'s resampler, the
//! path search, then the canceller. S0.3's own numbers (its measurement tool resampled the mic
//! rather than the far end, and read the 48 kHz originals) are in the assertions' messages.
//!
//! With `INK_ECHO_ROWS=<dir>`, each replay also writes its per-frame levels there (numbers only).

mod common;

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use common::{Outputs, cancel};
use ink_echo::measure::{active, dilate, erle_db, frame_db, projection_gain_db, ratio_db};
use ink_echo::{Alignment, FRAME, PathFinder, PathReport, RATE};
use serde_json::Value;

fn bench() -> PathBuf {
    let dir = std::env::var_os("INK_BENCH_DIR")
        .expect("set INK_BENCH_DIR to the bench directory to run the local echo replays");
    PathBuf::from(dir).join("out").join("s0.3")
}

fn read_f32(path: &Path) -> Vec<f32> {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_eq!(bytes.len() % 4, 0, "{} is not float32", path.display());
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn to_16k(x: &[f32], rate: u32) -> Vec<f32> {
    if rate == 16_000 {
        return x.to_vec();
    }
    ink_audio::resample(x, rate).expect("resample")
}

/// One channel of a take against its system tap, both from the same host instant (the earlier
/// stream's head dropped, as S0.3 did), at 16 kHz. Returns (mic, far, far-file seconds at
/// sample 0 of both).
fn load(take: &Path, role: &str, mic_file: Option<&Path>) -> (Vec<f32>, Vec<f32>, f64) {
    let meta = json(&take.join("meta.json"));
    let chans = meta["channels"].as_array().expect("channels");
    let ch = |r: &str| {
        chans
            .iter()
            .find(|c| c["role"] == r)
            .unwrap_or_else(|| panic!("no {r} channel"))
            .clone()
    };
    let (m, s) = (ch(role), ch("system"));
    let rate = |c: &Value| c["sample_rate"].as_u64().expect("rate") as u32;
    let first = |c: &Value| c["first_host_ns"].as_i64().expect("host ns");
    let mic_path = mic_file
        .map(Path::to_path_buf)
        .unwrap_or_else(|| take.join(m["file"].as_str().expect("file")));
    let mut mic = read_f32(&mic_path);
    let mut far = read_f32(&take.join(s["file"].as_str().expect("file")));
    let mic_start_s = (first(&m) - first(&s)) as f64 / 1e9;
    let mut far_trim_s = 0.0;
    if mic_start_s > 0.0 {
        far_trim_s = mic_start_s;
        let n = ((mic_start_s * f64::from(rate(&s))) as usize).min(far.len());
        far.drain(..n);
    } else if mic_start_s < 0.0 {
        let n = ((-mic_start_s * f64::from(rate(&m))) as usize).min(mic.len());
        mic.drain(..n);
    }
    (to_16k(&mic, rate(&m)), to_16k(&far, rate(&s)), far_trim_s)
}

fn find(mic: &[f32], far: &[f32]) -> PathReport {
    let mut p = PathFinder::new();
    for (m, f) in mic.chunks(1_600).zip(far.chunks(1_600)) {
        p.push(m, f).expect("in step");
    }
    p.estimate()
}

/// Seconds on the far file's timeline at the centre of each 10 ms frame of the mic grid.
fn far_times(frames: usize, path: Alignment, far_trim_s: f64) -> Vec<f64> {
    (0..frames)
        .map(|f| {
            let k = (f * FRAME + FRAME / 2) as f64;
            far_trim_s + (k - path.delay) / (1.0 + path.drift) / RATE
        })
        .collect()
}

fn within(t: &[f64], a: f64, b: f64) -> Vec<bool> {
    t.iter().map(|t| *t >= a && *t < b).collect()
}

fn and(a: &[bool], b: &[bool]) -> Vec<bool> {
    a.iter().zip(b).map(|(x, y)| *x && *y).collect()
}

fn count(v: &[bool]) -> usize {
    v.iter().filter(|x| **x).count()
}

fn dump_rows(name: &str, out: &Outputs, t_far: &[f64], extra: &[(&str, Vec<f64>)]) {
    let Some(dir) = std::env::var_os("INK_ECHO_ROWS") else {
        return;
    };
    let dir = PathBuf::from(dir);
    fs::create_dir_all(&dir).expect("rows dir");
    let (m, r, l, f) = (
        frame_db(&out.mic),
        frame_db(&out.reference),
        frame_db(&out.linear),
        frame_db(&out.full),
    );
    let mut s = String::from("frame\tt_far_s\tmic_db\tref_db\tlin_db\tfull_db");
    for (k, _) in extra {
        s.push('\t');
        s.push_str(k);
    }
    s.push('\n');
    for i in 0..m.len() {
        s.push_str(&format!(
            "{i}\t{:.3}\t{:.2}\t{:.2}\t{:.2}\t{:.2}",
            t_far[i], m[i], r[i], l[i], f[i]
        ));
        for (_, v) in extra {
            s.push_str(&format!("\t{:.2}", v[i]));
        }
        s.push('\n');
    }
    let mut file = fs::File::create(dir.join(format!("{name}.tsv"))).expect("rows file");
    file.write_all(s.as_bytes()).expect("write rows");
}

/// The speakers take: echo only. S0.3: path found at +46.0 ms, +1.8 ppm (67 of 72
/// candidates, 81 windows); ERLE 15.6 dB whole, 24.3 dB after 10 s, 11–12 dB in the first 10 s,
/// 5.7 dB from the linear stage; the output 15.7 dB under the mic in the 26–76 s window.
#[test]
#[ignore = "local: needs INK_BENCH_DIR and S0.3's recordings"]
fn the_speakers_take_reproduces_the_measured_path_and_erle() {
    let take = bench().join("speakers");
    let onset = json(&take.join("analysis.json"))["far_onset_s"]
        .as_f64()
        .expect("onset");
    let (mic, far, trim) = load(&take, "mic", None);
    let report = find(&mic, &far);
    eprintln!("speakers: {report}");
    let path = report.path.expect("S0.3 found a path here");
    assert!((path.delay_ms() - 46.0).abs() < 0.5, "{report}");
    assert!((path.drift_ppm() - 1.8).abs() < 1.5, "{report}");

    let out = cancel(&mic, &far, path);
    let t = far_times(frame_db(&out.mic).len(), path, trim);
    let far_act = active(&frame_db(&out.reference), -70.0);
    let quiet = within(&t, onset + 0.5, onset + 19.5);
    let far_only = and(&far_act, &quiet);
    let late = and(&far_only, &within(&t, onset + 10.0, f64::MAX));
    let early = and(&far_only, &within(&t, f64::MIN, onset + 10.0));
    let whole = erle_db(&out.mic, &out.full, &far_only);
    let after = erle_db(&out.mic, &out.full, &late);
    let first = erle_db(&out.mic, &out.full, &early);
    let linear = erle_db(&out.mic, &out.linear, &far_only);
    let window = within(&t, onset + 26.5, onset + 75.5);
    let drop_26_76 = erle_db(&out.mic, &out.full, &and(&far_act, &window));
    eprintln!(
        "speakers ERLE: whole {whole:.1} (S0.3 15.6), after 10 s {after:.1} (S0.3 24.3), first 10 s \
         {first:.1} (S0.3 11-12), linear {linear:.1} (S0.3 5.7); 26-76 s window {drop_26_76:.1} dB \
         (S0.3 15.7); far-only frames {} ({} after 10 s)",
        count(&far_only),
        count(&late)
    );
    dump_rows("speakers", &out, &t, &[]);
    assert!(after >= 20.0, "ERLE after 10 s {after:.1} dB");
    assert!(
        (after - 24.3).abs() < 2.0,
        "after 10 s {after:.1} vs S0.3's 24.3"
    );
    assert!(
        (whole - 15.6).abs() < 2.0,
        "whole {whole:.1} vs S0.3's 15.6"
    );
}

/// The earbuds take: neither mic heard the far end. S0.3: no path on either (0 of 81
/// windows).
#[test]
#[ignore = "local: needs INK_BENCH_DIR and S0.3's recordings"]
fn the_earbuds_take_has_no_echo_path_on_either_mic() {
    let take = bench().join("speakers2");
    for role in ["mic", "builtin"] {
        let (mic, far, _) = load(&take, role, None);
        let report = find(&mic, &far);
        eprintln!("earbuds take, {role}: {report}");
        assert!(report.path.is_none(), "{role}: {report}");
    }
}

/// Double talk: the echo-only mic plus the reading. S0.3: path +46.0 ms, +1.6 ppm; ERLE
/// far-only 18.7 dB; the near end in double talk kept at −0.2 dB by the linear stage and −3.2 dB
/// by the full output.
#[test]
#[ignore = "local: needs INK_BENCH_DIR and S0.3's recordings"]
fn the_double_talk_mix_keeps_the_near_end_on_the_linear_output() {
    let s3 = bench();
    let take = s3.join("speakers");
    let mix_dir = s3.join("mix-speakers-speakers2");
    let onset = json(&take.join("analysis.json"))["far_onset_s"]
        .as_f64()
        .expect("onset");
    let result = json(&mix_dir.join("result.json"));
    let near_active_db = result["near_active_db"].as_f64().expect("near_active_db");
    let (mic, far, trim) = load(&take, "mic", Some(&mix_dir.join("mix.f32")));
    let (near, _, _) = load(&take, "mic", Some(&mix_dir.join("near.f32")));
    let report = find(&mic, &far);
    eprintln!("mix: {report}");
    let path = report.path.expect("S0.3 found a path here");
    let out = cancel(&mic, &far, path);
    let n = out.mic.len().min(near.len());
    let t = far_times(frame_db(&out.mic[..n]).len(), path, trim);
    let far_act = active(&frame_db(&out.reference[..n]), -70.0);
    let near_db = frame_db(&near[..n]);
    let near_act = dilate(&active(&near_db, near_active_db), 5, 5);
    let dt = and(&far_act, &near_act);
    let far_only: Vec<bool> = far_act
        .iter()
        .zip(&near_act)
        .map(|(f, a)| *f && !*a)
        .collect();
    let lin = projection_gain_db(&out.linear[..n], &near[..n], &dt);
    let full = projection_gain_db(&out.full[..n], &near[..n], &dt);
    let erle = erle_db(&out.mic[..n], &out.full[..n], &far_only);
    let before = ratio_db(
        ink_echo::measure::energy(&out.mic[..n], &dt),
        ink_echo::measure::energy(&near[..n], &dt),
    );
    eprintln!(
        "mix: double talk {} frames, voice {:.1} dB under the mix (S0.3 6.0); near end kept: linear \
         {lin:+.2} dB (S0.3 -0.2), full {full:+.2} dB (S0.3 -3.2); ERLE far-only {erle:.1} dB \
         (S0.3 18.7)",
        count(&dt),
        before
    );
    let _ = onset;
    dump_rows("mix", &out, &t, &[("near_db", near_db)]);
    assert!(
        lin > -1.0,
        "the linear output lost {lin:.2} dB of the near end"
    );
}

/// S0.3's synthetic mixes. S0.3: delay and drift within ±2 ms and ±5 ppm on spec
/// (+75 ms, +50 ppm) and mic-leads (−75 ms, −50 ppm); no path on no-echo (0 of 89 windows).
#[test]
#[ignore = "local: needs INK_BENCH_DIR and S0.3's synthetic mixes"]
fn the_synthetic_mixes_align_and_the_no_echo_mix_does_not() {
    let dir = bench().join("synthetic");
    for variant in ["spec", "mic-leads", "no-echo"] {
        let v = dir.join(variant);
        let truth = json(&v.join("truth.json"));
        let rate = truth["fs"].as_u64().expect("fs") as u32;
        let mic = to_16k(&read_f32(&v.join("mic.f32")), rate);
        let far = to_16k(&read_f32(&v.join("system.f32")), rate);
        let near = to_16k(&read_f32(&v.join("near.f32")), rate);
        let report = find(&mic, &far);
        eprintln!("synthetic {variant}: {report}");
        if truth["echo"] == Value::Bool(false) {
            assert!(report.path.is_none(), "{variant}: {report}");
            continue;
        }
        let path = report.path.expect("a path");
        let want = truth["expected_delay_ms_at_t0"].as_f64().expect("delay");
        let ppm = truth["drift_ppm"].as_f64().expect("ppm");
        assert!((path.delay_ms() - want).abs() <= 2.0, "{variant}: {report}");
        assert!((path.drift_ppm() - ppm).abs() <= 5.0, "{variant}: {report}");
        let out = cancel(&mic, &far, path);
        let n = out.mic.len().min(near.len());
        let far_act = active(&frame_db(&out.reference[..n]), -70.0);
        let near_act = dilate(&active(&frame_db(&near[..n]), -70.0), 5, 5);
        let far_only: Vec<bool> = far_act
            .iter()
            .zip(&near_act)
            .map(|(f, a)| *f && !*a)
            .collect();
        let t = far_times(far_act.len(), path, 0.0);
        let late = and(&far_only, &within(&t, 10.0, f64::MAX));
        let dt = and(&far_act, &near_act);
        eprintln!(
            "synthetic {variant}: ERLE whole {:.1} dB, after 10 s {:.1} dB (S0.3 37.6), linear {:.1} \
             dB (S0.3 7.0); near end in double talk: linear {:+.2} dB, full {:+.2} dB",
            erle_db(&out.mic[..n], &out.full[..n], &far_only),
            erle_db(&out.mic[..n], &out.full[..n], &late),
            erle_db(&out.mic[..n], &out.linear[..n], &far_only),
            projection_gain_db(&out.linear[..n], &near[..n], &dt),
            projection_gain_db(&out.full[..n], &near[..n], &dt),
        );
        dump_rows(
            &format!("synthetic-{variant}"),
            &out,
            &t,
            &[("near_db", frame_db(&near[..n]))],
        );
    }
}
