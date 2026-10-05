//! Dictation end to end on the chain (S2.7): a second press, the stuck-key watchdog, voice edit,
//! the polish switch, live words while the key is held, and the mic let go of while idle.
//!
//! The same rig as `dictation.rs`: mocks for the platform and the engines, synthetic audio on a
//! mock clock.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Rig, speech_48k};
use ink_core::mock::{MockEngine, MockLlm};
use ink_core::{
    AppRef, AsrEvent, CancelToken, Channel, Endpoint, EngineError, EngineInfo, EngineStream,
    EventSink, FocusInfo, InsertOutcome, Job, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse,
    RecordKind, RecordQuery, StreamingEngine,
};
use ink_pipeline::events::{DictationEvent, Discard, EditFailure, Warning};
use ink_pipeline::modes::{Mode, ModeStore};
use ink_pipeline::transition::RecordingMode;

fn has(events: &[DictationEvent], wanted: impl Fn(&DictationEvent) -> bool) -> bool {
    events.iter().any(wanted)
}

fn count(events: &[DictationEvent], wanted: impl Fn(&DictationEvent) -> bool) -> usize {
    events.iter().filter(|e| wanted(e)).count()
}

// ---------------------------------------------------------------------------------------------
// A second press never wipes the take
// ---------------------------------------------------------------------------------------------

/// Inkwell 0.2 once cleared its buffer when a press arrived while already recording (a lost
/// release, a repeated key-down) and began again, losing everything said before it. Here the
/// second press changes nothing: the engine hears exactly what it would have without it.
#[test]
fn a_second_press_while_held_never_wipes_the_take() {
    let rig = Rig::builder().build();
    let speech = speech_48k(2.0, -25.0, 3);
    rig.teach(&speech, "the whole sentence");
    let (first, second) = speech.split_at(speech.len() / 2);
    rig.silence(0.5);
    rig.press();
    rig.feed(first);
    // The key-down arrives again while the key is held.
    rig.press();
    rig.feed(second);
    rig.release();
    rig.silence(0.6);
    assert_eq!(rig.inserted(), ["The whole sentence. "]);
    let events = rig.events();
    assert_eq!(
        count(&events, |e| matches!(e, DictationEvent::Started { .. })),
        1,
        "one take, never restarted: {events:?}"
    );
}

/// In toggle mode the second press is the stop, and the take it stops is inserted whole.
#[test]
fn in_toggle_mode_the_second_press_stops_and_keeps_the_take() {
    let rig = Rig::builder()
        .settings(|s| s.recording_mode = RecordingMode::Toggle)
        .build();
    rig.answer_anything("toggled");
    rig.silence(0.5);
    rig.press();
    rig.feed(&speech_48k(1.5, -25.0, 4));
    // Toggle: the key comes up at once and the take goes on.
    rig.release();
    rig.feed(&speech_48k(0.5, -25.0, 5));
    rig.press();
    rig.release();
    rig.silence(0.6);
    assert_eq!(rig.inserted(), ["Toggled. "]);
}

/// A press during the previous take's tail ends that take first (it is inserted) and starts the
/// next: neither is lost.
#[test]
fn a_press_during_the_tail_keeps_the_first_take_and_starts_the_next() {
    let rig = Rig::builder().build();
    rig.answer_anything("one of two");
    rig.silence(0.5);
    rig.press();
    rig.feed(&speech_48k(1.0, -25.0, 6));
    rig.release();
    // The tail is still open (the speech was loud to the end) when the key goes down again.
    rig.press();
    assert_eq!(rig.inserted(), ["One of two. "]);
    rig.feed(&speech_48k(1.0, -25.0, 7));
    rig.release();
    rig.silence(0.6);
    assert_eq!(rig.inserted().len(), 2);
}

// ---------------------------------------------------------------------------------------------
// The stuck-key watchdog
// ---------------------------------------------------------------------------------------------

