//! The meeting AGC: a slow gain toward the normaliser's target that never pumps up on pauses.
//!
//! Levels are compared in dB with stated tolerances. The AGC delays its output by exactly
//! [`Agc::LATENCY`] samples; the helpers below undo that so output and input line up.

use ink_audio::Agc;
use ink_audio::agc::{FALL_DB_PER_S, LIMITER_BLOCK, RISE_DB_PER_S};
use ink_audio::gain::{LEVEL_FRAME, MAX_GAIN, TARGET_PEAK, robust_peak, to_dbfs};
use ink_audio::synth::{Slope, breathy_speech, mix, noise, rumble, speech_like, with_noise};

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

#[test]
fn agc_ramps_down_ahead_of_a_transient_and_keeps_its_gain_after_it() {
    // A quiet talker has the gain locked high (about +39 dB) when a cough arrives: 40 ms of noise
    // peaking near full scale. The look-ahead lets the limiter ramp the gain down before the cough
    // reaches the output, so it neither clips nor steps; and a transient that short does not move
    // the level, so the speech after it comes out at the target, not ducked.
    let speech = speech_like(12.0, -60.0, 20);
    let mut input = speech.clone();
    // Mid-syllable, 8 s in: the samples either side of the cough are voiced, so the gain can be
    // read off every one of them.
    let cough_at = (secs(8.0)..)
        .find(|&i| input[i - 400..i + 400].iter().all(|&s| s != 0.0))
        .expect("a voiced stretch");
    let burst = noise(0.04, -12.0, 21);
    for (i, s) in burst.iter().enumerate() {
        let t = i as f32 / SR as f32;
        input[cough_at + i] += s * (t / 0.003).min(1.0) * (-t / 0.012).exp();
    }
    let cough_end = cough_at + burst.len();
    let (out, gains) = run(&input);

    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak <= 1.0, "clipped: output peak {peak}");

    // Sample-to-sample gain change, read off the output: every change is spread over a whole
    // limiter block, so no two consecutive samples differ by more than the widest possible
    // change (the full 60 dB range) over one block.
    let bound_db = to_dbfs(MAX_GAIN) / LIMITER_BLOCK as f32 + 0.01;
    let gain_at = |n: usize| (input[n].abs() >= 1e-6).then(|| to_dbfs((out[n] / input[n]).abs()));
    let mut steepest = 0.0f32;
    for n in 0..input.len() - 1 {
        if let (Some(a), Some(b)) = (gain_at(n), gain_at(n + 1)) {
            steepest = steepest.max((b - a).abs());
        }
    }
    assert!(
        steepest <= bound_db,
        "the gain stepped {steepest:.2} dB between two samples (bound {bound_db:.3} dB)"
    );
    // The gain really was high before the ramp (which starts at most two blocks early), and
    // really did come down for the cough.
    let before = gain_at(cough_at - 2 * LIMITER_BLOCK - 80).expect("speech before the cough");
    assert!(before > 35.0, "locked at {before:.1} dB");
    let lowest = (cough_at..cough_end)
        .filter_map(gain_at)
        .fold(f32::MAX, f32::min);
    assert!(lowest < 10.0, "the cough got {lowest:.1} dB");

    // The persistent gain is what it would have been without the cough. The cough's two or three
    // frames join the level window and shift which frame is the robust peak by a place or two, so
    // allow 0.1 dB; a duck would be tens of dB.
    let (_, without) = run(&speech);
    let moved = gains[cough_at / LEVEL_FRAME..]
        .iter()
        .zip(&without[cough_at / LEVEL_FRAME..])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        moved < 0.1,
        "the cough moved the gain by {moved:.3} dB from where it would have been"
    );
    let after = db_from_target(&out[cough_end + secs(0.2)..cough_end + secs(1.2)]);
    assert!(
        after.abs() < CONVERGED_TOLERANCE_DB,
        "speech after the cough came out {after:+.2} dB from the target"
    );
}

#[test]
fn agc_lifts_low_crest_speech_in_noise_to_the_target() {
    // Continuous, breathy speech 8 dB over room tone, at −75 dBFS RMS overall: its loud frames
    // stand only 5–9 dB over its quiet ones. The AGC decides level, as the normaliser does, so it
    // must lift this to the target rather than wait for frames that stand 10 dB clear.
    let input = with_noise(&breathy_speech(20.0, -75.0, 0.35, 1), 8.0, -75.0, 2);
    let (out, _) = run(&input);
    let converged = db_from_target(&out[secs(10.0)..]);
    assert!(
        converged.abs() < CONVERGED_TOLERANCE_DB,
        "held {converged:+.2} dB from the target"
    );
}

