//! Meeting summaries: a headline, a body, decisions, actions and open questions, each decision and
//! action citing the transcript line it came from.
//!
//! Structured rather than prose because the parts are used apart: the headline becomes the
//! record's title, actions become commitments (and meet the harvester's in [`super::dedup`]), and
//! the UI links each item back to the moment it was said. The line citation is the load-bearing
//! field: an action with one is a record you can check, and without one it is an assertion.
//!
//! A citation is checked, not trusted. Every decision and action carries a short verbatim quote
//! from the line it cites, and an item whose quote is not in that line is dropped (and counted in
//! [`SummaryOutcome::unverified`]), as the commitment judge's quote is checked against its
//! sentence. Without the check, a line in the transcript that reads like an instruction could
//! get a fabricated action filed under a real, unrelated citation. The quote needs at least
//! [`MIN_QUOTE_WORDS`] words, or the whole line when the line is shorter, so a stray "the"
//! cannot vouch for anything. It proves the line says those words, not what the item makes of
//! them.
//!
//! A transcript too long for one request is summarised in overlapping time windows, which are then
//! combined (map, then reduce). The overlap is the point: a decision made at the end of one
//! window and confirmed at the start of the next is invisible to a clean cut. Line numbers are
//! global, so citations survive both passes.

use ink_core::{CancelToken, Llm, LlmError, LlmRequest, NewCommitment, Segment, Span, Summary};
use serde_json::{Value, json};

use super::commitments::RecordContext;
use super::due::resolve_due;
use super::{ask, quote_found, transcript};
use crate::json::{Fields, bad, bad_field, object_in};

/// A decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// What was decided.
    pub text: String,
    /// The transcript line it came from.
    pub line: Option<usize>,
    /// Words copied from that line, which prove the citation.
    pub quote: Option<String>,
}

/// An action item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    /// What is to be done.
    pub text: String,
    /// Who, as said.
    pub owner: Option<String>,
    /// When, as said.
    pub due: Option<String>,
    /// The transcript line it came from.
    pub line: Option<usize>,
    /// Words copied from that line, which prove the citation.
    pub quote: Option<String>,
}

/// A summary as the model wrote it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SummaryDraft {
    /// One or two sentences: what a colleague would say if asked what it was about.
    pub headline: String,
    /// Markdown.
    pub body: String,
    /// Decisions.
    pub decisions: Vec<Decision>,
    /// Action items.
    pub actions: Vec<Action>,
    /// Questions left open.
    pub open_questions: Vec<String>,
    /// What the model looked for and could not find. Explicit, because a section silently left
    /// out reads the same as one the model forgot.
    pub not_found: Vec<String>,
}

/// The summary's answer, as JSON Schema. [`parse_summary`] enforces the same top-level required
/// fields. Items are asked for their `line` and `quote` too; an item without them is not a
/// malformed answer, it is dropped as unverified ([`summarize`]).
pub const SUMMARY_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "headline": {"type": "string"},
    "body": {"type": "string"},
    "decisions": {"type": "array", "items": {"type": "object", "properties": {
      "text": {"type": "string"}, "line": {"type": ["integer", "null"]},
      "quote": {"type": ["string", "null"]}}, "required": ["text", "line", "quote"]}},
    "actions": {"type": "array", "items": {"type": "object", "properties": {
      "text": {"type": "string"}, "owner": {"type": ["string", "null"]},
      "due": {"type": ["string", "null"]}, "line": {"type": ["integer", "null"]},
      "quote": {"type": ["string", "null"]}}, "required": ["text", "owner", "due", "line", "quote"]}},
    "open_questions": {"type": "array", "items": {"type": "string"}},
    "not_found": {"type": "array", "items": {"type": "string"}}
  },
  "required": ["headline", "body", "decisions", "actions"]
}"#;

/// Fields [`parse_summary`] requires; [`SUMMARY_SCHEMA`] lists the same.
pub const SUMMARY_REQUIRED: [&str; 4] = ["headline", "body", "decisions", "actions"];

const SUMMARY_SYSTEM: &str = r#"You write a meeting record. It will be read later by someone deciding what to do, and quoted back to people who were there, so accuracy matters more than polish.

