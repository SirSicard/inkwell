//! The dictation chain end to end, on mocks: the step's verification and one named test per
//! carry-over requirement.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Rig, VadKind, rms_dbfs, speech_48k, transcript};
use ink_audio::gain::{TARGET_PEAK, robust_peak, to_dbfs};
use ink_audio::synth;
use ink_core::mock::MockLlm;
use ink_core::{
    CancelToken, Endpoint, FocusInfo, InsertOutcome, Llm, LlmError, LlmInfo, LlmRequest,
    LlmResponse,
};
use ink_pipeline::events::{
    DictationEvent, Discard, TakeFailure, VadUnavailable, VoiceDetection, Warning,
};
use ink_pipeline::gain_stage::Vad;
use ink_pipeline::voicecommand::CommandAction;

/// How far the engine's input may sit from the gain target, in dB.
const TARGET_TOLERANCE_DB: f32 = 0.5;

fn db_from_target(audio: &[f32]) -> f32 {
    to_dbfs(robust_peak(audio)) - to_dbfs(TARGET_PEAK)
}

fn has(events: &[DictationEvent], wanted: impl Fn(&DictationEvent) -> bool) -> bool {
    events.iter().any(wanted)
}

fn inserted_text(events: &[DictationEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            DictationEvent::Inserted { text, .. } => Some(text.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The step's verification
// ---------------------------------------------------------------------------------------------

#[test]
fn the_dictation_fixture_gives_the_expected_text() {
    let rig = Rig::builder().build();
    rig.dictate_fixture("please check the notes before the review", 2.5, -30.0);

    // Stage 6 (style) and the trailing-space setting shape what lands in the app.
    assert_eq!(
        rig.inserted(),
        vec!["Please check the notes before the review. ".to_owned()]
    );
    let events = rig.events();
    assert_eq!(
        inserted_text(&events),
        vec!["Please check the notes before the review.".to_owned()]
    );
    assert!(has(&events, |e| matches!(e, DictationEvent::Started)));
    assert!(has(&events, |e| matches!(e, DictationEvent::Stopped)));
    assert!(
        !has(&events, |e| matches!(
            e,
            DictationEvent::Warning(_) | DictationEvent::Failed(_)
        )),
        "{events:?}"
    );

    // Stage 10: the record holds the text, on the mic channel.
    let records = rig.dictation_records();
    assert_eq!(records.len(), 1);
    let segments = rig.store.segments(&records[0].id).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(
        segments[0].text,
        "Please check the notes before the review."
    );
    assert!(records[0].ended_at_unix_ms.is_some());
}

#[test]
fn a_minus_75_dbfs_dictation_reaches_the_engine_at_the_gain_target() {
    let rig = Rig::builder().build();
    let speech = rig.dictate_fixture("quiet words", 3.0, -75.0);
    assert!(
        (rms_dbfs(&speech) + 75.0).abs() < 0.01,
        "the fixture is −75 dBFS RMS"
    );

    assert_eq!(rig.inserted(), vec!["Quiet words. ".to_owned()]);
    // The mock engine's own record of its input level...
    let calls = rig.engine.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].rms_dbfs > -30.0,
        "reached the engine at {} dBFS RMS",
        calls[0].rms_dbfs
    );
    assert!(
        calls[0].peak >= TARGET_PEAK * 0.99,
        "peak {}",
        calls[0].peak
    );
    // ...and the robust peak the gain stage keys on, measured on the exact input.
    let input = &rig.engine_inputs()[0];
    let off = db_from_target(input);
    assert!(
        off.abs() < TARGET_TOLERANCE_DB,
        "robust peak {off:+.2} dB from the target"
    );
}

// ---------------------------------------------------------------------------------------------
// Carry-over 1: with a VAD installed, a take with no speech reaches no engine
// ---------------------------------------------------------------------------------------------

#[test]
fn a_noise_only_take_reaches_no_engine() {
    let rig = Rig::builder().vad(VadKind::Never).build();
    let noise = common::upsample3(&synth::rumble(2.0, -60.0, 120.0, synth::Slope::Steep, 3));
    rig.dictate(&noise);

    assert!(rig.engine.calls().is_empty(), "no engine saw the take");
    assert!(rig.inserted().is_empty());
    assert!(rig.dictation_records().is_empty());
    let events = rig.events();
    assert!(
        has(&events, |e| *e
            == DictationEvent::Discarded(Discard::NoSpeech)),
        "{events:?}"
    );
}

#[test]
fn a_take_whose_only_speech_is_too_short_is_discarded_as_speech_too_short() {
    // A quick "no": the VAD hears 160 ms (5 windows) of speech, one window short of the level
    // minimum. The take reaches no engine and is reported as too short, which the app can say,
    // not as no speech (the VAD did hear some) or as a hold that was too short (it was long enough).
    let rig = Rig::builder().vad(VadKind::Scripted(&[(20, 5)])).build();
    let mut take = common::upsample3(&synth::noise(1.2, -70.0, 21));
    let word = speech_48k(0.16, -40.0, 22);
    for (t, w) in take[20 * 512 * 3..].iter_mut().zip(&word) {
        *t += w;
    }
    rig.dictate(&take);

    assert!(rig.engine.calls().is_empty(), "no engine saw the take");
    assert!(rig.inserted().is_empty());
    let events = rig.events();
    assert!(
        has(&events, |e| *e
            == DictationEvent::Discarded(Discard::SpeechTooShort)),
        "{events:?}"
    );
    assert!(
        !has(&events, |e| matches!(
            e,
            DictationEvent::Discarded(Discard::NoSpeech | Discard::TooShort { .. })
        )),
        "{events:?}"
    );
}

#[test]
fn knock_and_rumble_false_positives_set_no_gain_and_reach_no_engine() {
    // The real VAD's measured false positives, replayed on the audio they came from: steep 500 Hz
    // rumble with one 96 ms segment, and knocks with segments of 128 and 160 ms.
    /// VAD windows heard as speech: (first window, windows).
    type Runs = &'static [(usize, usize)];
    let cases: [(&str, Vec<f32>, Runs); 2] = [
        (
            "rumble",
            synth::rumble(3.0, -70.0, 500.0, synth::Slope::Steep, 23),
            &[(40, 3)],
        ),
        ("knocks", synth::knocks(3.0, -60.0, 24), &[(20, 4), (60, 5)]),
    ];
    for (what, audio, runs) in cases {
        let rig = Rig::builder().vad(VadKind::Scripted(runs)).build();
        rig.dictate(&common::upsample3(&audio));
        assert!(
            rig.engine.calls().is_empty(),
            "{what}: an engine saw the take"
        );
        assert!(rig.inserted().is_empty(), "{what}");
        let events = rig.events();
        assert!(
            has(&events, |e| *e
                == DictationEvent::Discarded(Discard::SpeechTooShort)),
            "{what}: {events:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Carry-over 2: no VAD installed → the fallback, and a visible state
// ---------------------------------------------------------------------------------------------

#[test]
fn without_a_vad_the_fallback_levels_the_take_and_says_detection_is_unavailable() {
    let rig = Rig::builder()
        .vad(VadKind::Unavailable(VadUnavailable::Downloading))
        .build();
    let events = rig.events();
    assert_eq!(
        events.first(),
        Some(&DictationEvent::VoiceDetection(
            VoiceDetection::Unavailable(VadUnavailable::Downloading)
        )),
        "the state is announced before any take"
    );

    rig.dictate_fixture("still heard", 2.0, -75.0);
    assert_eq!(rig.inserted(), vec!["Still heard. ".to_owned()]);
    let off = db_from_target(&rig.engine_inputs()[0]);
    assert!(
        off.abs() < TARGET_TOLERANCE_DB,
        "fallback levelled to {off:+.2} dB"
    );

    // Installing the VAD changes the state, once.
    rig.clear_events();
    rig.chain.borrow_mut().set_vad(VadKind::Energy.build());
    rig.chain.borrow_mut().set_vad(VadKind::Energy.build());
    assert_eq!(
        rig.events(),
        vec![DictationEvent::VoiceDetection(VoiceDetection::Available)]
    );
    rig.chain
        .borrow_mut()
        .set_vad(Vad::Unavailable(VadUnavailable::ModelMissing));
    assert_eq!(
        rig.events().last(),
        Some(&DictationEvent::VoiceDetection(
            VoiceDetection::Unavailable(VadUnavailable::ModelMissing)
        ))
    );
}

// ---------------------------------------------------------------------------------------------
// Carry-over 3: a VAD error is reported and the take goes through the fallback
// ---------------------------------------------------------------------------------------------

#[test]
fn a_failing_vad_is_reported_and_the_take_is_levelled_by_the_fallback() {
    let rig = Rig::builder().vad(VadKind::Failing).build();
    rig.dictate_fixture("heard anyway", 2.0, -75.0);

    let events = rig.events();
    assert!(
        has(&events, |e| matches!(
            e,
            DictationEvent::Warning(Warning::VadFailed(_))
        )),
        "{events:?}"
    );
    assert_eq!(rig.inserted(), vec!["Heard anyway. ".to_owned()]);
    // Levelled, not passed on raw: the fallback lifted it to the target.
    let off = db_from_target(&rig.engine_inputs()[0]);
    assert!(off.abs() < TARGET_TOLERANCE_DB, "levelled to {off:+.2} dB");
}

// ---------------------------------------------------------------------------------------------
// Carry-over 4: a minimum hold before a press is a dictation
// ---------------------------------------------------------------------------------------------

#[test]
fn a_modifier_tapped_in_a_shortcut_starts_nothing() {
    let rig = Rig::builder().build();
    rig.silence(0.5);
    rig.press();
    rig.feed(&speech_48k(0.12, -30.0, 5)); // 120 ms: a right-hand Command used for a shortcut
    rig.release();
    rig.silence(0.6);

    let events = rig.events();
    assert!(
        has(&events, |e| *e == DictationEvent::ShortPressIgnored),
        "{events:?}"
    );
    assert!(
        !has(&events, |e| *e == DictationEvent::Started),
        "nothing was shown"
    );
    assert!(rig.engine.calls().is_empty());
    assert!(!rig.chain.borrow().is_recording());

    // The next real dictation is unaffected.
    rig.dictate_fixture("after the shortcut", 2.0, -30.0);
    assert_eq!(rig.inserted(), vec!["After the shortcut. ".to_owned()]);
}

#[test]
fn a_hold_past_the_minimum_starts_while_the_key_is_down() {
    let rig = Rig::builder()
        .settings(|s| s.min_hold = Duration::from_millis(200))
        .build();
    rig.silence(0.5);
    rig.press();
    rig.feed(&speech_48k(0.15, -30.0, 5));
    assert!(
        !has(&rig.events(), |e| *e == DictationEvent::Started),
        "not yet"
    );
    rig.feed(&speech_48k(0.1, -30.0, 6));
    assert!(
        has(&rig.events(), |e| *e == DictationEvent::Started),
        "held 250 ms"
    );
}

#[test]
fn a_hold_between_the_minimum_hold_and_the_minimum_length_is_too_short() {
    let rig = Rig::builder().build();
    rig.silence(0.5);
    rig.press();
    rig.feed(&speech_48k(0.25, -30.0, 5));
    rig.release();
    rig.silence(0.6);
    let events = rig.events();
    assert!(has(&events, |e| *e == DictationEvent::Started));
    assert!(
        has(&events, |e| matches!(
            e,
            DictationEvent::Discarded(Discard::TooShort { live_ms }) if (240..=260).contains(live_ms)
        )),
        "{events:?}"
    );
    assert!(rig.engine.calls().is_empty());
}

// ---------------------------------------------------------------------------------------------
// The adaptive tail (0.2 waited a fixed ~450 ms after every release)
// ---------------------------------------------------------------------------------------------

#[test]
fn the_tail_ends_as_soon_as_the_speech_has_ended() {
    let rig = Rig::builder().build();
    let mut take = speech_48k(1.5, -30.0, 11);
    take.extend(vec![0.0; 48_000 * 3 / 10]); // stopped talking 300 ms before letting go
    rig.teach(&take, "done early");

    rig.silence(0.5);
    rig.press();
    rig.feed(&take);
    rig.release();
    // Only the audio still in flight at the release is waited for: the resampler's lookahead and
    // one device block, never a tail.
    let blocks = rig
        .blocks_until_inserted(&[], 40)
        .expect("the take completed");
    assert!(
        blocks <= 4,
        "took {blocks} blocks of 10 ms after the release"
    );
    assert_eq!(rig.inserted(), vec!["Done early. ".to_owned()]);
}

#[test]
fn the_tail_waits_while_the_last_word_is_still_sounding() {
    let held = speech_48k(1.5, -30.0, 12);
    // Released mid-word: a sustained vowel goes on 120 ms after the release (6 dB under the
    // speech's peaks, so not quiet), then silence.
    let after = synth::tone(0.12, 150.0, 0.05, 48_000);
    let (held, after) = (held.as_slice(), after.as_slice());
    let run = |rig: &Rig| {
        rig.silence(0.5);
        rig.press();
        rig.feed(held);
        rig.release();
        rig.blocks_until_inserted(after, 40)
    };
    let probe = Rig::builder().build();
    assert_eq!(run(&probe), None, "the probe's engine knows nothing");
    let rig = Rig::builder().build();
    rig.engine
        .add_fixture(&probe.engine_inputs()[0], transcript("last word"));

    let blocks = run(&rig).expect("the take completed");
    // The word (120 ms) and a quiet run (80 ms) are waited for; 0.2's fixed wait was 450 ms, and
    // the tail itself never runs past 300 ms.
    assert!(
        (20..30).contains(&blocks),
        "took {blocks} blocks of 10 ms after the release"
    );
    assert_eq!(rig.inserted(), vec!["Last word. ".to_owned()]);
}

#[test]
fn a_stalled_mic_ends_the_tail_at_the_deadline() {
    let rig = Rig::builder().build();
    let speech = speech_48k(1.5, -30.0, 13);
    // Probe: the same take, ended by the deadline.
    let probe = Rig::builder().build();
    probe.silence(0.5);
    probe.press();
    probe.feed(&speech);
    probe.release();
    probe.stall(Duration::from_millis(600));
    assert_eq!(
        probe.engine_inputs().len(),
        1,
        "the deadline ended the probe's take"
    );
    rig.engine
        .add_fixture(&probe.engine_inputs()[0], transcript("cut short"));

    rig.silence(0.5);
    rig.press();
    rig.feed(&speech);
    rig.release();
    rig.stall(Duration::from_millis(200));
    assert!(rig.inserted().is_empty(), "the full tail is not due yet");
    rig.stall(Duration::from_millis(400));
    assert_eq!(rig.inserted(), vec!["Cut short. ".to_owned()]);
    assert!(has(&rig.events(), |e| *e
        == DictationEvent::Warning(Warning::TailCutShort)));
}

// ---------------------------------------------------------------------------------------------
// Stages 5, 9, 10, 11: commands, polish, persistence, insertion, and their failures
// ---------------------------------------------------------------------------------------------

#[test]
fn a_style_command_pins_its_mode_and_inserts_nothing() {
    let rig = Rig::builder()
        .settings(|s| s.commands.enabled = true)
        .build();
    rig.dictate_fixture("inkwell casual mode", 2.0, -30.0);
    assert!(rig.inserted().is_empty());
    assert!(has(&rig.events(), |e| matches!(
        e,
        DictationEvent::Command(CommandAction::ChangeStyle { .. })
    )));
    assert!(
        rig.dictation_records().is_empty(),
        "a command is not a dictation"
    );

    // The default store has no casual mode, so the command had nothing to pin.
    assert!(has(&rig.events(), |e| *e
        == DictationEvent::Warning(Warning::NoModeForStyle)));
}

#[test]
fn a_style_command_changes_how_the_next_dictation_is_written() {
    let rig = Rig::builder()
        .settings(|s| {
            s.commands.enabled = true;
            let mut chat = ink_pipeline::modes::Mode::builtin_default();
            chat.id = "chat".into();
            chat.name = "Chat".into();
            chat.style = ink_pipeline::style::Style::Casual;
            s.modes.modes.push(chat);
        })
        .build();
    let command = speech_48k(1.0, -30.0, 21);
    let words = speech_48k(1.5, -30.0, 22);
    rig.teach(&command, "inkwell casual mode");
    rig.dictate(&command);
    assert_eq!(rig.chain.borrow().pinned_mode(), Some("chat"));
    let probe = Rig::builder().build();
    probe.dictate(&words);
    rig.engine
        .add_fixture(&probe.engine_inputs()[0], transcript("see you soon."));
    rig.dictate(&words);
    assert_eq!(rig.inserted(), vec!["See you soon ".to_owned()]);
}

#[test]
fn polish_uses_the_model_and_a_failure_keeps_the_local_text() {
    let polishing = |s: &mut ink_pipeline::chain::DictationSettings| {
        s.modes.modes[0].polish_enabled = true;
    };
    let rig = Rig::builder()
        .settings(polishing)
        .llm(Arc::new(MockLlm::new(
            Endpoint::InProcess,
            "Polished text.",
        )))
        .build();
    rig.dictate_fixture("raw text", 2.0, -30.0);
    assert_eq!(rig.inserted(), vec!["Polished text. ".to_owned()]);

    let failing = Rig::builder()
        .settings(polishing)
        .llm(Arc::new(DownLlm))
        .build();
    failing.dictate_fixture("raw text", 2.0, -30.0);
    assert_eq!(failing.inserted(), vec!["Raw text. ".to_owned()]);
    assert!(has(&failing.events(), |e| matches!(
        e,
        DictationEvent::Warning(Warning::PolishFailed(LlmError::Http { status: 503 }))
    )));

    let missing = Rig::builder().settings(polishing).build();
    missing.dictate_fixture("raw text", 2.0, -30.0);
    assert_eq!(missing.inserted(), vec!["Raw text. ".to_owned()]);
    assert!(has(&missing.events(), |e| *e
        == DictationEvent::Warning(Warning::PolishUnavailable)));
}

struct DownLlm;

impl Llm for DownLlm {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "down".into(),
            model: "down".into(),
            endpoint: Endpoint::InProcess,
        }
    }

    fn complete(&self, _: &LlmRequest, _: &CancelToken) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Http { status: 503 })
    }
}

