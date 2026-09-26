//! The per-utterance gain stage: VAD-gated ([`normalise_speech`]) and the no-VAD fallback
//! ([`normalise_without_vad`]).
//!
//! The VAD is scripted (see `common`): an oracle that knows where the speech is, and a deaf one
//! that also cannot hear windows quieter than −50 dBFS RMS, as a real VAD cannot hear −75 dBFS
//! speech. Levels are compared in dB with stated tolerances, never bit-for-bit across platforms.

mod common;

use common::{Always, DeafOracle, Failing, Oracle, speech_mask};
use ink_audio::gain::{
    GainEvidence, GainOutcome, LEVEL_FRAME, MAX_GAIN, MIN_DYNAMICS_DB, NOISE_FLOOR, TARGET_PEAK,
    TRANSIENT_FRAMES, apply_gain, from_dbfs, levels, normalise_speech, normalise_without_vad,
    provisional_gain, rms, robust_peak, to_dbfs,
};
use ink_audio::speech_band::{contrast_db, envelope};
use ink_audio::synth::{
    Slope, SpeechShape, cycling_fan, knocks, mix, noise, rumble, speech_like, speech_with, swing,
    tone, with_noise,
};
use ink_audio::vad::{VAD_WINDOW, VadConfig, speech_segments, trim_ends};

/// How close to the target a lifted take's level must land. The gain is computed from the level it
/// keys on, so only float rounding separates them.
const TARGET_TOLERANCE_DB: f32 = 0.05;

/// The same, where the test measures the speech frames from the truth rather than from the VAD's
/// segments (the VAD's hangover adds a few quiet gap frames, which never move the robust peak).
const SPEECH_TOLERANCE_DB: f32 = 0.1;

/// The syllable (segment) lengths every speech fixture comes in: brisk 7–8 syllables a second
/// down to slow speech.
const SYLLABLES: [f64; 5] = [0.12, 0.13, 0.15, 0.20, 0.26];

fn db_from_target(samples: &[f32]) -> f32 {
    to_dbfs(robust_peak(samples)) - to_dbfs(TARGET_PEAK)
}

/// The robust peak of the level frames whose middle sample lies in a speech window of `mask`, in
/// dB from the target.
fn speech_db_from_target(samples: &[f32], mask: &[bool]) -> f32 {
    let mut peaks: Vec<f32> = samples
        .chunks(LEVEL_FRAME)
        .enumerate()
        .filter(|(k, _)| {
            let middle = k * LEVEL_FRAME + LEVEL_FRAME / 2;
            mask.get(middle / VAD_WINDOW).copied().unwrap_or(false)
        })
        .map(|(_, f)| f.iter().fold(0.0f32, |m, s| m.max(s.abs())))
        .collect();
    assert!(!peaks.is_empty(), "no speech frames in the truth");
    peaks.sort_by(|a, b| b.total_cmp(a));
    to_dbfs(peaks[TRANSIENT_FRAMES.min(peaks.len() - 1)]) - to_dbfs(TARGET_PEAK)
}

/// Every speech fixture the stages are held to, at −75 dBFS RMS: plain and breathy speech at each
/// syllable length, clean and 8 dB over white noise and over gentle and steep 100 Hz rumble. Each
/// comes with its clean speech, for the truth.
fn speech_fixtures(seconds: f64, seed: u64) -> Vec<(String, Vec<f32>, Vec<f32>)> {
    let mut out = Vec::new();
    for (i, &syllable) in SYLLABLES.iter().enumerate() {
        for (kind, shape) in [
            ("plain", SpeechShape::plain(syllable)),
            ("breathy", SpeechShape::breathy(syllable)),
        ] {
            let seed = seed + 10 * i as u64;
            let clean = speech_with(seconds, -75.0, shape, seed);
            let name = |over: &str| format!("{kind} speech, {syllable} s syllables, {over}");
            let steep = rumble(seconds, -75.0, 100.0, Slope::Steep, seed + 1);
            let gentle = rumble(seconds, -75.0, 100.0, Slope::Gentle, seed + 2);
            out.push((name("clean"), clean.clone(), clean.clone()));
            out.push((
                name("8 dB over white noise"),
                with_noise(&clean, 8.0, -75.0, seed + 3),
                clean.clone(),
            ));
            out.push((
                name("8 dB over steep 100 Hz rumble"),
                mix(&clean, &steep, 8.0, -75.0),
                clean.clone(),
            ));
            out.push((
                name("8 dB over gentle 100 Hz rumble"),
                mix(&clean, &gentle, 8.0, -75.0),
                clean,
            ));
        }
    }
    out
}