/// A push-to-talk key held past the limit (its release lost) is stopped there, and what was said
/// is processed and inserted, never discarded.
#[test]
fn a_hold_past_the_stuck_limit_is_stopped_and_processed() {
    let rig = Rig::builder()
        .settings(|s| s.stuck_after = Duration::from_secs(3))
        .build();
    rig.answer_anything("kept after all");
    rig.silence(0.5);
    rig.press();
    assert!(
        rig.chain.borrow().deadline_ns().is_some(),
        "a held push-to-talk key has a deadline, so the worker wakes for it"
    );
    rig.feed(&speech_48k(1.0, -25.0, 8));
    // No release ever comes; the audio goes on (quiet), and the rig ticks the chain.
    rig.silence(3.0);
    let events = rig.events();
    assert!(
        has(&events, |e| *e
            == DictationEvent::Warning(Warning::ReleaseMissed)),
        "{events:?}"
    );
    assert_eq!(rig.inserted(), ["Kept after all. "]);
    // The key's real release, whenever it comes, starts nothing.
    rig.release();
    rig.silence(0.5);
    assert_eq!(rig.inserted().len(), 1);
}

/// The watchdog wakes by the worker's deadline alone: with the mic stalled as well (no audio at
/// all), the tick still stops the take.
#[test]
fn the_watchdog_fires_on_the_deadline_without_audio() {
    let rig = Rig::builder()
        .settings(|s| s.stuck_after = Duration::from_secs(2))
        .build();
    rig.answer_anything("still here");
    rig.silence(0.5);
    rig.press();
    rig.feed(&speech_48k(1.0, -25.0, 9));
    rig.stall(Duration::from_secs(2));
    let events = rig.events();
    assert!(has(&events, |e| *e
        == DictationEvent::Warning(Warning::ReleaseMissed)));
    // The tail's own deadline then ends the take with the audio it has.
    rig.stall(Duration::from_secs(1));
    assert_eq!(rig.inserted(), ["Still here. "]);
}

/// A toggle take is exempt: a long one is deliberate, and its stop is a press.
#[test]
fn a_long_toggle_take_is_not_stopped_by_the_watchdog() {
    let rig = Rig::builder()
        .settings(|s| {
            s.recording_mode = RecordingMode::Toggle;
            s.stuck_after = Duration::from_secs(2);
        })
        .build();
    rig.answer_anything("long on purpose");
    rig.silence(0.5);
    rig.press();
    // Held past the minimum, so the tap is a take, then let go: toggle goes on.
    rig.feed(&speech_48k(0.3, -25.0, 28));
    rig.release();
    rig.feed(&speech_48k(3.0, -25.0, 10));
    assert!(rig.chain.borrow().deadline_ns().is_none());
    assert!(rig.inserted().is_empty());
    assert!(!has(&rig.events(), |e| *e
        == DictationEvent::Warning(Warning::ReleaseMissed)));
    rig.press();
    rig.release();
    rig.silence(0.6);
    assert_eq!(rig.inserted(), ["Long on purpose. "]);
}

// ---------------------------------------------------------------------------------------------
// Voice edit
// ---------------------------------------------------------------------------------------------

/// Select text, hold the edit key, say what to change: the model's rewrite replaces the
/// selection (no trailing space), and nothing is saved to the library.
#[test]
fn a_voice_edit_replaces_the_selection_with_the_rewrite() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "```\nDear team,\n```"));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| s.edit_consent = Some(ink_pipeline::consent::LlmConsent::OnDevice))
        .build();
    rig.answer_anything("make it formal");
    rig.platform.set_selection(Some("hey all"));
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(1.2, -25.0, 11));
    rig.edit_release();
    rig.silence(0.6);
    assert_eq!(rig.inserted(), ["Dear team,"]);
    let events = rig.events();
    assert!(
        has(&events, |e| matches!(
            e,
            DictationEvent::Started {
                edit: true,
                mode: None,
                ..
            }
        )),
        "{events:?}"
    );
    assert!(has(&events, |e| *e
        == DictationEvent::Edited {
            outcome: InsertOutcome::Pasted
        }));
    assert_eq!(llm.calls(), 1);
    assert!(rig.dictation_records().is_empty(), "an edit is not saved");
}