#[test]
fn a_take_of_only_fillers_is_nothing_left_not_nothing_heard() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Should not be asked."));
    let rig = Rig::builder()
        .settings(|s| s.modes.modes[0].polish_enabled = true)
        .llm(llm.clone())
        .build();
    rig.dictate_fixture("um uh hmm", 2.0, -30.0);
    let events = rig.events();
    assert!(
        has(&events, |e| *e
            == DictationEvent::Discarded(Discard::NothingLeft)),
        "{events:?}"
    );
    assert!(!has(&events, |e| *e
        == DictationEvent::Discarded(Discard::NothingHeard)));
    assert_eq!(llm.calls(), 0, "nothing was sent to be polished");
    assert!(rig.inserted().is_empty());
    assert!(rig.dictation_records().is_empty());
}

#[test]
fn a_blank_polish_answer_keeps_the_text_instead_of_emptying_it() {
    let rig = Rig::builder()
        .settings(|s| s.modes.modes[0].polish_enabled = true)
        .llm(Arc::new(MockLlm::new(Endpoint::InProcess, "   ")))
        .build();
    rig.dictate_fixture("keep me", 2.0, -30.0);
    assert_eq!(rig.inserted(), vec!["Keep me. ".to_owned()]);
    assert!(has(&rig.events(), |e| matches!(
        e,
        DictationEvent::Warning(Warning::PolishFailed(LlmError::BadResponse(_)))
    )));
}

