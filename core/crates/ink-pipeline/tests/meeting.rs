//! The meeting chain end to end on mocks: live, final pass, diarization, supersede, summary and
//! commitments. Each carry-over from earlier reviews has a named test here:
//!
//! | Carry-over | Test |
//! |---|---|
//! | 1: the AGC with a VAD; times compensate its 20 ms | `the_live_agc_learns_level_from_vad_speech_only`, `live_final_times_compensate_the_agc_latency` |
//! | 2: a VAD failure switches to the fallback and goes on | `a_mid_stream_vad_failure_switches_the_live_agc_to_the_fallback`, `a_final_pass_vad_failure_falls_back_and_is_reported` |
//! | 3: only VAD speech reaches the final engine; empty speech regions are reported | `only_vad_speech_regions_reach_the_final_engine`, `a_speech_region_the_engine_returns_empty_is_reported`, `a_live_final_over_silence_is_not_saved` |
//! | 5: the end is never before the start | `the_end_is_never_recorded_before_the_start` |
//!
//! Carry-over 4 (real-VAD tests use AMI audio) is `tests/meeting_fixture.rs`; 6 (the chunk sync on
//! the pump) is `tests/fsync_stall.rs`; 7 (I5) is `tests/meeting_privacy.rs` and the source lint.

mod meeting_rig;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ink_audio::gain::{TARGET_PEAK, robust_peak, to_dbfs};
use ink_audio::{Agc, SpeechProbability};
use ink_core::mock::MemStore;
use ink_core::{
    CancelToken, Channel, Clock, EngineError, LlmError, LlmRequest, Segment, SpeakerId, Store,
    StoreError, StreamFormat,
};
use ink_pipeline::events::{VadUnavailable, VoiceDetection};
use ink_pipeline::meeting::events::{
    ChannelPass, Diarization, KeptLive, MeetingEvent, MeetingWarning, Phase,
};
use ink_pipeline::speech::VadSource;
use meeting_rig::*;

const STEREO_48K: StreamFormat = StreamFormat {
    sample_rate: 48_000,
    channels: 2,
};

/// A short two-sided meeting: you speak at 0.5 s and 6 s, they speak at 3 s.
fn conversation() -> (Vec<f32>, Vec<f32>) {
    let mic = join(&[
        silence(0.5),
        speech(2.0, -30.0, 1),
        silence(3.5),
        speech(1.5, -30.0, 2),
        silence(0.5),
    ]);
    let far = join(&[silence(3.0), speech(2.0, -30.0, 3), silence(3.0)]);
    (mic, far)
}

fn assert_monotonic(segments: &[Segment]) {
    for channel in [Channel::Mic, Channel::Far] {
        let mut last = 0;
        for s in segments.iter().filter(|s| s.channel == channel) {
            assert!(s.end_ms >= s.start_ms, "{s:?}");
            assert!(
                s.start_ms >= last,
                "{channel:?} went back in time: {segments:#?}"
            );
            last = s.start_ms;
        }
    }
    assert!(
        segments.windows(2).all(|w| w[0].start_ms <= w[1].start_ms),
        "the record is in time order"
    );
}

fn you_and_them(segments: &[Segment], mic: &str, far: &str) {
    for s in segments {
        let expected = match s.channel {
            Channel::Mic => mic,
            Channel::Far => far,
        };
        assert!(
            s.text.starts_with(expected),
            "{:?} holds {:?}",
            s.channel,
            s.text
        );
    }
}

#[test]
fn two_sides_give_revision_2_with_you_and_them_and_monotonic_times() {
    let mut rig = RigBuilder::default().build();
    let record = rig.chain().record().clone();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);

    // Live: revision 1 holds the live finals, each on its own side.
    let ended = rig.stop();
    let live = rig.store.segments(&record).unwrap();
    assert_eq!(rig.store.record(&record).unwrap().unwrap().revision, 1);
    assert_eq!(live.iter().filter(|s| s.channel == Channel::Mic).count(), 2);
    assert_eq!(live.iter().filter(|s| s.channel == Channel::Far).count(), 1);
    you_and_them(&live, "live you", "live them");
    assert_monotonic(&live);

    // Final: revision 2, the same sides, times in order.
    let outcome = ended.finalize(&rig.chunks, &CancelToken::new()).unwrap();
    assert!(outcome.superseded);
    assert_eq!(outcome.revision, Some(2));
    assert_eq!(rig.store.record(&record).unwrap().unwrap().revision, 2);
    let finals = rig.store.segments(&record).unwrap();
    assert_eq!(finals.len(), 3, "{finals:#?}");
    you_and_them(&finals, "mic words", "far words");
    assert_monotonic(&finals);
    // Regions sit on their speech, padded by 250 ms.
    let near = |ms: u64, want: u64| ms.abs_diff(want) <= 100;
    assert!(
        near(finals[0].start_ms, 250) && near(finals[0].end_ms, 2_750),
        "{finals:?}"
    );
    assert!(
        near(finals[1].start_ms, 2_750) && near(finals[1].end_ms, 5_250),
        "{finals:?}"
    );
    assert!(
        near(finals[2].start_ms, 5_750) && near(finals[2].end_ms, 7_750),
        "{finals:?}"
    );
    // Each call named its side.
    let calls = rig.engine.mock.calls();
    assert_eq!(
        calls.iter().map(|c| c.channel).collect::<Vec<_>>(),
        [Channel::Mic, Channel::Mic, Channel::Far]
    );
    let events = rig.events();
    assert!(events.contains(&MeetingEvent::Superseded { revision: 2 }));
    assert_eq!(
        events.last(),
        Some(&MeetingEvent::Finished { revision: Some(2) })
    );
    assert_eq!(
        rig.warnings(),
        [MeetingWarning::SummaryUnavailable],
        "no model in this rig"
    );
}