/// A model that refuses the edit leaves the selection as it was: its refusal is never pasted over
/// the user's text, and the edit's failure is said.
#[test]
fn a_refused_edit_leaves_the_selection() {
    let refusal = "I am a foundation model developed by Apple. I cannot fulfill this request.";
    let rig = Rig::builder()
        .llm(Arc::new(MockLlm::new(Endpoint::InProcess, refusal)))
        .settings(|s| s.edit_consent = Some(ink_pipeline::consent::LlmConsent::OnDevice))
        .build();
    rig.answer_anything("make it formal");
    rig.platform.set_selection(Some("hey all"));
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(1.2, -25.0, 17));
    rig.edit_release();
    rig.silence(0.6);
    assert!(rig.inserted().is_empty(), "nothing replaced the selection");
    assert!(has(&rig.events(), |e| matches!(
        e,
        DictationEvent::EditFailed(EditFailure::Model(LlmError::BadResponse(_)))
    )));
}

/// With nothing selected, the edit ends at its confirmation: nothing is transcribed, rewritten or
/// inserted, and the shell hears why.
#[test]
fn an_edit_with_nothing_selected_ends_before_anything_is_heard() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "never used"));
    let rig = Rig::builder().llm(llm.clone()).build();
    rig.answer_anything("anything");
    rig.platform.set_selection(None);
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(1.0, -25.0, 12));
    rig.edit_release();
    rig.silence(0.6);
    let events = rig.events();
    assert!(has(&events, |e| *e
        == DictationEvent::EditFailed(EditFailure::NoSelection)));
    assert!(!has(&events, |e| matches!(
        e,
        DictationEvent::Started { .. }
    )));
    assert!(rig.engine_inputs().is_empty());
    assert!(rig.inserted().is_empty());
    assert_eq!(llm.calls(), 0);
}

/// A modifier tapped in a shortcut never reads the selection: the edit key's short press is
/// ignored like the dictation key's.
#[test]
fn a_tapped_edit_key_reads_nothing_and_says_nothing_is_selected_never() {
    let rig = Rig::builder().build();
    rig.platform.set_selection(None);
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(0.05, -25.0, 13));
    rig.edit_release();
    rig.silence(0.3);
    let events = rig.events();
    assert!(has(&events, |e| *e == DictationEvent::ShortPressIgnored));
    assert!(!has(&events, |e| matches!(
        e,
        DictationEvent::EditFailed(_)
    )));
}

/// One take at a time: the edit key pressed and released during a dictation changes nothing, and
/// the dictation goes in whole.
#[test]
fn the_other_key_during_a_take_is_ignored() {
    let rig = Rig::builder().build();
    rig.platform.set_selection(Some("selected"));
    let speech = speech_48k(2.0, -25.0, 14);
    rig.teach(&speech, "dictated");
    let (first, second) = speech.split_at(speech.len() / 2);
    rig.silence(0.5);
    rig.press();
    rig.feed(first);
    rig.edit_press();
    rig.edit_release();
    rig.feed(second);
    rig.release();
    rig.silence(0.6);
    assert_eq!(rig.inserted(), ["Dictated. "]);
    assert!(!has(&rig.events(), |e| matches!(
        e,
        DictationEvent::Started { edit: true, .. }
    )));
}

struct NeverAnswers;

impl Llm for NeverAnswers {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "test".into(),
            model: "hangs".into(),
            endpoint: Endpoint::InProcess,
        }
    }

    fn complete(&self, _: &LlmRequest, cancel: &CancelToken) -> Result<LlmResponse, LlmError> {
        while !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(LlmError::Cancelled)
    }
}

/// A rewrite that never comes is given up at the edit's budget, and the selection is left alone.
#[test]
fn an_edit_whose_model_never_answers_times_out_and_leaves_the_selection() {
    let rig = Rig::builder()
        .llm(Arc::new(NeverAnswers))
        .settings(|s| {
            s.edit_budget = Duration::from_millis(100);
            s.edit_consent = Some(ink_pipeline::consent::LlmConsent::OnDevice);
        })
        .build();
    rig.answer_anything("shorten it");
    rig.platform.set_selection(Some("a long paragraph"));
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(1.0, -25.0, 15));
    rig.edit_release();
    rig.silence(0.6);
    assert!(has(&rig.events(), |e| *e
        == DictationEvent::EditFailed(EditFailure::TimedOut)));
    assert!(rig.inserted().is_empty());
}

/// Without a language model an edit says so, and touches nothing.
#[test]
fn an_edit_without_a_model_says_so() {
    let rig = Rig::builder().build();
    rig.answer_anything("fix the typo");
    rig.platform.set_selection(Some("teh cat"));
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(1.0, -25.0, 16));
    rig.edit_release();
    rig.silence(0.6);
    assert!(has(&rig.events(), |e| *e
        == DictationEvent::EditFailed(EditFailure::NoModel)));
    assert!(rig.inserted().is_empty());
}