/// Longer than any budget these tests set: a model still hanging this long was never cancelled,
/// and fails its test instead of hanging it.
const HANG_CAP: Duration = Duration::from_secs(10);

/// A model that never answers the call after [`hang_next`](Self::hang_next) is set, until its
/// token is cancelled, and answers every other call at once.
#[derive(Default)]
struct HangingLlm {
    hang_next: AtomicBool,
    /// How long the hung call waited before it saw its token cancelled.
    hung_for: Mutex<Option<Duration>>,
}

impl Llm for HangingLlm {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "hanging".into(),
            model: "hanging".into(),
            endpoint: Endpoint::InProcess,
        }
    }

    fn complete(&self, _: &LlmRequest, cancel: &CancelToken) -> Result<LlmResponse, LlmError> {
        if !self.hang_next.swap(false, Ordering::SeqCst) {
            return Ok(LlmResponse {
                text: "Polished text.".into(),
            });
        }
        let started = Instant::now();
        while started.elapsed() < HANG_CAP {
            if cancel.is_cancelled() {
                *self.hung_for.lock().unwrap() = Some(started.elapsed());
                return Err(LlmError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(LlmError::Network("never cancelled".into()))
    }
}

#[test]
fn a_polish_that_never_answers_is_cancelled_at_its_budget_and_the_next_take_is_polished() {
    const BUDGET: Duration = Duration::from_millis(200);
    let llm = Arc::new(HangingLlm::default());
    let rig = Rig::builder()
        .settings(|s| {
            s.modes.modes[0].polish_enabled = true;
            s.polish_budget = BUDGET;
        })
        .llm(llm.clone())
        .build();
    let first = speech_48k(2.0, -30.0, 71);
    let second = speech_48k(2.0, -30.0, 72);
    rig.teach(&first, "first take");
    rig.teach(&second, "second take");

    llm.hang_next.store(true, Ordering::SeqCst);
    rig.dictate(&first);
    let hung_for = llm
        .hung_for
        .lock()
        .unwrap()
        .expect("the hung call saw its token cancelled");
    assert!(
        hung_for >= BUDGET && hung_for < BUDGET + Duration::from_secs(1),
        "cancelled at the budget, not before or long after: {hung_for:?}"
    );
    assert_eq!(
        rig.inserted(),
        vec!["First take. ".to_owned()],
        "as written"
    );
    // A timeout, told apart from a cancel, and carrying no text.
    assert!(has(&rig.events(), |e| *e
        == DictationEvent::Warning(Warning::PolishTimedOut)));
    assert!(!has(&rig.events(), |e| matches!(
        e,
        DictationEvent::Warning(Warning::PolishFailed(_))
    )));

    // The next take is not held up, and polish works again for it.
    rig.dictate(&second);
    assert_eq!(
        rig.inserted(),
        vec!["First take. ".to_owned(), "Polished text. ".to_owned()]
    );
}

/// A model that stops by itself before the budget runs out: what a shell engine does when the
/// core shuts down under it.
struct StopsItself;

impl Llm for StopsItself {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "stops".into(),
            model: "stops".into(),
            endpoint: Endpoint::InProcess,
        }
    }

    fn complete(&self, _: &LlmRequest, _: &CancelToken) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Cancelled)
    }
}