#[test]
fn the_supersede_bumps_the_revision_in_sqlite_too() {
    let sqlite: Arc<dyn Store> = Arc::new(ink_store::SqliteStore::open_in_memory().unwrap());
    let mut rig = RigBuilder {
        store: Some(sqlite.clone()),
        ..RigBuilder::default()
    }
    .build();
    let record = rig.chain().record().clone();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);
    let outcome = rig.finish().unwrap();
    assert_eq!(outcome.revision, Some(2));
    let stored = sqlite.record(&record).unwrap().unwrap();
    assert_eq!(stored.revision, 2);
    assert!(stored.ended_at_unix_ms.is_some());
    let finals = sqlite.segments(&record).unwrap();
    you_and_them(&finals, "mic words", "far words");
    assert_monotonic(&finals);
}

/// Architecture rule 11 and the step's check: a −75 dBFS meeting reaches the final engine at the
/// gain target on both sides, the far side through a 48 kHz stereo device (resampled, averaged).
#[test]
fn meeting_finals_reach_the_engine_at_the_gain_target() {
    let mut rig = RigBuilder {
        far_format: STEREO_48K,
        ..RigBuilder::default()
    }
    .build();
    let mic = join(&[silence(1.0), speech(3.0, -75.0, 11), silence(4.0)]);
    let far = join(&[silence(4.5), speech(3.0, -75.0, 12), silence(0.5)]);
    assert!((rms_dbfs(&speech(3.0, -75.0, 11)) + 75.0).abs() < 0.01);
    rig.feed(&mic, &far);
    let outcome = rig.finish().unwrap();
    assert_eq!(outcome.revision, Some(2));

    let calls = rig.engine.mock.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    for call in &calls {
        assert!(call.rms_dbfs > -30.0, "{call:?}");
        assert!(call.peak >= TARGET_PEAK * 0.99, "{call:?}");
    }
    for (channel, input) in rig.engine.inputs.lock().unwrap().iter() {
        let off = to_dbfs(robust_peak(input)) - to_dbfs(TARGET_PEAK);
        assert!(off.abs() < 1.0, "{channel:?} is {off:.2} dB off the target");
    }
    // The far side's chunks hold the device's own format, every frame of it.
    let far_chunks = rig.chunks.chunks(Channel::Far).unwrap();
    assert!(far_chunks.unreadable.is_empty());
    assert!(far_chunks.chunks.iter().all(|c| c.format == STEREO_48K));
    let frames: u64 = far_chunks.chunks.iter().map(|c| c.frames).sum();
    assert_eq!(frames, far.len() as u64 * 3);
}

/// Carry-over 1: the live AGC is the VAD-gated one. Quiet speech is lifted when the VAD hears it,
/// and left alone when it hears nothing (the fallback, which would lift it, is not in use).
#[test]
fn the_live_agc_learns_level_from_vad_speech_only() {
    let mic = join(&[silence(0.5), speech(4.0, -60.0, 21), silence(0.5)]);
    let far = silence(5.0);
    let lifted = |vad: VadSource| {
        let mut rig = RigBuilder {
            vad,
            ..RigBuilder::default()
        }
        .build();
        rig.feed(&mic, &far);
        let _ = rig.stop();
        let heard = rig.live.received(Channel::Mic);
        // The last two seconds of speech, after any lock.
        (rms_dbfs(&heard[40_000..72_000]), rig.events())
    };
    let (with_vad, events) = lifted(energy_vad());
    assert!(with_vad > -40.0, "lifted from −60 to {with_vad:.1} dBFS");
    for channel in [Channel::Mic, Channel::Far] {
        assert!(events.contains(&MeetingEvent::VoiceDetection {
            channel,
            state: VoiceDetection::Available
        }));
    }
    let (deaf, _) = lifted(vad_source(|_| Box::new(NeverVad)));
    // The same samples at the input: output sample o is input sample o − 320.
    let input = rms_dbfs(&mic[40_000 - Agc::LATENCY..72_000 - Agc::LATENCY]);
    assert!(
        (deaf - input).abs() < 0.01,
        "a VAD that hears nothing moves no gain: {deaf:.2} against {input:.2} dBFS"
    );
}

/// A tone that starts at exactly `at_s` seconds, for timing.
fn tone_at(at_s: f64, seconds: f64, total_s: f64) -> Vec<f32> {
    let mut x = silence(total_s);
    let start = (at_s * RATE as f64) as usize;
    let n = (seconds * RATE as f64) as usize;
    for i in 0..n {
        x[start + i] = 0.1 * (i as f32 * 0.2).cos();
    }
    x
}