// ---------------------------------------------------------------------------------------------
// The polish switch and the mode shown
// ---------------------------------------------------------------------------------------------

fn polishing_modes() -> ModeStore {
    ModeStore {
        default_id: "default".into(),
        modes: vec![Mode {
            polish_enabled: true,
            ..Mode::builtin_default()
        }],
    }
}

/// "Polish my words" off: nothing is polished, even in a mode that polishes.
#[test]
fn with_the_polish_switch_off_nothing_is_polished() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Polished."));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| {
            s.modes = polishing_modes();
            s.polish_wish = false;
            s.polish_consents = vec![ink_pipeline::consent::LlmConsent::OnDevice];
        })
        .build();
    rig.answer_anything("as said");
    rig.dictate(&speech_48k(1.0, -25.0, 17));
    assert_eq!(rig.inserted(), ["As said. "]);
    assert_eq!(llm.calls(), 0);
}

/// On, the mode's polish runs.
#[test]
fn with_the_polish_switch_on_the_modes_that_polish_do() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Polished."));
    let rig = Rig::builder()
        .llm(llm.clone())
        .settings(|s| {
            s.modes = polishing_modes();
            s.polish_consents = vec![ink_pipeline::consent::LlmConsent::OnDevice];
        })
        .build();
    rig.answer_anything("as said");
    rig.dictate(&speech_48k(1.0, -25.0, 18));
    assert_eq!(rig.inserted(), ["Polished. "]);
    assert_eq!(llm.calls(), 1);
}

/// The take's start names the app in front and the mode it picks, for the Drop.
#[test]
fn the_start_names_the_app_in_front_and_its_mode() {
    let rig = Rig::builder()
        .settings(|s| {
            s.modes = ModeStore {
                default_id: "default".into(),
                modes: vec![
                    Mode {
                        id: "chat".into(),
                        name: "Chat".into(),
                        apps: vec!["com.example.chat".into()],
                        ..Mode::builtin_default()
                    },
                    Mode::builtin_default(),
                ],
            }
        })
        .build();
    rig.answer_anything("hi");
    rig.platform.set_focus(FocusInfo {
        app: Some(AppRef {
            id: "com.example.chat".into(),
            pid: Some(1),
            name: "Example Chat".into(),
        }),
        secure_input: false,
    });
    rig.dictate(&speech_48k(1.0, -25.0, 19));
    assert!(has(&rig.events(), |e| *e
        == DictationEvent::Started {
            take: 0,
            edit: false,
            mode: Some("Chat".into()),
            app: Some("Example Chat".into()),
        }));
}

/// Windows names the app in front by its executable, spelled as Windows spells it (ink-platform-
/// win's focus reader), and a mode holds that name in lower case (Settings > Modes): the mode is
/// picked by the foreground process, whatever the case.
#[test]
fn on_windows_the_mode_is_picked_by_the_foreground_process() {
    let rig = Rig::builder()
        .settings(|s| {
            s.modes = ModeStore {
                default_id: "default".into(),
                modes: vec![
                    Mode {
                        id: "docs".into(),
                        name: "Documents".into(),
                        apps: vec!["winword.exe".into()],
                        ..Mode::builtin_default()
                    },
                    Mode::builtin_default(),
                ],
            }
        })
        .build();
    rig.answer_anything("hi");
    rig.platform.set_focus(FocusInfo {
        app: Some(AppRef {
            id: "WINWORD.EXE".into(),
            pid: Some(4242),
            name: "WINWORD".into(),
        }),
        secure_input: false,
    });
    rig.dictate(&speech_48k(1.0, -25.0, 23));
    assert!(has(&rig.events(), |e| *e
        == DictationEvent::Started {
            take: 0,
            edit: false,
            mode: Some("Documents".into()),
            app: Some("WINWORD".into()),
        }));
}

// ---------------------------------------------------------------------------------------------
// Live words while the key is held
// ---------------------------------------------------------------------------------------------

/// A live engine that records what it hears and reports a partial per push; counts closes.
struct Listener {
    inner: MockEngine,
    opened: AtomicUsize,
    closed: Arc<AtomicUsize>,
    first_push: Arc<Mutex<Option<usize>>>,
}

