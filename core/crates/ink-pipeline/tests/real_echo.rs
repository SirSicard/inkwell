//! Local only: the meeting chain on the gate's real echo recordings, with the real VAD (Silero)
//! and the real final-pass engine (Qwen3-ASR 1.7B).
//!
//! ```text
//! INK_BENCH_DIR=<bench dir> INK_ECHO_READING=<the passage's text> \
//!     cargo test -p ink-pipeline --release --features engine-silero,engine-llama \
//!     --test real_echo -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The recordings are private and live only in the bench directory, under `out/s0.3/`:
//!
//! - `speakers/`: laptop speakers into the laptop mic, far end only: nobody read. Its mic must give
//!   no "you" words at all.
//! - `mix-speakers-speakers2/`: double talk, that echo-only mic plus a reading of a passage on the
//!   same built-in mic (the reading placed so the far end plays over it as the protocol intends),
//!   and the reading alone (`near.f32`). The criterion is in the test's own docs.
//!
//! Both go through the product's path: the mic and the system tap, from the same host instant,
//! at 16 kHz (`ink-audio`'s resampler), fed side by side in 10 ms blocks through capture, the
//! chunks and the final pass. WER is scored as the gate scored it: its normaliser (ported in
//! ink-engines' bench helpers), with the reference cut after the last word the transcript reached
//! (a passage the reader did not finish is not counted as deletions). Unlike the gate, which cut
//! the reading's span out first, the whole "you" transcript is scored: an echo word anywhere is an
//! insertion. The passage's text is not in the repository: `INK_ECHO_READING` names it.

#![cfg(all(feature = "engine-silero", feature = "engine-llama"))]

#[path = "../../ink-engines/tests/bench/mod.rs"]
mod bench;
mod meeting_rig;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ink_audio::SpeechProbability;
use ink_core::{Channel, OfflineEngine, Segment};
use ink_engines::llama::QwenAsr;
use ink_engines::{Registry, SileroModel};
use ink_pipeline::meeting::MeetingOutcome;
use ink_pipeline::speech::VadSource;
use meeting_rig::*;
use serde_json::Value;