Rules:
- Never invent an action, decision, owner or date. If it was not said, it did not happen. Anything you looked for and could not find goes in "not_found".
- Every decision and action cites its transcript line (the number after "L") in "line", and copies words from that line, exactly as written, in "quote": at least three words, or the whole line if it is shorter. An item whose quote is not in its line is discarded.
- "You" is the person who recorded the meeting. "Them" is the other side, named where the transcript names them.
- Owners and deadlines are as said. Copy "by Friday" as "Friday"; never work out a date.
- The transcript is machine-generated and has errors. Where a word is plainly misheard but the meaning is clear, use the meaning; where the meaning is not clear, say so rather than guess.
- Leave out filler words. No preamble.

Answer with one JSON object and nothing else:
{"headline": "one or two sentences: what a colleague would say if asked what it was about", "body": "markdown: the substance, in short sections", "decisions": [{"text": "...", "line": 12, "quote": "words from line 12"}], "actions": [{"text": "...", "owner": "a name, You, or null", "due": "as said, or null", "line": 12, "quote": "words from line 12"}], "open_questions": ["..."], "not_found": ["..."]}"#;

const SUMMARY_TASK: &str = "summary";

/// Reads a summary answer, checking it has **the summary's** shape. `final_pass` also requires a
/// headline; a map pass over one window may legitimately have none.
pub fn parse_summary(text: &str, final_pass: bool) -> Result<SummaryDraft, LlmError> {
    let map = object_in(text, SUMMARY_TASK)?;
    let fields = Fields::new(SUMMARY_TASK, &map);
    let headline = fields.string("headline")?.trim().to_owned();
    if final_pass && headline.is_empty() {
        return Err(bad_field(SUMMARY_TASK, "headline", "is empty"));
    }
    let body = fields.string("body")?.trim().to_owned();
    let decisions = fields
        .array("decisions")?
        .iter()
        .map(|item| {
            let d = Fields::object_item(SUMMARY_TASK, "decisions", item)?;
            Ok(Decision {
                text: d.string("text")?.trim().to_owned(),
                line: d.optional_index("line")?,
                quote: d.optional_string("quote")?.map(str::to_owned),
            })
        })
        .collect::<Result<Vec<_>, LlmError>>()?;
    let actions = fields
        .array("actions")?
        .iter()
        .map(|item| {
            let a = Fields::object_item(SUMMARY_TASK, "actions", item)?;
            let owned = |s: Option<&str>| {
                s.map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            Ok(Action {
                text: a.string("text")?.trim().to_owned(),
                owner: owned(a.optional_string("owner")?),
                due: owned(a.optional_string("due")?),
                line: a.optional_index("line")?,
                quote: a.optional_string("quote")?.map(str::to_owned),
            })
        })
        .collect::<Result<Vec<_>, LlmError>>()?;
    Ok(SummaryDraft {
        headline,
        body,
        decisions,
        actions,
        open_questions: fields.optional_strings("open_questions")?,
        not_found: fields.optional_strings("not_found")?,
    })
}

/// When to split a transcript into windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SummaryOptions {
    /// A rendered transcript longer than this many bytes is summarised in windows. Set it from
    /// the model's context: roughly four bytes of English per token, leaving room for the prompt
    /// and the answer.
    pub single_pass_chars: usize,
    /// Window length.
    pub window_ms: u64,
    /// How much consecutive windows overlap.
    pub overlap_ms: u64,
}

impl Default for SummaryOptions {
    /// About 60,000 tokens in one pass; otherwise 15-minute windows overlapping by a minute.
    fn default() -> Self {
        Self {
            single_pass_chars: 240_000,
            window_ms: 15 * 60_000,
            overlap_ms: 60_000,
        }
    }
}

/// The fewest words a citation's quote may have, unless it is the whole line.
pub const MIN_QUOTE_WORDS: usize = 3;

