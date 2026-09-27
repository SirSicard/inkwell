//! Echo cancellation in the meeting chain, on generated scenes: a far end played through a seeded
//! room into the mic ([`echo_rig`]), fed through the whole chain on mocks ([`meeting_rig`]).
//!
//! The VAD here is the rig's energy VAD at −40 dBFS on the copies the passes lift: it hears the
//! near end (lifted to about −20 dBFS RMS) and not the residual echo AEC3's full output leaves
//! once converged (−72 dBFS before the lift). The real VAD is tested on real recordings, locally
//! (`real_echo.rs`).

mod echo_rig;
mod meeting_rig;

use std::sync::OnceLock;

use echo_rig::{Scene, Spec, echo_to_near_db};
use ink_core::{Channel, Segment, TimedText, Transcript};
use ink_pipeline::meeting::MeetingOutcome;
use ink_pipeline::meeting::events::MeetingEvent;
use meeting_rig::*;

const VAD_DB: f32 = -40.0;

fn echo_vad() -> ink_pipeline::speech::VadSource {
    vad_source(|_| Box::new(EnergyVad(VAD_DB)))
}

/// One scene through the chain, and what came out.
struct Run {
    scene: Scene,
    outcome: MeetingOutcome,
    events: Vec<MeetingEvent>,
    segments: Vec<Segment>,
    /// The lines the store keeps as removed.
    removed: Vec<Segment>,
    /// What the final-pass engine was given for the mic, in call order.
    mic_inputs: Vec<Vec<f32>>,
}

fn run(scene: Scene, answer: Answer) -> Run {
    let mut rig = RigBuilder {
        vad: echo_vad(),
        answer,
        // The final pass alone: nothing live for it to be checked against.
        no_live_engine: true,
        ..Default::default()
    }
    .build();
    let record = rig.chain().record().clone();
    rig.feed(&scene.mic, &scene.far);
    let outcome = rig.finish().expect("the final pass");
    let segments = rig.store.segments(&record).unwrap();
    let removed = rig.store.removed(&record).unwrap();
    let mic_inputs = rig
        .engine
        .inputs
        .lock()
        .unwrap()
        .iter()
        .filter(|(c, _)| *c == Channel::Mic)
        .map(|(_, a)| a.clone())
        .collect();
    Run {
        scene,
        outcome,
        events: rig.events(),
        segments,
        removed,
        mic_inputs,
    }
}

fn speakers() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| run(echo_rig::protocol(Spec::speakers()), numbered()))
}

fn earbuds() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| run(echo_rig::protocol(Spec::earbuds()), numbered()))
}

fn mic_segments(run: &Run) -> Vec<&Segment> {
    run.segments
        .iter()
        .filter(|s| s.channel == Channel::Mic)
        .collect()
}

