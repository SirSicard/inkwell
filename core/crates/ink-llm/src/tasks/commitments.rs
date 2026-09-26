//! Commitments: what the user promised in a meeting, with where they said it.
//!
//! Two stages, ported from an earlier implementation:
//!
//! 1. **Spot** ([`spot`]): cheap, deterministic, high-recall matching of first-person promise
//!    phrases, **on the mic channel only**. The mic is the user with no attribution error, and a
//!    commitment is a first-person act, so the whole "someone else said they'd do it" class of
//!    false positive is removed by which stream the audio came from, not by a model. Recall over
//!    precision: a promise missed here is gone, while a false candidate costs the judge one
//!    answer. Negation and hypotheticals are left to the judge.
//! 2. **Judge** ([`judge_request`], [`parse_judgement`]): a model sees each candidate with about
//!    90 seconds of conversation either side and picks one of six classes, quoting the sentence.
//!    Context is what separates them: "I'll send the draft agenda to the design team" is a
//!    commitment on its own, and a hypothetical right after "what if you…". A forced choice
//!    between six classes invites thought where "is this a commitment?" invites yes.
//!
//! The judge's quote must appear verbatim in the candidate sentence ([`quote_holds`]); an answer
//! that cannot quote is dropped as a hallucination. The judge's answer is validated against the
//! judge's own shape, never another task's (see [`crate::tasks`]).

use ink_core::{
    CancelToken, Channel, Llm, LlmError, LlmRequest, NewCommitment, Segment, Span, SpeakerId,
};

use super::due::{RecordTime, resolve_due};
use super::{ask, is_fatal, quote_found, transcript};
use crate::json::{Fields, bad_field, object_in};

/// First-person promise openers, lowercase, matched at word boundaries. "We'll" is in: on the
/// mic, "we" usually includes the speaker, and the judge separates it from "someone on my team
/// will". "I can" is out: it states ability, not intent, and floods the judge with options talk.
pub const TRIGGERS: &[&str] = &[
    "i'll",
    "i will",
    "i shall",
    "let me",
    "leave that with me",
    "leave it with me",
    "i'm going to",
    "i'm gonna",
    "i am going to",
    "we'll",
    "we will",
    "i need to",
    "i have to",
    "i must",
    "i promise",
    "my job to",
    "i'm on it",
    "i take",
];

/// A sentence worth judging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The phrase that matched.
    pub trigger: &'static str,
    /// The sentence containing it: what the judge classifies and must quote from.
    pub sentence: String,
    /// The segment's index in the record's segments.
    pub segment: usize,
    /// The segment's span: the commitment's provenance.
    pub span: Span,
}

/// **Any thread except realtime.** Spots candidates in the **mic** segments of `segments`; far-end
/// segments are skipped here, not by the caller, so they cannot slip in. One candidate per
/// sentence, first trigger wins: "I'll check and I'll send it" is one sentence to judge.
pub fn spot(segments: &[Segment]) -> Vec<Candidate> {
    let mut found = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        if segment.channel != Channel::Mic {
            continue;
        }
        for sentence in split_sentences(&segment.text) {
            let lowered = sentence.to_lowercase().replace('\u{2019}', "'");
            if let Some(trigger) = TRIGGERS.iter().find(|t| contains_phrase(&lowered, t)) {
                found.push(Candidate {
                    trigger,
                    sentence: sentence.to_owned(),
                    segment: index,
                    span: Span {
                        channel: segment.channel,
                        start_ms: segment.start_ms,
                        end_ms: segment.end_ms,
                    },
                });
            }
        }
    }
    found
}