/// Whether `quote` proves a citation of `line`: it is in the line verbatim, and has at least
/// [`MIN_QUOTE_WORDS`] words or all of the line's.
pub fn quote_cites(quote: &str, line: &str) -> bool {
    let words = quote.split_whitespace().count();
    quote_found(quote, line)
        && (words >= MIN_QUOTE_WORDS || words >= line.split_whitespace().count())
}

/// A finished summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummaryOutcome {
    /// The parts as the model gave them, less the decisions and actions whose citation did not
    /// check out.
    pub draft: SummaryDraft,
    /// What to store: the draft rendered as markdown.
    pub summary: Summary,
    /// The draft's actions, as commitments with their cited line as provenance.
    pub actions: Vec<NewCommitment>,
    /// Decisions and actions dropped because their line or quote did not check out: no line,
    /// a line out of range, no quote, or a quote not in the cited line.
    pub unverified: usize,
    /// Model calls made: 1, or one per window plus the combining pass.
    pub calls: usize,
}

/// **Worker.** Summarises a record's segments with `llm`. An empty transcript is refused without
/// a call. `now_unix_ms` stamps the summary.
pub fn summarize(
    segments: &[Segment],
    record: &RecordContext<'_>,
    options: &SummaryOptions,
    now_unix_ms: i64,
    llm: &dyn Llm,
    cancel: &CancelToken,
) -> Result<SummaryOutcome, LlmError> {
    if segments.iter().all(|s| s.text.trim().is_empty()) {
        return Err(bad(
            SUMMARY_TASK,
            "the transcript is empty; nothing was sent",
        ));
    }
    let header = header(record);
    let full = transcript::render(segments, 0..segments.len(), record.speaker_names);
    let (mut draft, calls) = if full.len() <= options.single_pass_chars {
        let request = summary_request(format!("{header}\n\n## Transcript\n{full}"), 4096);
        (parse_summary(&ask(llm, &request, cancel)?, true)?, 1)
    } else {
        let windows = windows(segments, options);
        let total = windows.len();
        let mut parts = Vec::with_capacity(total);
        for (i, window) in windows.into_iter().enumerate() {
            let lines = transcript::render(segments, window, record.speaker_names);
            let request = summary_request(
                format!(
                    "{header}\n\nThis is PART {n} OF {total} of the meeting. Report only what is in this part; another pass combines the parts. A part may start or end mid-thought: report the fragment, do not complete it.\n\n## Transcript, part {n} of {total}\n{lines}",
                    n = i + 1
                ),
                2048,
            );
            parts.push(parse_summary(&ask(llm, &request, cancel)?, false)?);
        }
        let rendered: Vec<String> = parts
            .iter()
            .enumerate()
            .map(|(i, p)| format!("### Part {}\n{}", i + 1, draft_json(p)))
            .collect();
        let request = summary_request(
            format!(
                "{header}\n\n## Combining\nThe parts below overlap, so one item can appear twice in different words: merge those, keeping the earliest line. Add nothing that is in no part. Where parts disagree, prefer the one with more context and say so in \"not_found\". Order decisions and actions by line.\n\n## Parts\n{}",
                rendered.join("\n\n")
            ),
            4096,
        );
        (
            parse_summary(&ask(llm, &request, cancel)?, true)?,
            total + 1,
        )
    };

    // Keep only the items whose citation checks out.
    let cited = |text: &str, line: Option<usize>, quote: Option<&str>| -> Option<&Segment> {
        let segment = segments.get(line?)?;
        (!text.is_empty() && quote_cites(quote?, &segment.text)).then_some(segment)
    };
    let mut unverified = 0;
    draft.decisions.retain(|d| {
        let kept = cited(&d.text, d.line, d.quote.as_deref()).is_some();
        unverified += usize::from(!kept);
        kept
    });
    draft.actions.retain(|a| {
        let kept = cited(&a.text, a.line, a.quote.as_deref()).is_some();
        unverified += usize::from(!kept);
        kept
    });

    let info = llm.info();
    let actions = draft
        .actions
        .iter()
        .filter_map(|action| {
            let segment = cited(&action.text, action.line, action.quote.as_deref())?;
            Some(NewCommitment {
                text: action.text.clone(),
                owner: action.owner.clone(),
                due: action.due.clone(),
                due_at_unix_ms: action
                    .due
                    .as_deref()
                    .and_then(|due| resolve_due(due, record.time)),
                provenance: vec![Span {
                    channel: segment.channel,
                    start_ms: segment.start_ms,
                    end_ms: segment.end_ms,
                }],
            })
        })
        .collect();
    Ok(SummaryOutcome {
        summary: Summary {
            text: render_markdown(&draft),
            model: format!("{}/{}", info.provider, info.model),
            created_at_unix_ms: now_unix_ms,
        },
        actions,
        draft,
        unverified,
        calls,
    })
}

