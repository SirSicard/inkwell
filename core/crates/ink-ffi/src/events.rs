//! Events as the shell receives them: JSON objects matching `schema/events.schema.json`.
//!
//! The pipeline's own events are mapped here, one arm per variant. Their enums are
//! `#[non_exhaustive]`, so a variant added later maps to the schema's `other` (and is logged)
//! until it gets its own arm. Optional fields are left out when absent, never `null`.
//!
//! The words the user said travel only in `dictation.inserted`, `meeting.partial` and
//! `meeting.final`; errors and everything else name what failed, never what was said (I5).

use ink_core::{Channel, InsertOutcome, Job, RecordId};
use ink_pipeline::events::{
    DictationEvent, Discard, TakeFailure, VadUnavailable, VoiceDetection, Warning,
};
use ink_pipeline::meeting::events::{
    ChannelPass, EchoFailure, EchoPass, EchoSearch, EchoState, KeptLive, MeetingEvent,
    MeetingWarning, Phase, RemovedEcho,
};
use ink_pipeline::meeting::watchdog::SideState;
use ink_pipeline::voicecommand::{CommandAction, RiskLevel};
use serde_json::{Map, Value, json};

/// The ABI version this core implements (`INK_ABI_VERSION` in `inkwell.h`).
pub const ABI_VERSION: u32 = 2;

/// Builds an event: `type`, then the fields that are present.
pub(crate) fn event(ty: &str, fields: &[(&str, Option<Value>)]) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::from(ty));
    for (name, value) in fields {
        if let Some(value) = value {
            map.insert((*name).into(), value.clone());
        }
    }
    Value::Object(map)
}

fn some(v: impl Into<Value>) -> Option<Value> {
    Some(v.into())
}

/// A job's schema name.
pub fn job(job: Job) -> &'static str {
    match job {
        Job::DictationFinal => "dictation_final",
        Job::MeetingFinal => "meeting_final",
        Job::LivePartials => "live_partials",
        Job::Diarization => "diarization",
        Job::VoiceActivity => "voice_activity",
        // `Job` is non_exhaustive; the router never routes a job this build does not know.
        _ => "dictation_final",
    }
}

/// The job for a schema name.
pub fn parse_job(name: &str) -> Option<Job> {
    Some(match name {
        "dictation_final" => Job::DictationFinal,
        "meeting_final" => Job::MeetingFinal,
        "live_partials" => Job::LivePartials,
        "diarization" => Job::Diarization,
        "voice_activity" => Job::VoiceActivity,
        _ => return None,
    })
}

/// A channel's schema name.
pub fn channel(c: Channel) -> &'static str {
    match c {
        Channel::Mic => "mic",
        Channel::Far => "far",
    }
}

fn vad_unavailable(why: VadUnavailable) -> &'static str {
    match why {
        VadUnavailable::ModelMissing => "model_missing",
        VadUnavailable::Downloading => "downloading",
        VadUnavailable::LoadFailed => "load_failed",
        VadUnavailable::Failed => "failed",
        _ => "other",
    }
}

fn detection(state: VoiceDetection) -> (Option<Value>, Option<Value>) {
    match state {
        VoiceDetection::Available => (some(true), None),
        VoiceDetection::Unavailable(why) => (some(false), some(vad_unavailable(why))),
    }
}

fn unmapped(what: &str) -> &'static str {
    // The variant's name only: a Debug print could carry text.
    log::warn!("events: a {what} this build has no schema name for; sent as \"other\"");
    "other"
}