/// Whether `phrase` occurs in `text` at word boundaries: "i'll" must not match inside "still".
fn contains_phrase(text: &str, phrase: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = text.get(from..).and_then(|rest| rest.find(phrase)) {
        let start = from + offset;
        let end = start + phrase.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphabetic());
        let after_ok = text[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphabetic());
        if before_ok && after_ok {
            return true;
        }
        // Step past this match's first character, staying on a char boundary.
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// Sentences in ASR text, which has punctuation but no reliable structure. Segments are already
/// utterance-sized; this only splits long ones.
fn split_sentences(text: &str) -> impl Iterator<Item = &str> {
    text.split(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The judge's six classes, in the order the prompt lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JudgeClass {
    /// The speaker took on a concrete obligation to do something after the meeting.
    Commitment,
    /// Conditional or exploratory.
    Hypothetical,
    /// Weighing possibilities; no obligation taken.
    OptionDiscussion,
    /// Refused, or walked back in the context shown.
    DeclinedOrRetracted,
    /// The obligation lands on someone else.
    Delegated,
    /// Refers to finished work.
    AlreadyDone,
}

impl JudgeClass {
    /// Every class.
    pub const ALL: [Self; 6] = [
        Self::Commitment,
        Self::Hypothetical,
        Self::OptionDiscussion,
        Self::DeclinedOrRetracted,
        Self::Delegated,
        Self::AlreadyDone,
    ];

    /// The name the model answers with.
    pub fn name(self) -> &'static str {
        match self {
            Self::Commitment => "commitment",
            Self::Hypothetical => "hypothetical",
            Self::OptionDiscussion => "option_discussion",
            Self::DeclinedOrRetracted => "declined_or_retracted",
            Self::Delegated => "delegated",
            Self::AlreadyDone => "already_done",
        }
    }
}

/// The judge's answer, as JSON Schema: a grammar for a local model, documentation for the rest.
/// [`parse_judgement`] enforces the same shape; a test keeps the two in step.
pub const JUDGE_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "class": {"type": "string", "enum": ["commitment", "hypothetical", "option_discussion", "declined_or_retracted", "delegated", "already_done"]},
    "confidence": {"type": "number", "minimum": 0, "maximum": 1},
    "task": {"type": ["string", "null"]},
    "due": {"type": ["string", "null"]},
    "quote": {"type": "string"}
  },
  "required": ["class", "confidence", "task", "due", "quote"],
  "additionalProperties": false
}"#;

const JUDGE_SYSTEM: &str = r#"You classify ONE sentence from a meeting transcript. The sentence was said by the user, shown as "You". That is certain: it comes from their own microphone.

Pick exactly one class:
- commitment: the speaker took on a concrete obligation to do something after the meeting
- hypothetical: conditional or exploratory ("if we did that, I'd ...")
- option_discussion: weighing possibilities, no obligation taken
- declined_or_retracted: refused, or walked back within the context shown
- delegated: the obligation lands on someone else
- already_done: refers to work that is already finished

Answer with JSON only, no prose:
{"class": "one of the six", "confidence": 0.0 to 1.0, "task": "the obligation as an instruction, for example \"Send the draft agenda to the design team\", or null unless the class is commitment", "due": "the deadline exactly as said, or null", "quote": "a verbatim part of the sentence that proves the class"}

Copy a deadline as said ("Friday"); never work out a date. The quote must appear word for word in the sentence. If you cannot quote it, the class is wrong."#;

/// About how much conversation the judge sees either side of the sentence.
pub const CONTEXT_MS: u64 = 90_000;

/// What the judge and the resolver need to know about the record.
#[derive(Clone, Copy, Debug)]
pub struct RecordContext<'a> {
    /// The record's title, if it has one.
    pub title: Option<&'a str>,
    /// When it started, for the prompt's date and for resolving deadlines.
    pub time: RecordTime,
    /// Names the user gave far-end speakers.
    pub speaker_names: &'a [(SpeakerId, String)],
}

