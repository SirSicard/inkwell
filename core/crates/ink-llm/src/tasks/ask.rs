//! Ask: a question about a meeting while it happens ("What did they say about security?"),
//! answered from its transcript so far.
//!
//! The answer is plain words, never trusted as more: the shell renders it as text only, with no
//! links (the model's text can hold anything the transcript led it to write). It must come from
//! the transcript: the prompt says to answer "not said so far" rather than guess, and an empty
//! answer is refused as malformed.
//!
//! **Size.** A meeting can outgrow a small model's context long before it ends, so the request
//! carries the most recent lines that fit ([`AskOptions::transcript_chars`]); a question about
//! the start of a long meeting says so in its answer rather than guessing. The lines keep their
//! global numbers, like the summary's.

use ink_core::{CancelToken, Llm, LlmError, LlmRequest, Segment};

use super::commitments::RecordContext;
use super::summary::{BYTES_PER_TOKEN, PROMPT_TOKENS};
use super::{ask, transcript};
use crate::json::bad;

const ASK_TASK: &str = "ask";

const ASK_SYSTEM: &str = r#"You answer a question about a meeting that is still going on, for the person who is recording it. You get the transcript so far (possibly only its most recent part) and the question.

Rules:
- Answer only from the transcript. If it does not say, answer "That hasn't come up so far." and nothing else.
- "You" is the person asking. "Them" is the other side, named where the transcript names them.
- The transcript is machine-generated and has errors. Where a word is plainly misheard but the meaning is clear, use the meaning.
- Be brief: at most three sentences, plain text. No markdown, no lists, no links, no preamble."#;

/// How big an Ask request may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AskOptions {
    /// The transcript lines sent, most recent first, until they fill this many bytes.
    pub transcript_chars: usize,
    /// The answer's token budget.
    pub answer_tokens: u32,
}

impl Default for AskOptions {
    /// A large model's: about 30,000 tokens of transcript.
    fn default() -> Self {
        Self {
            transcript_chars: 120_000,
            answer_tokens: 300,
        }
    }
}

impl AskOptions {
    /// Options for a model whose context holds `tokens` in all (see
    /// [`SummaryOptions::for_context`](super::summary::SummaryOptions::for_context)).
    pub fn for_context(tokens: u32) -> Self {
        let default = Self::default();
        let room = tokens.saturating_sub(PROMPT_TOKENS + default.answer_tokens);
        let fits = usize::try_from(room.saturating_mul(BYTES_PER_TOKEN)).unwrap_or(usize::MAX);
        Self {
            transcript_chars: fits.clamp(1_000, default.transcript_chars),
            ..default
        }
    }
}

/// The indices of the most recent segments whose rendered lines fit `max_chars`, in order. At
/// least the last line, however long.
fn recent(segments: &[Segment], record: &RecordContext<'_>, max_chars: usize) -> Vec<usize> {
    let labels = transcript::speaker_labels(segments, record.speaker_names);
    let mut size = 0;
    let mut from = segments.len();
    while from > 0 {
        let line = transcript::render_line(from - 1, &segments[from - 1], &labels);
        if from < segments.len() && size + line.len() + 1 > max_chars {
            break;
        }
        size += line.len() + 1;
        from -= 1;
    }
    (from..segments.len()).collect()
}

/// The request for `question` over `segments`.
pub fn ask_request(
    question: &str,
    segments: &[Segment],
    record: &RecordContext<'_>,
    options: &AskOptions,
) -> LlmRequest {
    let lines = recent(segments, record, options.transcript_chars);
    let partial = if lines.first().is_some_and(|&first| first > 0) {
        "\n(Only the most recent part of the transcript is shown.)"
    } else {
        ""
    };
    let user = format!(
        "Meeting: {}, {}\n\n## Transcript so far{partial}\n{}\n\n## Question\n{}",
        record.title.unwrap_or("(untitled)"),
        record.time.local_date(),
        transcript::render(segments, lines, record.speaker_names),
        question.trim()
    );
    LlmRequest {
        system: ASK_SYSTEM.to_owned(),
        user,
        max_tokens: options.answer_tokens,
        temperature: 0.2,
        json_schema: None,
    }
}