/// Carry-over 1: the AGC's output is its input 20 ms late, so the live engine hears a tone that
/// starts at 1.000 s at 1.020 s of its stream. The chain moves every final back by that latency.
#[test]
fn live_final_times_compensate_the_agc_latency() {
    let mut rig = RigBuilder::default().build();
    let record = rig.chain().record().clone();
    rig.feed(&tone_at(1.0, 0.5, 3.0), &silence(3.0));
    let _ = rig.stop();
    let heard = rig.live.received(Channel::Mic);
    let onset = heard.iter().position(|s| s.abs() > 0.001).unwrap();
    assert_eq!(
        onset,
        16_000 + Agc::LATENCY,
        "the engine hears it 20 ms late"
    );
    let live = rig.store.segments(&record).unwrap();
    assert_eq!(live.len(), 1, "{live:?}");
    assert_eq!(live[0].start_ms, 1_000, "placed where it was said");
    assert_eq!(live[0].end_ms, 1_500);
}

/// Carry-over 2: the mic's VAD fails 1.92 s in. The AGC is flushed into the stream, the fallback
/// takes over, the shell hears it once, and the side goes on: the live engine gets every sample
/// exactly once, the audio after the failure is still levelled, and later times are still right.
#[test]
fn a_mid_stream_vad_failure_switches_the_live_agc_to_the_fallback() {
    let vad = vad_source(|n| -> Box<dyn SpeechProbability> {
        if n == 0 {
            Box::new(FailAfter {
                inner: EnergyVad(-50.0),
                windows: 60,
            })
        } else {
            Box::new(EnergyVad(-50.0))
        }
    });
    let mut rig = RigBuilder {
        vad,
        ..RigBuilder::default()
    }
    .build();
    let record = rig.chain().record().clone();
    let mut mic = join(&[silence(0.5), speech(4.0, -60.0, 31), silence(1.0)]);
    mic.extend(tone_at(0.0, 1.0, 1.5));
    let far = silence(mic.len() as f64 / RATE as f64);
    rig.feed(&mic, &far);
    let _ = rig.stop();

    let warnings = rig.warnings();
    let failures: Vec<_> = warnings
        .iter()
        .filter(|w| matches!(w, MeetingWarning::VadFailed { .. }))
        .collect();
    assert_eq!(failures.len(), 1, "{warnings:?}");
    assert!(matches!(
        failures[0],
        MeetingWarning::VadFailed {
            channel: Channel::Mic,
            phase: Phase::Live,
            ..
        }
    ));
    let events = rig.events();
    assert!(events.contains(&MeetingEvent::VoiceDetection {
        channel: Channel::Mic,
        state: VoiceDetection::Unavailable(VadUnavailable::Failed)
    }));
    assert!(
        !events.contains(&MeetingEvent::VoiceDetection {
            channel: Channel::Far,
            state: VoiceDetection::Unavailable(VadUnavailable::Failed)
        }),
        "the far end keeps its VAD"
    );

    // Every sample once: the input, plus the first AGC's leading silence.
    let heard = rig.live.received(Channel::Mic);
    assert_eq!(heard.len(), mic.len() + Agc::LATENCY);
    // Still levelled two seconds after the switch, by the fallback.
    let late = rms_dbfs(&heard[64_000..72_000]);
    assert!(late > -45.0, "{late:.1} dBFS from −60");
    // The tone at 5.500 s is placed at 5.500 s: the replacement's own latency is not added.
    let live = rig.store.segments(&record).unwrap();
    let tone = live
        .iter()
        .find(|s| s.start_ms >= 5_000)
        .expect("the tone's final");
    assert_eq!(tone.start_ms, 5_500);
}

/// Carry-over 2, the final pass: the VAD fails there, the pass levels with the fallback, finishes,
/// and says so.
#[test]
fn a_final_pass_vad_failure_falls_back_and_is_reported() {
    // Instances 0 and 1 are live; 2 is the mic's final pass.
    let vad = vad_source(|n| -> Box<dyn SpeechProbability> {
        if n == 2 {
            Box::new(FailAfter {
                inner: EnergyVad(-50.0),
                windows: 0,
            })
        } else {
            Box::new(EnergyVad(-50.0))
        }
    });
    let mut rig = RigBuilder {
        vad,
        ..RigBuilder::default()
    }
    .build();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);
    let outcome = rig.finish().unwrap();
    assert!(rig.warnings().iter().any(|w| matches!(
        w,
        MeetingWarning::VadFailed {
            channel: Channel::Mic,
            phase: Phase::Final,
            ..
        }
    )));
    // The fallback found no quiet place to split the mic: one region, still transcribed.
    assert_eq!(outcome.mic.regions, 1);
    assert_eq!(outcome.far.regions, 1);
    assert!(outcome.superseded);
}

/// The longest run of exact zeros in `x`, in samples.
fn longest_zero_run(x: &[f32]) -> usize {
    let (mut best, mut run) = (0, 0);
    for &s in x {
        run = if s == 0.0 { run + 1 } else { 0 };
        best = best.max(run);
    }
    best
}