struct CountedStream {
    inner: Box<dyn EngineStream>,
    closed: Arc<AtomicUsize>,
    first_push: Arc<Mutex<Option<usize>>>,
}

impl EngineStream for CountedStream {
    fn push(&mut self, audio: &[f32]) -> Result<(), EngineError> {
        self.first_push.lock().unwrap().get_or_insert(audio.len());
        self.inner.push(audio)
    }

    fn finish(self: Box<Self>) -> Result<(), EngineError> {
        Ok(())
    }
}

impl Drop for CountedStream {
    fn drop(&mut self) {
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
}

impl StreamingEngine for Listener {
    fn info(&self) -> EngineInfo {
        StreamingEngine::info(&self.inner)
    }

    fn open_stream(
        &self,
        channel: Channel,
        events: EventSink<AsrEvent>,
    ) -> Result<Box<dyn EngineStream>, EngineError> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountedStream {
            inner: self.inner.open_stream(channel, events)?,
            closed: self.closed.clone(),
            first_push: self.first_push.clone(),
        }))
    }
}

/// While the key is held the Drop gets live words for this take, starting with the lead (the
/// same audio the final hears); none follows the take's stop, and the stream is closed by then.
#[test]
fn live_words_arrive_while_held_and_stop_with_the_take() {
    let rig = Rig::builder().build();
    rig.answer_anything("final words");
    let listener = Arc::new(Listener {
        inner: MockEngine::new("mock-live", &[Job::LivePartials]),
        opened: AtomicUsize::new(0),
        closed: Arc::new(AtomicUsize::new(0)),
        first_push: Arc::new(Mutex::new(None)),
    });
    rig.chain.borrow_mut().set_live(Some(listener.clone()));
    rig.dictate(&speech_48k(1.0, -25.0, 20));

    let events = rig.events();
    let started = events
        .iter()
        .position(|e| matches!(e, DictationEvent::Started { .. }))
        .expect("started");
    let stopped = events
        .iter()
        .position(|e| *e == DictationEvent::Stopped)
        .expect("stopped");
    let partials: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, DictationEvent::Partial { take: 0, .. }))
        .map(|(i, _)| i)
        .collect();
    assert!(!partials.is_empty(), "{events:?}");
    assert!(partials.iter().all(|&i| started < i && i < stopped));
    assert_eq!(listener.opened.load(Ordering::SeqCst), 1);
    assert_eq!(
        listener.closed.load(Ordering::SeqCst),
        1,
        "closed at the stop"
    );
    // The first push held the take so far: the 300 ms lead and the 200 ms of minimum hold.
    let first = listener.first_push.lock().unwrap().expect("pushed");
    assert!(first >= 4_800 + 3_000, "{first} samples");
    assert_eq!(rig.inserted(), ["Final words. "]);
}

/// Without a live engine, dictation works the same, without live words.
#[test]
fn without_a_live_engine_there_are_no_live_words_and_the_take_is_unchanged() {
    let rig = Rig::builder().build();
    rig.answer_anything("no live");
    rig.dictate(&speech_48k(1.0, -25.0, 21));
    assert!(!has(&rig.events(), |e| matches!(
        e,
        DictationEvent::Partial { .. }
    )));
    assert_eq!(rig.inserted(), ["No live. "]);
}

/// An edit shows no live words: the instruction is not the text.
#[test]
fn an_edit_opens_no_live_stream() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "rewritten"));
    let rig = Rig::builder()
        .llm(llm)
        .settings(|s| s.edit_consent = Some(ink_pipeline::consent::LlmConsent::OnDevice))
        .build();
    rig.answer_anything("rewrite it");
    rig.platform.set_selection(Some("text"));
    let listener = Arc::new(Listener {
        inner: MockEngine::new("mock-live", &[Job::LivePartials]),
        opened: AtomicUsize::new(0),
        closed: Arc::new(AtomicUsize::new(0)),
        first_push: Arc::new(Mutex::new(None)),
    });
    rig.chain.borrow_mut().set_live(Some(listener.clone()));
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(1.0, -25.0, 22));
    rig.edit_release();
    rig.silence(0.6);
    assert_eq!(listener.opened.load(Ordering::SeqCst), 0);
    assert_eq!(rig.inserted(), ["rewritten"]);
}