#[test]
fn a_polish_cancelled_before_its_budget_is_a_cancel_not_a_timeout() {
    let rig = Rig::builder()
        .settings(|s| {
            s.modes.modes[0].polish_enabled = true;
            s.polish_budget = Duration::from_secs(60);
        })
        .llm(Arc::new(StopsItself))
        .build();
    rig.dictate_fixture("stopped early", 2.0, -30.0);
    assert_eq!(
        rig.inserted(),
        vec!["Stopped early. ".to_owned()],
        "as written"
    );
    assert!(has(&rig.events(), |e| *e
        == DictationEvent::Warning(Warning::PolishFailed(
            LlmError::Cancelled
        ))));
    assert!(!has(&rig.events(), |e| *e
        == DictationEvent::Warning(Warning::PolishTimedOut)));
}

#[test]
fn a_panic_while_saving_leaves_no_half_written_record() {
    for fault in [common::Fault::AppendPanics, common::Fault::FinishPanics] {
        let store = Arc::new(common::FaultyStore::new(fault));
        let rig = Rig::builder().store(store.clone()).build();
        let speech = speech_48k(2.0, -30.0, 61);
        rig.teach(&speech, "half saved");
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rig.dictate(&speech)));
        assert!(outcome.is_err(), "{fault:?}: the store panicked");
        assert!(
            rig.dictation_records().is_empty(),
            "{fault:?}: the record the panic interrupted was deleted"
        );
    }
}