/// Carry-over 3: the final engine hears speech regions only. Eight seconds of digital silence
/// between two utterances, and a far end that is silent throughout, never reach it.
#[test]
fn only_vad_speech_regions_reach_the_final_engine() {
    let mut rig = RigBuilder::default().build();
    let mic = join(&[speech(1.0, -30.0, 41), silence(8.0), speech(1.0, -30.0, 42)]);
    rig.feed(&mic, &silence(10.0));
    let outcome = rig.finish().unwrap();

    let inputs = rig.engine.inputs.lock().unwrap().clone();
    assert_eq!(
        inputs.len(),
        2,
        "one call per utterance, none for the silent far end"
    );
    assert!(inputs.iter().all(|(c, _)| *c == Channel::Mic));
    let sent: usize = inputs.iter().map(|(_, a)| a.len()).sum();
    assert!(sent <= 3 * RATE + RATE / 5, "{sent} samples of 10 s sent");
    for (_, audio) in &inputs {
        let zeros = longest_zero_run(audio);
        assert!(
            zeros < 6_400,
            "{zeros} samples of silence reached the engine"
        );
    }
    assert_eq!(
        outcome.far,
        ChannelPass {
            channel: Channel::Far,
            regions: 0,
            empty_regions: 0,
            failed_regions: 0,
            word_count: 0,
            speech_ms: 0,
        }
    );
    assert_eq!(outcome.mic.regions, 2);
    assert!(outcome.mic.speech_ms < 3_200, "{:?}", outcome.mic);
}

/// Carry-over 3: an engine returns a segment for everything it is given, so only the VAD says
/// where speech was. A region the VAD called speech that comes back without words is reported.
#[test]
fn a_speech_region_the_engine_returns_empty_is_reported() {
    let answer: Answer = Arc::new(|channel, n, audio| {
        if channel == Channel::Mic && n == 2 {
            // What a real engine does: the window, with nothing in it.
            Ok(words("", audio.len()))
        } else {
            numbered()(channel, n, audio)
        }
    });
    let mut rig = RigBuilder {
        answer,
        ..RigBuilder::default()
    }
    .build();
    let mic = join(&[speech(1.0, -30.0, 41), silence(8.0), speech(1.0, -30.0, 42)]);
    rig.feed(&mic, &silence(10.0));
    let outcome = rig.finish().unwrap();
    let empty: Vec<_> = rig
        .warnings()
        .into_iter()
        .filter_map(|w| match w {
            MeetingWarning::EmptySpeechRegion {
                channel,
                start_ms,
                end_ms,
            } => Some((channel, start_ms, end_ms)),
            _ => None,
        })
        .collect();
    assert_eq!(empty.len(), 1, "{empty:?}");
    let (channel, start_ms, end_ms) = empty[0];
    assert_eq!(channel, Channel::Mic);
    assert!(
        start_ms.abs_diff(8_750) <= 100 && end_ms.abs_diff(10_000) <= 100,
        "{empty:?}"
    );
    assert_eq!(outcome.mic.empty_regions, 1);
}

/// Carry-over 3, live: a final the live engine wrote over silence is not saved.
#[test]
fn a_live_final_over_silence_is_not_saved() {
    let mut rig = RigBuilder::default().build();
    let record = rig.chain().record().clone();
    // At 5.02 s of the engine's stream: 5.00 s of the meeting, in the middle of the silence.
    rig.live
        .extra
        .lock()
        .unwrap()
        .push((Channel::Mic, 5_020, 6_020, "words from nowhere".into()));
    let mic = join(&[speech(1.0, -30.0, 41), silence(8.0), speech(1.0, -30.0, 42)]);
    rig.feed(&mic, &silence(10.0));
    let _ = rig.stop();
    assert!(
        rig.warnings()
            .contains(&MeetingWarning::FinalWithoutSpeech {
                channel: Channel::Mic,
                start_ms: 5_000,
                end_ms: 6_000
            })
    );
    let live = rig.store.segments(&record).unwrap();
    assert_eq!(live.len(), 2, "the two real utterances only: {live:?}");
    assert!(live.iter().all(|s| s.text.starts_with("live you")));
}

/// A clock whose wall time is set back after the meeting starts.
struct SetBack {
    calls: AtomicUsize,
}

impl Clock for SetBack {
    fn now_ns(&self) -> u64 {
        T0_NS
    }
    fn unix_ms(&self) -> i64 {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            T0_UNIX_MS
        } else {
            T0_UNIX_MS - 3_600_000
        }
    }
}

/// Carry-over 5: the wall clock can step back during a meeting; the end is never recorded before
/// the start, and the shell hears why.
#[test]
fn the_end_is_never_recorded_before_the_start() {
    let mut rig = RigBuilder {
        clock: Some(Arc::new(SetBack {
            calls: AtomicUsize::new(0),
        })),
        ..RigBuilder::default()
    }
    .build();
    let record = rig.chain().record().clone();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);
    let _ = rig.stop();
    let stored = rig.store.record(&record).unwrap().unwrap();
    assert_eq!(stored.started_at_unix_ms, T0_UNIX_MS);
    assert_eq!(stored.ended_at_unix_ms, Some(T0_UNIX_MS));
    assert!(rig.warnings().contains(&MeetingWarning::ClockWentBack));
}

/// Two far speakers, 3 s each, half a second apart: one region, cut between them.
fn two_far_speakers() -> (Vec<f32>, Vec<f32>) {
    let mic = join(&[speech(1.5, -30.0, 51), silence(7.0)]);
    let far = join(&[
        silence(2.0),
        speech(3.0, -30.0, 52),
        silence(0.5),
        speech(3.0, -30.0, 53),
    ]);
    (mic, far)
}

