//! Silero VAD on the real model (`engine-silero`).
//!
//! No ONNX reference runtime is part of this build, so the adapter is proved by behaviour rather
//! than by numerical parity. The model comes from `INK_SILERO_MODEL`, the path of
//! `silero_vad_16k_op15.onnx` (checked here against the registry row's size and SHA-256; CI
//! downloads it from the pinned commit and sets the variable).
//!
//! - **Model only** (run whenever `INK_SILERO_MODEL` is set; skipped, with a note, when it is
//!   not): golden probabilities on fixed synthetic inputs, determinism, state carried from window
//!   to window, a reset that restores the initial state exactly, clicks and typing, and
//!   ink-audio's synthetic speech.
//! - **`#[ignore]`, local:** the AMI tests need `INK_BENCH_DIR` holding `ami-ihm.tsv` and the
//!   clips it lists under `ami-ihm/` (AMI individual headset mics, 16 kHz); the full noise
//!   measurement takes about a minute in a debug build.
//!
//! ```text
//! INK_SILERO_MODEL=<model> INK_BENCH_DIR=<bench data> \
//!     cargo test -p ink-engines --features engine-silero --release -- --include-ignored
//! ```
//!
//! Every test that stands for the pipeline feeds Silero what the pipeline feeds it: audio lifted
//! by the provisional gain (`ink_audio::gain`), never the raw take.

#![cfg(feature = "engine-silero")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ink_audio::gain::{
    self, GainEvidence, GainOutcome, TARGET_PEAK, apply_gain, from_dbfs, levels, provisional_gain,
    rms, to_dbfs,
};
use ink_audio::synth::{
    Lcg, Slope, SpeechShape, cycling_fan, knocks, noise, rumble, speech_like, speech_with, swing,
};
use ink_audio::vad::{self, SpeechProbability, VAD_WINDOW, VadConfig};
use ink_core::mock::MockClock;
use ink_core::{EngineError, Job};
use ink_engines::{ModelDir, Residency, SileroLoader, SileroModel, silero_vad};
use sha2::{Digest, Sha256};

// --- The model and the bench data. --------------------------------------------------------------

/// The model file `INK_SILERO_MODEL` names, if it is set.
fn model_path() -> Option<PathBuf> {
    std::env::var_os("INK_SILERO_MODEL").map(PathBuf::from)
}

/// The model at `path`, after checking the file is exactly the registry row's.
fn load(path: &Path) -> SileroModel {
    let row = silero_vad();
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(bytes.len() as u64, row.files[0].size, "model size");
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(sha, row.files[0].sha256, "model hash");
    SileroModel::load(path).unwrap()
}

/// The model, for the model-only tests: `None`, after saying so, when `INK_SILERO_MODEL` is not
/// set. CI always sets it.
fn model_if_configured(test: &str) -> Option<SileroModel> {
    match model_path() {
        Some(path) => Some(load(&path)),
        None => {
            eprintln!("{test}: skipped, INK_SILERO_MODEL is not set");
            None
        }
    }
}

/// The model, for the `#[ignore]` tests, which are run on purpose.
fn model() -> SileroModel {
    load(&model_path().expect("INK_SILERO_MODEL must name the Silero model file"))
}

fn bench() -> PathBuf {
    PathBuf::from(
        std::env::var_os("INK_BENCH_DIR")
            .expect("INK_BENCH_DIR must name the local benchmark directory"),
    )
}

/// A WAV as 16 kHz mono f32.
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
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32_768.0)
            .collect(),
    }
}

/// The AMI clips `ami-ihm.tsv` lists, with the number of words in each one's reference.
fn ami_clips() -> Vec<(String, Vec<f32>, usize)> {
    let tsv = fs::read_to_string(bench().join("ami-ihm.tsv")).unwrap();
    let clips: Vec<_> = tsv
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            let (file, text) = (cols[1], cols[2]);
            let audio = read_wav(&bench().join("ami-ihm").join(file));
            (file.to_string(), audio, text.split_whitespace().count())
        })
        .collect();
    assert!(!clips.is_empty(), "ami-ihm.tsv lists no clips");
    clips
}