/// Everything the stages must not learn a level from, `seconds` long: white room tone, rumble at
/// 100 Hz to 1 kHz (gentle and steep), rumble swinging slowly in level (8 s and 20 s periods, 10
/// and 15 dB deep), a cycling fan and knocks.
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

fn cfg() -> VadConfig {
    VadConfig::default()
}

// --- With a VAD. -------------------------------------------------------------------------------

#[test]
fn minus_75_dbfs_rms_speech_reaches_the_target() {
    // One recogniser returns empty text from about −70 dBFS RMS: this is the level that must not
    // reach it unlifted. The VAD here is deaf below −50 dBFS, as a real one is.
    let clean = speech_like(4.0, -75.0, 1);
    assert!((to_dbfs(rms(&clean)) + 75.0).abs() < 0.01);
    let mut take = clean.clone();
    let mut vad = DeafOracle::new(speech_mask(&clean));
    let report = normalise_speech(&mut take, &mut vad, &cfg()).unwrap();
    assert!(
        matches!(report.outcome, GainOutcome::Applied { .. }),
        "{report:?}"
    );
    let off = db_from_target(&take);
    assert!(
        off.abs() < TARGET_TOLERANCE_DB,
        "robust peak {off:+.3} dB from the target"
    );
}

#[test]
fn the_vad_hears_the_provisional_copy_not_the_raw_take() {
    // Why the flow lifts a copy first: a VAD that cannot hear −75 dBFS finds no speech in the raw
    // take at all, and every word of it would be discarded. Through the provisional gain it
    // finds the speech, and the take is lifted.
    let clean = speech_like(4.0, -75.0, 2);
    let mut deaf = DeafOracle::new(speech_mask(&clean));
    assert_eq!(trim_ends(&clean, &mut deaf, &cfg()).unwrap(), None);

    let mut take = clean.clone();
    let report = normalise_speech(&mut take, &mut deaf, &cfg()).unwrap();
    assert!(
        matches!(report.outcome, GainOutcome::Applied { .. }),
        "{report:?}"
    );
    let GainEvidence::Vad {
        provisional_gain,
        speech_frames,
        ..
    } = report.evidence
    else {
        panic!("{:?}", report.evidence)
    };
    assert_eq!(provisional_gain, provisional_gain_of(&clean));
    assert!(provisional_gain > 100.0, "{provisional_gain}");
    assert!(speech_frames > 0);
}

fn provisional_gain_of(take: &[f32]) -> f32 {
    provisional_gain(&levels(take))
}

#[test]
fn fast_and_slow_speech_reaches_the_target_in_noise_and_over_rumble() {
    // Plain and breathy speech at every syllable length from 0.12 s to 0.26 s, clean and 8 dB over
    // white noise and 100 Hz rumble, 4 s takes at −75 dBFS RMS: every one is lifted until its
    // speech sits at the target. Brisk breathy speech is what the earlier level heuristics
    // refused; rumble is what they lifted.
    for (what, input, clean) in speech_fixtures(4.0, 100) {
        let mask = speech_mask(&clean);
        let mut take = input.clone();
        let mut vad = DeafOracle::new(mask.clone());
        let report = normalise_speech(&mut take, &mut vad, &cfg()).unwrap();
        assert!(
            matches!(report.outcome, GainOutcome::Applied { .. }),
            "{what}: {report:?}"
        );
        let off = speech_db_from_target(&take, &mask);
        assert!(
            off.abs() < SPEECH_TOLERANCE_DB,
            "{what}: speech {off:+.3} dB from the target"
        );
    }
}

#[test]
fn non_speech_is_never_lifted_with_a_vad() {
    // Rumble of every shape, swinging rumble, a fan, knocks, room tone: the VAD hears no speech in
    // them, so no level is learned and nothing is lifted, whatever the audio looks like. The
    // report says there was no speech, and the caller discards the take. 4 s takes from 40 s of
    // each, so the 20 s swings are seen at every phase.
    for (what, audio) in non_speech_fixtures(40.0, 200) {
        for (k, chunk) in audio.chunks_exact(64_000).enumerate() {
            let mut take = chunk.to_vec();
            let report = normalise_speech(&mut take, &mut Always(0.05), &cfg()).unwrap();
            assert_eq!(report.outcome, GainOutcome::NoSpeech, "{what}, take {k}");
            assert!(
                matches!(
                    report.evidence,
                    GainEvidence::Vad {
                        speech: None,
                        speech_frames: 0,
                        ..
                    }
                ),
                "{what}, take {k}: {:?}",
                report.evidence
            );
            assert_eq!(take, chunk, "{what}, take {k}: touched");
        }
    }
}

