//! The meeting AGC: a slow gain toward the normaliser's target, learned from speech.
//!
//! The VAD is scripted (see `common`): an oracle that knows where the speech is, and a deaf one
//! that also cannot hear windows quieter than −50 dBFS RMS. Levels are compared in dB with stated
//! tolerances. The AGC delays its output by exactly [`Agc::LATENCY`] samples; the helpers below
//! undo that so output and input line up.

mod common;

use common::{Always, DeafOracle, Failing, Oracle, speech_mask};
use ink_audio::Agc;
use ink_audio::agc::{FALL_DB_PER_S, LIMITER_BLOCK, RISE_DB_PER_S, SPEECH_WINDOW_FRAMES};
use ink_audio::gain::{
    LEVEL_FRAME, MAX_GAIN, NOISE_FLOOR, TARGET_PEAK, from_dbfs, robust_peak, to_dbfs,
};
use ink_audio::synth::{
    Slope, SpeechShape, breathy_speech, cycling_fan, knocks, mix, noise, rumble, speech_like,
    speech_with, swing, with_noise,
};
use ink_audio::vad::{SpeechProbability, VAD_WINDOW, VadConfig};

const SR: usize = 16_000;

/// How close to the target a converged AGC must hold speech.
const CONVERGED_TOLERANCE_DB: f32 = 1.0;

/// An AGC whose VAD knows where `clean` has speech but, like a real one, cannot hear it below
/// −50 dBFS: it has to listen to the lifted copy.
fn deaf(clean: &[f32]) -> Agc {
    Agc::with_vad(DeafOracle::boxed(speech_mask(clean)), VadConfig::default())
}

/// An AGC whose VAD knows where `clean` has speech, at any level.
fn oracle(clean: &[f32]) -> Agc {
    Agc::with_vad(Oracle::boxed(speech_mask(clean)), VadConfig::default())
}

/// An AGC whose VAD says `p` about everything.
fn always(p: f32) -> Agc {
    Agc::with_vad(Box::new(Always(p)), VadConfig::default())
}