/// `audio` lifted by its provisional gain, as the gain stage hands it to the VAD.
fn lifted(audio: &[f32]) -> Vec<f32> {
    let mut out = audio.to_vec();
    apply_gain(&mut out, provisional_gain(&levels(audio)));
    out
}

/// The probability of every window of `audio`, from a reset source (the last window zero-padded,
/// as ink-audio pads it).
fn probabilities(vad: &mut dyn SpeechProbability, audio: &[f32]) -> Vec<f32> {
    vad.reset();
    audio
        .chunks(VAD_WINDOW)
        .map(|chunk| {
            let mut window = [0.0f32; VAD_WINDOW];
            window[..chunk.len()].copy_from_slice(chunk);
            vad.probability(&window).unwrap()
        })
        .collect()
}

/// Each window's RMS in dBFS.
fn window_dbfs(audio: &[f32]) -> Vec<f32> {
    audio.chunks(VAD_WINDOW).map(|w| to_dbfs(rms(w))).collect()
}

/// The energy oracle for close-talk headset audio: windows at or above −40 dBFS RMS are speech;
/// windows at least 256 ms into a run of at least 512 ms below −60 dBFS are pauses (the first
/// 256 ms of a pause are left out: speech decays into them). Everything else is not judged.
fn oracle(dbfs: &[f32]) -> (Vec<usize>, Vec<usize>) {
    const SPEECH_DB: f32 = -40.0;
    const PAUSE_DB: f32 = -60.0;
    const MIN_RUN: usize = 16;
    const SKIP: usize = 8;
    let speech = (0..dbfs.len()).filter(|&k| dbfs[k] >= SPEECH_DB).collect();
    let mut pauses = Vec::new();
    let mut run: Vec<usize> = Vec::new();
    for k in 0..=dbfs.len() {
        if k < dbfs.len() && dbfs[k] < PAUSE_DB {
            run.push(k);
            continue;
        }
        if run.len() >= MIN_RUN {
            pauses.extend(run.iter().skip(SKIP));
        }
        run.clear();
    }
    (speech, pauses)
}

fn share(ps: &[f32], windows: &[usize], keep: impl Fn(f32) -> bool) -> f64 {
    windows.iter().filter(|&&k| keep(ps[k])).count() as f64 / windows.len().max(1) as f64
}

fn mean(ps: &[f32], windows: &[usize]) -> f64 {
    windows.iter().map(|&k| f64::from(ps[k])).sum::<f64>() / windows.len().max(1) as f64
}

// --- Model only: golden values, state, reset, determinism. ---------------------------------------

/// The fixed inputs the golden values were recorded on, each 32 windows (1.024 s): knocks lifted
/// by their provisional gain (the model briefly scores them as speech, so the values span
/// 0.006-0.754), synthetic voiced syllables at -30 dBFS, and white noise at -30 dBFS.
fn golden_inputs() -> [(&'static str, Vec<f32>, &'static [f32; 32]); 3] {
    let seconds = 32.0 * VAD_WINDOW as f64 / 16_000.0;
    [
        (
            "knocks, lifted",
            lifted(&knocks(seconds, -60.0, 31)),
            &GOLDEN_KNOCKS,
        ),
        (
            "voiced syllables",
            speech_like(seconds, -30.0, 1),
            &GOLDEN_SYLLABLES,
        ),
        ("white noise", noise(seconds, -30.0, 3), &GOLDEN_NOISE),
    ]
}

/// How far a probability may sit from its recorded value: float summation order differs between
/// tract's arm64 and x86-64 kernels, by far less than this.
const GOLDEN_TOLERANCE: f32 = 1e-4;