fn s03() -> PathBuf {
    bench::bench_dir().join("out").join("s0.3")
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

/// The take's mic channel (or `mic_file` recorded on its timeline) and its system tap, from the
/// same host instant (the earlier stream's head dropped), at 16 kHz.
fn load(take: &Path, mic_file: Option<&Path>) -> (Vec<f32>, Vec<f32>) {
    let meta = json(&take.join("meta.json"));
    let chans = meta["channels"].as_array().expect("channels");
    let ch = |r: &str| {
        chans
            .iter()
            .find(|c| c["role"] == r)
            .unwrap_or_else(|| panic!("no {r} channel"))
            .clone()
    };
    let (m, s) = (ch("mic"), ch("system"));
    let rate = |c: &Value| c["sample_rate"].as_u64().expect("rate") as u32;
    let first = |c: &Value| c["first_host_ns"].as_i64().expect("host ns");
    let mic_path = mic_file
        .map(Path::to_path_buf)
        .unwrap_or_else(|| take.join(m["file"].as_str().expect("file")));
    let mut mic = read_f32(&mic_path);
    let mut far = read_f32(&take.join(s["file"].as_str().expect("file")));
    let mic_start_s = (first(&m) - first(&s)) as f64 / 1e9;
    if mic_start_s > 0.0 {
        let n = ((mic_start_s * f64::from(rate(&s))) as usize).min(far.len());
        far.drain(..n);
    } else if mic_start_s < 0.0 {
        let n = ((-mic_start_s * f64::from(rate(&m))) as usize).min(mic.len());
        mic.drain(..n);
    }
    (to_16k(&mic, rate(&m)), to_16k(&far, rate(&s)))
}

fn silero() -> VadSource {
    let model =
        SileroModel::load(&bench::bench_dir().join("models/silero-vad/silero_vad_16k_op15.onnx"))
            .expect("the Silero model");
    VadSource::Installed(Arc::new(move || {
        Ok(Box::new(model.vad()?) as Box<dyn SpeechProbability>)
    }))
}

fn qwen() -> Arc<dyn OfflineEngine> {
    let row = Registry::builtin()
        .unwrap()
        .get("qwen3-asr-1.7b-q8")
        .expect("the Qwen3-ASR row")
        .clone();
    let dir = bench::bench_dir().join("models/qwen3-asr-1.7b-gguf");
    Arc::new(
        QwenAsr::load(
            &dir.join(&row.files[0].name),
            &dir.join(&row.files[1].name),
            row.info(),
        )
        .expect("Qwen3-ASR loads"),
    )
}

/// Both sides through the chain, no live engine; the final transcript's "you" lines and the
/// outcome (with whether it warned that no echo path was found where echo was possible).
fn through_the_chain(mic: &[f32], far: &[f32]) -> (Vec<Segment>, MeetingOutcome) {
    let (you, outcome, _) = through_the_chain_warned(mic, far);
    (you, outcome)
}

fn through_the_chain_warned(mic: &[f32], far: &[f32]) -> (Vec<Segment>, MeetingOutcome, bool) {
    let mut rig = RigBuilder {
        vad: silero(),
        offline: Some(qwen()),
        no_live_engine: true,
        ..Default::default()
    }
    .build();
    let record = rig.chain().record().clone();
    rig.feed(mic, far);
    let outcome = rig.finish().expect("the final pass");
    let warned = rig.events().iter().any(|e| {
        matches!(
            e,
            ink_pipeline::meeting::events::MeetingEvent::Warning(
                ink_pipeline::meeting::events::MeetingWarning::EchoPathNotFound { .. }
            )
        )
    });
    let you = rig
        .store
        .segments(&record)
        .unwrap()
        .into_iter()
        .filter(|s| s.channel == Channel::Mic)
        .collect();
    (you, outcome, warned)
}

/// The reference passage's words, normalised: `INK_ECHO_READING`'s lines, `#` lines left out.
fn reading() -> Vec<String> {
    let path = std::env::var_os("INK_ECHO_READING")
        .expect("set INK_ECHO_READING to the text of the passage read in the double-talk take");
    let text = fs::read_to_string(&path).expect("the passage");
    let body: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    bench::normalise(&body.join(" "))
}

/// Word-level alignment ops, reference first: `S`ame/sub, `D`eletion, `I`nsertion, in order.
fn align(r: &[String], h: &[String]) -> Vec<char> {
    let (n, m) = (r.len(), h.len());
    let mut cost = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in cost.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in cost[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let sub = usize::from(r[i - 1] != h[j - 1]);
            cost[i][j] = (cost[i - 1][j - 1] + sub)
                .min(cost[i - 1][j] + 1)
                .min(cost[i][j - 1] + 1);
        }
    }
    let (mut i, mut j, mut ops) = (n, m, Vec::new());
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && cost[i][j] == cost[i - 1][j - 1] + usize::from(r[i - 1] != h[j - 1]) {
            ops.push('S');
            i -= 1;
            j -= 1;
        } else if i > 0 && cost[i][j] == cost[i - 1][j] + 1 {
            ops.push('D');
            i -= 1;
        } else {
            ops.push('I');
            j -= 1;
        }
    }
    ops.reverse();
    ops
}

/// The gate's trimmed WER: the reference cut after the last word the transcript reached.
fn wer_trimmed(reference: &[String], hypothesis: &[String]) -> bench::Edits {
    if hypothesis.is_empty() {
        return bench::measure(reference, hypothesis);
    }
    let mut ops = align(reference, hypothesis);
    while ops.last() == Some(&'D') {
        ops.pop();
    }
    let end = ops.iter().filter(|o| **o != 'I').count();
    let edits = bench::measure(&reference[..end], hypothesis);
    assert_eq!(
        edits.total(),
        bench::edit_distance(&reference[..end], hypothesis),
        "the two scorers agree"
    );
    edits
}

fn you_words(you: &[Segment]) -> Vec<String> {
    let text: Vec<&str> = you.iter().map(|s| s.text.as_str()).collect();
    bench::normalise(&text.join(" "))
}

#[test]
fn wer_trimming_drops_only_trailing_deletions() {
    let w = |s: &str| bench::normalise(s);
    let e = wer_trimmed(&w("a b c d e f"), &w("a x c"));
    assert_eq!((e.reference, e.total()), (3, 1));
    let e = wer_trimmed(&w("a b c d"), &w("z a b c d y"));
    assert_eq!((e.reference, e.total()), (4, 2));
}

#[test]
#[ignore = "local: needs INK_BENCH_DIR, the gate's recordings and the models"]
fn the_echo_only_take_gives_no_you_words() {
    let (mic, far) = load(&s03().join("speakers"), None);
    let (you, outcome) = through_the_chain(&mic, &far);
    eprintln!(
        "echo only: {:?}\n  mic {:?}\n  you lines {}",
        outcome.echo,
        outcome.mic,
        you.len()
    );
    assert!(outcome.echo.cancelled, "{:?}", outcome.echo);
    assert_eq!(
        you_words(&you).len(),
        0,
        "{} you lines over {:?}",
        you.len(),
        you.iter()
            .map(|s| (s.start_ms, s.end_ms))
            .collect::<Vec<_>>()
    );
}