/// Rule 5: only the far end is diarized, and with two substantial clusters the far segments are
/// labelled, each engine call holding one speaker.
#[test]
fn the_far_end_alone_is_diarized_and_labelled() {
    // On the far end's speech end to end: the first region starts 250 ms before its speech.
    let diarizer = diarizer(&[("spk0", 0, 3_300), ("spk1", 3_750, 7_000)]);
    let mut rig = RigBuilder {
        diarizer: Some(diarizer.clone()),
        ..RigBuilder::default()
    }
    .build();
    let record = rig.chain().record().clone();
    let (mic, far) = two_far_speakers();
    rig.feed(&mic, &far);
    let outcome = rig.finish().unwrap();
    assert_eq!(diarizer.calls(), 1, "one far end, diarized once");
    assert_eq!(
        outcome.diarization,
        Some(Diarization {
            clusters: 2,
            substantial: 2,
            labelled: true,
            attributed: 2
        })
    );
    let finals = rig.store.segments(&record).unwrap();
    let far: Vec<_> = finals
        .iter()
        .filter(|s| s.channel == Channel::Far)
        .collect();
    assert_eq!(far.len(), 2, "cut where the speaker changed: {finals:#?}");
    assert_eq!(far[0].speaker, Some(SpeakerId("spk0".into())));
    assert_eq!(far[1].speaker, Some(SpeakerId("spk1".into())));
    assert!(
        finals
            .iter()
            .filter(|s| s.channel == Channel::Mic)
            .all(|s| s.speaker.is_none())
    );
    // Only far audio reached the engine as far calls; the mic was never cut by speaker.
    assert_eq!(outcome.far.regions, 2);
    assert_eq!(outcome.mic.regions, 1);
    assert_monotonic(&finals);
}

/// Rule 5: a second cluster holding under 2 % is a fragment, and one substantial cluster keeps no
/// labels at all: "them" already says who spoke.
#[test]
fn one_substantial_far_speaker_keeps_no_labels() {
    // 100 ms of 6.7 s: 1.5 %.
    let diarizer = diarizer(&[("spk0", 0, 6_600), ("spk1", 6_600, 6_700)]);
    let mut rig = RigBuilder {
        diarizer: Some(diarizer),
        ..RigBuilder::default()
    }
    .build();
    let record = rig.chain().record().clone();
    let (mic, far) = two_far_speakers();
    rig.feed(&mic, &far);
    let outcome = rig.finish().unwrap();
    assert_eq!(
        outcome.diarization,
        Some(Diarization {
            clusters: 2,
            substantial: 1,
            labelled: false,
            attributed: 0
        })
    );
    assert_eq!(outcome.far.regions, 1, "transcribed whole");
    let finals = rig.store.segments(&record).unwrap();
    assert!(finals.iter().all(|s| s.speaker.is_none()));
}

#[test]
fn a_diarizer_failure_keeps_the_far_end_unlabelled() {
    struct Broken;
    impl ink_core::Diarizer for Broken {
        fn info(&self) -> ink_core::EngineInfo {
            ink_core::EngineInfo {
                id: "broken".into(),
                jobs: vec![],
                licence: "MIT".into(),
            }
        }
        fn diarize(
            &self,
            _: &[f32],
            _: &CancelToken,
        ) -> Result<Vec<ink_core::SpeakerTurn>, EngineError> {
            Err(EngineError::Failed("scripted diarizer failure".into()))
        }
        fn open_stream(
            &self,
            _: ink_core::EventSink<ink_core::SpeakerTurn>,
        ) -> Result<Box<dyn ink_core::EngineStream>, EngineError> {
            Err(EngineError::Unsupported("live"))
        }
    }
    let mut rig = RigBuilder {
        diarizer: Some(Arc::new(Broken)),
        ..RigBuilder::default()
    }
    .build();
    let (mic, far) = two_far_speakers();
    rig.feed(&mic, &far);
    let outcome = rig.finish().unwrap();
    assert!(outcome.superseded);
    assert_eq!(outcome.diarization, None);
    assert!(
        rig.warnings()
            .iter()
            .any(|w| matches!(w, MeetingWarning::DiarizationFailed(_)))
    );
}

/// The supersede guard: a final pass in which one side collapses (here the mic, to a word per
/// region) is refused, and the live transcript stands at revision 1.
#[test]
fn a_final_pass_the_guard_refuses_keeps_revision_1() {
    let answer: Answer = Arc::new(|channel, n, audio| match channel {
        Channel::Mic => Ok(words("hm", audio.len())),
        Channel::Far => numbered()(channel, n, audio),
    });
    let mut rig = RigBuilder {
        answer,
        ..RigBuilder::default()
    }
    .build();
    let record = rig.chain().record().clone();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);
    let outcome = rig.finish().unwrap();
    assert!(!outcome.superseded);
    assert_eq!(outcome.revision, Some(1));
    assert!(
        rig.events()
            .contains(&MeetingEvent::KeptLive(KeptLive::Refused(
                StoreError::SuspiciousSupersede {
                    channel: Channel::Mic,
                    previous_words: 10,
                    new_words: 2
                }
            )))
    );
    you_and_them(
        &rig.store.segments(&record).unwrap(),
        "live you",
        "live them",
    );
}

