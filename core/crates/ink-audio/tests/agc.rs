//! The meeting AGC: a slow gain toward the normaliser's target that never pumps up on pauses.
//!
//! Levels are compared in dB with stated tolerances. The AGC delays its output by exactly
//! [`Agc::LATENCY`] samples; the helpers below undo that so output and input line up.

use ink_audio::Agc;
use ink_audio::agc::{FALL_DB_PER_S, RISE_DB_PER_S};
use ink_audio::gain::{LEVEL_FRAME, TARGET_PEAK, robust_peak, to_dbfs};
use ink_audio::synth::{noise, speech_like};

const SR: usize = 16_000;

/// How close to the target a converged AGC must hold speech.
const CONVERGED_TOLERANCE_DB: f32 = 1.0;

/// Runs `input` through a fresh AGC one level frame per push and returns the output aligned with
/// the input, plus the AGC's gain (in dB) after every frame.
fn run(input: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut agc = Agc::new();
    let mut out = Vec::with_capacity(input.len() + Agc::LATENCY);
    let mut gains = Vec::with_capacity(input.len() / LEVEL_FRAME + 1);
    for chunk in input.chunks(LEVEL_FRAME) {
        let mut block = chunk.to_vec();
        agc.process(&mut block);
        out.extend_from_slice(&block);
        gains.push(to_dbfs(agc.gain()));
    }
    agc.flush(&mut out);
    (out[Agc::LATENCY..].to_vec(), gains)
}

fn db_from_target(samples: &[f32]) -> f32 {
    to_dbfs(robust_peak(samples)) - to_dbfs(TARGET_PEAK)
}

fn secs(s: f64) -> usize {
    (s * SR as f64) as usize
}

#[test]
fn minus_75_dbfs_fixture_reaches_the_target_through_the_agc() {
    // The same −75 dBFS RMS fixture the normaliser must lift. The AGC gets there from unity gain:
    // quickly (it locks on its first half second of speech) and then holds.
    let input = speech_like(20.0, -75.0, 1);
    let (out, _) = run(&input);
    let converged = db_from_target(&out[secs(10.0)..]);
    assert!(
        converged.abs() < CONVERGED_TOLERANCE_DB,
        "held {converged:+.2} dB from the target"
    );
    let early = db_from_target(&out[secs(2.0)..secs(4.0)]);
    assert!(
        early.abs() < CONVERGED_TOLERANCE_DB,
        "{early:+.2} dB from the target 2 s in"
    );
}

/// Speech, a pause, speech; returns the input and where the pause starts and ends.
fn speech_pause_speech(pause: Vec<f32>) -> (Vec<f32>, usize, usize) {
    let mut input = speech_like(8.0, -45.0, 2);
    let pause_start = input.len();
    input.extend(pause);
    let pause_end = input.len();
    input.extend(speech_like(8.0, -45.0, 3));
    (input, pause_start, pause_end)
}

fn assert_holds_through_the_pause(input: &[f32], pause_start: usize, pause_end: usize) {
    let (out, gains) = run(input);
    let frame = |sample: usize| sample / LEVEL_FRAME;
    let at_pause = gains[frame(pause_start) - 1];
    let most_in_pause = gains[frame(pause_start)..frame(pause_end)]
        .iter()
        .fold(f32::MIN, |m, &g| m.max(g));
    assert!(
        most_in_pause - at_pause < 0.01,
        "the gain rose {:.2} dB during the pause",
        most_in_pause - at_pause
    );
    // The first 300 ms of the second stretch of speech comes out at the target, not louder: no
    // gain was banked during the pause.
    let onset = db_from_target(&out[pause_end..pause_end + secs(0.3)]);
    assert!(
        onset < CONVERGED_TOLERANCE_DB,
        "speech after the pause came out {onset:+.2} dB over the target"
    );
    // And the speech before the pause was already at the target.
    let before = db_from_target(&out[pause_start - secs(3.0)..pause_start]);
    assert!(before.abs() < CONVERGED_TOLERANCE_DB, "{before:+.2} dB");
}

#[test]
fn agc_holds_its_gain_through_a_pause_of_room_tone() {
    // Ten seconds of −75 dBFS room tone: above the silence floor, so a naive AGC would climb on
    // it and blast the next word.
    let (input, start, end) = speech_pause_speech(noise(10.0, -75.0, 4));
    assert_holds_through_the_pause(&input, start, end);
}

#[test]
fn agc_holds_its_gain_through_digital_silence() {
    // A far-end tap delivers exact zeros while nobody talks.
    let (input, start, end) = speech_pause_speech(vec![0.0; secs(10.0)]);
    assert_holds_through_the_pause(&input, start, end);
}