/// Recorded on 2026-09-26 on Apple silicon from the hash-checked model, the recurrent state
/// carried from each window to the next (from a reset), to six decimals.
const GOLDEN_KNOCKS: [f32; 32] = [
    0.006326, 0.015979, 0.024683, 0.022962, 0.017245, 0.009920, 0.754430, 0.707930, 0.681417,
    0.735008, 0.641876, 0.608913, 0.383096, 0.243133, 0.193721, 0.137761, 0.162783, 0.078627,
    0.078575, 0.081253, 0.066716, 0.036225, 0.025447, 0.019592, 0.018586, 0.014942, 0.014057,
    0.012963, 0.011343, 0.012897, 0.011626, 0.009548,
];

/// As [`GOLDEN_KNOCKS`].
const GOLDEN_SYLLABLES: [f32; 32] = [
    0.018156, 0.087514, 0.040418, 0.013598, 0.010126, 0.039988, 0.024253, 0.008452, 0.004753,
    0.024392, 0.007472, 0.006975, 0.003528, 0.002010, 0.003168, 0.009627, 0.008344, 0.003258,
    0.014187, 0.036068, 0.010056, 0.005536, 0.006815, 0.013444, 0.003515, 0.001787, 0.004015,
    0.008688, 0.002042, 0.003072, 0.001965, 0.002226,
];

/// As [`GOLDEN_KNOCKS`].
const GOLDEN_NOISE: [f32; 32] = [
    0.019195, 0.011705, 0.016226, 0.019831, 0.014961, 0.012097, 0.013719, 0.011577, 0.010578,
    0.010964, 0.010980, 0.009364, 0.020695, 0.012010, 0.011694, 0.015268, 0.012230, 0.015803,
    0.010991, 0.027451, 0.014658, 0.009195, 0.008911, 0.021160, 0.014008, 0.018163, 0.012235,
    0.009562, 0.011455, 0.021907, 0.012636, 0.012573,
];

#[test]
fn golden_probabilities_on_fixed_synthetic_inputs() {
    // What proves the rewritten graph computes what the model computes: a wrong branch, a
    // dropped context or state, or a changed kernel moves these by far more than the tolerance.
    let Some(model) = model_if_configured("golden_probabilities_on_fixed_synthetic_inputs") else {
        return;
    };
    for (name, audio, golden) in golden_inputs() {
        let ps = probabilities(&mut model.vad().unwrap(), &audio);
        assert_eq!(ps.len(), golden.len(), "{name}");
        for (k, (p, g)) in ps.iter().zip(golden.iter()).enumerate() {
            assert!(
                (p - g).abs() <= GOLDEN_TOLERANCE,
                "{name}, window {k}: {p:.6} against the recorded {g:.6}"
            );
        }
    }
}

#[test]
fn output_is_bit_identical_across_sessions_and_loads() {
    let Some(first) = model_if_configured("output_is_bit_identical_across_sessions_and_loads")
    else {
        return;
    };
    let second = model();
    for (name, audio, _) in golden_inputs() {
        let a = probabilities(&mut first.vad().unwrap(), &audio);
        let b = probabilities(&mut first.vad().unwrap(), &audio);
        let c = probabilities(&mut second.vad().unwrap(), &audio);
        assert!(a.iter().all(|p| (0.0..=1.0).contains(p)), "{name}");
        assert_eq!(a, b, "{name}: two sessions of one load");
        assert_eq!(a, c, "{name}: two loads");
    }
}

#[test]
fn the_state_carries_from_window_to_window() {
    // The same windows scored with the recurrent state and context carried, and with both cleared
    // before every window. Measured: carried, the knocks peak at 0.754 and cleared at 0.148; the
    // mean difference is 0.165 on the knocks and 0.136 on the syllables.
    let Some(model) = model_if_configured("the_state_carries_from_window_to_window") else {
        return;
    };
    for (name, audio, _) in golden_inputs().into_iter().take(2) {
        let carried = probabilities(&mut model.vad().unwrap(), &audio);
        let mut vad = model.vad().unwrap();
        let cleared: Vec<f32> = audio
            .chunks_exact(VAD_WINDOW)
            .map(|w| {
                vad.reset();
                vad.probability(w.try_into().unwrap()).unwrap()
            })
            .collect();
        let diff = carried
            .iter()
            .zip(&cleared)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / carried.len() as f32;
        assert!(diff > 0.1, "{name}: mean difference {diff:.3}");
    }
}