// ---------------------------------------------------------------------------------------------
// The mic let go of while idle
// ---------------------------------------------------------------------------------------------

/// After the mic was let go of, a press made before the audio returns still becomes a take once
/// it does, and the take starts with the new audio (the room heard before the close is not its
/// lead).
#[test]
fn after_the_mic_was_let_go_of_the_next_press_starts_with_the_new_audio() {
    let rig = Rig::builder().build();
    rig.answer_anything("after the break");
    // Loud room before the close: were it kept, it would be the next take's lead.
    rig.feed(&speech_48k(1.0, -20.0, 23));
    rig.chain.borrow_mut().mic_closed();
    rig.platform.clock().advance_ns(60_000_000_000);
    rig.press();
    rig.feed(&speech_48k(1.0, -25.0, 24));
    rig.release();
    rig.silence(0.6);
    assert_eq!(rig.inserted(), ["After the break. "]);
    let inputs = rig.engine_inputs();
    assert_eq!(inputs.len(), 1);
    // 1 s of speech plus at most the 300 ms tail: no 300 ms lead of the old room in front.
    assert!(inputs[0].len() <= 16_000 + 4_800, "{}", inputs[0].len());
}

/// The records a dictation saves are unchanged by all of this: one per inserted dictation.
#[test]
fn dictations_are_saved_and_edits_are_not() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "Rewritten."));
    let rig = Rig::builder()
        .llm(llm)
        .settings(|s| s.edit_consent = Some(ink_pipeline::consent::LlmConsent::OnDevice))
        .build();
    rig.answer_anything("words");
    rig.platform.set_selection(Some("old"));
    rig.dictate(&speech_48k(1.0, -25.0, 25));
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(1.0, -25.0, 26));
    rig.edit_release();
    rig.silence(0.6);
    let records = rig
        .store
        .records(&RecordQuery {
            kind: Some(RecordKind::Dictation),
            before: None,
            limit: 10,
        })
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(rig.inserted(), ["Words. ", "Rewritten."]);
}

/// Too-short takes stay too short for edits too (nothing reaches the model).
#[test]
fn a_too_short_edit_is_discarded_like_a_dictation() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "never"));
    let rig = Rig::builder().llm(llm.clone()).build();
    rig.platform.set_selection(Some("text"));
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(0.25, -25.0, 27));
    rig.edit_release();
    rig.silence(0.6);
    assert!(has(&rig.events(), |e| matches!(
        e,
        DictationEvent::Discarded(Discard::TooShort { .. })
    )));
    assert_eq!(llm.calls(), 0);
}

/// A focus reader under Secure Input (a password field): counts every selection read.
struct SecureFocus {
    selection_reads: AtomicUsize,
}

impl ink_core::FocusReader for SecureFocus {
    fn focus(&self) -> Result<FocusInfo, ink_core::PlatformError> {
        Ok(FocusInfo {
            app: None,
            secure_input: true,
        })
    }

    fn selected_text(&self) -> Result<Option<String>, ink_core::PlatformError> {
        self.selection_reads.fetch_add(1, Ordering::SeqCst);
        Ok(Some("a password".into()))
    }
}

/// Under Secure Input the selection is never read, so it can never reach a language model: the
/// edit ends at its confirmation and says why.
#[test]
fn an_edit_under_secure_input_never_reads_the_selection() {
    let llm = Arc::new(MockLlm::new(Endpoint::InProcess, "never"));
    let focus = Arc::new(SecureFocus {
        selection_reads: AtomicUsize::new(0),
    });
    let rig = Rig::builder().llm(llm.clone()).focus(focus.clone()).build();
    rig.answer_anything("make it formal");
    rig.silence(0.5);
    rig.edit_press();
    rig.feed(&speech_48k(1.0, -25.0, 29));
    rig.edit_release();
    rig.silence(0.6);
    assert_eq!(focus.selection_reads.load(Ordering::SeqCst), 0);
    assert_eq!(llm.calls(), 0);
    assert!(rig.engine_inputs().is_empty(), "nothing transcribed either");
    assert!(rig.inserted().is_empty());
    assert!(has(&rig.events(), |e| *e
        == DictationEvent::EditFailed(EditFailure::SecureInput)));
}