/// A dictation chain's event.
pub fn dictation(e: &DictationEvent) -> Value {
    match e {
        DictationEvent::VoiceDetection(state) => {
            let (available, reason) = detection(*state);
            event(
                "dictation.voice_detection",
                &[("available", available), ("reason", reason)],
            )
        }
        DictationEvent::Started => event("dictation.started", &[]),
        DictationEvent::ShortPressIgnored => event("dictation.short_press_ignored", &[]),
        DictationEvent::Stopped => event("dictation.stopped", &[]),
        DictationEvent::Discarded(d) => {
            let (reason, live_ms) = match d {
                Discard::TooShort { live_ms } => ("too_short", some(*live_ms)),
                Discard::Silence => ("silence", None),
                Discard::NoSpeech => ("no_speech", None),
                Discard::SpeechTooShort => ("speech_too_short", None),
                Discard::NothingHeard => ("nothing_heard", None),
                Discard::NothingLeft => ("nothing_left", None),
                Discard::Cancelled => ("cancelled", None),
                _ => (unmapped("discard reason"), None),
            };
            event(
                "dictation.discarded",
                &[("reason", some(reason)), ("live_ms", live_ms)],
            )
        }
        DictationEvent::Command(action) => {
            let (name, value) = match action {
                CommandAction::Undo => ("undo", None),
                CommandAction::ChangeStyle { style } => ("change_style", some(style.as_str())),
                CommandAction::SwitchModel { model } => ("switch_model", some(model.as_str())),
                CommandAction::TogglePolish => ("toggle_polish", None),
                CommandAction::ToggleDictation => ("toggle_dictation", None),
                CommandAction::OpenUrl { url } => ("open_url", some(url.as_str())),
                CommandAction::OpenApp { path } => ("open_app", some(path.as_str())),
                CommandAction::InsertText { text } => ("insert_text", some(text.as_str())),
                _ => (unmapped("voice command"), None),
            };
            let risk = match action.risk() {
                RiskLevel::Safe => "safe",
                RiskLevel::Moderate => "moderate",
                RiskLevel::Dangerous => "dangerous",
            };
            event(
                "dictation.command",
                &[
                    ("action", some(name)),
                    ("risk", some(risk)),
                    ("value", value),
                ],
            )
        }
        DictationEvent::Inserted {
            text,
            outcome,
            record,
        } => event(
            "dictation.inserted",
            &[
                ("text", some(text.as_str())),
                ("outcome", some(insert_outcome(*outcome))),
                ("record", record.as_ref().map(|r| Value::from(r.0.as_str()))),
            ],
        ),
        DictationEvent::Failed(f) => {
            let (stage, message) = match f {
                TakeFailure::Transcription(e) => ("transcription", e.to_string()),
                TakeFailure::Insert(e) => ("insert", e.to_string()),
                _ => (unmapped("take failure"), String::new()),
            };
            event(
                "dictation.failed",
                &[("stage", some(stage)), ("message", some(message))],
            )
        }
        DictationEvent::Warning(w) => {
            let (kind, frames, message) = match w {
                Warning::VadFailed(e) => ("vad_failed", None, some(e.to_string())),
                Warning::AudioLost { frames } => ("audio_lost", some(*frames), None),
                Warning::TailCutShort => ("tail_cut_short", None, None),
                Warning::FocusUnreadable(e) => ("focus_unreadable", None, some(e.to_string())),
                Warning::PolishUnavailable => ("polish_unavailable", None, None),
                Warning::PolishFailed(e) => ("polish_failed", None, some(e.to_string())),
                Warning::PolishTimedOut => ("polish_timed_out", None, None),
                Warning::NoModeForStyle => ("no_mode_for_style", None, None),
                Warning::SaveFailed(e) => ("save_failed", None, some(e.to_string())),
                Warning::DeletedTextNotScrubbed => ("deleted_text_not_scrubbed", None, None),
                Warning::DeletedTextScrubbed => ("deleted_text_scrubbed", None, None),
                _ => (unmapped("dictation warning"), None, None),
            };
            event(
                "dictation.warning",
                &[
                    ("kind", some(kind)),
                    ("frames", frames),
                    ("message", message),
                ],
            )
        }
        DictationEvent::HotkeyLost => event("dictation.hotkey_lost", &[]),
        DictationEvent::WorkerFailed { recovered } => worker_failed(*recovered),
        _ => event(
            "dictation.warning",
            &[("kind", some(unmapped("dictation event")))],
        ),
    }
}

/// `dictation.worker_failed`. Without recovery the shell must unbind the hotkey, and the event
/// says so in a field of its own so no shell has to derive it.
pub fn worker_failed(recovered: bool) -> Value {
    event(
        "dictation.worker_failed",
        &[
            ("recovered", some(recovered)),
            ("unbind_hotkey", some(!recovered)),
        ],
    )
}

fn insert_outcome(o: InsertOutcome) -> &'static str {
    match o {
        InsertOutcome::Pasted => "pasted",
        InsertOutcome::Typed => "typed",
        InsertOutcome::Blocked => "blocked",
        InsertOutcome::InsertedClipboardNotRestored => "inserted_clipboard_not_restored",
    }
}

fn phase(p: Phase) -> &'static str {
    match p {
        Phase::Live => "live",
        Phase::Final => "final",
    }
}

fn side_state(s: SideState) -> &'static str {
    match s {
        SideState::Ok => "ok",
        SideState::Stopped => "stopped",
        SideState::Zeros => "zeros",
    }
}