fn header(record: &RecordContext<'_>) -> String {
    format!(
        "Meeting: {}\nDate: {}",
        record.title.unwrap_or("(untitled)"),
        record.time.local_date()
    )
}

fn summary_request(user: String, max_tokens: u32) -> LlmRequest {
    LlmRequest {
        system: SUMMARY_SYSTEM.to_owned(),
        user,
        max_tokens,
        temperature: 0.2,
        json_schema: Some(SUMMARY_SCHEMA.to_owned()),
    }
}

/// Segment indices per overlapping time window, empty windows skipped.
fn windows(segments: &[Segment], options: &SummaryOptions) -> Vec<Vec<usize>> {
    let window = options.window_ms.max(1);
    // An overlap as long as the window would never advance.
    let step = window.saturating_sub(options.overlap_ms).max(1);
    let last = segments.iter().map(|s| s.start_ms).max().unwrap_or(0);
    let mut out = Vec::new();
    let mut start = 0u64;
    while start <= last {
        let end = start.saturating_add(window);
        let slice: Vec<usize> = segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.start_ms >= start && s.start_ms < end)
            .map(|(i, _)| i)
            .collect();
        if !slice.is_empty() {
            out.push(slice);
        }
        start = start.saturating_add(step);
    }
    out
}

/// A draft as JSON, for the combining pass.
fn draft_json(draft: &SummaryDraft) -> String {
    let value = json!({
        "headline": draft.headline,
        "body": draft.body,
        "decisions": draft.decisions.iter().map(|d| json!({"text": d.text, "line": d.line, "quote": d.quote})).collect::<Vec<Value>>(),
        "actions": draft.actions.iter().map(|a| json!({"text": a.text, "owner": a.owner, "due": a.due, "line": a.line, "quote": a.quote})).collect::<Vec<Value>>(),
        "open_questions": draft.open_questions,
        "not_found": draft.not_found,
    });
    value.to_string()
}

