//! Downmix: the primary channel for the mic, the average for the far end.

use ink_audio::Downmix;
use ink_audio::gain::{rms, to_dbfs};
use ink_audio::synth::speech_like;
use ink_core::Channel;

/// Interleaves per-channel signals.
fn interleave(channels: &[&[f32]]) -> Vec<f32> {
    let frames = channels[0].len();
    let mut out = Vec::with_capacity(frames * channels.len());
    for i in 0..frames {
        for c in channels {
            out.push(c[i]);
        }
    }
    out
}

fn mix(how: Downmix, interleaved: &[f32], channels: u16) -> Vec<f32> {
    let mut out = Vec::new();
    how.apply(interleaved, channels, &mut out);
    out
}

#[test]
fn the_choice_is_per_stream() {
    assert_eq!(Downmix::for_channel(Channel::Mic), Downmix::Primary);
    assert_eq!(Downmix::for_channel(Channel::Far), Downmix::Average);
}

#[test]
fn mic_takes_the_primary_channel_when_the_second_is_dead() {
    // Averaging a live channel with a dead one halves the speech (−6 dB) before the gain stage
    // ever sees it.
    let voice = speech_like(1.0, -40.0, 1);
    let dead = vec![0.0f32; voice.len()];
    let stereo = interleave(&[&voice, &dead]);
    let mic = mix(Downmix::for_channel(Channel::Mic), &stereo, 2);
    assert_eq!(mic, voice);
    // What averaging would have done: the control for this test.
    let averaged = mix(Downmix::Average, &stereo, 2);
    let loss = to_dbfs(rms(&voice)) - to_dbfs(rms(&averaged));
    assert!((loss - 6.02).abs() < 0.01, "average lost {loss} dB");
}

#[test]
fn mic_takes_the_primary_channel_when_the_second_is_phase_inverted() {
    // A second capsule wired with inverted polarity cancels the voice outright when averaged.
    let voice = speech_like(1.0, -40.0, 2);
    let inverted: Vec<f32> = voice.iter().map(|s| -s).collect();
    let stereo = interleave(&[&voice, &inverted]);
    let mic = mix(Downmix::for_channel(Channel::Mic), &stereo, 2);
    assert_eq!(mic, voice);
    assert!(
        mix(Downmix::Average, &stereo, 2).iter().all(|&s| s == 0.0),
        "the control: averaging cancels it"
    );
}

#[test]
fn mic_array_takes_the_primary_channel_instead_of_comb_filtering() {
    // Three capsules hear the same voice a few samples apart; their sum comb-filters it.
    let voice = speech_like(1.0, -40.0, 3);
    let shifted = |d: usize| -> Vec<f32> {
        let mut v = vec![0.0; d];
        v.extend_from_slice(&voice[..voice.len() - d]);
        v
    };
    let (b, c) = (shifted(3), shifted(7));
    let array = interleave(&[&voice, &b, &c]);
    assert_eq!(mix(Downmix::for_channel(Channel::Mic), &array, 3), voice);
}

#[test]
fn far_end_averages_so_a_voice_panned_to_one_side_survives() {
    // Two remote participants panned hard left and hard right. The primary channel alone would
    // lose the right-hand one entirely; the average keeps both.
    let left_voice = speech_like(1.0, -30.0, 4);
    let right_voice = speech_like(1.0, -30.0, 5);
    let silent = vec![0.0f32; left_voice.len()];
    let only_right = interleave(&[&silent, &right_voice]);
    let far = mix(Downmix::for_channel(Channel::Far), &only_right, 2);
    let kept = to_dbfs(rms(&far)) - to_dbfs(rms(&right_voice));
    assert!((kept + 6.02).abs() < 0.01, "right voice kept at {kept} dB");
    assert!(
        mix(Downmix::Primary, &only_right, 2)
            .iter()
            .all(|&s| s == 0.0),
        "the control: the primary channel drops it"
    );
    let both = interleave(&[&left_voice, &right_voice]);
    let far = mix(Downmix::for_channel(Channel::Far), &both, 2);
    for (i, s) in far.iter().enumerate() {
        assert!((s - (left_voice[i] + right_voice[i]) / 2.0).abs() < 1e-7);
    }
}

#[test]
fn far_end_average_keeps_a_centred_voice_at_its_level() {
    let voice = speech_like(1.0, -30.0, 6);
    let stereo = interleave(&[&voice, &voice]);
    assert_eq!(mix(Downmix::for_channel(Channel::Far), &stereo, 2), voice);
}

#[test]
fn mono_passes_through_for_both_streams() {
    let voice = speech_like(0.5, -30.0, 7);
    for how in [Downmix::Primary, Downmix::Average] {
        assert_eq!(mix(how, &voice, 1), voice);
    }
}

#[test]
fn a_zero_channel_count_is_mono_and_a_partial_frame_is_ignored() {
    assert_eq!(
        mix(Downmix::Average, &[0.1, 0.2, 0.3], 0),
        vec![0.1, 0.2, 0.3]
    );
    // Five samples of stereo: two whole frames and a stray sample.
    assert_eq!(
        mix(Downmix::Average, &[0.1, 0.3, 0.5, 0.7, 0.9], 2),
        vec![0.2, 0.6]
    );
    assert_eq!(
        mix(Downmix::Primary, &[0.1, 0.3, 0.5, 0.7, 0.9], 2),
        vec![0.1, 0.5]
    );
}

#[test]
fn downmix_appends_to_what_is_already_there() {
    let mut out = vec![9.0];
    Downmix::Primary.apply(&[0.1, 0.2], 2, &mut out);
    assert_eq!(out, vec![9.0, 0.1]);
}