#[test]
fn an_engine_failure_inserts_and_saves_nothing() {
    let rig = Rig::builder().build();
    rig.dictate(&speech_48k(2.0, -30.0, 31)); // no fixture: the mock engine fails
    assert!(rig.inserted().is_empty());
    assert!(rig.dictation_records().is_empty());
    assert!(has(&rig.events(), |e| matches!(
        e,
        DictationEvent::Failed(TakeFailure::Transcription(_))
    )));
}

#[test]
fn secure_input_is_reported_as_blocked_and_the_dictation_is_kept() {
    let rig = Rig::builder().build();
    rig.platform.set_focus(FocusInfo {
        app: None,
        secure_input: true,
    });
    rig.dictate_fixture("into a password field", 2.0, -30.0);
    assert!(rig.inserted().is_empty());
    let events = rig.events();
    assert!(has(&events, |e| matches!(
        e,
        DictationEvent::Inserted {
            outcome: InsertOutcome::Blocked,
            record: Some(_),
            ..
        }
    )));
    assert_eq!(
        rig.dictation_records().len(),
        1,
        "kept, so it can be copied"
    );
}

#[test]
fn a_lost_hotkey_ends_the_take_and_keeps_what_was_said() {
    let rig = Rig::builder().build();
    let speech = speech_48k(1.5, -30.0, 41);
    let probe = Rig::builder().build();
    probe.silence(0.5);
    probe.press();
    probe.feed(&speech);
    assert!(probe.platform.lose_hotkey());
    probe.silence(0.6);
    // The probe's hotkey events were delivered on its next feed; nothing else differs.
    rig.engine
        .add_fixture(&probe.engine_inputs()[0], transcript("kept"));

    rig.silence(0.5);
    rig.press();
    rig.feed(&speech);
    assert!(rig.platform.lose_hotkey());
    rig.silence(0.6);
    assert_eq!(rig.inserted(), vec!["Kept. ".to_owned()]);
    assert!(has(&rig.events(), |e| *e == DictationEvent::HotkeyLost));
}