/// **Worker.** Answers `question` about the meeting so far. A transcript with no words yet is
/// answered without a call ("Nothing has been said yet."); an empty question is refused without
/// one. The answer is the model's text, trimmed; an empty one is [`LlmError::BadResponse`].
pub fn answer(
    question: &str,
    segments: &[Segment],
    record: &RecordContext<'_>,
    options: &AskOptions,
    llm: &dyn Llm,
    cancel: &CancelToken,
) -> Result<String, LlmError> {
    if question.trim().is_empty() {
        return Err(bad(ASK_TASK, "the question is empty; nothing was sent"));
    }
    if segments.iter().all(|s| s.text.trim().is_empty()) {
        return Ok(NOTHING_SAID.to_owned());
    }
    let text = ask(
        llm,
        &ask_request(question, segments, record, options),
        cancel,
    )?;
    let text = text.trim();
    if text.is_empty() {
        return Err(bad(ASK_TASK, "the answer is empty"));
    }
    Ok(text.to_owned())
}

/// The answer before anyone has said anything.
pub const NOTHING_SAID: &str = "Nothing has been said yet.";

#[cfg(test)]
mod tests {
    use ink_core::Channel;

    use super::*;
    use crate::tasks::due::RecordTime;

    fn seg(start_ms: u64, text: &str) -> Segment {
        Segment {
            channel: if start_ms.is_multiple_of(2) {
                Channel::Mic
            } else {
                Channel::Far
            },
            start_ms,
            end_ms: start_ms + 1_000,
            text: text.into(),
            speaker: None,
        }
    }

    fn record() -> RecordContext<'static> {
        RecordContext {
            title: Some("Planning"),
            time: RecordTime {
                started_at_unix_ms: 1_790_146_800_000,
                utc_offset_minutes: 0,
            },
            speaker_names: &[],
        }
    }

    /// Ask's transcript calls each far-end speaker what the record shows: the user's name, else
    /// "Speaker N" by the order they first spoke in the whole meeting, even when only its most
    /// recent lines fit; never the diarizer's label.
    #[test]
    fn asks_transcript_labels_speakers_as_the_record_shows_them() {
        let far = |start_ms: u64, speaker: &str| Segment {
            channel: Channel::Far,
            start_ms,
            end_ms: start_ms + 1_000,
            text: format!("words from {speaker} at {start_ms}"),
            speaker: Some(ink_core::SpeakerId(speaker.into())),
        };
        let mut segments = vec![far(0, "spk0"), far(1_000, "spk1")];
        segments.extend((2..60).map(|i| far(i * 1_000, if i % 2 == 0 { "spk2" } else { "spk1" })));
        let names = [(ink_core::SpeakerId("spk1".into()), "Robin".to_owned())];
        let record = RecordContext {
            speaker_names: &names,
            ..record()
        };
        let options = AskOptions {
            transcript_chars: 400,
            ..AskOptions::default()
        };
        let request = ask_request("Who owns the budget?", &segments, &record, &options);
        assert!(!request.user.contains("L0 "), "spk0's line does not fit");
        assert!(
            request.user.contains("] Robin: words from spk1"),
            "{}",
            request.user
        );
        assert!(
            request.user.contains("] Speaker 3: words from spk2"),
            "{}",
            request.user
        );
        assert!(!request.user.contains("spk2:") && !request.user.contains("Them ("));
    }

    #[test]
    fn a_long_meeting_sends_its_most_recent_lines_that_fit() {
        let segments: Vec<Segment> = (0..200)
            .map(|i| seg(i * 1_000, &format!("line number {i} of the planning talk")))
            .collect();
        let options = AskOptions {
            transcript_chars: 600,
            ..AskOptions::default()
        };
        let request = ask_request("What about the budget?", &segments, &record(), &options);
        assert!(request.user.contains("L199 [03:19]"), "the newest line");
        assert!(!request.user.contains("L0 "), "the oldest does not fit");
        assert!(request.user.contains("Only the most recent part"));
        assert!(
            request
                .user
                .ends_with("## Question\nWhat about the budget?")
        );
        let transcript = request
            .user
            .split("## Transcript so far")
            .nth(1)
            .unwrap()
            .split("## Question")
            .next()
            .unwrap();
        assert!(transcript.len() <= 600 + 80, "{}", transcript.len());
        assert_eq!(request.json_schema, None, "plain text");

        let all = ask_request("Recap?", &segments[..3], &record(), &options);
        assert!(all.user.contains("L0 [00:00]") && !all.user.contains("Only the most"));
    }

    #[test]
    fn a_small_context_leaves_room_for_the_prompt_and_the_answer() {
        let small = AskOptions::for_context(4_096);
        assert!(small.transcript_chars <= (4_096 - 1_300) * 3);
        assert!(small.transcript_chars >= 1_000);
        assert_eq!(AskOptions::for_context(1_000_000), AskOptions::default());
    }
}