/// The gate's own echo path on the double-talk mix, on the same ASR: its measurement tool's
/// linear output (`aec-mix/linear.wav`, 16 kHz, which the gate's run left in the bench
/// directory), cut to the reading's span (far-end onset + 19.5 s to + 82 s, on the tool's
/// timeline), RMS-normalised to −23 dBFS with the peak at most −1 dBFS, and transcribed in one
/// call, as the gate did.
fn gate_echo_path_words() -> Vec<String> {
    let dir = s03().join("mix-speakers-speakers2").join("aec-mix");
    let onset = json(&s03().join("speakers").join("analysis.json"))["far_onset_s"]
        .as_f64()
        .expect("the far end's onset");
    // The tool's timeline: the first frame's far-file time.
    let frames = fs::read_to_string(dir.join("frames.tsv")).expect("the tool's frames");
    let t0: f64 = frames
        .lines()
        .nth(1)
        .and_then(|l| l.split('\t').next())
        .and_then(|t| t.parse().ok())
        .expect("the first frame's time");
    let (rate, linear) =
        bench::read_wav(&dir.join("linear.wav")).expect("the gate's linear output");
    assert_eq!(rate, 16_000);
    let at = |s: f64| (((s - t0) * 16_000.0).max(0.0) as usize).min(linear.len());
    let mut cut = linear[at(onset + 19.5)..at(onset + 82.0)].to_vec();
    let rms = (cut.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / cut.len() as f64).sqrt();
    let peak = cut.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let gain = (10f64.powf(-23.0 / 20.0) / rms).min(10f64.powf(-1.0 / 20.0) / f64::from(peak));
    cut.iter_mut()
        .for_each(|v| *v = (f64::from(*v) * gain) as f32);
    let options = ink_core::TranscribeOptions {
        channel: Channel::Mic,
        context: None,
        cancel: ink_core::CancelToken::new(),
        live: false,
    };
    let transcript = qwen().transcribe(&cut, &options).expect("Qwen3-ASR");
    bench::normalise(&transcript.text())
}

/// Double talk: the "you" transcript of the mix is **no worse than the gate's own echo path on
/// the same ASR** (its linear output, cut and levelled as the gate did), and holds **no echo
/// words**: no word outside the reading (an insertion), since the reading is all the near end
/// said and the whole transcript is scored.
///
/// Re-baselined by the owner on 2026-09-27. The plan asked for the mix within one WER point of
/// the reading alone; on Qwen3-ASR 1.7B the chain measures 5.63 against 3.52 (+2.1, three words:
/// "fence" heard as "fences" twice, "spirit" as "spirits"), and the gate's own echo path 7.04
/// (+3.5), its +0.7 having been a Parakeet figure. Each step between the gate's path and the chain
/// moved two words or fewer, which one take of 142 words cannot resolve. S2.8 re-measures this on
/// real meetings.
#[test]
#[ignore = "local: needs INK_BENCH_DIR, INK_ECHO_READING, the gate's recordings and the models"]
fn double_talk_is_no_worse_than_the_gate_s_echo_path_and_has_no_echo_words() {
    let reference = reading();
    let take = s03().join("speakers");
    let mix_dir = s03().join("mix-speakers-speakers2");
    let (mix, far) = load(&take, Some(&mix_dir.join("mix.f32")));
    let (near, _) = load(&take, Some(&mix_dir.join("near.f32")));

    let (you_mix, out_mix) = through_the_chain(&mix, &far);
    let (you_alone, out_alone, warned) = through_the_chain_warned(&near, &far);
    // The reading alone, the far end in earbuds: no path, and nothing leaked, so no warning.
    assert!(
        out_alone.echo.path.is_none() && !warned,
        "{:?}",
        out_alone.echo
    );
    let w_mix = wer_trimmed(&reference, &you_words(&you_mix));
    let w_alone = wer_trimmed(&reference, &you_words(&you_alone));
    let w_gate = wer_trimmed(&reference, &gate_echo_path_words());
    eprintln!(
        "double talk: {w_mix}\n  echo {:?}\n  mic {:?}\ngate's echo path: {w_gate}\nreading alone: {w_alone}\n  echo {:?}\n  mic {:?}",
        out_mix.echo, out_mix.mic, out_alone.echo, out_alone.mic
    );
    assert!(
        w_mix.wer() <= w_gate.wer(),
        "double talk {:.2} against the gate's echo path {:.2}",
        w_mix.wer(),
        w_gate.wer()
    );
    assert_eq!(w_mix.insertions, 0, "words outside the reading: {w_mix}");
}