#[test]
fn a_loud_noise_in_the_take_does_not_set_the_level() {
    // Quiet speech (−60 dBFS RMS) with half a second of a fan 30 dB louder in the middle: the
    // loudest frames of the take are the fan's. The level is learned from the speech frames only,
    // so the speech still reaches the target (the fan is shaved at full scale, as any transient
    // above the level is).
    let clean = speech_like(4.0, -60.0, 14);
    let mut take = clean.clone();
    let fan = cycling_fan(0.5, -30.0, 15);
    let at = 24_000;
    let mut truth = clean.clone();
    for (i, f) in fan.iter().enumerate() {
        take[at + i] = *f;
        truth[at + i] = 0.0;
    }
    let mask = speech_mask(&truth);
    let report = normalise_speech(&mut take, &mut Oracle::new(mask.clone()), &cfg()).unwrap();
    assert!(
        matches!(report.outcome, GainOutcome::Applied { .. }),
        "{report:?}"
    );
    let off = speech_db_from_target(&take, &mask);
    assert!(
        off.abs() < SPEECH_TOLERANCE_DB,
        "speech {off:+.3} dB from the target"
    );
}

#[test]
fn the_report_gives_the_range_to_keep_from_the_same_vad_pass() {
    // The pipeline trims with the range the report carries: exactly what trimming the lifted copy
    // gives, so the VAD runs once.
    let mut clean = vec![0.0f32; 16_000];
    clean.extend(speech_like(2.0, -75.0, 3));
    clean.extend(vec![0.0f32; 16_000]);
    let mask = speech_mask(&clean);
    let mut take = clean.clone();
    let report = normalise_speech(&mut take, &mut Oracle::new(mask.clone()), &cfg()).unwrap();
    let GainEvidence::Vad { speech, .. } = &report.evidence else {
        panic!("{:?}", report.evidence)
    };
    let mut lifted = clean.clone();
    apply_gain(&mut lifted, provisional_gain_of(&clean));
    let expected = trim_ends(&lifted, &mut Oracle::new(mask), &cfg()).unwrap();
    assert_eq!(speech, &expected);
    assert!(expected.is_some_and(|r| r.start > 0 && r.end < clean.len()));
}

#[test]
fn a_single_click_does_not_veto_amplifying_quiet_speech() {
    // −60 dBFS speech with one full-scale keyboard click in the middle. An absolute-peak level
    // sees the click and leaves the take alone; the robust peak must ignore it.
    let clean = speech_like(2.0, -60.0, 4);
    let mut take = clean.clone();
    take[16_000] = 0.9;
    take[16_001] = -0.9;
    normalise_speech(&mut take, &mut Oracle::new(speech_mask(&clean)), &cfg()).unwrap();
    let off = db_from_target(&take);
    assert!(
        off.abs() < TARGET_TOLERANCE_DB,
        "quiet speech was not lifted past a transient: {off:+.3} dB"
    );
}

#[test]
fn preserves_relative_dynamics() {
    // A quieter second half (−6 dB) stays 6 dB quieter: one gain for the whole utterance.
    let mut take = speech_like(2.0, -60.0, 5);
    take.extend(speech_like(2.0, -66.0, 6));
    let before = to_dbfs(rms(&take[..32_000])) - to_dbfs(rms(&take[32_000..]));
    normalise_speech(&mut take, &mut Always(0.9), &cfg()).unwrap();
    let after = to_dbfs(rms(&take[..32_000])) - to_dbfs(rms(&take[32_000..]));
    assert!(
        (before - after).abs() < 0.01,
        "{before} dB became {after} dB"
    );
    assert!((after - 6.0).abs() < 0.05);
}

#[test]
fn leaves_healthy_audio_alone() {
    let healthy = speech_like(2.0, -20.0, 7);
    assert!(robust_peak(&healthy) >= TARGET_PEAK);
    let mut take = healthy.clone();
    let report = normalise_speech(&mut take, &mut Oracle::new(speech_mask(&healthy)), &cfg());
    assert_eq!(report.unwrap().outcome, GainOutcome::Healthy);
    assert_eq!(take, healthy, "healthy audio must pass through bit for bit");
}