#[test]
fn in_toggle_mode_missed_key_events_keep_the_take_but_a_lost_hotkey_ends_it() {
    let rig = Rig::builder()
        .settings(|s| s.recording_mode = ink_pipeline::transition::RecordingMode::Toggle)
        .build();
    rig.silence(0.5);
    rig.press();
    rig.feed(&speech_48k(0.5, -30.0, 51));
    rig.release();
    rig.feed(&speech_48k(0.5, -30.0, 52));
    assert!(
        rig.chain.borrow().is_recording(),
        "a toggle take outlives its key"
    );
    assert!(rig.platform.cancel_hotkey());
    rig.feed(&speech_48k(0.5, -30.0, 53));
    assert!(
        rig.chain.borrow().is_recording(),
        "missed events do not end a toggle take"
    );
    assert!(rig.platform.lose_hotkey());
    rig.silence(0.6);
    assert!(!rig.chain.borrow().is_recording());
    assert_eq!(rig.engine.calls().len(), 1, "what was said was transcribed");
    assert!(has(&rig.events(), |e| *e == DictationEvent::HotkeyLost));
}

#[test]
fn audio_lost_during_a_take_is_reported() {
    use ink_core::mock::{MemStore, MockEngine, MockPlatform};
    use ink_core::{HotkeyEvent, Job};
    use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
    use std::sync::Mutex;

    let platform = Arc::new(MockPlatform::new());
    let engine = MockEngine::new("mock", &[Job::DictationFinal]);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut chain = DictationChain::new(
        Services {
            engine: Arc::new(engine),
            store: Arc::new(MemStore::new()),
            inserter: platform.clone(),
            focus: platform.clone(),
            clock: platform.clock(),
            llm: None,
        },
        DictationSettings::default(),
        VadKind::Energy.build(),
        Arc::new(move |e| sink.lock().unwrap().push(e)),
    );
    let block = vec![0.01f32; 160];
    let mut t = 1_000_000_000u64;
    for _ in 0..50 {
        chain.push_audio(&block, t, 0);
        t += 10_000_000;
    }
    chain.hotkey(HotkeyEvent::Pressed { at_ns: t });
    for k in 0..100 {
        chain.push_audio(&block, t, if k == 40 { 480 } else { 0 });
        t += 10_000_000;
    }
    chain.hotkey(HotkeyEvent::Released { at_ns: t });
    for _ in 0..50 {
        chain.push_audio(&block, t, 0);
        t += 10_000_000;
    }
    let events = events.lock().unwrap();
    assert!(
        events.contains(&DictationEvent::Warning(Warning::AudioLost { frames: 480 })),
        "{events:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// The real-engine variant: local only, with bench data and a model
// ---------------------------------------------------------------------------------------------

/// Word error rate of `hypothesis` against `reference`: word-level edit distance over the
/// reference's length, case and punctuation ignored.
fn wer(reference: &str, hypothesis: &str) -> f64 {
    let words = |s: &str| -> Vec<String> {
        s.split_whitespace()
            .map(|w| {
                w.chars()
                    .filter(|c| c.is_alphanumeric() || *c == '\'')
                    .collect::<String>()
                    .to_lowercase()
            })
            .filter(|w| !w.is_empty())
            .collect()
    };
    let (r, h) = (words(reference), words(hypothesis));
    let mut row: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut diagonal = row[0];
        row[0] = i;
        for j in 1..=h.len() {
            let above = row[j];
            row[j] = (row[j] + 1)
                .min(row[j - 1] + 1)
                .min(diagonal + usize::from(r[i - 1] != h[j - 1]));
            diagonal = above;
        }
    }
    row[h.len()] as f64 / r.len().max(1) as f64
}

#[test]
fn wer_counts_word_edits() {
    assert_eq!(wer("the cat sat", "the cat sat"), 0.0);
    assert!((wer("the cat sat", "The cat, sat on") - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(wer("a b", ""), 1.0);
}

/// Qwen3-ASR 1.7B, the dictation engine the router picks on a Mac (the gate's choice), loaded from
/// `$INK_BENCH_DIR/models/qwen3-asr-1.7b-gguf/`.
#[cfg(feature = "engine-llama")]
fn real_dictation_engine(bench: &std::path::Path) -> Arc<dyn ink_core::OfflineEngine> {
    let row = ink_engines::Registry::builtin()
        .expect("the built-in registry")
        .get("qwen3-asr-1.7b-q8")
        .expect("the Qwen3-ASR row")
        .clone();
    let dir = bench.join("models/qwen3-asr-1.7b-gguf");
    Arc::new(
        ink_engines::llama::QwenAsr::load(
            &dir.join(&row.files[0].name),
            &dir.join(&row.files[1].name),
            row.info(),
        )
        .expect("Qwen3-ASR loads"),
    )
}

/// A bench clip: 16 kHz mono, stored as 32-bit float (the AMI IHM clips) or 16-bit PCM.
#[cfg(feature = "engine-llama")]
fn read_clip(path: &std::path::Path) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).expect("the clip");
    let spec = reader.spec();
    assert_eq!(
        (spec.sample_rate, spec.channels),
        (16_000, 1),
        "a 16 kHz mono clip"
    );
    match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.expect("a sample"))
            .collect(),
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| f32::from(s.expect("a sample")) / 32_768.0)
            .collect(),
    }
}