/// The judge's request for one candidate, with the lines around it.
pub fn judge_request(
    candidate: &Candidate,
    segments: &[Segment],
    record: &RecordContext<'_>,
) -> LlmRequest {
    let from = candidate.span.start_ms.saturating_sub(CONTEXT_MS);
    let to = candidate.span.start_ms.saturating_add(CONTEXT_MS);
    let nearby = segments
        .iter()
        .enumerate()
        .filter(|(_, s)| (from..=to).contains(&s.start_ms))
        .map(|(i, _)| i);
    let user = format!(
        "Meeting: {}, {}\n\n## Context (about 90 seconds either side)\n{}\n\n## The sentence to classify\n\"{}\"",
        record.title.unwrap_or("(untitled)"),
        record.time.local_date(),
        transcript::render(segments, nearby, record.speaker_names),
        candidate.sentence
    );
    LlmRequest {
        system: JUDGE_SYSTEM.to_owned(),
        user,
        max_tokens: 400,
        temperature: 0.0,
        json_schema: Some(JUDGE_SCHEMA.to_owned()),
    }
}

/// The judge's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Judgement {
    /// The class.
    pub class: JudgeClass,
    /// The judge's confidence, 0 to 1.
    pub confidence: f64,
    /// The obligation as an instruction, for a commitment.
    pub task: Option<String>,
    /// The deadline as said.
    pub due: Option<String>,
    /// The judge's proof, from the sentence.
    pub quote: String,
}

const JUDGE_TASK: &str = "commitment judgement";

/// Fields [`parse_judgement`] requires; [`JUDGE_SCHEMA`] lists the same.
pub const JUDGE_REQUIRED: [&str; 5] = ["class", "confidence", "task", "due", "quote"];

/// Reads a judge's answer, checking it has **the judge's** shape. The error names the field that
/// is wrong, never its value.
pub fn parse_judgement(text: &str) -> Result<Judgement, LlmError> {
    let map = object_in(text, JUDGE_TASK)?;
    let fields = Fields::new(JUDGE_TASK, &map);
    let class_name = fields.string("class")?;
    let class = JudgeClass::ALL
        .into_iter()
        .find(|c| c.name() == class_name)
        .ok_or_else(|| bad_field(JUDGE_TASK, "class", "is not one of the six classes"))?;
    let confidence = fields.number("confidence")?;
    if !(0.0..=1.0).contains(&confidence) {
        return Err(bad_field(
            JUDGE_TASK,
            "confidence",
            "is not between 0 and 1",
        ));
    }
    let task = fields.nullable_string("task")?;
    let due = fields.nullable_string("due")?;
    let quote = fields.string("quote")?;
    let owned = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Ok(Judgement {
        class,
        confidence,
        task: owned(task),
        due: owned(due),
        quote: quote.trim().to_owned(),
    })
}

/// The anti-hallucination gate: the quote is non-empty and appears verbatim in the sentence
/// (surrounding quotation marks aside).
pub fn quote_holds(quote: &str, candidate: &Candidate) -> bool {
    quote_found(quote, &candidate.sentence)
}

/// What [`harvest`] did, so a run that files nothing can say why instead of looking empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Harvest {
    /// Commitments to store, in the order they were said.
    pub commitments: Vec<NewCommitment>,
    /// Candidates spotted, and so judge calls made.
    pub candidates: usize,
    /// Answers that did not have the judge's shape.
    pub malformed: usize,
    /// Answers whose quote was not in the sentence.
    pub unquoted: usize,
    /// Commitments the judge was not confident enough about to file.
    pub maybes: usize,
}

/// The confidence a commitment needs to be filed.
pub const MIN_CONFIDENCE: f64 = 0.5;