#[test]
fn a_reset_restores_the_initial_state_exactly() {
    let Some(model) = model_if_configured("a_reset_restores_the_initial_state_exactly") else {
        return;
    };
    let [(_, knocks, _), (_, syllables, _), _] = golden_inputs();
    let fresh = probabilities(&mut model.vad().unwrap(), &knocks);
    let mut used = model.vad().unwrap();
    probabilities(&mut used, &syllables);
    // `probabilities` resets first.
    assert_eq!(probabilities(&mut used, &knocks), fresh);
}

// --- Speech and pauses. -------------------------------------------------------------------------

#[test]
#[ignore = "needs INK_SILERO_MODEL and the AMI clips under INK_BENCH_DIR"]
fn speech_scores_high_and_pauses_low_on_ami_ihm() {
    // Measured on 2026-09-26: speech windows 96.5-98.7 % at >= 0.5 (mean 0.96-0.97); pause
    // windows 99.1-100 % below 0.35 (mean 0.04-0.05). The share of all judged windows it
    // misjudges is the registry row's error rate.
    let model = model();
    let (mut judged, mut wrong) = (0usize, 0usize);
    for (name, audio, _) in ami_clips() {
        let heard = lifted(&audio);
        let ps = probabilities(&mut model.vad().unwrap(), &heard);
        let (speech, pauses) = oracle(&window_dbfs(&audio));
        assert!(
            speech.len() > 200 && pauses.len() > 100,
            "{name}: too little to judge"
        );
        let hit = share(&ps, &speech, |p| p >= 0.5);
        let quiet = share(&ps, &pauses, |p| p < 0.35);
        judged += speech.len() + pauses.len();
        wrong += speech.iter().filter(|&&k| ps[k] < 0.5).count()
            + pauses.iter().filter(|&&k| ps[k] >= 0.35).count();
        println!(
            "{name}: speech {} windows, {:.1} % >= 0.5 (mean {:.3}); pauses {} windows, {:.1} % < 0.35 (mean {:.3})",
            speech.len(),
            100.0 * hit,
            mean(&ps, &speech),
            pauses.len(),
            100.0 * quiet,
            mean(&ps, &pauses)
        );
        assert!(
            hit >= 0.95 && mean(&ps, &speech) >= 0.9,
            "{name}: speech missed"
        );
        assert!(
            quiet >= 0.97 && mean(&ps, &pauses) <= 0.1,
            "{name}: pauses heard as speech"
        );
    }
    let rate = 100.0 * wrong as f64 / judged as f64;
    let row = f64::from(silero_vad().wer(Job::VoiceActivity).unwrap());
    println!("misjudged {wrong} of {judged} windows: {rate:.2} %; the registry row carries {row}");
    assert!((rate - row).abs() < 0.05, "{rate:.2} against {row}");
}

#[test]
#[ignore = "needs INK_SILERO_MODEL and the AMI clips under INK_BENCH_DIR"]
fn detected_speech_matches_the_reference_word_rate() {
    // A sanity check against the references' words: the speech the segmenter finds, at
    // ink-audio's defaults, holds conversational speech at 2-6 words a second. Everything called
    // speech would dilute it; too little would inflate it. Measured: 3.5-4.4.
    let model = model();
    let cfg = VadConfig::default();
    for (name, audio, words) in ami_clips() {
        let segments =
            vad::speech_segments(&lifted(&audio), &mut model.vad().unwrap(), &cfg).unwrap();
        let seconds: f64 = segments
            .iter()
            .map(|s| (s.end - s.start) as f64 * VAD_WINDOW as f64 / 16_000.0)
            .sum();
        let rate = words as f64 / seconds;
        println!(
            "{name}: {words} words over {seconds:.1} s of detected speech ({:.1} s of audio): {rate:.2} words/s",
            audio.len() as f64 / 16_000.0
        );
        assert!((2.0..=6.0).contains(&rate), "{name}: {rate:.2} words/s");
    }
}