#[test]
fn agc_never_lifts_a_long_stretch_of_room_tone() {
    // Three minutes of a quiet room and nothing else: long enough for rare loud noise frames to
    // turn up, and none of them may start the gain climbing.
    let input = noise(180.0, -70.0, 23);
    let (_, gains) = run(&input);
    let highest = gains.iter().fold(f32::MIN, |m, &g| m.max(g));
    assert_eq!(
        highest, 0.0,
        "the gain rose to {highest:.2} dB on room tone"
    );
}

#[test]
fn agc_never_lifts_rumble() {
    // Two minutes each of room rumble low-passed at 100, 250, 500 and 1000 Hz, gentle and steep:
    // a meeting's quiet stretch in a real room. The gain never moves off unity.
    for (i, slope) in [Slope::Gentle, Slope::Steep].into_iter().enumerate() {
        for (j, hz) in [100.0, 250.0, 500.0, 1_000.0].into_iter().enumerate() {
            let input = rumble(120.0, -70.0, hz, slope, 60 + (i * 4 + j) as u64);
            let (_, gains) = run(&input);
            let highest = gains.iter().fold(f32::MIN, |m, &g| m.max(g));
            assert_eq!(
                highest, 0.0,
                "{slope:?} {hz} Hz rumble lifted to {highest:.2} dB"
            );
        }
    }
}

#[test]
fn agc_lifts_speech_in_noise_and_over_rumble_to_the_target() {
    // Speech 8 dB over white room tone, and speech and breathy speech 8 dB over 100 Hz rumble,
    // at −75 dBFS RMS overall.
    let speech = speech_like(20.0, -75.0, 70);
    let breathy = breathy_speech(20.0, -75.0, 0.35, 71);
    let steep = rumble(20.0, -75.0, 100.0, Slope::Steep, 72);
    let gentle = rumble(20.0, -75.0, 100.0, Slope::Gentle, 73);
    for (what, input) in [
        ("speech over white", with_noise(&speech, 8.0, -75.0, 74)),
        ("speech over steep rumble", mix(&speech, &steep, 8.0, -75.0)),
        (
            "breathy over gentle rumble",
            mix(&breathy, &gentle, 8.0, -75.0),
        ),
        (
            "breathy over steep rumble",
            mix(&breathy, &steep, 8.0, -75.0),
        ),
    ] {
        let (out, _) = run(&input);
        let converged = db_from_target(&out[secs(10.0)..]);
        assert!(
            converged.abs() < CONVERGED_TOLERANCE_DB,
            "{what}: held {converged:+.2} dB from the target"
        );
    }
}

#[test]
fn flush_keeps_the_step_bound_when_a_take_ends_on_a_transient() {
    // A take can end anywhere, on anything. Its last samples sit in a partial limiter block, as
    // short as one sample; however the level jumps into or out of that block, the flush neither
    // clips nor steps by more than the stated bound. The gain is locked high (about +39 dB) on
    // continuous speech, so every sample's gain can be read off.
    let bed = breathy_speech(7.0, -60.0, 0.35, 80);
    let whole = 600 * LIMITER_BLOCK; // 6 s of whole blocks
    let bound_db = to_dbfs(MAX_GAIN) / LIMITER_BLOCK as f32 + 0.01;
    let click = |s: &mut [f32]| {
        for (i, v) in s.iter_mut().enumerate() {
            *v = if i.is_multiple_of(2) { 0.9 } else { -0.9 };
        }
    };
    for partial in [1usize, 5, 80] {
        let mut loud_partial = bed[..whole + partial].to_vec();
        click(&mut loud_partial[whole..]);
        let mut loud_last_block = bed[..whole + partial].to_vec();
        click(&mut loud_last_block[whole - 100..whole - 20]);
        for (what, input) in [
            ("a click in the partial block", loud_partial),
            ("a click just before a quiet partial block", loud_last_block),
        ] {
            let (out, _) = run(&input);
            assert_eq!(out.len(), input.len());
            let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(
                peak <= 1.0,
                "{what}, {partial}-sample partial: clipped at {peak}"
            );
            let gain_at =
                |n: usize| (input[n].abs() >= 1e-6).then(|| to_dbfs((out[n] / input[n]).abs()));
            let mut steepest = 0.0f32;
            for n in input.len() - 2 * LIMITER_BLOCK - partial..input.len() - 1 {
                if let (Some(a), Some(b)) = (gain_at(n), gain_at(n + 1)) {
                    steepest = steepest.max((b - a).abs());
                }
            }
            assert!(
                steepest <= bound_db,
                "{what}, {partial}-sample partial: stepped {steepest:.2} dB (bound {bound_db:.3})"
            );
            let before = gain_at(whole - 3 * LIMITER_BLOCK).expect("voiced");
            assert!(before > 35.0, "{what}: locked at {before:.1} dB");
        }
    }
}
