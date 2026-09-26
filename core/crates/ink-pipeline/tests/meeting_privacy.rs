//! I5 as behaviour, for meetings and imports: a meeting whose every word is a known secret, live
//! and final, runs down every path that logs or reports (partials, finals, the final pass, an
//! empty region, a live final over silence, a model that answers badly, an import), and neither
//! the log nor any event, as the shell would print it, holds a word of it. The source lint
//! (`tests/privacy_lint.rs`) covers the new modules too: it scans every file under `src/`.

mod meeting_rig;

use std::sync::{Arc, Mutex, OnceLock};

use ink_core::mock::{MemStore, MockClock};
use ink_core::{CancelToken, Channel, LlmError, LlmRequest, StreamFormat};
use ink_pipeline::import::{ImportServices, ImportSettings, import_samples};
use ink_pipeline::meeting::events::{KeptLive, MeetingEvent, MeetingWarning};
use meeting_rig::*;

const SECRET: &str = "zebra marmalade quartz";

fn captured() -> &'static Mutex<Vec<String>> {
    static LINES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    static INSTALLED: OnceLock<()> = OnceLock::new();
    let lines = LINES.get_or_init(Mutex::default);
    INSTALLED.get_or_init(|| {
        struct Capture;
        impl log::Log for Capture {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                let line = format!("{} {}: {}", record.level(), record.target(), record.args());
                captured().lock().unwrap().push(line);
            }
            fn flush(&self) {}
        }
        static CAPTURE: Capture = Capture;
        log::set_logger(&CAPTURE).expect("no other logger in this binary");
        log::set_max_level(log::LevelFilter::Trace);
    });
    lines
}

fn assert_no_secret(what: &str, text: &str) {
    for word in SECRET.split(' ') {
        assert!(
            !text.to_lowercase().contains(word),
            "{what} contains a meeting's word: {text}"
        );
    }
}

/// Every event as the shell could print it: its `Debug`, and the `Display` of any error inside.
fn printed(events: &[MeetingEvent]) -> Vec<String> {
    events
        .iter()
        .flat_map(|e| {
            let mut out = vec![format!("{e:?}")];
            match e {
                MeetingEvent::Warning(w) => match w {
                    MeetingWarning::VadFailed { error, .. }
                    | MeetingWarning::LiveEngineFailed { error, .. }
                    | MeetingWarning::FinalEngineFailed { error, .. }
                    | MeetingWarning::DiarizationFailed(error) => out.push(error.to_string()),
                    MeetingWarning::StoreFailed(error) => out.push(error.to_string()),
                    MeetingWarning::SummaryFailed(error)
                    | MeetingWarning::CommitmentsFailed(error) => out.push(error.to_string()),
                    MeetingWarning::Capture { issue, .. } => out.push(issue.to_string()),
                    _ => {}
                },
                MeetingEvent::KeptLive(KeptLive::Refused(error)) => out.push(error.to_string()),
                _ => {}
            }
            out
        })
        .collect()
}

/// A model that answers every task with prose instead of JSON, quoting the meeting back: the
/// summary and the commitments fail, and their errors must not carry the quote.
struct Chatty;

impl ink_core::Llm for Chatty {
    fn info(&self) -> ink_core::LlmInfo {
        ink_core::LlmInfo {
            provider: "chatty".into(),
            model: "test".into(),
            endpoint: ink_core::Endpoint::InProcess,
        }
    }
    fn complete(
        &self,
        request: &LlmRequest,
        _: &CancelToken,
    ) -> Result<ink_core::LlmResponse, LlmError> {
        Ok(ink_core::LlmResponse {
            text: format!("Sure! You said: {}", request.user),
        })
    }
}

#[test]
fn meeting_logs_and_events_carry_no_transcript() {
    let captured = captured();
    // Every final-pass answer is the secret; the second mic region comes back empty.
    let answer: Answer = Arc::new(|channel, n, audio| {
        if channel == Channel::Mic && n == 2 {
            Ok(words("", audio.len()))
        } else {
            Ok(words(&format!("I'll {SECRET} by Friday"), audio.len()))
        }
    });
    let mut rig = RigBuilder {
        answer,
        llm: Some(Arc::new(Chatty)),
        ..RigBuilder::default()
    }
    .build();
    *rig.live.words.lock().unwrap() = SECRET.into();
    rig.live
        .extra
        .lock()
        .unwrap()
        .push((Channel::Mic, 5_020, 6_020, SECRET.into()));
    let mic = join(&[speech(1.0, -30.0, 91), silence(8.0), speech(1.0, -30.0, 92)]);
    let far = join(&[silence(2.0), speech(2.0, -30.0, 93), silence(6.0)]);
    rig.feed(&mic, &far);
    rig.finish().unwrap();

    let events = rig.events();
    // The run went down the paths it was meant to.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, MeetingEvent::Partial { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, MeetingEvent::Final { .. }))
    );
    let warnings = rig.warnings();
    for expected in [
        |w: &MeetingWarning| matches!(w, MeetingWarning::EmptySpeechRegion { .. }),
        |w: &MeetingWarning| matches!(w, MeetingWarning::FinalWithoutSpeech { .. }),
        |w: &MeetingWarning| matches!(w, MeetingWarning::SummaryFailed(_)),
    ] {
        assert!(warnings.iter().any(expected), "{warnings:?}");
    }

    // An import of the same words.
    let store = Arc::new(MemStore::new());
    let engine = Final::new(Arc::new(|_, _, audio| Ok(words(SECRET, audio.len()))));
    import_samples(
        &join(&[silence(0.5), speech(1.0, -30.0, 94), silence(0.5)]),
        StreamFormat::CANONICAL,
        Some("memo".into()),
        &ImportServices {
            engine,
            store,
            clock: Arc::new(MockClock::new(T0_NS, T0_UNIX_MS)),
        },
        &ImportSettings::default(),
        &energy_vad(),
        &CancelToken::new(),
    )
    .unwrap();

    for line in printed(&events) {
        assert_no_secret("an event", &line);
    }
    let lines = captured.lock().unwrap().clone();
    assert!(lines.len() >= 3, "the run logged: {lines:?}");
    for line in &lines {
        assert_no_secret("a log line", line);
    }
}