/// An AMI close-talk clip dictated through the whole chain with Qwen3-ASR and a real resampler
/// path, against its human transcript. Run with
/// `INK_BENCH_DIR=<bench data> cargo test -p ink-pipeline --features engine-llama --release -- --ignored a_real_engine`.
/// Measured on an M5 Pro (2026-09-26): WER 0.186 on `ami-ihm-00.wav` (70.6 s).
#[cfg(feature = "engine-llama")]
#[test]
#[ignore = "needs $INK_BENCH_DIR (an AMI clip and the Qwen3-ASR model); run locally"]
fn a_real_engine_transcribes_a_bench_clip_through_the_chain() {
    use ink_core::mock::{MemStore, MockPlatform};
    use ink_pipeline::chain::{DictationChain, DictationSettings, Services};
    use std::sync::Mutex;

    let dir = std::path::PathBuf::from(std::env::var("INK_BENCH_DIR").expect("INK_BENCH_DIR"));
    let tsv = std::fs::read_to_string(dir.join("ami-ihm.tsv")).expect("ami-ihm.tsv");
    let mut fields = tsv.lines().next().expect("a clip").splitn(3, '\t').skip(1);
    let (file, reference) = (
        fields.next().expect("a file"),
        fields.next().expect("a text"),
    );
    // The reference column is followed by more tab-separated columns; the text is the first.
    let reference = reference.split('\t').next().expect("a text");
    let clip = read_clip(&dir.join("ami-ihm").join(file));

    let platform = Arc::new(MockPlatform::new());
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut chain = DictationChain::new(
        Services {
            engine: real_dictation_engine(&dir),
            store: Arc::new(MemStore::new()),
            inserter: platform.clone(),
            focus: platform.clone(),
            clock: platform.clock(),
            llm: None,
        },
        DictationSettings::default(),
        VadKind::Energy.build(),
        Arc::new(move |e| sink.lock().unwrap().push(e)),
    );
    let mut t = 1_000_000_000u64;
    let feed = |chain: &mut DictationChain, samples: &[f32], t: &mut u64| {
        for block in samples.chunks(160) {
            chain.push_audio(block, *t, 0);
            *t += 10_000_000;
        }
    };
    feed(&mut chain, &[0.0; 8_000], &mut t);
    chain.hotkey(ink_core::HotkeyEvent::Pressed { at_ns: t });
    feed(&mut chain, &clip, &mut t);
    chain.hotkey(ink_core::HotkeyEvent::Released { at_ns: t });
    feed(&mut chain, &[0.0; 9_600], &mut t);

    let inserted = platform.inserted();
    assert_eq!(inserted.len(), 1, "{:?}", events.lock().unwrap());
    let error = wer(reference, &inserted[0]);
    eprintln!("WER {error:.3} on {file}");
    // A sanity bound for the whole chain, not the engine's measured rate (its own adapter test
    // holds that): conversational AMI speech with fillers the chain removes on purpose.
    assert!(error < 0.5, "WER {error:.3}");
}