#[test]
#[ignore = "needs INK_SILERO_MODEL and the AMI clips under INK_BENCH_DIR"]
fn quiet_ami_speech_is_heard_through_the_provisional_gain() {
    // 4 s takes of AMI speech at -75 dBFS RMS. Heard raw, Silero misses most of them; heard
    // through the provisional gain, as normalise_speech hands them over, every take is found and
    // lifted, and its speech frames reach the target. Measured: raw found 23 of 57 takes.
    let model = model();
    let cfg = VadConfig::default();
    let (mut takes, mut raw_found) = (0, 0);
    for (name, audio, _) in ami_clips() {
        for (k, chunk) in audio.chunks_exact(64_000).enumerate() {
            let scale = from_dbfs(-75.0) / rms(chunk);
            let quiet: Vec<f32> = chunk.iter().map(|s| s * scale).collect();
            let mut vad = model.vad().unwrap();
            takes += 1;
            if !vad::speech_segments(&quiet, &mut vad, &cfg)
                .unwrap()
                .is_empty()
            {
                raw_found += 1;
            }
            let mut take = quiet.clone();
            let report = gain::normalise_speech(&mut take, &mut vad, &cfg).unwrap();
            assert!(
                matches!(report.outcome, GainOutcome::Applied { .. }),
                "{name} take {k}: {report:?}"
            );
            // The speech Silero found in the lifted copy (the same pass normalise_speech made)
            // now sits at the target.
            let segments = vad::speech_segments(&lifted(&quiet), &mut vad, &cfg).unwrap();
            let speech = gain::speech_levels(&take, &segments).unwrap();
            let off = to_dbfs(speech.robust_peak) - to_dbfs(TARGET_PEAK);
            assert!(
                off.abs() < 0.05,
                "{name} take {k}: speech {off:+.3} dB from the target"
            );
        }
    }
    println!("-75 dBFS takes: {takes}; Silero on the raw take found speech in {raw_found}");
    assert!(
        raw_found * 2 < takes,
        "the raw takes were not quiet enough to test this"
    );
}

#[test]
#[ignore = "needs INK_SILERO_MODEL and the AMI clips under INK_BENCH_DIR"]
fn the_state_carries_from_window_to_window_on_speech() {
    // The same windows scored with the recurrent state and context carried, and with both cleared
    // before every window. Carried, the scores follow the speech; cleared, they collapse. So the
    // adapter feeds each window's state and context into the next.
    let model = model();
    for (name, audio, _) in ami_clips() {
        let heard = lifted(&audio);
        let carried = probabilities(&mut model.vad().unwrap(), &heard);
        let mut vad = model.vad().unwrap();
        let cleared: Vec<f32> = heard
            .chunks_exact(VAD_WINDOW)
            .map(|w| {
                vad.reset();
                vad.probability(w.try_into().unwrap()).unwrap()
            })
            .collect();
        let (speech, _) = oracle(&window_dbfs(&audio));
        let speech: Vec<usize> = speech.into_iter().filter(|&k| k < cleared.len()).collect();
        let (with, without) = (mean(&carried, &speech), mean(&cleared, &speech));
        println!("{name}: mean speech score {with:.3} with state, {without:.3} cleared");
        assert!(
            with - without > 0.2,
            "{name}: {with:.3} against {without:.3}"
        );
    }
}

// --- Noise: the S1.2b follow-up. ----------------------------------------------------------------

