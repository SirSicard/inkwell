//! The per-utterance gain stage: the robust-peak normaliser ahead of every engine.
//!
//! Levels are compared in dB with stated tolerances, never bit-for-bit across platforms.

use ink_audio::gain::{
    GainOutcome, LEVEL_FRAME, MAX_GAIN, MIN_DYNAMICS_DB, NOISE_FLOOR, TARGET_PEAK, from_dbfs,
    levels, normalise, rms, robust_peak, to_dbfs,
};
use ink_audio::synth::{breathy_speech, cycling_fan, knocks, noise, speech_like, with_noise};

/// How close to the target a lifted buffer must land. The gain is computed from the robust peak,
/// so only float rounding separates them.
const TARGET_TOLERANCE_DB: f32 = 0.05;

fn db_from_target(samples: &[f32]) -> f32 {
    to_dbfs(robust_peak(samples)) - to_dbfs(TARGET_PEAK)
}

/// The contrast the dynamics guard judges: robust peak over the 10th-percentile frame, in dB.
fn contrast_db(samples: &[f32]) -> f32 {
    let l = levels(samples);
    to_dbfs(l.robust_peak) - to_dbfs(l.quiet)
}

fn assert_lifted_to_target(take: &mut [f32], what: &str) {
    let report = normalise(take);
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
fn minus_75_dbfs_rms_speech_reaches_the_target() {
    // One recogniser returns empty text from about −70 dBFS RMS: this is the level that must not
    // reach it unlifted.
    let mut take = speech_like(4.0, -75.0, 1);
    assert!((to_dbfs(rms(&take)) + 75.0).abs() < 0.01);
    let report = normalise(&mut take);
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
fn a_single_click_does_not_veto_amplifying_quiet_speech() {
    // −60 dBFS speech with one full-scale keyboard click in the middle. An absolute-peak level
    // sees the click and leaves the take alone; the robust peak must ignore it.
    let mut take = speech_like(2.0, -60.0, 2);
    take[16_000] = 0.9;
    take[16_001] = -0.9;
    normalise(&mut take);
    let off = db_from_target(&take);
    assert!(
        off.abs() < TARGET_TOLERANCE_DB,
        "quiet speech was not lifted past a transient: {off:+.3} dB"
    );
}

#[test]
fn preserves_relative_dynamics() {
    // A quieter second half (−6 dB) stays 6 dB quieter: one gain for the whole utterance.
    let mut take = speech_like(2.0, -60.0, 3);
    take.extend(speech_like(2.0, -66.0, 4));
    let before = to_dbfs(rms(&take[..32_000])) - to_dbfs(rms(&take[32_000..]));
    normalise(&mut take);
    let after = to_dbfs(rms(&take[..32_000])) - to_dbfs(rms(&take[32_000..]));
    assert!(
        (before - after).abs() < 0.01,
        "{before} dB became {after} dB"
    );
    assert!((after - 6.0).abs() < 0.05);
}

#[test]
fn leaves_healthy_audio_alone() {
    let healthy = speech_like(2.0, -20.0, 5);
    assert!(robust_peak(&healthy) >= TARGET_PEAK);
    let mut take = healthy.clone();
    let report = normalise(&mut take);
    assert_eq!(report.outcome, GainOutcome::Healthy);
    assert_eq!(take, healthy, "healthy audio must pass through bit for bit");
}

#[test]
fn never_amplifies_silence_into_hiss() {
    // A converter's own noise, far below the floor: nothing to rescue.
    let quiet_room = noise(2.0, -95.0, 6);
    assert!(robust_peak(&quiet_room) < NOISE_FLOOR);
    let mut take = quiet_room.clone();
    assert_eq!(normalise(&mut take).outcome, GainOutcome::Silence);
    assert_eq!(take, quiet_room);

    let mut digital_silence = vec![0.0f32; 16_000];
    assert_eq!(
        normalise(&mut digital_silence).outcome,
        GainOutcome::Silence
    );
    assert!(digital_silence.iter().all(|&s| s == 0.0));
}

#[test]
fn room_tone_above_the_floor_is_not_lifted_into_hiss() {
    // Room tone at −70 dBFS RMS: above the silence floor, and 60 dB of gain would turn it into
    // full-level hiss. It has no loud frames standing out, so it stays as it is.
    let room = noise(3.0, -70.0, 7);
    assert!(robust_peak(&room) > NOISE_FLOOR);
    let mut take = room.clone();
    assert_eq!(normalise(&mut take).outcome, GainOutcome::Stationary);
    assert_eq!(take, room);
}

#[test]
fn quiet_speech_in_room_tone_is_still_lifted() {
    // The dynamics guard must not mistake real speech in a real room for room tone.
    let mut take = speech_like(3.0, -65.0, 8);
    for (s, n) in take.iter_mut().zip(noise(3.0, -90.0, 9)) {
        *s += n;
    }
    let report = normalise(&mut take);
    assert!(
        matches!(report.outcome, GainOutcome::Applied { .. }),
        "{report:?}"
    );
    assert!(db_from_target(&take).abs() < TARGET_TOLERANCE_DB);
}

#[test]
fn gain_is_capped() {
    // A robust peak just above the floor, where the target would need about 65 dB.
    let mut take = speech_like(2.0, -86.0, 10);
    let before = robust_peak(&take);
    assert!(before > NOISE_FLOOR && TARGET_PEAK / before > MAX_GAIN);
    let report = normalise(&mut take);
    assert_eq!(report.outcome, GainOutcome::Applied { gain: MAX_GAIN });
    let after = robust_peak(&take);
    assert!(
        (to_dbfs(after) - to_dbfs(before * MAX_GAIN)).abs() < 0.01,
        "gain exceeded the cap: {before} -> {after}"
    );
}

#[test]
fn never_clips() {
    // A full-scale click in a −75 dBFS take gets ~54 dB of gain; it is shaved to full scale, never
    // wrapped or pushed past it, and the speech still reaches the target.
    let mut take = speech_like(3.0, -75.0, 11);
    take[20_000] = 1.0;
    take[20_001] = -1.0;
    normalise(&mut take);
    assert!(take.iter().all(|s| s.abs() <= 1.0));
    assert!(db_from_target(&take).abs() < TARGET_TOLERANCE_DB);
}

#[test]
fn a_take_too_short_to_judge_is_left_alone() {
    // 150 ms: fewer frames than it takes to tell speech from noise.
    let short: Vec<f32> = speech_like(0.15, -60.0, 12);
    let mut take = short.clone();
    assert_eq!(normalise(&mut take).outcome, GainOutcome::TooShort);
    assert_eq!(take, short);
}

#[test]
fn the_report_says_what_was_measured_and_applied() {
    let mut take = speech_like(2.0, -60.0, 13);
    let before = robust_peak(&take);
    let report = normalise(&mut take);
    assert_eq!(report.before.robust_peak, before);
    assert!((report.gain() - TARGET_PEAK / before).abs() / report.gain() < 1e-6);
    assert!((to_dbfs(from_dbfs(-60.0)) + 60.0).abs() < 1e-4);
}

#[test]
fn low_crest_breathy_speech_is_lifted_to_the_target() {
    // Continuous, breathy speech at −75 dBFS RMS: no pauses, and its quiet frames sit only about
    // 8 dB below its loud ones.
    let mut take = breathy_speech(4.0, -75.0, 0.35, 1);
    let contrast = contrast_db(&take);
    assert!(
        (7.0..9.5).contains(&contrast),
        "fixture contrast {contrast:.2} dB"
    );
    assert_lifted_to_target(&mut take, "breathy speech");
}

#[test]
fn low_crest_speech_at_8_db_snr_is_lifted_to_the_target() {
    // The same speech 8 dB over room tone, at −75 dBFS RMS overall. The noise fills its troughs
    // and the contrast falls to 5–6 dB, under a 6 dB guard; every take must still be lifted.
    let mut lowest = f32::MAX;
    for seed in 0..8 {
        for seconds in [1.0, 4.0] {
            let speech = breathy_speech(seconds, -75.0, 0.35, seed);
            let mut take = with_noise(&speech, 8.0, -75.0, seed + 100);
            lowest = lowest.min(contrast_db(&take));
            assert_lifted_to_target(&mut take, &format!("seed {seed}, {seconds} s"));
        }
    }
    assert!(
        lowest < 6.0,
        "the fixtures should reach below 6 dB of contrast; lowest {lowest:.2} dB"
    );
}

/// 100 level frames alternating between a loud level and one `contrast_db` below it.
fn two_level_frames(contrast_db: f32) -> Vec<f32> {
    let loud = 1.0e-3;
    let quiet = loud / from_dbfs(contrast_db);
    (0..100)
        .flat_map(|k| {
            let v = if k % 2 == 0 { loud } else { quiet };
            std::iter::repeat_n(v, LEVEL_FRAME)
        })
        .collect()
}

#[test]
fn the_dynamics_guard_stops_lifting_below_4_db_of_contrast() {
    // Exactly where the guard sits. Above it, anything is lifted; below it, the take is treated as
    // stationary (room tone, hum) and left alone.
    assert_eq!(MIN_DYNAMICS_DB, 4.0);
    let mut above = two_level_frames(4.2);
    assert!(matches!(
        normalise(&mut above).outcome,
        GainOutcome::Applied { .. }
    ));
    let below = two_level_frames(3.8);
    let mut take = below.clone();
    assert_eq!(normalise(&mut take).outcome, GainOutcome::Stationary);
    assert_eq!(take, below);
}

#[test]
fn bursty_noise_is_lifted_because_the_gain_stage_is_not_a_speech_detector() {
    // Knocks and a cycling fan have loud frames standing well clear of quiet ones, as speech does.
    // The gain stage decides level only, so it lifts them to the target like speech. A take with
    // no speech is the VAD's to discard (`trim_ends` returns `None`) before any engine sees it.
    for (what, audio) in [
        ("knocks", knocks(4.0, -60.0, 1)),
        ("cycling fan", cycling_fan(6.0, -60.0, 2)),
    ] {
        let mut take = audio;
        assert!(contrast_db(&take) > 10.0, "{what}");
        assert_lifted_to_target(&mut take, what);
    }
}