/// The stored text: headline, body, then each non-empty list as a section.
pub fn render_markdown(draft: &SummaryDraft) -> String {
    let mut out = draft.headline.clone();
    if !draft.body.is_empty() {
        out.push_str("\n\n");
        out.push_str(&draft.body);
    }
    let mut section = |title: &str, items: Vec<String>| {
        if !items.is_empty() {
            out.push_str(&format!("\n\n## {title}\n"));
            out.push_str(
                &items
                    .iter()
                    .map(|i| format!("- {i}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
    };
    section(
        "Decisions",
        draft.decisions.iter().map(|d| d.text.clone()).collect(),
    );
    section(
        "Actions",
        draft
            .actions
            .iter()
            .map(|a| {
                let detail: Vec<&str> = [a.owner.as_deref(), a.due.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect();
                if detail.is_empty() {
                    a.text.clone()
                } else {
                    format!("{} ({})", a.text, detail.join(", "))
                }
            })
            .collect(),
    );
    section("Open questions", draft.open_questions.clone());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::Channel;

    #[test]
    fn the_summary_schema_and_the_parser_agree() {
        let schema: Value = serde_json::from_str(SUMMARY_SCHEMA).unwrap();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(required, SUMMARY_REQUIRED);
    }

    #[test]
    fn a_summary_answer_parses() {
        let answer = r#"{"headline": "Agreed the launch plan.", "body": "We went through the plan.",
            "decisions": [{"text": "Launch on the 5th", "line": 3}],
            "actions": [{"text": "Send the checklist", "owner": "You", "due": "Friday", "line": 1, "quote": "I'll send the checklist"},
                        {"text": "Book the venue", "owner": null, "due": null}],
            "open_questions": ["Who signs off?"]}"#;
        let draft = parse_summary(answer, true).unwrap();
        assert_eq!(draft.decisions[0].line, Some(3));
        assert_eq!(draft.actions[0].owner.as_deref(), Some("You"));
        assert_eq!(draft.actions[1].line, None);
        assert_eq!(
            draft.actions[0].quote.as_deref(),
            Some("I'll send the checklist")
        );
        assert_eq!(draft.actions[1].quote, None);
        assert_eq!(draft.open_questions, ["Who signs off?"]);
        assert!(draft.not_found.is_empty());
    }

    #[test]
    fn malformed_summaries_are_refused_by_field() {
        for (answer, field) in [
            (
                r#"{"body": "", "decisions": [], "actions": []}"#,
                "`headline`",
            ),
            (
                r#"{"headline": "", "body": "", "decisions": [], "actions": []}"#,
                "`headline` is empty",
            ),
            (
                r#"{"headline": "h", "body": "b", "decisions": {}, "actions": []}"#,
                "`decisions`",
            ),
            (
                r#"{"headline": "h", "body": "b", "decisions": [], "actions": [{"owner": "You"}]}"#,
                "`text`",
            ),
            (
                r#"{"headline": "h", "body": "b", "decisions": [], "actions": [{"text": "t", "line": -1}]}"#,
                "`line`",
            ),
        ] {
            let err = parse_summary(answer, true).unwrap_err().to_string();
            assert!(err.contains(field), "{answer} -> {err}");
        }
        // A window's partial may lack a headline.
        assert!(
            parse_summary(
                r#"{"headline": "", "body": "", "decisions": [], "actions": []}"#,
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn a_quote_cites_a_line_only_verbatim_and_long_enough() {
        let line = "Sure, I'll send the onboarding checklist by Friday.";
        assert!(quote_cites("I'll send the onboarding checklist", line));
        assert!(quote_cites("\"the onboarding checklist\"", line));
        assert!(
            !quote_cites("I will send the checklist", line),
            "not verbatim"
        );
        assert!(
            !quote_cites("i'll send the onboarding", line),
            "case matters"
        );
        assert!(!quote_cites("the", line), "too short to vouch");
        assert!(
            !quote_cites("onboarding checklist", line),
            "too short to vouch"
        );
        assert!(!quote_cites("", line));
        // A short line can be quoted whole.
        assert!(quote_cites("Agreed.", "Agreed."));
        assert!(quote_cites("Sounds good", " Sounds good. "));
    }

    #[test]
    fn windows_overlap_and_skip_empty_stretches() {
        let seg = |start_ms: u64| Segment {
            channel: Channel::Mic,
            start_ms,
            end_ms: start_ms + 1_000,
            text: "x".into(),
            speaker: None,
        };
        let segments = [seg(0), seg(50_000), seg(95_000), seg(300_000)];
        let options = SummaryOptions {
            single_pass_chars: 0,
            window_ms: 100_000,
            overlap_ms: 10_000,
        };
        // Windows start at 0, 90 000, 180 000 (empty, skipped) and 270 000.
        assert_eq!(
            windows(&segments, &options),
            vec![vec![0, 1, 2], vec![2], vec![3]]
        );
        let stuck = SummaryOptions {
            overlap_ms: 100_000,
            ..options
        };
        assert!(
            !windows(&segments, &stuck).is_empty(),
            "an overlap as long as the window still advances"
        );
    }

    #[test]
    fn markdown_has_the_sections_that_have_items() {
        let draft = SummaryDraft {
            headline: "Agreed the plan.".into(),
            body: "Details.".into(),
            actions: vec![Action {
                text: "Send the checklist".into(),
                owner: Some("You".into()),
                due: Some("Friday".into()),
                line: Some(1),
                quote: None,
            }],
            ..SummaryDraft::default()
        };
        assert_eq!(
            render_markdown(&draft),
            "Agreed the plan.\n\nDetails.\n\n## Actions\n- Send the checklist (You, Friday)"
        );
    }
}