/// A region that failed leaves the final pass incomplete, so it is not saved over the live one.
#[test]
fn a_failed_region_keeps_revision_1() {
    let answer: Answer = Arc::new(|channel, n, audio| {
        if channel == Channel::Mic && n == 2 {
            Err(EngineError::Failed("scripted engine failure".into()))
        } else {
            numbered()(channel, n, audio)
        }
    });
    let mut rig = RigBuilder {
        answer,
        ..RigBuilder::default()
    }
    .build();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);
    let outcome = rig.finish().unwrap();
    assert!(!outcome.superseded);
    assert_eq!(outcome.revision, Some(1));
    assert_eq!(outcome.mic.failed_regions, 1);
    assert!(
        rig.events()
            .contains(&MeetingEvent::KeptLive(KeptLive::Incomplete {
                failed_regions: 1
            }))
    );
    assert!(rig.warnings().iter().any(|w| matches!(
        w,
        MeetingWarning::FinalEngineFailed {
            channel: Channel::Mic,
            ..
        }
    )));
}

#[test]
fn a_cancelled_final_pass_saves_nothing() {
    let mut rig = RigBuilder::default().build();
    let record = rig.chain().record().clone();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);
    let ended = rig.stop();
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(
        ended.finalize(&rig.chunks, &cancel),
        Err(ink_pipeline::meeting::FinalizeError::Cancelled)
    ));
    assert_eq!(rig.store.record(&record).unwrap().unwrap().revision, 1);
    assert!(rig.engine.mock.calls().is_empty());
}

/// A stretch of device time lost upstream (the chunk writer sees the stamps jump): the live final
/// after it and the final pass's region both land where the audio was said.
#[test]
fn a_gap_in_capture_keeps_later_times_right() {
    let mut rig = RigBuilder::default().build();
    let record = rig.chain().record().clone();
    // A final about the first utterance that the engine only sends at the very end, long after the
    // gap: it is still placed by the anchors from before the gap.
    rig.live
        .extra
        .lock()
        .unwrap()
        .push((Channel::Mic, 520, 1_520, "late words".into()));
    let first = join(&[silence(0.5), speech(1.0, -30.0, 61), silence(0.5)]);
    rig.feed(&first, &silence(2.0));
    // One second of the mic never arrives.
    rig.lose(Channel::Mic, 16_000);
    rig.feed(&tone_at(1.5, 1.0, 3.0), &silence(3.0));
    let ended = rig.stop();
    let live = rig.store.segments(&record).unwrap();
    let tone = live
        .iter()
        .find(|s| s.start_ms > 2_000)
        .expect("the tone's final");
    // Fed at 3.5 s of the mic's samples, captured at 4.5 s.
    assert_eq!(tone.start_ms, 4_500, "{live:?}");
    let late = live.iter().find(|s| s.text == "late words").expect("saved");
    assert_eq!((late.start_ms, late.end_ms), (500, 1_500));
    ended.finalize(&rig.chunks, &CancelToken::new()).unwrap();
    let finals = rig.store.segments(&record).unwrap();
    let mic: Vec<_> = finals
        .iter()
        .filter(|s| s.channel == Channel::Mic)
        .collect();
    assert_eq!(mic.len(), 2, "{finals:?}");
    // The region's 250 ms lead, less the VAD's 32 ms window grid.
    assert!(mic[1].start_ms.abs_diff(4_250) <= 50, "{finals:?}");
}

/// A chunk the final pass cannot read (a torn header recovery could not repair) leaves a gap, is
/// reported, and what was recorded after it stays where it was said.
#[test]
fn an_unreadable_chunk_leaves_a_gap_and_later_audio_in_place() {
    let mut rig = RigBuilder::default().build();
    let record = rig.chain().record().clone();
    // Chunks are 10 s: speech in the first and the third, silence in the second.
    let mic = join(&[
        silence(1.0),
        speech(1.0, -30.0, 101),
        silence(23.0),
        speech(1.0, -30.0, 102),
        silence(4.0),
    ]);
    rig.feed(&mic, &silence(30.0));
    let ended = rig.stop();
    let torn = rig
        .chunks
        .chunks(Channel::Mic)
        .unwrap()
        .chunks
        .into_iter()
        .find(|c| c.index == 1)
        .expect("a second mic chunk");
    let mut bytes = std::fs::read(&torn.path).unwrap();
    bytes[..8].fill(0);
    std::fs::write(&torn.path, bytes).unwrap();

    ended.finalize(&rig.chunks, &CancelToken::new()).unwrap();
    assert!(rig.warnings().contains(&MeetingWarning::AudioUnreadable {
        channel: Channel::Mic,
        chunks: 1
    }));
    let finals = rig.store.segments(&record).unwrap();
    let mic: Vec<_> = finals
        .iter()
        .filter(|s| s.channel == Channel::Mic)
        .collect();
    assert_eq!(mic.len(), 2, "{finals:?}");
    assert!(mic[0].start_ms.abs_diff(750) <= 50, "{mic:?}");
    assert!(
        mic[1].start_ms.abs_diff(24_750) <= 50,
        "after the hole, in place: {mic:?}"
    );
}

/// Audio the capture ring dropped is reported as lost.
#[test]
fn ring_overruns_are_reported() {
    let mut rig = RigBuilder::default().build();
    let block = vec![0.01f32; BLOCK];
    // Three seconds pushed while the pump is stalled: the 2 s ring overflows.
    for _ in 0..300 {
        rig.push_block_unpumped(Channel::Mic, &block);
    }
    rig.pump();
    rig.push_block(Channel::Mic, &block);
    assert!(rig.warnings().iter().any(|w| matches!(
        w,
        MeetingWarning::AudioLost {
            channel: Channel::Mic,
            frames
        } if *frames > 0
    )));
}

// ---------------------------------------------------------------------------------------------
// Summary and commitments