/// ink-audio's non-speech fixtures, as its gain tests build them: room tone, gentle and steep
/// rumble from 100 Hz to 1 kHz, rumble swinging slowly in level, a cycling fan and knocks.
fn non_speech_fixtures(seconds: f64, seed: u64) -> Vec<(String, Vec<f32>)> {
    let mut out = vec![("white room tone".to_string(), noise(seconds, -70.0, seed))];
    for (i, slope) in [Slope::Gentle, Slope::Steep].into_iter().enumerate() {
        for (j, hz) in [100.0, 250.0, 500.0, 1_000.0].into_iter().enumerate() {
            let seed = seed + 1 + (i * 4 + j) as u64;
            out.push((
                format!("{slope:?} {hz} Hz rumble"),
                rumble(seconds, -70.0, hz, slope, seed),
            ));
        }
        for period in [8.0, 20.0] {
            for depth in [10.0, 15.0] {
                let base = rumble(seconds, -70.0, 250.0, slope, seed + 20 + i as u64);
                out.push((
                    format!("{slope:?} 250 Hz rumble swinging {depth} dB every {period} s"),
                    swing(&base, period, depth),
                ));
            }
        }
    }
    out.push((
        "cycling fan".to_string(),
        cycling_fan(seconds, -60.0, seed + 30),
    ));
    out.push(("knocks".to_string(), knocks(seconds, -60.0, seed + 31)));
    out
}

/// Typing: per key a press click and a release click 60-120 ms later, each 2-6 ms of broadband
/// noise under a 1-2 ms decay, `keys_per_second` on average, over room tone 45 dB below. Scaled
/// to `rms_dbfs` RMS.
fn keyboard(seconds: f64, rms_dbfs: f32, keys_per_second: f64, seed: u64) -> Vec<f32> {
    const RATE: f64 = 16_000.0;
    let total = (seconds * RATE) as usize;
    let mut rng = Lcg::new(seed);
    let mut out: Vec<f64> = (0..total).map(|_| 0.005 * rng.next_gaussian()).collect();
    let mut at = (0.1 * RATE) as usize;
    while at < total {
        for (click, amplitude) in [(0, 1.0), (1, 0.6)] {
            let start = at + click * ((0.06 + 0.06 * rng.next_f64()) * RATE) as usize;
            let tau = 0.001 + 0.001 * rng.next_f64();
            for (i, v) in out
                .iter_mut()
                .skip(start)
                .take((0.02 * RATE) as usize)
                .enumerate()
            {
                *v += amplitude * rng.next_gaussian() * (-(i as f64) / RATE / tau).exp();
            }
        }
        at += ((0.5 + rng.next_f64()) / keys_per_second * RATE) as usize;
    }
    let ms = out.iter().map(|v| v * v).sum::<f64>() / out.len() as f64;
    let scale = f64::from(from_dbfs(rms_dbfs)) / ms.sqrt();
    out.iter().map(|v| (v * scale) as f32).collect()
}

/// Room tone with one near-full-scale two-sample click in the middle: the hotkey's own click.
fn hotkey_click(seconds: f64, seed: u64) -> Vec<f32> {
    let mut x = noise(seconds, -70.0, seed);
    let mid = x.len() / 2;
    x[mid] = 0.9;
    x[mid + 1] = -0.9;
    x
}

/// What Silero did with one fixture's 4 s takes, each heard through its provisional gain.
#[derive(Debug, Default)]
struct FalsePositives {
    takes: usize,
    /// Windows scored at or above the speech threshold.
    windows: usize,
    /// Speech segments the segmenter closed (at least 64 ms each).
    segments: usize,
    /// The longest, in windows.
    longest: usize,
    /// Takes whose gain normalise_speech set from those segments.
    gain_set: usize,
    /// Level frames the gain learned from, in the take that learned from the fewest.
    fewest_frames: Option<usize>,
}

fn false_positives(model: &SileroModel, audio: &[f32]) -> FalsePositives {
    let cfg = VadConfig::default();
    let mut out = FalsePositives::default();
    for take in audio.chunks_exact(64_000) {
        out.takes += 1;
        let heard = lifted(take);
        let mut vad = model.vad().unwrap();
        out.windows += probabilities(&mut vad, &heard)
            .iter()
            .filter(|&&p| p >= cfg.threshold)
            .count();
        let segments = vad::speech_segments(&heard, &mut vad, &cfg).unwrap();
        out.segments += segments.len();
        out.longest = segments
            .iter()
            .map(|s| s.end - s.start)
            .fold(out.longest, usize::max);
        let mut raw = take.to_vec();
        let report = gain::normalise_speech(&mut raw, &mut vad, &cfg).unwrap();
        if let (GainOutcome::Applied { .. }, GainEvidence::Vad { speech_frames, .. }) =
            (report.outcome, report.evidence)
        {
            out.gain_set += 1;
            out.fewest_frames = Some(
                out.fewest_frames
                    .map_or(speech_frames, |f| f.min(speech_frames)),
            );
        }
    }
    out
}