/// Where `input` (a region's audio, levelled) starts in `reference`: the offset near `approx`
/// with the best correlation.
fn locate(input: &[f32], reference: &[f32], approx: usize) -> usize {
    let n = input.len().min(16_000);
    let corr = |at: usize| -> f64 {
        input[..n]
            .iter()
            .zip(&reference[at..at + n])
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum()
    };
    (approx.saturating_sub(32)..=approx + 32)
        .max_by(|a, b| corr(*a).total_cmp(&corr(*b)))
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// The final pass

#[test]
fn a_found_path_makes_the_you_pass_read_the_linear_output() {
    let run = speakers();
    let echo = &run.outcome.echo;
    let path = echo.path.expect("the speakers scene has an echo path");
    assert!((path.delay_ms - 75.0).abs() <= 2.0, "{echo:?}");
    assert!((path.drift_ppm - 50.0).abs() <= 5.0, "{echo:?}");
    assert!(echo.cancelled, "{echo:?}");

    // A region in the double talk (27–40 s): what the engine got holds the near end with the
    // echo well under it, where the raw mic holds both equally.
    let mics = mic_segments(run);
    let (k, seg) = mics
        .iter()
        .enumerate()
        .find(|(_, s)| s.start_ms <= 31_000 && s.end_ms >= 39_000)
        .unwrap_or_else(|| panic!("no mic region over the double talk: {mics:?}"));
    let input = &run.mic_inputs[k];
    let at = locate(input, &run.scene.mic, seg.start_ms as usize * 16);
    let span = 31 * 16_000..(39 * 16_000).min(at + input.len());
    let heard = echo_to_near_db(
        &input[span.start - at..span.end - at],
        &run.scene.near[span.clone()],
        &run.scene.echo[span.clone()],
    );
    let raw = echo_to_near_db(
        &run.scene.mic[span.clone()],
        &run.scene.near[span.clone()],
        &run.scene.echo[span],
    );
    assert!(
        raw.abs() < 1.0,
        "the raw mic holds both equally: {raw:.1} dB"
    );
    assert!(
        heard < raw - 3.0,
        "the engine heard the echo at {heard:.1} dB against the near end (raw mic {raw:.1})"
    );
}

#[test]
fn the_final_pass_fits_the_path_over_the_whole_recording() {
    // Every 2 s window of the 45 s recording, a hop of 1 s: the final pass's own search, not the
    // live one (which stops once it has a path).
    let echo = &speakers().outcome.echo;
    assert!(echo.windows >= 43, "{echo:?}");
}

#[test]
fn erle_is_reported_for_the_first_10_s_and_after() {
    let echo = &speakers().outcome.echo;
    let first = echo.erle_first_db.expect("the first 10 s of far-end audio");
    let after = echo
        .erle_db
        .expect("far-end-only audio after the first 10 s");
    eprintln!(
        "ERLE first 10 s {first:.1} dB, after {after:.1} dB, linear stage {:?} dB",
        echo.linear_erle_db
    );
    assert!(after >= 20.0, "{echo:?}");
}

#[test]
fn the_echo_pass_is_an_event_before_the_supersede() {
    let events = &speakers().events;
    let at = |f: &dyn Fn(&MeetingEvent) -> bool| events.iter().position(f);
    let pass = at(&|e| matches!(e, MeetingEvent::EchoPass(_))).expect("an EchoPass event");
    let superseded = at(&|e| matches!(e, MeetingEvent::Superseded { .. })).unwrap();
    assert!(pass < superseded);
}

#[test]
fn with_no_echo_path_the_mic_is_transcribed_as_captured() {
    let run = earbuds();
    let echo = &run.outcome.echo;
    assert!(echo.path.is_none(), "{echo:?}");
    assert!(!echo.cancelled);
    assert_eq!(echo.erle_db, None);
    // What the engine got is exactly what the plain pass over the raw mic gives.
    let mut pass = ink_pipeline::speech::SpeechPass::new(
        ink_pipeline::gain_stage::Vad::Installed(Box::new(EnergyVad(VAD_DB))),
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let mut regions = Vec::new();
    pass.push(&run.scene.mic, &mut regions).unwrap();
    pass.finish(&mut regions);
    let plain: Vec<&Vec<f32>> = regions.iter().map(|r| &r.audio).collect();
    assert_eq!(run.mic_inputs.iter().collect::<Vec<_>>(), plain);
}

#[test]
fn a_mic_that_hears_only_echo_gives_no_you_words() {
    // AEC3 adapted on the recording's start first, so its first 10 s leak nothing either.
    let run = run(echo_rig::echo_only(30.0, Spec::speakers()), numbered());
    assert!(run.outcome.echo.cancelled, "{:?}", run.outcome.echo);
    assert_eq!(run.outcome.mic.regions, 0, "{:?}", run.outcome.mic);
    assert!(mic_segments(&run).is_empty());
    assert!(run.outcome.far.word_count > 0);
}

/// The far end says two lines; the user reads the first back over their own speech, and the
/// second reaches the mic only as echo, inside a region of the user's talk. The engine (scripted)
/// writes the echo into the "you" transcript there.
fn read_back() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(|| {
        let scene = echo_rig::scripted(
            30.0,
            // Talk to find the path by, then the two lines.
            &[
                (1.0, 3.5),
                (5.0, 3.5),
                (9.0, 3.0),
                (16.2, 1.7),
                (19.55, 1.0),
            ],
            // The user: 17.0–19.2 and 21.0–23.0, one region (a 1.8 s pause).
            &[(17.0, 2.2), (21.0, 2.0)],
            Spec::speakers(),
        );
        let answer: Answer = std::sync::Arc::new(|channel, _, audio| {
            let secs = audio.len() as f64 / 16_000.0;
            let seg = |a: u64, b: u64, text: &str| TimedText {
                start_ms: a,
                end_ms: b,
                text: text.into(),
            };
            Ok(Transcript {
                segments: match channel {
                    // The far end's region over its two lines: 15.974–20.826 s (the scene and
                    // the pipeline are deterministic).
                    Channel::Far if (4.5..6.0).contains(&secs) => vec![
                        seg(226, 1_926, "please send the report today"),
                        seg(3_576, 4_576, "the budget is due on friday"),
                    ],
                    // The user's region, 16.806–23.194 s: their read-back, then the echo
                    // (19.7–20.5 s, 250 ms clear of their speech on both sides), then their reply.
                    Channel::Mic if (6.0..7.0).contains(&secs) => vec![
                        seg(194, 2_394, "please send the report today"),
                        seg(2_894, 3_694, "the budget is due on friday"),
                        seg(4_194, 6_194, "sounds good thanks"),
                    ],
                    _ => vec![seg(0, (secs * 1000.0) as u64, "other words here")],
                },
            })
        });
        run(scene, answer)
    })
}

#[test]
fn a_you_line_that_is_the_far_end_s_echo_is_removed_and_handed_back() {
    let run = read_back();
    let removed = &run.outcome.removed_as_echo;
    assert_eq!(removed.len(), 1, "{removed:?}");
    let line = &removed[0];
    assert_eq!(line.text.as_str(), "the budget is due on friday");
    assert!(
        (19_600..=19_800).contains(&line.start_ms),
        "{}",
        line.start_ms
    );
    assert_eq!(line.far.len(), 1, "matched one far-end line: {line:?}");
    assert!(
        (19_500..=19_600).contains(&line.far[0].start_ms),
        "{line:?}"
    );
    // Gone from the saved transcript; the read-back and the reply stay.
    let you: Vec<&str> = mic_segments(run).iter().map(|s| s.text.as_str()).collect();
    assert!(!you.contains(&"the budget is due on friday"), "{you:?}");
    assert!(you.contains(&"please send the report today"), "{you:?}");
    assert!(you.contains(&"sounds good thanks"), "{you:?}");
    let echo = &run.outcome.echo;
    assert_eq!(echo.removed, 1);
    assert_eq!(echo.kept_near_speech, 1, "the read-back: {echo:?}");
    // The shell hears it too, with the text only as `Spoken`.
    let event = run
        .events
        .iter()
        .find_map(|e| match e {
            MeetingEvent::RemovedAsEcho(lines) => Some(lines),
            _ => None,
        })
        .expect("a RemovedAsEcho event");
    assert_eq!(event, removed);
    assert!(!format!("{event:?}").contains("budget"));
}

#[test]
fn lines_removed_as_echo_are_stored_with_the_record() {
    let run = read_back();
    let line = &run.outcome.removed_as_echo[0];
    assert_eq!(
        run.removed,
        vec![Segment {
            channel: Channel::Mic,
            start_ms: line.start_ms,
            end_ms: line.end_ms,
            text: "the budget is due on friday".into(),
            speaker: None,
        }]
    );
    // With nothing removed, the store keeps nothing.
    assert!(speakers().removed.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Live

use ink_pipeline::meeting::events::{EchoFailure, EchoSearch, EchoState, MeetingWarning};

/// A scene through the chain with the rig's live engine on, fed in `parts`: each a range of the
/// scene in seconds and what to feed there (both sides, or the far end only), with an action
/// before each part.
struct LiveRun {
    scene: Scene,
    events: Vec<MeetingEvent>,
    /// What the live engine received on the mic, from the start of its stream.
    received: Vec<f32>,
}

#[derive(Clone, Copy)]
enum Part {
    Both(f64, f64),
    FarOnly(f64, f64),
    SwitchDevice,
}

fn live_run(scene: Scene, vad: ink_pipeline::speech::VadSource, parts: &[Part]) -> LiveRun {
    live_run_with(scene, vad, parts, &[])
}

/// [`live_run`], with extra "you" finals the live engine reports at the end, at (start_ms,
/// end_ms) of its stream.
fn live_run_with(
    scene: Scene,
    vad: ink_pipeline::speech::VadSource,
    parts: &[Part],
    extra: &[(u64, u64, &str)],
) -> LiveRun {
    let mut rig = RigBuilder {
        vad,
        ..Default::default()
    }
    .build();
    rig.live.extra.lock().unwrap().extend(
        extra
            .iter()
            .map(|&(a, b, t)| (Channel::Mic, a, b, t.to_string())),
    );
    let at = |s: f64| ((s * 16_000.0) as usize).min(scene.mic.len());
    for part in parts {
        match *part {
            Part::Both(a, b) => rig.feed(&scene.mic[at(a)..at(b)], &scene.far[at(a)..at(b)]),
            Part::FarOnly(a, b) => rig.feed_side(Channel::Far, &scene.far[at(a)..at(b)]),
            Part::SwitchDevice => rig.chain().set_routing(Default::default()),
        }
    }
    rig.stop();
    LiveRun {
        received: rig.live.received(Channel::Mic),
        events: rig.events(),
        scene,
    }
}

fn echo_states(events: &[MeetingEvent]) -> Vec<EchoState> {
    events
        .iter()
        .filter_map(|e| match e {
            MeetingEvent::Echo(s) => Some(*s),
            _ => None,
        })
        .collect()
}

/// The live engine's own finals: the rig's engine hears one burst from start to end here (the
/// room's floor, lifted by the AGC, never falls under its threshold), so these are scripted: two
/// over the far end alone once cancelling (12–14 s, 41–43 s), one over the near end alone
/// (23–25 s) and one over double talk (33–35 s).
const LIVE_FINALS: [(u64, u64, &str); 4] = [
    (12_000, 14_000, "echo words one"),
    (41_000, 43_000, "echo words two"),
    (23_000, 25_000, "near words alone"),
    (33_000, 35_000, "near words over the far end"),
];

fn live_speakers() -> &'static LiveRun {
    static RUN: OnceLock<LiveRun> = OnceLock::new();
    RUN.get_or_init(|| {
        live_run_with(
            echo_rig::protocol(Spec::speakers()),
            echo_vad(),
            &[Part::Both(0.0, 45.0)],
            &LIVE_FINALS,
        )
    })
}

/// Where cancellation began in a live run.
fn cancelling_from(states: &[EchoState]) -> u64 {
    states
        .iter()
        .find_map(|s| match s {
            EchoState::Cancelling { from_ms, .. } => Some(*from_ms),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no Cancelling state: {states:?}"))
}

#[test]
fn live_the_mic_is_unprotected_until_the_search_finds_the_path() {
    let states = echo_states(&live_speakers().events);
    eprintln!("{states:?}");
    assert_eq!(
        states[0],
        EchoState::Searching {
            since_ms: 0,
            why: EchoSearch::Start
        }
    );
    let EchoState::Cancelling {
        from_ms,
        unprotected_ms,
        stable_from_ms,
        delay_ms,
        drift_ppm,
    } = states[1]
    else {
        panic!("{states:?}");
    };
    // At least 10 s of the mic before the first look, and every second of it unprotected.
    assert!(from_ms >= 10_000, "{from_ms}");
    assert_eq!(unprotected_ms, from_ms);
    assert!(
        stable_from_ms.is_some_and(|s| s <= from_ms),
        "{stable_from_ms:?}"
    );
    assert!((delay_ms - 75.0).abs() <= 2.0, "{delay_ms}");
    // Under 10 s of windows the fit is delay only; a later one with the drift restarts
    // cancellation along it once the two part by 2 ms, protected throughout.
    assert!(
        drift_ppm == 0.0 || (drift_ppm - 50.0).abs() <= 5.0,
        "{drift_ppm}"
    );
    for s in &states[2..] {
        if let EchoState::Cancelling {
            unprotected_ms,
            drift_ppm,
            ..
        } = s
        {
            assert_eq!(*unprotected_ms, 0, "{states:?}");
            assert!((drift_ppm - 50.0).abs() <= 5.0, "{states:?}");
        }
    }
}

#[test]
fn live_the_mic_channel_hears_the_cancelled_output_once_found() {
    let run = live_speakers();
    let from = cancelling_from(&echo_states(&run.events)) as usize * 16;
    // The live engine hears the AGC's output, 20 ms (320 samples) behind its input. In the double
    // talk (31–39 s) it holds the echo well under the near end; before cancellation began (the far
    // end alone, 2–9 s), the echo as captured.
    let latency = 320;
    let span = 31 * 16_000..39 * 16_000;
    assert!(from < span.start, "{from}");
    let heard = echo_to_near_db(
        &run.received[span.start + latency..span.end + latency],
        &run.scene.near[span.clone()],
        &run.scene.echo[span.clone()],
    );
    assert!(
        heard < -3.0,
        "the live engine heard the echo at {heard:.1} dB against the near end"
    );
    let before = 2 * 16_000..9 * 16_000;
    let corr = |x: &[f32], y: &[f32]| -> f64 {
        let dot = |a: &[f32], b: &[f32]| -> f64 {
            a.iter()
                .zip(b)
                .map(|(p, q)| f64::from(*p) * f64::from(*q))
                .sum()
        };
        dot(x, y) / (dot(x, x) * dot(y, y)).sqrt()
    };
    let raw = corr(
        &run.received[before.start + latency..before.end + latency],
        &run.scene.echo[before],
    );
    assert!(
        raw > 0.95,
        "unprotected, the live engine heard the echo itself: {raw:.3}"
    );
}

#[test]
fn live_a_you_final_over_echo_only_audio_is_not_saved() {
    let run = live_speakers();
    let dropped: Vec<(u64, u64)> = run
        .events
        .iter()
        .filter_map(|e| match e {
            MeetingEvent::Warning(MeetingWarning::EchoOnlyFinal { start_ms, end_ms }) => {
                Some((*start_ms, *end_ms))
            }
            _ => None,
        })
        .collect();
    let saved: Vec<(u64, u64)> = run
        .events
        .iter()
        .filter_map(|e| match e {
            MeetingEvent::Final {
                channel: Channel::Mic,
                start_ms,
                end_ms,
                ..
            } => Some((*start_ms, *end_ms)),
            _ => None,
        })
        .collect();
    // Placed 20 ms earlier than the stream says: the AGC's latency.
    let near = |ms: u64| ms.saturating_sub(40)..=ms;
    let has = |list: &[(u64, u64)], start: u64| list.iter().any(|(a, _)| near(start).contains(a));
    for (start, _, _) in &LIVE_FINALS[..2] {
        assert!(
            has(&dropped, *start),
            "the echo at {start} ms: dropped {dropped:?}, saved {saved:?}"
        );
        assert!(!has(&saved, *start), "saved {saved:?}");
    }
    for (start, _, _) in &LIVE_FINALS[2..] {
        assert!(
            has(&saved, *start),
            "the user at {start} ms: saved {saved:?}"
        );
    }
    assert_eq!(dropped.len(), 2, "{dropped:?}");
}

#[test]
fn live_with_earbuds_the_search_never_finds_a_path() {
    let states = echo_states(&earbuds().events);
    assert_eq!(
        states,
        vec![EchoState::Searching {
            since_ms: 0,
            why: EchoSearch::Start
        }]
    );
}

#[test]
fn live_a_device_switch_restarts_the_search_visibly() {
    let run = live_run(
        echo_rig::protocol(Spec::speakers()),
        echo_vad(),
        &[
            Part::Both(0.0, 25.0),
            Part::SwitchDevice,
            Part::Both(25.0, 45.0),
        ],
    );
    let states = echo_states(&run.events);
    eprintln!("{states:?}");
    let first = cancelling_from(&states);
    assert!(first < 25_000, "{states:?}");
    let switch = states
        .iter()
        .position(|s| {
            matches!(
                s,
                EchoState::Searching {
                    why: EchoSearch::DeviceSwitch,
                    ..
                }
            )
        })
        .unwrap_or_else(|| panic!("no DeviceSwitch search: {states:?}"));
    let EchoState::Searching { since_ms, .. } = states[switch] else {
        unreachable!()
    };
    assert!((24_990..=25_010).contains(&since_ms), "{since_ms}");
    // A path again later is found from the new search's own audio.
    if let Some(EchoState::Cancelling {
        from_ms,
        unprotected_ms,
        ..
    }) = states.get(switch + 1)
    {
        assert!(*from_ms >= 35_000, "{states:?}");
        assert_eq!(*unprotected_ms, from_ms - since_ms);
    }
}

#[test]
fn live_a_side_that_runs_10_s_ahead_is_a_visible_failure() {
    // Cancelling by 20 s; then the mic stops delivering while the far end plays on for 15 s.
    let run = live_run(
        echo_rig::protocol(Spec::speakers()),
        echo_vad(),
        &[Part::Both(0.0, 20.0), Part::FarOnly(20.0, 35.0)],
    );
    let states = echo_states(&run.events);
    eprintln!("{states:?}");
    let failed = states
        .iter()
        .position(|s| {
            *s == EchoState::Failed(EchoFailure::Backlog {
                ahead: Channel::Far,
            })
        })
        .unwrap_or_else(|| panic!("no Backlog failure: {states:?}"));
    assert!(matches!(
        states[failed + 1],
        EchoState::Searching {
            why: EchoSearch::AfterFailure,
            ..
        }
    ));
}

#[test]
fn live_cancellation_that_stops_working_is_degraded_visibly() {
    // Cancelling from 10 s; from 15 s the mic hears the echo of audio the tap does not carry.
    let scene = echo_rig::untapped(55.0, 15.0, Spec::speakers());
    let run = live_run(scene, echo_vad(), &[Part::Both(0.0, 55.0)]);
    let states = echo_states(&run.events);
    eprintln!("{states:?}");
    let degraded = states
        .iter()
        .find_map(|s| match s {
            EchoState::Degraded { erle_db } => Some(*erle_db),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no Degraded state: {states:?}"));
    assert!(degraded < 10.0, "{degraded}");
}

#[test]
fn live_a_gate_vad_that_cannot_load_is_reported() {
    // VAD instances: the live mic's and far end's AGCs, then the gate's once a path is found.
    let made = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let vad = ink_pipeline::speech::VadSource::Installed(std::sync::Arc::new(move || {
        match made.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
            2 => Err(ink_core::EngineError::ModelMissing("gate vad".into())),
            _ => Ok(Box::new(EnergyVad(VAD_DB)) as Box<dyn ink_audio::SpeechProbability>),
        }
    }));
    let run = live_run_with(
        echo_rig::protocol(Spec::speakers()),
        vad,
        &[Part::Both(0.0, 45.0)],
        &LIVE_FINALS,
    );
    let warned = run.events.iter().any(|e| {
        matches!(
            e,
            MeetingEvent::Warning(MeetingWarning::EchoGateVadFailed(
                ink_core::EngineError::ModelMissing(_)
            ))
        )
    });
    assert!(warned, "{:?}", echo_states(&run.events));
    // Without evidence the gate drops nothing, the echo over the far end alone included.
    assert!(!run.events.iter().any(|e| matches!(
        e,
        MeetingEvent::Warning(MeetingWarning::EchoOnlyFinal { .. })
    )));
    let saved = |start: u64| {
        run.events.iter().any(|e| {
            matches!(e, MeetingEvent::Final { channel: Channel::Mic, start_ms, .. }
                if (start - 40..=start).contains(start_ms))
        })
    };
    assert!(LIVE_FINALS.iter().all(|(start, _, _)| saved(*start)));
}