/// **Worker.** Spots and judges the commitments in a record's segments.
///
/// A malformed or unquotable answer drops that candidate and is counted. A cancellation, a
/// refusal, a missing key or a network failure ends the harvest with that error: every later
/// call would fail the same way (or prompt again).
///
/// The mic channel is the user, so a harvested commitment is the user's: `owner` stays `None`
/// (nobody was named). `due_at_unix_ms` is set only when the deadline resolves unambiguously
/// ([`resolve_due`]).
pub fn harvest(
    segments: &[Segment],
    record: &RecordContext<'_>,
    llm: &dyn Llm,
    cancel: &CancelToken,
) -> Result<Harvest, LlmError> {
    let candidates = spot(segments);
    let mut harvest = Harvest {
        candidates: candidates.len(),
        ..Harvest::default()
    };
    for candidate in &candidates {
        let answer = match ask(llm, &judge_request(candidate, segments, record), cancel) {
            Ok(answer) => answer,
            Err(e) if is_fatal(&e) => return Err(e),
            Err(_) => {
                harvest.malformed += 1;
                continue;
            }
        };
        let Ok(judgement) = parse_judgement(&answer) else {
            harvest.malformed += 1;
            continue;
        };
        if !quote_holds(&judgement.quote, candidate) {
            harvest.unquoted += 1;
            continue;
        }
        if judgement.class != JudgeClass::Commitment {
            continue;
        }
        if judgement.confidence < MIN_CONFIDENCE {
            harvest.maybes += 1;
            continue;
        }
        let due_at_unix_ms = judgement
            .due
            .as_deref()
            .and_then(|due| resolve_due(due, record.time));
        harvest.commitments.push(NewCommitment {
            text: judgement.task.unwrap_or_else(|| candidate.sentence.clone()),
            owner: None,
            due: judgement.due,
            due_at_unix_ms,
            provenance: vec![candidate.span],
        });
    }
    Ok(harvest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(channel: Channel, start_ms: u64, text: &str) -> Segment {
        Segment {
            channel,
            start_ms,
            end_ms: start_ms + 3_000,
            text: text.into(),
            speaker: None,
        }
    }

    #[test]
    fn promises_are_spotted_on_the_mic_channel_only() {
        let segments = [
            seg(Channel::Far, 0, "I'll send you the contract tonight."),
            seg(Channel::Mic, 5_000, "Great. I'll review it by Friday."),
            seg(Channel::Mic, 9_000, "The weather is still nice."),
        ];
        let found = spot(&segments);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].sentence, "I'll review it by Friday");
        assert_eq!(found[0].trigger, "i'll");
        assert_eq!(found[0].segment, 1);
        assert_eq!(
            found[0].span,
            Span {
                channel: Channel::Mic,
                start_ms: 5_000,
                end_ms: 8_000
            }
        );
    }

    #[test]
    fn triggers_match_at_word_boundaries_only() {
        assert!(!contains_phrase("i'm still willing", "i'll"));
        assert!(!contains_phrase("willpower", "i will"));
        assert!(contains_phrase("so i'll do it", "i'll"));
        assert!(contains_phrase("i'll", "i'll"));
        assert!(contains_phrase("okay, let me check", "let me"));
        assert!(!contains_phrase("outlet meeting", "let me"));
        // A second occurrence at a boundary is still found after a first inside a word.
        assert!(contains_phrase("chill'll i'll", "i'll"));
        // Non-ASCII text around a match does not trip the boundary check.
        assert!(contains_phrase("é i'll ü", "i'll"));
    }

    #[test]
    fn curly_apostrophes_from_asr_still_match() {
        let found = spot(&[seg(Channel::Mic, 0, "I\u{2019}ll book the room.")]);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn one_candidate_per_sentence_and_one_per_promise_sentence() {
        let found = spot(&[seg(
            Channel::Mic,
            0,
            "I'll check and I'll send it. Then let me write the notes!",
        )]);
        let sentences: Vec<&str> = found.iter().map(|c| c.sentence.as_str()).collect();
        assert_eq!(
            sentences,
            ["I'll check and I'll send it", "Then let me write the notes"]
        );
    }

    #[test]
    fn the_judge_sees_the_sentence_and_its_neighbourhood() {
        let segments = [
            seg(Channel::Far, 0, "Long ago."),
            seg(Channel::Far, 100_000, "Could you share the slides?"),
            seg(
                Channel::Mic,
                120_000,
                "Sure, I'll share the slides tomorrow.",
            ),
            seg(Channel::Far, 400_000, "Much later."),
        ];
        let candidate = &spot(&segments)[0];
        let record = RecordContext {
            title: Some("Weekly sync"),
            time: RecordTime {
                started_at_unix_ms: 1_790_146_800_000,
                utc_offset_minutes: 120,
            },
            speaker_names: &[],
        };
        let request = judge_request(candidate, &segments, &record);
        assert!(request.user.contains("Weekly sync, 2026-09-23 (Wednesday)"));
        assert!(
            request
                .user
                .contains("L1 [01:40] Them: Could you share the slides?")
        );
        assert!(
            request
                .user
                .contains("\"Sure, I'll share the slides tomorrow\"")
        );
        assert!(!request.user.contains("Long ago"));
        assert!(!request.user.contains("Much later"));
        assert_eq!(request.json_schema.as_deref(), Some(JUDGE_SCHEMA));
        assert_eq!(request.temperature, 0.0);
    }

    #[test]
    fn the_judge_schema_and_the_parser_agree() {
        let schema: serde_json::Value = serde_json::from_str(JUDGE_SCHEMA).unwrap();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(required, JUDGE_REQUIRED);
        let classes: Vec<&str> = schema["properties"]["class"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let names: Vec<&str> = JudgeClass::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(classes, names);
        for class in JUDGE_SYSTEM_CLASSES_CHECK {
            assert!(JUDGE_SYSTEM.contains(class), "the prompt lists {class}");
        }
    }

    const JUDGE_SYSTEM_CLASSES_CHECK: [&str; 6] = [
        "commitment:",
        "hypothetical:",
        "option_discussion:",
        "declined_or_retracted:",
        "delegated:",
        "already_done:",
    ];

    #[test]
    fn a_well_formed_judgement_parses_inside_fences() {
        let answer = "```json\n{\"class\": \"commitment\", \"confidence\": 0.9, \"task\": \"Share the slides\", \"due\": \"tomorrow\", \"quote\": \"I'll share the slides\"}\n```";
        let j = parse_judgement(answer).unwrap();
        assert_eq!(j.class, JudgeClass::Commitment);
        assert_eq!(j.task.as_deref(), Some("Share the slides"));
        assert_eq!(j.due.as_deref(), Some("tomorrow"));
    }

    #[test]
    fn malformed_judgements_are_refused_by_field() {
        let cases = [
            (
                r#"{"class": "promise", "confidence": 0.9, "task": null, "due": null, "quote": "x"}"#,
                "`class`",
            ),
            (
                r#"{"class": "commitment", "confidence": 1.5, "task": null, "due": null, "quote": "x"}"#,
                "`confidence`",
            ),
            (
                r#"{"class": "commitment", "confidence": "high", "task": null, "due": null, "quote": "x"}"#,
                "`confidence`",
            ),
            (
                r#"{"class": "commitment", "confidence": 0.9, "due": null, "quote": "x"}"#,
                "`task`",
            ),
            (
                r#"{"class": "commitment", "confidence": 0.9, "task": null, "due": null}"#,
                "`quote`",
            ),
            ("I think it is a commitment.", "no JSON object"),
        ];
        for (answer, field) in cases {
            let err = parse_judgement(answer).unwrap_err().to_string();
            assert!(err.contains(field), "{answer} -> {err}");
        }
    }

    #[test]
    fn the_quote_must_appear_verbatim() {
        let candidate = &spot(&[seg(Channel::Mic, 0, "I'll share the slides tomorrow.")])[0];
        assert!(quote_holds("I'll share the slides", candidate));
        assert!(quote_holds("\"share the slides\"", candidate));
        assert!(!quote_holds("I will share the slides", candidate));
        assert!(!quote_holds("  ", candidate));
    }
}