#[test]
#[ignore = "a measurement: needs INK_SILERO_MODEL, and about a minute in a debug build"]
fn non_speech_false_positives_are_short_and_set_no_gain() {
    // Whether a false positive can set a take's gain: 40 s of each of ink-audio's non-speech fixtures in 4 s takes,
    // each lifted by its provisional gain as normalise_speech lifts it.
    //
    // Measured on 2026-09-26: no speech in room tone, gentle rumble, any swinging rumble, the fan,
    // typing or the hotkey click. Two fixtures fool it briefly: steep 500 Hz rumble (one 96 ms
    // segment in 10 takes) and knocks (5 segments of 128-160 ms in 2 takes). Before ink-audio's
    // 192 ms level-segment minimum each of those takes had its gain set (from 5, 14 and 23
    // frames); now none does.
    let model = model();
    let mut fixtures = non_speech_fixtures(40.0, 200);
    fixtures.push(("typing, 6 keys/s".into(), keyboard(40.0, -60.0, 6.0, 300)));
    fixtures.push(("typing, 9 keys/s".into(), keyboard(40.0, -55.0, 9.0, 301)));
    fixtures.push(("hotkey click".into(), hotkey_click(40.0, 302)));
    let (mut longest, mut gain_set) = (0, 0);
    let mut fooled = Vec::new();
    println!("fixture | takes | windows >= 0.5 | segments | longest ms | gain set");
    for (name, audio) in &fixtures {
        let fp = false_positives(&model, audio);
        println!(
            "{name} | {} | {} | {} | {} | {}",
            fp.takes,
            fp.windows,
            fp.segments,
            fp.longest * 32,
            fp.gain_set,
        );
        longest = longest.max(fp.longest);
        gain_set += fp.gain_set;
        if fp.segments > 0 {
            fooled.push(name.as_str());
        }
    }
    // The minimum rests on this: every false segment is at most 5 windows (160 ms), one short of
    // ink_audio::gain::MIN_LEVEL_SEGMENT_WINDOWS, and only these two fixtures produce one.
    assert!(
        longest < gain::MIN_LEVEL_SEGMENT_WINDOWS,
        "a false segment of {} ms",
        longest * 32
    );
    assert_eq!(fooled, ["Steep 500 Hz rumble", "knocks"]);
    assert_eq!(gain_set, 0);
}

#[test]
#[ignore = "needs INK_SILERO_MODEL and the AMI clips under INK_BENCH_DIR"]
fn the_level_segment_minimum_costs_real_speech_little() {
    // AMI speech cut into takes of 4, 2 and 1 s (the last at arbitrary points, so many start or
    // end mid-word), lifted as normalise_speech lifts them: the takes in which Silero finds speech
    // but every segment is shorter than the 192 ms minimum, so the take now has no speech.
    // Measured: 0 of 57, 0 of 115 and 4 of 222.
    let model = model();
    let cfg = VadConfig::default();
    let clips = ami_clips();
    for (seconds, allowed) in [(4usize, 0usize), (2, 0), (1, 10)] {
        let (mut with_speech, mut lost) = (0, 0);
        for (_, audio, _) in &clips {
            for take in audio.chunks_exact(seconds * 16_000) {
                let mut vad = model.vad().unwrap();
                if vad::speech_segments(&lifted(take), &mut vad, &cfg)
                    .unwrap()
                    .is_empty()
                {
                    continue;
                }
                with_speech += 1;
                let mut out = take.to_vec();
                let report = gain::normalise_speech(&mut out, &mut vad, &cfg).unwrap();
                if report.outcome == GainOutcome::NoSpeech {
                    lost += 1;
                }
            }
        }
        println!("{seconds} s takes: {lost} of {with_speech} with speech learn no level");
        assert!(lost <= allowed, "{seconds} s takes: {lost} lost");
    }
}