#[test]
fn never_amplifies_silence_into_hiss() {
    // A converter's own noise, far below the floor: nothing to rescue, and the VAD is not even
    // asked (this one would fail if it were).
    let quiet_room = noise(2.0, -95.0, 8);
    assert!(robust_peak(&quiet_room) < NOISE_FLOOR);
    let mut take = quiet_room.clone();
    let report = normalise_speech(&mut take, &mut Failing, &cfg()).unwrap();
    assert_eq!(report.outcome, GainOutcome::Silence);
    assert_eq!(take, quiet_room);

    let mut digital_silence = vec![0.0f32; 16_000];
    let report = normalise_speech(&mut digital_silence, &mut Failing, &cfg()).unwrap();
    assert_eq!(report.outcome, GainOutcome::Silence);
    assert!(digital_silence.iter().all(|&s| s == 0.0));
}

#[test]
fn gain_is_capped() {
    // A robust peak just above the floor, where the target would need about 65 dB.
    let mut take = speech_like(2.0, -86.0, 9);
    let before = robust_peak(&take);
    assert!(before > NOISE_FLOOR && TARGET_PEAK / before > MAX_GAIN);
    let report = normalise_speech(&mut take, &mut Always(0.9), &cfg()).unwrap();
    assert_eq!(report.outcome, GainOutcome::Applied { gain: MAX_GAIN });
    let after = robust_peak(&take);
    assert!(
        (to_dbfs(after) - to_dbfs(before * MAX_GAIN)).abs() < 0.01,
        "gain exceeded the cap: {before} -> {after}"
    );
}

#[test]
fn never_clips() {
    // A full-scale click in a −75 dBFS take gets over 50 dB of gain; it is shaved to full scale,
    // never wrapped or pushed past it, and the speech still reaches the target.
    let clean = speech_like(3.0, -75.0, 10);
    let mut take = clean.clone();
    take[20_000] = 1.0;
    take[20_001] = -1.0;
    normalise_speech(&mut take, &mut Oracle::new(speech_mask(&clean)), &cfg()).unwrap();
    assert!(take.iter().all(|s| s.abs() <= 1.0));
    assert!(db_from_target(&take).abs() < TARGET_TOLERANCE_DB);
}

#[test]
fn a_take_too_short_to_judge_is_left_alone() {
    // 150 ms: fewer frames than the stage judges.
    let short: Vec<f32> = speech_like(0.15, -60.0, 11);
    let mut take = short.clone();
    let report = normalise_speech(&mut take, &mut Always(0.9), &cfg()).unwrap();
    assert_eq!(report.outcome, GainOutcome::TooShort);
    assert_eq!(take, short);
}

#[test]
fn a_failing_vad_is_an_error_not_a_guess() {
    let clean = speech_like(2.0, -75.0, 12);
    let mut take = clean.clone();
    let err = normalise_speech(&mut take, &mut Failing, &cfg()).unwrap_err();
    assert_eq!(err, ink_core::EngineError::ModelMissing("vad".into()));
    assert_eq!(take, clean, "nothing is applied on a failure");
}

#[test]
fn the_report_says_what_was_measured_and_applied() {
    let mut take = speech_like(2.0, -60.0, 13);
    let before = robust_peak(&take);
    let report = normalise_speech(&mut take, &mut Always(0.9), &cfg()).unwrap();
    assert_eq!(report.before.robust_peak, before);
    assert!((report.gain() - TARGET_PEAK / before).abs() / report.gain() < 1e-6);
    let segments = speech_segments(&take, &mut Always(0.9), &cfg()).unwrap();
    assert_eq!(segments.len(), 1, "one segment, the whole take");
    assert!((to_dbfs(from_dbfs(-60.0)) + 60.0).abs() < 1e-4);
}

// --- The fallback, without a VAD. --------------------------------------------------------------

fn assert_fallback_lifts_to_target(take: &mut [f32], what: &str) {
    let report = normalise_without_vad(take);
    assert!(
        matches!(report.outcome, GainOutcome::Applied { .. }),
        "{what}: {report:?}"
    );
    let off = db_from_target(take);
    assert!(
        off.abs() < TARGET_TOLERANCE_DB,
        "{what}: {off:+.3} dB from the target"
    );
}

#[test]
fn fallback_lifts_fast_and_slow_speech() {
    // Without a VAD the stage errs toward lifting: every speech fixture, brisk breathy speech
    // included (which a correlation rule refused), is lifted to the target.
    for (what, mut take, _) in speech_fixtures(4.0, 300) {
        assert_fallback_lifts_to_target(&mut take, &what);
    }
    for seconds in [1.0, 4.0] {
        for syllable in [0.13, 0.15] {
            let mut take = speech_with(seconds, -75.0, SpeechShape::breathy(syllable), 301);
            assert_fallback_lifts_to_target(
                &mut take,
                &format!("breathy {syllable} s, {seconds} s"),
            );
        }
    }
}