fn pass(p: &ChannelPass) -> Value {
    event_fields(&[
        ("channel", some(channel(p.channel))),
        ("chunks_written", p.chunks_written.map(Value::from)),
        ("chunks", some(p.chunks)),
        ("captured_ms", some(p.captured_ms)),
        ("backlogged_finals", some(p.backlogged_finals)),
        ("audible_ms", some(p.audible_ms)),
        ("regions", some(p.regions)),
        ("empty_regions", some(p.empty_regions)),
        ("failed_regions", some(p.failed_regions)),
        ("word_count", some(p.word_count)),
        ("speech_ms", some(p.speech_ms)),
    ])
}

fn event_fields(fields: &[(&str, Option<Value>)]) -> Value {
    let mut map = Map::new();
    for (name, value) in fields {
        if let Some(value) = value {
            map.insert((*name).into(), value.clone());
        }
    }
    Value::Object(map)
}

fn echo_search(why: EchoSearch) -> &'static str {
    match why {
        EchoSearch::Start => "start",
        EchoSearch::DeviceSwitch => "device_switch",
        EchoSearch::AfterFailure => "after_failure",
    }
}

/// An echo failure's name, and the side that ran ahead for a backlog.
fn echo_failure(f: EchoFailure) -> [(&'static str, Option<Value>); 2] {
    match f {
        EchoFailure::Backlog { ahead } => [
            ("failure", some("backlog")),
            ("channel", some(channel(ahead))),
        ],
        EchoFailure::BadAlignment => [("failure", some("bad_alignment")), ("channel", None)],
        EchoFailure::Stalled => [("failure", some("stalled")), ("channel", None)],
        EchoFailure::Internal => [("failure", some("internal")), ("channel", None)],
    }
}

/// A measured number: omitted when it is not finite (JSON has no NaN), and a level in dB to
/// hundredths.
fn number(x: f64) -> Option<Value> {
    x.is_finite().then(|| Value::from(x))
}

fn db(x: f32) -> Option<Value> {
    number((f64::from(x) * 100.0).round() / 100.0)
}

/// `meeting.echo`'s fields for one state.
fn echo_state(s: &EchoState) -> Vec<(&'static str, Option<Value>)> {
    let state = |name: &'static str| ("state", some(name));
    match *s {
        EchoState::Searching { since_ms, why } => vec![
            state("searching"),
            ("since_ms", some(since_ms)),
            ("why", some(echo_search(why))),
        ],
        EchoState::Cancelling {
            from_ms,
            unprotected_ms,
            stable_from_ms,
            delay_ms,
            drift_ppm,
        } => vec![
            state("cancelling"),
            ("from_ms", some(from_ms)),
            ("unprotected_ms", some(unprotected_ms)),
            ("stable_from_ms", stable_from_ms.map(Value::from)),
            ("delay_ms", number(delay_ms)),
            ("drift_ppm", number(drift_ppm)),
        ],
        EchoState::Degraded { erle_db } => vec![state("degraded"), ("erle_db", db(erle_db))],
        EchoState::Failed(f) => [vec![state("failed")], echo_failure(f).to_vec()].concat(),
        EchoState::FoundAtEnd {
            unprotected_ms,
            stable_from_ms,
        } => vec![
            state("found_at_end"),
            ("unprotected_ms", some(unprotected_ms)),
            ("stable_from_ms", stable_from_ms.map(Value::from)),
        ],
    }
}

fn echo_pass(p: &EchoPass) -> Vec<(&'static str, Option<Value>)> {
    let path = p.path.map(|path| {
        event_fields(&[
            ("delay_ms", number(path.delay_ms)),
            ("drift_ppm", number(path.drift_ppm)),
            ("inliers", some(path.inliers)),
            ("stable_from_ms", path.stable_from_ms.map(Value::from)),
        ])
    });
    vec![
        ("path", path),
        ("windows", some(p.windows)),
        ("candidates", some(p.candidates)),
        ("cancelled", some(p.cancelled)),
        ("erle_first_db", p.erle_first_db.and_then(db)),
        ("erle_db", p.erle_db.and_then(db)),
        ("linear_erle_db", p.linear_erle_db.and_then(db)),
        ("removed", some(p.removed)),
        ("kept_near_speech", some(p.kept_near_speech)),
        ("kept_no_evidence", some(p.kept_no_evidence)),
        ("live_echo_finals", some(p.live_echo_finals)),
    ]
}

/// The lines removed as echo, by their place in the record's removed lines (the store keeps them
/// in this order, by start time) and their spans: never their words, which stay in the store.
fn removed_lines(lines: &[RemovedEcho]) -> Value {
    Value::Array(
        lines
            .iter()
            .enumerate()
            .map(|(index, l)| {
                let far = l
                    .far
                    .iter()
                    .map(|f| json!({"start_ms": f.start_ms, "end_ms": f.end_ms}))
                    .collect::<Vec<_>>();
                event_fields(&[
                    ("index", some(index)),
                    ("start_ms", some(l.start_ms)),
                    ("end_ms", some(l.end_ms)),
                    ("far", some(far)),
                    ("words", some(l.words)),
                    ("matched", some(l.matched)),
                ])
            })
            .collect(),
    )
}

/// The fields of `meeting.warning` for one warning: its kind, then what it carries.
fn meeting_warning(w: &MeetingWarning) -> Vec<(&'static str, Option<Value>)> {
    let ch = |c: &Channel| ("channel", some(channel(*c)));
    let msg = |m: String| ("message", some(m));
    let span = |s: &u64, e: &u64| [("start_ms", some(*s)), ("end_ms", some(*e))];
    let kind = |k: &'static str| ("kind", some(k));
    match w {
        MeetingWarning::VadFailed {
            channel: c,
            phase: p,
            error,
        } => vec![
            kind("vad_failed"),
            ch(c),
            ("phase", some(phase(*p))),
            msg(error.to_string()),
        ],
        MeetingWarning::Capture { channel: c, issue } => {
            vec![kind("capture"), ch(c), msg(issue.to_string())]
        }
        MeetingWarning::AudioLost { channel: c, frames } => {
            vec![kind("audio_lost"), ch(c), ("frames", some(*frames))]
        }
        MeetingWarning::LiveEngineFailed { channel: c, error } => {
            vec![kind("live_engine_failed"), ch(c), msg(error.to_string())]
        }
        MeetingWarning::LiveFinalsBacklog { channel: c } => {
            vec![kind("live_finals_backlog"), ch(c)]
        }
        MeetingWarning::LiveEventsAfterStop { count } => {
            vec![kind("live_events_after_stop"), ("count", some(*count))]
        }
        MeetingWarning::FarEndQuietWhileYouSpeak { quiet_ms } => vec![
            kind("far_end_quiet_while_you_speak"),
            ("quiet_ms", some(*quiet_ms)),
        ],
        MeetingWarning::LiveEngineStalled { channel: c } => {
            vec![kind("live_engine_stalled"), ch(c)]
        }
        MeetingWarning::FinalWithoutSpeech {
            channel: c,
            start_ms,
            end_ms,
        } => [
            vec![kind("final_without_speech"), ch(c)],
            span(start_ms, end_ms).to_vec(),
        ]
        .concat(),
        MeetingWarning::EmptySpeechRegion {
            channel: c,
            start_ms,
            end_ms,
        } => [
            vec![kind("empty_speech_region"), ch(c)],
            span(start_ms, end_ms).to_vec(),
        ]
        .concat(),
        MeetingWarning::FinalEngineFailed {
            channel: c,
            start_ms,
            end_ms,
            error,
        } => [
            vec![kind("final_engine_failed"), ch(c), msg(error.to_string())],
            span(start_ms, end_ms).to_vec(),
        ]
        .concat(),
        MeetingWarning::AudioUnreadable { channel: c, chunks } => {
            vec![kind("audio_unreadable"), ch(c), ("chunks", some(*chunks))]
        }
        MeetingWarning::LittleSpeechHeard {
            channel: c,
            audible_ms,
            speech_ms,
        } => vec![
            kind("little_speech_heard"),
            ch(c),
            ("audible_ms", some(*audible_ms)),
            ("speech_ms", some(*speech_ms)),
        ],
        MeetingWarning::CapturedOnlyZeros { channel: c } => {
            vec![kind("captured_only_zeros"), ch(c)]
        }
        MeetingWarning::BluetoothMicOnlyZeros => {
            vec![kind("bluetooth_mic_only_zeros"), ("channel", some("mic"))]
        }
        MeetingWarning::AudioUnlisted { channel: c, reason } => {
            vec![kind("audio_unlisted"), ch(c), msg(reason.clone())]
        }
        MeetingWarning::NothingCaptured { channel: c } => vec![kind("nothing_captured"), ch(c)],
        MeetingWarning::DiarizationFailed(e) => vec![
            kind("diarization_failed"),
            ("channel", some("far")),
            msg(e.to_string()),
        ],
        MeetingWarning::StoreFailed(e) => vec![kind("store_failed"), msg(e.to_string())],
        MeetingWarning::ClockWentBack => vec![kind("clock_went_back")],
        MeetingWarning::SummaryUnavailable => vec![kind("summary_unavailable")],
        MeetingWarning::SummaryFailed(e) => vec![kind("summary_failed"), msg(e.to_string())],
        MeetingWarning::CommitmentsFailed(e) => {
            vec![kind("commitments_failed"), msg(e.to_string())]
        }
        MeetingWarning::EchoOnlyFinal { start_ms, end_ms } => [
            vec![kind("echo_only_final"), ("channel", some("mic"))],
            span(start_ms, end_ms).to_vec(),
        ]
        .concat(),
        MeetingWarning::EchoGateVadFailed(e) => vec![
            kind("echo_gate_vad_failed"),
            ("channel", some("mic")),
            ("phase", some("live")),
            msg(e.to_string()),
        ],
        MeetingWarning::DeletedTextNotScrubbed => vec![kind("deleted_text_not_scrubbed")],
        MeetingWarning::DeletedTextScrubbed => vec![kind("deleted_text_scrubbed")],
        MeetingWarning::NotCrashProtected(why) => {
            vec![kind("not_crash_protected"), msg(why.clone())]
        }
        MeetingWarning::EchoPathNotFound { heard_ms } => vec![
            kind("echo_path_not_found"),
            ("channel", some("mic")),
            ("phase", some("final")),
            ("audible_ms", some(*heard_ms)),
        ],
        MeetingWarning::EchoFailed(f) => {
            let mut fields = vec![kind("echo_failed"), ("phase", some("final"))];
            fields.extend(echo_failure(*f));
            // A backlog names the side that ran ahead; otherwise it is the mic's pass.
            if !matches!(f, EchoFailure::Backlog { .. }) {
                fields.push(("channel", some("mic")));
            }
            fields
        }
        _ => vec![kind(unmapped("meeting warning"))],
    }
}

/// A meeting chain's event, for the meeting `record`.
pub fn meeting(record: &RecordId, e: &MeetingEvent) -> Value {
    let rec = ("record", some(record.0.as_str()));
    match e {
        MeetingEvent::Started { record } => {
            event("meeting.started", &[("record", some(record.0.as_str()))])
        }
        MeetingEvent::VoiceDetection { channel: c, state } => {
            let (available, reason) = detection(*state);
            event(
                "meeting.voice_detection",
                &[
                    rec,
                    ("channel", some(channel(*c))),
                    ("available", available),
                    ("reason", reason),
                ],
            )
        }
        MeetingEvent::SideState { channel: c, state } => event(
            "meeting.side_state",
            &[
                rec,
                ("channel", some(channel(*c))),
                ("state", some(side_state(*state))),
            ],
        ),
        MeetingEvent::Partial { channel: c, text } => event(
            "meeting.partial",
            &[
                rec,
                ("channel", some(channel(*c))),
                ("text", some(text.as_str())),
            ],
        ),
        MeetingEvent::Final {
            channel: c,
            start_ms,
            end_ms,
            text,
        } => event(
            "meeting.final",
            &[
                rec,
                ("channel", some(channel(*c))),
                ("start_ms", some(*start_ms)),
                ("end_ms", some(*end_ms)),
                ("text", some(text.as_str())),
            ],
        ),
        MeetingEvent::Warning(w) => {
            let mut fields = vec![rec];
            fields.extend(meeting_warning(w));
            event("meeting.warning", &fields)
        }
        MeetingEvent::Stopped => event("meeting.stopped", &[rec]),
        MeetingEvent::Transcribed(p) => {
            event("meeting.transcribed", &[rec, ("pass", Some(pass(p)))])
        }
        MeetingEvent::Diarized(d) => event(
            "meeting.diarized",
            &[
                rec,
                ("clusters", some(d.clusters)),
                ("substantial", some(d.substantial)),
                ("labelled", some(d.labelled)),
                ("attributed", some(d.attributed)),
            ],
        ),
        MeetingEvent::Echo(state) => {
            event("meeting.echo", &[vec![rec], echo_state(state)].concat())
        }
        MeetingEvent::EchoPass(p) => {
            event("meeting.echo_pass", &[vec![rec], echo_pass(p)].concat())
        }
        MeetingEvent::RemovedAsEcho(lines) => event(
            "meeting.removed_as_echo",
            &[rec, ("lines", Some(removed_lines(lines)))],
        ),
        MeetingEvent::Superseded { revision } => {
            event("meeting.superseded", &[rec, ("revision", some(*revision))])
        }
        MeetingEvent::KeptLive(k) => {
            let fields: Vec<(&str, Option<Value>)> = match k {
                KeptLive::Refused(e) => vec![
                    ("reason", some("refused")),
                    ("message", some(e.to_string())),
                ],
                KeptLive::Incomplete { failed_regions } => vec![
                    ("reason", some("incomplete")),
                    ("failed_regions", some(*failed_regions)),
                ],
                _ => vec![("reason", some(unmapped("kept-live reason")))],
            };
            event("meeting.kept_live", &[[rec].to_vec(), fields].concat())
        }
        MeetingEvent::Summarized { unverified } => event(
            "meeting.summarized",
            &[rec, ("unverified", some(*unverified))],
        ),
        MeetingEvent::Commitments { filed, merged } => event(
            "meeting.commitments",
            &[rec, ("filed", some(*filed)), ("merged", some(*merged))],
        ),
        MeetingEvent::LooksDone { suggested } => event(
            "meeting.looks_done",
            &[rec, ("suggested", some(*suggested))],
        ),
        MeetingEvent::Finished { revision } => event(
            "meeting.finished",
            &[rec, ("revision", revision.map(Value::from))],
        ),
        _ => event(
            "meeting.warning",
            &[rec, ("kind", some(unmapped("meeting event")))],
        ),
    }
}

/// `command.failed`.
pub fn command_failed(command: &str, id: Option<&str>, message: &str) -> Value {
    event(
        "command.failed",
        &[
            ("command", some(command)),
            ("id", id.map(Value::from)),
            ("message", some(message)),
        ],
    )
}

/// `core.ready`.
pub fn ready() -> Value {
    json!({"type": "core.ready", "abi": ABI_VERSION, "version": env!("CARGO_PKG_VERSION")})
}

#[cfg(test)]
mod tests {
    use ink_core::{EngineError, LlmError, PlatformError, StoreError};
    use ink_pipeline::capture::CaptureIssue;
    use ink_pipeline::meeting::events::{Diarization, EchoPath, FarLine};
    use ink_pipeline::redact::Spoken;

    use super::*;
    use crate::schema::{EVENTS_SCHEMA, Schema};

    /// Every variant the pipeline has today, mapped and checked against the schema: the Rust side
    /// cannot emit anything the generated Swift fails to decode.
    #[test]
    fn every_mapped_event_matches_the_schema() {
        let schema = Schema::parse(EVENTS_SCHEMA).unwrap();
        let e = || EngineError::Failed("engine x".into());
        let record = RecordId("r1".into());
        let mut all = vec![
            ready(),
            command_failed("x", Some("7"), "why"),
            worker_failed(false),
        ];
        let dictation_events = vec![
            DictationEvent::VoiceDetection(VoiceDetection::Available),
            DictationEvent::VoiceDetection(VoiceDetection::Unavailable(
                VadUnavailable::Downloading,
            )),
            DictationEvent::Started,
            DictationEvent::ShortPressIgnored,
            DictationEvent::Stopped,
            DictationEvent::Discarded(Discard::TooShort { live_ms: 120 }),
            DictationEvent::Discarded(Discard::SpeechTooShort),
            DictationEvent::Discarded(Discard::NothingLeft),
            DictationEvent::Command(CommandAction::OpenUrl {
                url: "https://example.com".into(),
            }),
            DictationEvent::Command(CommandAction::Undo),
            DictationEvent::Inserted {
                text: Spoken::new("hello"),
                outcome: InsertOutcome::InsertedClipboardNotRestored,
                record: Some(record.clone()),
            },
            DictationEvent::Inserted {
                text: Spoken::new("hello"),
                outcome: InsertOutcome::Blocked,
                record: None,
            },
            DictationEvent::Failed(TakeFailure::Transcription(e())),
            DictationEvent::Failed(TakeFailure::Insert(PlatformError::Failed("x".into()))),
            DictationEvent::Warning(Warning::AudioLost { frames: 480 }),
            DictationEvent::Warning(Warning::PolishFailed(LlmError::NoKey)),
            DictationEvent::Warning(Warning::PolishTimedOut),
            DictationEvent::Warning(Warning::SaveFailed(StoreError::NotFound)),
            DictationEvent::Warning(Warning::DeletedTextNotScrubbed),
            DictationEvent::Warning(Warning::DeletedTextScrubbed),
            DictationEvent::HotkeyLost,
            DictationEvent::WorkerFailed { recovered: true },
        ];
        all.extend(dictation_events.iter().map(dictation));
        let p = ChannelPass {
            channel: Channel::Far,
            chunks_written: None,
            chunks: 3,
            captured_ms: 30_000,
            backlogged_finals: 0,
            audible_ms: 20_000,
            regions: 4,
            empty_regions: 1,
            failed_regions: 0,
            word_count: 50,
            speech_ms: 15_000,
        };
        let warnings = vec![
            MeetingWarning::VadFailed {
                channel: Channel::Mic,
                phase: Phase::Final,
                error: e(),
            },
            MeetingWarning::Capture {
                channel: Channel::Far,
                issue: CaptureIssue::RateMismatch {
                    declared_hz: 48_000,
                    measured_hz: 44_100,
                },
            },
            MeetingWarning::AudioLost {
                channel: Channel::Mic,
                frames: 3,
            },
            MeetingWarning::LiveEventsAfterStop { count: 2 },
            MeetingWarning::FarEndQuietWhileYouSpeak { quiet_ms: 60_000 },
            MeetingWarning::FinalEngineFailed {
                channel: Channel::Far,
                start_ms: 1,
                end_ms: 2,
                error: e(),
            },
            MeetingWarning::AudioUnreadable {
                channel: Channel::Mic,
                chunks: 1,
            },
            MeetingWarning::LittleSpeechHeard {
                channel: Channel::Mic,
                audible_ms: 1,
                speech_ms: 0,
            },
            MeetingWarning::BluetoothMicOnlyZeros,
            MeetingWarning::AudioUnlisted {
                channel: Channel::Far,
                reason: "gone".into(),
            },
            MeetingWarning::DiarizationFailed(e()),
            MeetingWarning::StoreFailed(StoreError::NotFound),
            MeetingWarning::ClockWentBack,
            MeetingWarning::SummaryUnavailable,
            MeetingWarning::CommitmentsFailed(LlmError::Cancelled),
            MeetingWarning::EchoOnlyFinal {
                start_ms: 2_000,
                end_ms: 4_000,
            },
            MeetingWarning::EchoGateVadFailed(e()),
            MeetingWarning::EchoFailed(EchoFailure::Backlog {
                ahead: Channel::Far,
            }),
            MeetingWarning::EchoFailed(EchoFailure::Internal),
            MeetingWarning::EchoPathNotFound { heard_ms: 12_000 },
            MeetingWarning::DeletedTextNotScrubbed,
            MeetingWarning::DeletedTextScrubbed,
            MeetingWarning::NotCrashProtected("the marker could not be written".into()),
        ];
        let mut meeting_events = vec![
            MeetingEvent::Started {
                record: record.clone(),
            },
            MeetingEvent::VoiceDetection {
                channel: Channel::Far,
                state: VoiceDetection::Unavailable(VadUnavailable::ModelMissing),
            },
            MeetingEvent::SideState {
                channel: Channel::Mic,
                state: SideState::Zeros,
            },
            MeetingEvent::Partial {
                channel: Channel::Mic,
                text: Spoken::new("hi"),
            },
            MeetingEvent::Final {
                channel: Channel::Far,
                start_ms: 0,
                end_ms: 10,
                text: Spoken::new("hi"),
            },
            MeetingEvent::Stopped,
            MeetingEvent::Transcribed(p),
            MeetingEvent::Diarized(Diarization {
                clusters: 3,
                substantial: 2,
                labelled: true,
                attributed: 9,
            }),
            MeetingEvent::Superseded { revision: 2 },
            MeetingEvent::KeptLive(KeptLive::Refused(StoreError::EmptySupersede)),
            MeetingEvent::KeptLive(KeptLive::Incomplete { failed_regions: 2 }),
            MeetingEvent::Summarized { unverified: 1 },
            MeetingEvent::Commitments {
                filed: 3,
                merged: 1,
            },
            MeetingEvent::LooksDone { suggested: 2 },
            MeetingEvent::Finished { revision: Some(2) },
            MeetingEvent::Finished { revision: None },
            MeetingEvent::Echo(EchoState::Searching {
                since_ms: 0,
                why: EchoSearch::Start,
            }),
            MeetingEvent::Echo(EchoState::Searching {
                since_ms: 25_000,
                why: EchoSearch::DeviceSwitch,
            }),
            MeetingEvent::Echo(EchoState::Searching {
                since_ms: 30_000,
                why: EchoSearch::AfterFailure,
            }),
            MeetingEvent::Echo(EchoState::Cancelling {
                from_ms: 10_000,
                unprotected_ms: 10_000,
                stable_from_ms: Some(7_000),
                delay_ms: -4.5,
                drift_ppm: 1.6,
            }),
            MeetingEvent::Echo(EchoState::Cancelling {
                from_ms: 30_000,
                unprotected_ms: 0,
                stable_from_ms: None,
                delay_ms: 46.0,
                drift_ppm: 0.0,
            }),
            MeetingEvent::Echo(EchoState::Degraded { erle_db: -1.5 }),
            MeetingEvent::Echo(EchoState::Failed(EchoFailure::Backlog {
                ahead: Channel::Mic,
            })),
            MeetingEvent::Echo(EchoState::Failed(EchoFailure::BadAlignment)),
            MeetingEvent::Echo(EchoState::Failed(EchoFailure::Stalled)),
            MeetingEvent::Echo(EchoState::FoundAtEnd {
                unprotected_ms: 45_000,
                stable_from_ms: Some(9_000),
            }),
            MeetingEvent::EchoPass(EchoPass::default()),
            MeetingEvent::EchoPass(echo_pass()),
            MeetingEvent::RemovedAsEcho(vec![removed_line()]),
        ];
        meeting_events.extend(warnings.into_iter().map(MeetingEvent::Warning));
        all.extend(meeting_events.iter().map(|m| meeting(&record, m)));
        for v in &all {
            if let Err(e) = schema.validate(v) {
                panic!("{v} does not match the schema: {e}");
            }
        }
    }

    fn echo_pass() -> EchoPass {
        EchoPass {
            path: Some(EchoPath {
                delay_ms: 46.04,
                drift_ppm: 1.63,
                inliers: 64,
                stable_from_ms: Some(7_000),
            }),
            windows: 81,
            candidates: 72,
            cancelled: true,
            erle_first_db: Some(25.75),
            erle_db: Some(26.0),
            linear_erle_db: Some(11.5),
            removed: 1,
            kept_near_speech: 2,
            kept_no_evidence: 0,
            live_echo_finals: 3,
        }
    }

    fn removed_line() -> RemovedEcho {
        RemovedEcho {
            start_ms: 19_700,
            end_ms: 20_500,
            text: Spoken::new("the budget is due on friday"),
            far: vec![FarLine {
                start_ms: 19_550,
                end_ms: 20_550,
            }],
            words: 6,
            matched: 6,
        }
    }

    #[test]
    fn the_echo_state_and_pass_reach_the_shell_field_by_field() {
        let record = RecordId("r1".into());
        let found = meeting(
            &record,
            &MeetingEvent::Echo(EchoState::Cancelling {
                from_ms: 10_000,
                unprotected_ms: 10_000,
                stable_from_ms: Some(7_000),
                delay_ms: 46.5,
                drift_ppm: 1.5,
            }),
        );
        assert_eq!(
            found,
            json!({"type": "meeting.echo", "record": "r1", "state": "cancelling",
                   "from_ms": 10_000, "unprotected_ms": 10_000, "stable_from_ms": 7_000,
                   "delay_ms": 46.5, "drift_ppm": 1.5})
        );
        let failed = meeting(
            &record,
            &MeetingEvent::Echo(EchoState::Failed(EchoFailure::Backlog {
                ahead: Channel::Far,
            })),
        );
        assert_eq!(
            failed,
            json!({"type": "meeting.echo", "record": "r1", "state": "failed",
                   "failure": "backlog", "channel": "far"})
        );
        let pass = meeting(&record, &MeetingEvent::EchoPass(echo_pass()));
        assert_eq!(pass["type"], "meeting.echo_pass");
        assert_eq!(pass["path"]["delay_ms"], 46.04);
        assert_eq!(pass["live_echo_finals"], 3);
        let none = meeting(&record, &MeetingEvent::EchoPass(EchoPass::default()));
        assert!(
            none.get("path").is_none() && none.get("erle_db").is_none(),
            "{none}"
        );
    }

    /// The lines removed as echo go to the shell as their place in the record's removed list and
    /// their spans; the words stay in the store (`Store::removed`).
    #[test]
    fn lines_removed_as_echo_reach_the_shell_without_their_words() {
        let record = RecordId("r1".into());
        let mut second = removed_line();
        second.start_ms = 30_000;
        second.end_ms = 31_000;
        let v = meeting(
            &record,
            &MeetingEvent::RemovedAsEcho(vec![removed_line(), second]),
        );
        assert_eq!(
            v,
            json!({"type": "meeting.removed_as_echo", "record": "r1", "lines": [
                {"index": 0, "start_ms": 19_700, "end_ms": 20_500,
                 "far": [{"start_ms": 19_550, "end_ms": 20_550}], "words": 6, "matched": 6},
                {"index": 1, "start_ms": 30_000, "end_ms": 31_000,
                 "far": [{"start_ms": 19_550, "end_ms": 20_550}], "words": 6, "matched": 6},
            ]})
        );
        assert!(!v.to_string().contains("budget"));
    }
}