#[test]
fn clicks_and_typing_never_reach_the_minimum_segment() {
    // Typing at two speeds and the hotkey's own click, lifted as a take would be: no window even
    // reaches the speech threshold, so nothing gets near the 64 ms minimum segment.
    let Some(model) = model_if_configured("clicks_and_typing_never_reach_the_minimum_segment")
    else {
        return;
    };
    for (name, audio) in [
        ("typing, 6 keys/s", keyboard(40.0, -60.0, 6.0, 300)),
        ("typing, 9 keys/s", keyboard(40.0, -55.0, 9.0, 301)),
        ("hotkey click", hotkey_click(40.0, 302)),
    ] {
        let fp = false_positives(&model, &audio);
        assert_eq!(
            (fp.windows, fp.segments, fp.gain_set),
            (0, 0, 0),
            "{name}: {fp:?}"
        );
    }
}

#[test]
fn ink_audio_synthetic_speech_is_not_speech_to_silero() {
    // A finding for the pipeline's tests: the synthetic voiced-syllable fixtures that ink-audio's
    // gain tests drive with scripted VADs are not speech to the real model at any syllable rate.
    // With Silero installed they are discarded as NoSpeech, so tests that need a real VAD must use
    // real speech (AMI).
    let Some(model) = model_if_configured("ink_audio_synthetic_speech_is_not_speech_to_silero")
    else {
        return;
    };
    let cfg = VadConfig::default();
    let mut takes = vec![("speech_like".to_string(), speech_like(4.0, -75.0, 1))];
    for syllable in [0.12, 0.15, 0.20, 0.26] {
        for (kind, shape) in [
            ("plain", SpeechShape::plain(syllable)),
            ("breathy", SpeechShape::breathy(syllable)),
        ] {
            takes.push((
                format!("{kind}, {syllable} s"),
                speech_with(4.0, -75.0, shape, 100),
            ));
        }
    }
    for (name, mut take) in takes {
        let report = gain::normalise_speech(&mut take, &mut model.vad().unwrap(), &cfg).unwrap();
        assert_eq!(report.outcome, GainOutcome::NoSpeech, "{name}");
    }
}

// --- Loading through the registry's directory layout. -------------------------------------------

fn temp_dir(label: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("ink-engines-silero-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_loader_reports_a_missing_model_file() {
    let root = temp_dir("missing");
    let loader = SileroLoader::new(ModelDir::new(&root));
    let err = ink_engines::Loader::load(&loader, &silero_vad()).unwrap_err();
    let _ = fs::remove_dir_all(&root);
    assert!(matches!(err, EngineError::ModelMissing(_)), "{err:?}");
}

#[test]
fn the_loader_loads_the_installed_row_through_residency() {
    let Some(source) = model_path() else {
        eprintln!(
            "the_loader_loads_the_installed_row_through_residency: skipped, INK_SILERO_MODEL is not set"
        );
        return;
    };
    let root = temp_dir("installed");
    let dir = ModelDir::new(&root);
    let row = silero_vad();
    let path = dir.file_path(&row, &row.files[0]);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::copy(&source, &path).unwrap();
    let residency = Residency::new(
        Arc::new(SileroLoader::new(dir)),
        Arc::new(MockClock::new(0, 0)),
    );
    let lease = residency.acquire(&row).unwrap();
    let [(_, knocks, golden), ..] = golden_inputs();
    let ps = probabilities(&mut lease.vad().unwrap(), &knocks);
    drop(lease);
    let _ = fs::remove_dir_all(&root);
    assert!(
        ps.iter()
            .zip(golden.iter())
            .all(|(p, g)| (p - g).abs() <= GOLDEN_TOLERANCE)
    );
}