#[test]
fn fallback_leaves_white_room_tone_and_gentle_rumble_alone() {
    // What its contrast rule still refuses: steady white room tone and gently low-passed rumble,
    // in 1 s and 4 s takes, have no speech-band contrast to speak of.
    let mut audio = vec![("white room tone".to_string(), noise(40.0, -70.0, 400))];
    for (j, hz) in [100.0, 250.0, 500.0, 1_000.0].into_iter().enumerate() {
        audio.push((
            format!("gentle {hz} Hz rumble"),
            rumble(40.0, -70.0, hz, Slope::Gentle, 401 + j as u64),
        ));
    }
    for (what, audio) in audio {
        for take_len in [16_000, 64_000] {
            for (k, chunk) in audio.chunks_exact(take_len).enumerate() {
                let mut take = chunk.to_vec();
                let report = normalise_without_vad(&mut take);
                assert_eq!(report.outcome, GainOutcome::Stationary, "{what}, take {k}");
                assert_eq!(take, chunk);
            }
        }
    }
}

#[test]
fn fallback_can_lift_steep_and_swinging_rumble() {
    // The price of the fallback, committed so nobody mistakes it for a VAD: rumble low-passed
    // steeply, and rumble swinging slowly in level, have speech-band contrast, and some of their
    // takes are lifted to the target like speech. With a VAD none are
    // (`non_speech_is_never_lifted_with_a_vad`).
    for (what, audio) in [
        (
            "steep 100 Hz rumble",
            rumble(40.0, -70.0, 100.0, Slope::Steep, 500),
        ),
        (
            "gentle 250 Hz rumble swinging 15 dB every 8 s",
            swing(&rumble(40.0, -70.0, 250.0, Slope::Gentle, 501), 8.0, 15.0),
        ),
    ] {
        let mut lifted = 0;
        for chunk in audio.chunks_exact(64_000) {
            let mut take = chunk.to_vec();
            if let GainOutcome::Applied { .. } = normalise_without_vad(&mut take).outcome {
                lifted += 1;
                assert!(db_from_target(&take).abs() < TARGET_TOLERANCE_DB, "{what}");
            }
        }
        assert!(
            lifted > 0,
            "{what}: expected the fallback to lift some takes"
        );
    }
}

#[test]
fn fallback_can_lift_a_fan_and_knocks() {
    // A cycling fan stands out in the speech band and is lifted; knocks may be. Only a VAD tells
    // them from speech.
    let mut fan = cycling_fan(6.0, -60.0, 2);
    assert_fallback_lifts_to_target(&mut fan, "cycling fan");
    let mut take = knocks(4.0, -60.0, 1);
    match normalise_without_vad(&mut take).outcome {
        GainOutcome::Applied { .. } => {
            assert!(db_from_target(&take).abs() < TARGET_TOLERANCE_DB, "knocks");
        }
        GainOutcome::Stationary => {}
        other => panic!("knocks: {other:?}"),
    }
}

/// 1 kHz tone in runs of five level frames, alternating between a loud level and one
/// `contrast_db` below it, 100 frames in all: in the speech band, so its band envelope has exactly
/// this contrast.
fn tone_runs(contrast_db: f32) -> Vec<f32> {
    let carrier = tone(2.0, 1_000.0, 1.0, 16_000);
    let loud = 1.0e-3;
    let quiet = loud / from_dbfs(contrast_db);
    carrier
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let frame = i / LEVEL_FRAME;
            c * if (frame / 5).is_multiple_of(2) {
                loud
            } else {
                quiet
            }
        })
        .collect()
}

#[test]
fn fallback_stops_lifting_below_4_db_of_speech_band_contrast() {
    // Exactly where the fallback's line sits. Above it, the take is lifted; below it, it is
    // treated as a stationary room and left alone.
    assert_eq!(MIN_DYNAMICS_DB, 4.0);
    let mut above = tone_runs(4.2);
    assert!((contrast_db(&envelope(&above)) - 4.2).abs() < 0.1);
    assert!(matches!(
        normalise_without_vad(&mut above).outcome,
        GainOutcome::Applied { .. }
    ));
    let below = tone_runs(3.8);
    let mut take = below.clone();
    assert_eq!(
        normalise_without_vad(&mut take).outcome,
        GainOutcome::Stationary
    );
    assert_eq!(take, below);
}