#[test]
fn agc_follows_a_level_step_slowly_and_never_clips() {
    // Loud, then 25 dB quieter (a different talker), then loud again.
    let mut input = speech_like(10.0, -30.0, 5);
    let quiet_at = input.len();
    input.extend(speech_like(25.0, -55.0, 6));
    let loud_again_at = input.len();
    input.extend(speech_like(6.0, -30.0, 7));
    let (out, gains) = run(&input);

    assert!(
        out.iter().all(|s| s.abs() <= 1.0),
        "the output clipped: peak {}",
        out.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    );

    for (label, range) in [
        ("loud", secs(6.0)..quiet_at),
        ("quiet", loud_again_at - secs(4.0)..loud_again_at),
        ("loud again", loud_again_at + secs(3.0)..out.len()),
    ] {
        let off = db_from_target(&out[range]);
        assert!(
            off.abs() < CONVERGED_TOLERANCE_DB,
            "{label}: {off:+.2} dB from the target"
        );
    }

    // Slow up, fast down: after its first lock, the gain never rises faster than the rise rate.
    let per_frame_rise = RISE_DB_PER_S / 50.0 + 0.001;
    let first_lock = gains.iter().position(|&g| g > 0.0).unwrap_or(0);
    for w in gains[first_lock + 1..].windows(2) {
        assert!(
            w[1] - w[0] <= per_frame_rise,
            "rose {:.3} dB in one frame",
            w[1] - w[0]
        );
        assert!(
            w[0] - w[1] <= FALL_DB_PER_S / 50.0 + 0.001,
            "fell {:.3} dB in one frame",
            w[0] - w[1]
        );
    }
    // The climb after the step down took seconds, not a frame.
    let step = quiet_at / LEVEL_FRAME;
    let climbed = gains[step + secs(1.0) / LEVEL_FRAME] - gains[step];
    assert!(climbed < 1.0, "climbed {climbed:.2} dB in the first second");
}

#[test]
fn agc_leaves_healthy_audio_alone() {
    // −15 dBFS RMS: a robust peak about 6 dB over the target.
    let input = speech_like(5.0, -15.0, 8);
    let (out, gains) = run(&input);
    assert!(gains.iter().all(|&g| g == 0.0), "gain moved off unity");
    assert_eq!(out, input);
}

#[test]
fn agc_never_lifts_room_tone_alone() {
    // Twenty seconds of nothing but a quiet room: no speech, so no reason to move.
    let input = noise(20.0, -70.0, 9);
    let (out, gains) = run(&input);
    assert!(gains.iter().all(|&g| g == 0.0), "gain rose on room tone");
    assert_eq!(out, input);
}

#[test]
fn agc_output_is_its_input_one_frame_late() {
    // The one-frame look-ahead is what lets the gain drop before a loud frame, not after it.
    assert_eq!(Agc::LATENCY, LEVEL_FRAME);
    let input = speech_like(1.0, -15.0, 10);
    let mut agc = Agc::new();
    let mut block = input.clone();
    agc.process(&mut block);
    assert!(block[..Agc::LATENCY].iter().all(|&s| s == 0.0));
    assert_eq!(block[Agc::LATENCY..], input[..input.len() - Agc::LATENCY]);
    let mut tail = Vec::new();
    agc.flush(&mut tail);
    assert_eq!(tail, input[input.len() - Agc::LATENCY..]);
}

#[test]
fn agc_push_sizes_do_not_change_the_output() {
    // Allowed: 1e-7 absolute. The AGC works in whole level frames whatever the push size, so in
    // practice the outputs are identical.
    let mut input = speech_like(6.0, -60.0, 11);
    input.extend(speech_like(4.0, -35.0, 12));
    let (whole, _) = run(&input);
    let mut agc = Agc::new();
    let mut out = Vec::new();
    let sizes = [1usize, 17, 320, 999, 3, 4_096];
    let mut at = 0;
    let mut k = 0;
    while at < input.len() {
        let n = sizes[k % sizes.len()].min(input.len() - at);
        let mut block = input[at..at + n].to_vec();
        agc.process(&mut block);
        out.extend_from_slice(&block);
        at += n;
        k += 1;
    }
    agc.flush(&mut out);
    let odd = &out[Agc::LATENCY..];
    assert_eq!(odd.len(), whole.len());
    let worst = odd
        .iter()
        .zip(&whole)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst <= 1e-7, "push sizes changed the output by {worst}");
}