const SUMMARY: &str = r#"{"headline": "The report is due Friday.", "body": "Status of the quarterly report.", "decisions": [], "actions": [{"text": "Send the quarterly report", "owner": "You", "due": "Friday", "line": 0, "quote": "send the quarterly report"}], "open_questions": [], "not_found": []}"#;
const JUDGE: &str = r#"{"class": "commitment", "confidence": 0.9, "task": "Send the quarterly report to the team", "due": "Friday", "quote": "I'll send the quarterly report"}"#;
const SAME: &str = r#"{"same": true, "keep": "B", "why": "one promise"}"#;

struct Scripted {
    calls: AtomicUsize,
}

impl ink_core::Llm for Scripted {
    fn info(&self) -> ink_core::LlmInfo {
        ink_core::LlmInfo {
            provider: "scripted".into(),
            model: "test".into(),
            endpoint: ink_core::Endpoint::InProcess,
        }
    }
    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<ink_core::LlmResponse, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = if request.system.contains("meeting record") {
            SUMMARY
        } else if request.system.contains("classify ONE sentence") {
            JUDGE
        } else {
            SAME
        };
        Ok(ink_core::LlmResponse { text: text.into() })
    }
}

fn promise() -> Answer {
    Arc::new(|channel, n, audio| match channel {
        Channel::Mic => Ok(words(
            "I'll send the quarterly report by Friday",
            audio.len(),
        )),
        Channel::Far => Ok(words(&format!("That works for us {n}"), audio.len())),
    })
}

/// After the supersede: the summary is saved and names an untitled meeting, and the summary's
/// action and the mic's promise are one commitment, "said twice".
#[test]
fn the_summary_and_commitments_follow_the_supersede() {
    let llm = Arc::new(Scripted {
        calls: AtomicUsize::new(0),
    });
    let mut rig = RigBuilder {
        answer: promise(),
        llm: Some(llm.clone()),
        title: None,
        ..RigBuilder::default()
    }
    .build();
    let record = rig.chain().record().clone();
    let mic = join(&[silence(0.5), speech(2.0, -30.0, 71), silence(4.0)]);
    let far = join(&[silence(3.0), speech(2.0, -30.0, 72), silence(1.5)]);
    rig.feed(&mic, &far);
    let outcome = rig.finish().unwrap();
    assert!(outcome.superseded);
    let summary = rig.store.summary(&record).unwrap().expect("saved");
    assert!(summary.text.starts_with("The report is due Friday."));
    assert_eq!(summary.model, "scripted/test");
    let stored = rig.store.record(&record).unwrap().unwrap();
    assert_eq!(stored.title.as_deref(), Some("The report is due Friday."));
    let commitments = rig.store.commitments(&record).unwrap();
    assert_eq!(commitments.len(), 2);
    assert_eq!(
        commitments
            .iter()
            .filter(|c| c.merged_into.is_some())
            .count(),
        1,
        "said twice, filed once"
    );
    assert!(
        commitments
            .iter()
            .all(|c| c.due.as_deref() == Some("Friday"))
    );
    let events = rig.events();
    assert!(events.contains(&MeetingEvent::Summarized { unverified: 0 }));
    assert!(events.contains(&MeetingEvent::Commitments {
        filed: 2,
        merged: 1
    }));
    assert!(llm.calls.load(Ordering::SeqCst) >= 3);
}

/// MUST from review: once revision 2 is committed, no read of the record can turn the pass into a
/// bare error. The record, its speaker names and its live transcript are read before anything is
/// written; each read that fails is reported, the pass goes on without it, and it finishes.
#[test]
fn record_reads_that_fail_are_reported_and_the_pass_still_finishes() {
    let flaky = Arc::new(FlakyStore::default());
    let llm = Arc::new(Scripted {
        calls: AtomicUsize::new(0),
    });
    let mut rig = RigBuilder {
        answer: promise(),
        llm: Some(llm),
        store: Some(flaky.clone()),
        title: None,
        ..RigBuilder::default()
    }
    .build();
    let record = rig.chain().record().clone();
    let mic = join(&[silence(0.5), speech(2.0, -30.0, 71), silence(4.0)]);
    let far = join(&[silence(3.0), speech(2.0, -30.0, 72), silence(1.5)]);
    rig.feed(&mic, &far);
    let ended = rig.stop();
    flaky.fail(&["record", "speaker_names", "segments"]);

    let outcome = ended.finalize(&rig.chunks, &CancelToken::new()).unwrap();
    assert!(outcome.superseded);
    assert_eq!(outcome.revision, Some(2));
    let events = rig.events();
    assert_eq!(
        events.last(),
        Some(&MeetingEvent::Finished { revision: Some(2) })
    );
    let failed = rig
        .warnings()
        .into_iter()
        .filter(|w| matches!(w, MeetingWarning::StoreFailed(StoreError::Backend(_))))
        .count();
    assert_eq!(failed, 3, "one per failed read: {:?}", rig.warnings());
    // The summary is written from the transcript in hand; the title is left alone, since whether
    // the record had one could not be read.
    assert!(events.contains(&MeetingEvent::Summarized { unverified: 0 }));
    let stored = flaky.inner.record(&record).unwrap().unwrap();
    assert_eq!(stored.revision, 2);
    assert_eq!(stored.title, None);
    assert!(flaky.inner.summary(&record).unwrap().is_some());
}