/// Runs `input` through `agc` one level frame per push and returns the output aligned with the
/// input, plus the AGC's gain (in dB) after every frame.
fn run(mut agc: Agc, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
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
    let (out, _) = run(deaf(&input), &input);
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
    // The truth: speech either side, nothing in the pause.
    let mut clean = input.to_vec();
    clean[pause_start..pause_end].fill(0.0);
    let (out, gains) = run(oracle(&clean), input);
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
    let (out, gains) = run(oracle(&input), &input);

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

    // Slow up, fast down: after its first lock, the gain never rises faster than the rise rate
    // nor falls faster than the fall rate. Measured over 100 ms (five frames), allowing one frame's
    // step more: frames wait for their VAD window's verdict, so two can be learned in one frame's
    // time and none in the next.
    let (rise_step, fall_step) = (RISE_DB_PER_S / 50.0, FALL_DB_PER_S / 50.0);
    let first_lock = gains.iter().position(|&g| g > 0.0).unwrap_or(0);
    for w in gains[first_lock + 1..].windows(6) {
        let change = w[5] - w[0];
        assert!(
            change <= 6.0 * rise_step + 0.001,
            "rose {change:.3} dB in 100 ms"
        );
        assert!(
            -change <= 6.0 * fall_step + 0.001,
            "fell {:.3} dB in 100 ms",
            -change
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
    let (out, gains) = run(oracle(&input), &input);
    assert!(gains.iter().all(|&g| g == 0.0), "gain moved off unity");
    assert_eq!(out, input);
}

#[test]
fn agc_output_is_its_input_one_frame_late() {
    // The one-frame look-ahead is what lets the gain drop before a loud frame, not after it.
    assert_eq!(Agc::LATENCY, LEVEL_FRAME);
    let input = speech_like(1.0, -15.0, 10);
    let mut agc = always(0.9);
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
    let (whole, _) = run(deaf(&input), &input);
    let mut agc = deaf(&input);
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
    let (out, gains) = run(oracle(&speech), &input);

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
    let (_, without) = run(oracle(&speech), &speech);
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
            let (out, _) = run(oracle(&bed[..input.len()]), &input);
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

/// Plain and breathy speech at `syllables`, clean and 8 dB over white noise and steep 100 Hz
/// rumble, at −75 dBFS RMS, `seconds` long, each with its clean speech for the truth.
fn speech_fixtures(
    syllables: &[f64],
    seconds: f64,
    seed: u64,
) -> Vec<(String, Vec<f32>, Vec<f32>)> {
    let mut out = Vec::new();
    for (i, &syllable) in syllables.iter().enumerate() {
        for (kind, shape) in [
            ("plain", SpeechShape::plain(syllable)),
            ("breathy", SpeechShape::breathy(syllable)),
        ] {
            let seed = seed + 10 * i as u64;
            let clean = speech_with(seconds, -75.0, shape, seed);
            let name = |over: &str| format!("{kind} speech, {syllable} s syllables, {over}");
            let steep = rumble(seconds, -75.0, 100.0, Slope::Steep, seed + 1);
            out.push((name("clean"), clean.clone(), clean.clone()));
            out.push((
                name("8 dB over white noise"),
                with_noise(&clean, 8.0, -75.0, seed + 2),
                clean.clone(),
            ));
            out.push((
                name("8 dB over steep 100 Hz rumble"),
                mix(&clean, &steep, 8.0, -75.0),
                clean,
            ));
        }
    }
    out
}

/// Where a converged AGC must hold speech: the last half of the output, speech frames only.
fn converged_speech_db(out: &[f32], clean: &[f32]) -> f32 {
    let mask = speech_mask(clean);
    let half = out.len() / 2;
    let mut peaks: Vec<f32> = out
        .chunks(LEVEL_FRAME)
        .enumerate()
        .skip(half / LEVEL_FRAME)
        .filter(|(k, _)| {
            let middle = k * LEVEL_FRAME + LEVEL_FRAME / 2;
            mask.get(middle / VAD_WINDOW).copied().unwrap_or(false)
        })
        .map(|(_, f)| f.iter().fold(0.0f32, |m, s| m.max(s.abs())))
        .collect();
    peaks.sort_by(|a, b| b.total_cmp(a));
    to_dbfs(peaks[8.min(peaks.len() - 1)]) - to_dbfs(TARGET_PEAK)
}

#[test]
fn agc_lifts_fast_and_slow_speech_to_the_target() {
    // Plain and breathy speech at every syllable length from 0.12 s to 0.26 s, clean and 8 dB over
    // white noise and steep rumble, at −75 dBFS RMS, through a VAD that cannot hear below −50 dBFS:
    // every one ends at the target.
    for (what, input, clean) in speech_fixtures(&[0.12, 0.13, 0.15, 0.20, 0.26], 10.0, 100) {
        let (out, _) = run(deaf(&clean), &input);
        let off = converged_speech_db(&out, &clean);
        assert!(
            off.abs() < CONVERGED_TOLERANCE_DB,
            "{what}: held {off:+.2} dB from the target"
        );
    }
}

#[test]
fn the_agc_vad_listens_to_a_lifted_copy() {
    // The VAD cannot hear −75 dBFS speech in the raw input: fed the raw windows, it finds none.
    // The AGC starts at unity gain, so listening to its own output would never start either. Its
    // VAD hears the input lifted by a provisional gain, and the AGC locks.
    let input = speech_like(10.0, -75.0, 30);
    let mut raw = DeafOracle::new(speech_mask(&input));
    let heard = input
        .chunks_exact(VAD_WINDOW)
        .filter(|w| {
            let w: &[f32; VAD_WINDOW] = (*w).try_into().unwrap();
            raw.probability(w).unwrap() > 0.5
        })
        .count();
    assert_eq!(heard, 0, "the deaf VAD heard the raw input");
    let (out, _) = run(deaf(&input), &input);
    assert!(converged_speech_db(&out, &input).abs() < CONVERGED_TOLERANCE_DB);
}

/// Everything the AGC must not learn a level from, `seconds` long: white room tone, rumble at
/// 100 Hz to 1 kHz (gentle and steep), rumble swinging slowly in level (8 s and 20 s periods, 10
/// and 15 dB deep), a cycling fan and knocks.
fn non_speech_fixtures(seconds: f64, seed: u64) -> Vec<(String, Vec<f32>)> {
    let mut out = vec![("white room tone".to_string(), noise(seconds, -70.0, seed))];
    for (i, slope) in [Slope::Gentle, Slope::Steep].into_iter().enumerate() {
        for (j, hz) in [100.0, 250.0, 500.0, 1_000.0].into_iter().enumerate() {
            out.push((
                format!("{slope:?} {hz} Hz rumble"),
                rumble(seconds, -70.0, hz, slope, seed + 1 + (i * 4 + j) as u64),
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

#[test]
fn agc_learns_only_from_speech() {
    // A minute of each: rumble of every shape, swinging rumble, a fan, knocks, room tone. The VAD
    // hears no speech in them, and the gain never moves off unity, whatever the audio looks like.
    for (what, input) in non_speech_fixtures(60.0, 200) {
        let (out, gains) = run(always(0.05), &input);
        let highest = gains.iter().fold(f32::MIN, |m, &g| m.max(g));
        assert_eq!(highest, 0.0, "{what}: the gain rose to {highest:.2} dB");
        assert_eq!(out, input, "{what}");
    }
}

#[test]
fn a_failing_vad_holds_the_gain_and_says_so() {
    // A VAD that fails (its model gone) is never guessed around: the AGC learns nothing more,
    // holds its gain, and reports the error for the pipeline to act on.
    let input = speech_like(5.0, -60.0, 31);
    let mut agc = Agc::with_vad(Box::new(Failing), VadConfig::default());
    assert!(agc.uses_vad());
    let mut block = input.clone();
    agc.process(&mut block);
    assert_eq!(agc.gain(), 1.0);
    assert_eq!(
        agc.vad_error(),
        Some(&ink_core::EngineError::ModelMissing("vad".into()))
    );
}

// --- The fallback, without a VAD. --------------------------------------------------------------

#[test]
fn agc_without_vad_never_lifts_white_room_tone_or_gentle_rumble() {
    // What its contrast rule still refuses: three minutes of white room tone, and a minute each of
    // gently low-passed rumble.
    let mut audio = vec![("white room tone".to_string(), noise(180.0, -70.0, 300))];
    for (j, hz) in [100.0, 250.0, 500.0, 1_000.0].into_iter().enumerate() {
        audio.push((
            format!("gentle {hz} Hz rumble"),
            rumble(60.0, -70.0, hz, Slope::Gentle, 301 + j as u64),
        ));
    }
    for (what, input) in audio {
        let (_, gains) = run(Agc::without_vad(), &input);
        let highest = gains.iter().fold(f32::MIN, |m, &g| m.max(g));
        assert_eq!(highest, 0.0, "{what}: the gain rose to {highest:.2} dB");
    }
}

#[test]
fn agc_without_vad_lifts_fast_and_slow_speech() {
    // Without a VAD the AGC errs toward lifting: brisk speech (which a correlation rule refused)
    // and slow speech, clean and in noise, all reach the target.
    for (what, input, clean) in speech_fixtures(&[0.12, 0.13, 0.15, 0.26], 10.0, 400) {
        let (out, _) = run(Agc::without_vad(), &input);
        let off = converged_speech_db(&out, &clean);
        assert!(
            off.abs() < CONVERGED_TOLERANCE_DB,
            "{what}: held {off:+.2} dB from the target"
        );
    }
}

#[test]
fn agc_without_vad_can_lift_steep_and_swinging_rumble() {
    // The price of the fallback, committed so nobody mistakes it for a VAD: steeply low-passed
    // rumble and rumble swinging slowly in level have speech-band contrast, and the gain climbs on
    // them. With a VAD it never does (`agc_learns_only_from_speech`).
    for (what, input) in [
        (
            "steep 100 Hz rumble",
            rumble(30.0, -70.0, 100.0, Slope::Steep, 500),
        ),
        (
            "steep 250 Hz rumble swinging 15 dB every 8 s",
            swing(&rumble(30.0, -70.0, 250.0, Slope::Steep, 501), 8.0, 15.0),
        ),
    ] {
        let (_, gains) = run(Agc::without_vad(), &input);
        let highest = gains.iter().fold(f32::MIN, |m, &g| m.max(g));
        assert!(
            highest > 10.0,
            "{what}: expected the fallback to climb; {highest:.2} dB"
        );
    }
}

#[test]
fn agc_without_vad_holds_its_gain_through_a_pause_of_room_tone() {
    // The fallback's pause behaviour: speech, ten seconds of −75 dBFS room tone, speech.
    let (input, start, end) = speech_pause_speech(noise(10.0, -75.0, 4));
    let (_, gains) = run(Agc::without_vad(), &input);
    let frame = |sample: usize| sample / LEVEL_FRAME;
    let at_pause = gains[frame(start) - 1];
    let most = gains[frame(start)..frame(end)]
        .iter()
        .fold(f32::MIN, |m, &g| m.max(g));
    assert!(most - at_pause < 0.01, "rose {:.2} dB", most - at_pause);
    assert!(!Agc::without_vad().uses_vad());
}

/// How long after a pause a deaf VAD's AGC may take to reach the level of speech that resumes
/// 6 dB quieter. Derived from the design, not measured: the old frames leave the level window
/// only once all but the skipped few have been replaced (142 of 150 learned frames, 2.84 s at one
/// per frame, ×1.5 for the frames speech gaps do not supply), then the gain climbs 6 dB at the
/// rise rate.
fn resume_bound_s() -> f64 {
    let turnover = SPEECH_WINDOW_FRAMES as f64 / 50.0 * 1.5 * (142.0 / 150.0);
    turnover + 6.0 / f64::from(RISE_DB_PER_S)
}

/// A deaf VAD's AGC through speech (−45 dBFS), `pause`, then speech 6 dB quieter: the gain holds
/// through the pause, and learns from the speech after it. After a pause longer than the 1.5 s
/// the provisional gain looks back over, that gain reads the pause (silence reads as MAX_GAIN), so
/// the VAD's copy of the first resumed frames is lifted hard and clamped; the VAD must still hear
/// them, and the output must still never clip.
fn assert_deaf_holds_then_learns(pause: Vec<f32>) {
    let mut input = speech_like(8.0, -45.0, 2);
    let pause_start = input.len();
    input.extend(pause);
    let pause_end = input.len();
    input.extend(speech_like(10.0, -51.0, 3));
    let mut clean = input.clone();
    clean[pause_start..pause_end].fill(0.0);
    let (out, gains) = run(deaf(&clean), &input);

    // (a) The gain holds through the pause.
    let frame = |sample: usize| sample / LEVEL_FRAME;
    let at_pause = gains[frame(pause_start) - 1];
    let most = gains[frame(pause_start)..frame(pause_end)]
        .iter()
        .fold(f32::MIN, |m, &g| m.max(g));
    assert!(
        most - at_pause < 0.01,
        "the gain rose {:.2} dB during the pause",
        most - at_pause
    );

    // (b) The speech after the pause is heard and learned from: the gain climbs to meet it, and by
    // the stated bound it sits at the target and stays there.
    let bound = secs(resume_bound_s());
    let settled = converged_speech_db(
        &out[pause_end + bound..pause_end + bound + secs(2.0)],
        &clean[pause_end + bound..pause_end + bound + secs(2.0)],
    );
    assert!(
        settled.abs() < CONVERGED_TOLERANCE_DB,
        "{settled:+.2} dB from the target {:.1} s after the pause",
        resume_bound_s()
    );
    let climbed = gains[gains.len() - 1] - at_pause;
    assert!(
        climbed > 5.0,
        "the gain climbed only {climbed:.2} dB after the pause"
    );

    // And the output, played at the AGC's gain, never clips.
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak <= 1.0, "the output clipped at {peak}");
}

#[test]
fn a_deaf_vad_holds_through_a_pause_of_room_tone_and_learns_after_it() {
    assert_deaf_holds_then_learns(noise(10.0, -75.0, 4));
}

#[test]
fn a_deaf_vad_holds_through_digital_silence_and_learns_after_it() {
    assert_deaf_holds_then_learns(vec![0.0; secs(10.0)]);
}

#[test]
fn agc_hears_a_talker_at_the_noise_floor_through_the_capped_copy() {
    // The cap-bound margin: the quietest take that reaches the VAD (a robust peak just above
    // NOISE_FLOOR) is lifted by MAX_GAIN to at least NOISE_FLOOR × MAX_GAIN (−20 dBFS peak),
    // far above the deaf VAD's −50 dBFS. Raising NOISE_FLOOR's partners (lowering MAX_GAIN, or a
    // VAD that needs more level) must keep this true, or such a talker is never heard.
    assert!(NOISE_FLOOR * MAX_GAIN > from_dbfs(DeafOracle::HEARING_DBFS));
    let input = speech_like(10.0, -86.0, 40);
    let robust = robust_peak(&input);
    assert!(robust > NOISE_FLOOR && TARGET_PEAK / robust > MAX_GAIN);
    let (_, gains) = run(deaf(&input), &input);
    assert_eq!(
        gains[gains.len() - 1],
        to_dbfs(MAX_GAIN),
        "the AGC did not learn the talker at the floor"
    );
}