/// Cancelling once the final pass is saved stops the summary, is reported, and the pass still
/// finishes: revision 2 is not taken back, and the caller is not told nothing was saved.
#[test]
fn a_cancellation_after_the_supersede_still_finishes() {
    struct Cancels(CancelToken);
    impl ink_core::Llm for Cancels {
        fn info(&self) -> ink_core::LlmInfo {
            ink_core::LlmInfo {
                provider: "cancels".into(),
                model: "test".into(),
                endpoint: ink_core::Endpoint::InProcess,
            }
        }
        fn complete(
            &self,
            _: &LlmRequest,
            _: &CancelToken,
        ) -> Result<ink_core::LlmResponse, LlmError> {
            self.0.cancel();
            Err(LlmError::Cancelled)
        }
    }
    let cancel = CancelToken::new();
    let mut rig = RigBuilder {
        llm: Some(Arc::new(Cancels(cancel.clone()))),
        ..RigBuilder::default()
    }
    .build();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);
    let outcome = rig.stop().finalize(&rig.chunks, &cancel).unwrap();
    assert_eq!(outcome.revision, Some(2));
    assert!(
        rig.warnings()
            .contains(&MeetingWarning::SummaryFailed(LlmError::Cancelled))
    );
    assert!(
        !rig.warnings()
            .iter()
            .any(|w| matches!(w, MeetingWarning::CommitmentsFailed(_))),
        "nothing more is asked of the model"
    );
    assert_eq!(
        rig.events().last(),
        Some(&MeetingEvent::Finished { revision: Some(2) })
    );
}

#[test]
fn no_model_means_no_summary_and_says_so() {
    let mut rig = RigBuilder::default().build();
    let record = rig.chain().record().clone();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);
    rig.finish().unwrap();
    assert!(rig.warnings().contains(&MeetingWarning::SummaryUnavailable));
    assert_eq!(rig.store.summary(&record).unwrap(), None);
}

#[test]
fn a_silent_meeting_keeps_its_empty_live_transcript_and_asks_no_model() {
    let llm = Arc::new(Scripted {
        calls: AtomicUsize::new(0),
    });
    let mut rig = RigBuilder {
        llm: Some(llm.clone()),
        ..RigBuilder::default()
    }
    .build();
    rig.feed(&silence(3.0), &silence(3.0));
    let outcome = rig.finish().unwrap();
    assert!(!outcome.superseded);
    assert!(
        rig.events()
            .contains(&MeetingEvent::KeptLive(KeptLive::Refused(
                StoreError::EmptySupersede
            )))
    );
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0);
    assert!(
        rig.engine.mock.calls().is_empty(),
        "no speech, no engine call"
    );
}

/// The chain moves to its worker thread, and an ended meeting to wherever the final pass runs.
#[test]
fn the_chain_can_move_between_threads() {
    fn send<T: Send>() {}
    send::<ink_pipeline::meeting::MeetingChain>();
    send::<ink_pipeline::meeting::EndedMeeting>();
    send::<ink_pipeline::capture::SideCapture>();
}

/// A cancelled final pass can run again, and then saves.
#[test]
fn a_cancelled_final_pass_can_run_again() {
    let mut rig = RigBuilder::default().build();
    let (mic, far) = conversation();
    rig.feed(&mic, &far);
    let ended = rig.stop();
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(ended.finalize(&rig.chunks, &cancel).is_err());
    let outcome = ended.finalize(&rig.chunks, &CancelToken::new()).unwrap();
    assert_eq!(outcome.revision, Some(2));
}

/// Without a live engine, the meeting still records and the final pass makes the transcript.
#[test]
fn no_live_engine_still_gives_a_final_transcript() {
    let services =
        |engine: Arc<Final>, store: Arc<dyn Store>| ink_pipeline::meeting::MeetingServices {
            live: None,
            offline: engine,
            diarizer: None,
            store,
            clock: Arc::new(ink_core::mock::MockClock::new(T0_NS, T0_UNIX_MS)),
            llm: None,
        };
    let dir = TempDir::new("no-live");
    let chunks = ink_audio::ChunkStore::open(dir.path().join("r")).unwrap();
    let store: Arc<dyn Store> = Arc::new(MemStore::new());
    let engine = Final::new(numbered());
    let mut chain = ink_pipeline::meeting::MeetingChain::start(
        services(engine.clone(), store.clone()),
        Default::default(),
        energy_vad(),
        Arc::new(|_| {}),
        Default::default(),
    )
    .unwrap();
    let record = chain.record().clone();
    let (mic, _) = conversation();
    let mut writer = chunks
        .writer(Channel::Mic, StreamFormat::CANONICAL)
        .unwrap();
    for (k, block) in mic.chunks(BLOCK).enumerate() {
        let host = T0_NS + k as u64 * 10_000_000;
        writer
            .write(
                &ink_core::AudioBlock {
                    samples: block,
                    format: StreamFormat::CANONICAL,
                    host_time_ns: host,
                },
                0,
            )
            .unwrap();
        chain.push_audio(Channel::Mic, block, host, 0);
    }
    writer.finish().unwrap();
    assert!(store.segments(&record).unwrap().is_empty(), "nothing live");
    let outcome = chain.stop().finalize(&chunks, &CancelToken::new()).unwrap();
    assert_eq!(outcome.revision, Some(2));
    assert_eq!(store.segments(&record).unwrap().len(), 2);
}
