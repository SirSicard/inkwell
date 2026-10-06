//! Voice editing: select text, hold a key, say what to change, and the rewrite replaces the
//! selection. The selection is the subject, speech is the instruction. Ported from Inkwell 0.2's
//! `voiceedit.rs`.

use ink_core::{CancelToken, Llm, LlmError, LlmRequest};

use super::{ask, polish};
use crate::json::bad;

/// The instruction to the model. Boring on purpose: whatever comes back is pasted straight over
/// the user's text, so the failure that matters is a model that explains, apologises, or wraps
/// the answer in quotes.
///
/// Edits that take text away are named and told to be applied. 0.2's "If the instruction cannot
/// be applied, reply with the original text unchanged." gave small models a way out: in the local
/// model bench all four candidates answered "delete the last sentence" with the text unchanged
/// (the only edit of ten any of them failed). Giving the text back is still the answer to an
/// instruction that is not an edit of it, cannot be done to it, or would be harmful. "Harmful",
/// not "should not be done": a small model may read that as leave to skip any edit. An empty
/// answer (deleting everything) or one that talks about itself leaves the selection alone anyway
/// ([`apply_edit`]).
pub const EDIT_SYSTEM_PROMPT: &str = "\
You rewrite text according to an instruction. Apply the instruction even when it removes, \
moves or shortens text: \"delete the last sentence\" means replying with the text without its \
last sentence. Reply with the rewritten text and nothing else: no preamble, no explanation, no \
quotation marks around the result, no markdown fences. Preserve the original language unless \
told otherwise. Reply with the original text unchanged only when the instruction is not a \
change to the text, cannot be done to it, or would be harmful.";

/// The answer's token budget.
const MAX_TOKENS: u32 = 1024;

/// Packs the selection and the spoken instruction into one user message, delimited: "delete the
/// last sentence" is indistinguishable from text to edit unless the boundary is explicit. The
/// selection is not trimmed, since indentation matters in code and lists.
pub fn build_user_message(selection: &str, instruction: &str) -> String {
    format!(
        "Instruction:\n{}\n\nText:\n{}",
        instruction.trim(),
        selection
    )
}

/// Strips the wrappers models add even when told not to: one code fence, or quotes around the
/// whole answer when it contains no other quotes. Being wrong here costs one unwrapped layer,
/// never corruption.
pub fn clean_response(text: &str) -> String {
    let mut out = text.trim();

    if out.starts_with("```") {
        // Drop the opening fence and its language tag, then the closing fence.
        if let Some((_, rest)) = out.split_once('\n') {
            out = rest;
        }
        if let Some(stripped) = out.trim_end().strip_suffix("```") {
            out = stripped;
        }
        out = out.trim();
    }

    // "He said "hi"" must survive: only unwrap quotes that enclose everything and nothing else.
    let unwrapped = out
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .filter(|inner| !inner.contains('"'));
    if let Some(inner) = unwrapped {
        out = inner;
    }

    out.to_owned()
}

/// The request for applying `instruction` to `selection`.
pub fn request(selection: &str, instruction: &str) -> LlmRequest {
    LlmRequest {
        system: EDIT_SYSTEM_PROMPT.to_owned(),
        user: build_user_message(selection, instruction),
        max_tokens: MAX_TOKENS,
        temperature: 0.3,
        json_schema: None,
    }
}

/// **Worker.** Applies `instruction` to `selection`. An empty answer is an error: pasting it would
/// delete the selection and look as if the feature ate the user's text. So is one in which the
/// model speaks of itself or the request ("I cannot fulfill this request."), checked as polish
/// checks an answer under a custom prompt ([`polish::speaks_of_itself`]): pasted, a refusal would
/// replace the user's text. Phrases the selection or the instruction said are the user's. An
/// edit that legitimately opens with an introduction ("Here is the plan: the launch moves to
/// May.", a translation of "Aquí tienes:") is refused too: the selection stays, and the user can
/// ask again, which costs less than a preamble pasted into their text.
pub fn apply_edit(
    llm: &dyn Llm,
    selection: &str,
    instruction: &str,
    cancel: &CancelToken,
) -> Result<String, LlmError> {
    let answer = ask(llm, &request(selection, instruction), cancel)?;
    let cleaned = clean_response(&answer);
    if cleaned.is_empty() {
        return Err(bad(
            "voice edit",
            "the answer was empty, so the selection was left alone",
        ));
    }
    if polish::speaks_of_itself(&format!("{instruction}\n{selection}"), &cleaned) {
        return Err(bad(
            "voice edit",
            "the answer spoke about itself or the request, so the selection was left alone",
        ));
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::Endpoint;
    use ink_core::mock::MockLlm;

    #[test]
    fn instruction_and_text_stay_distinguishable() {
        let m = build_user_message("Hello world", "make it formal");
        assert!(m.contains("Instruction:\nmake it formal"));
        assert!(m.contains("Text:\nHello world"));
    }

    #[test]
    fn leading_whitespace_in_the_selection_is_preserved() {
        let m = build_user_message("    indented", "fix the typo");
        assert!(m.contains("Text:\n    indented"));
    }

    #[test]
    fn fenced_responses_are_unwrapped() {
        assert_eq!(clean_response("```\nrewritten\n```"), "rewritten");
        assert_eq!(clean_response("```text\nrewritten\n```"), "rewritten");
    }

    #[test]
    fn fully_quoted_responses_are_unwrapped() {
        assert_eq!(clean_response("\"rewritten\""), "rewritten");
    }

    #[test]
    fn inner_quotes_survive() {
        let q = "\"He said \"hi\" loudly\"";
        assert_eq!(clean_response(q), q);
    }

    #[test]
    fn ordinary_text_is_untouched() {
        assert_eq!(clean_response("  just text  "), "just text");
    }

    #[test]
    fn multiline_output_survives() {
        assert_eq!(clean_response("line one\nline two"), "line one\nline two");
    }

    #[test]
    fn the_edit_is_cleaned_and_returned() {
        let llm = MockLlm::new(Endpoint::InProcess, "```\nDear team,\n```");
        assert_eq!(
            apply_edit(&llm, "hey all", "make it formal", &CancelToken::new()).unwrap(),
            "Dear team,"
        );
    }

    fn edited(selection: &str, instruction: &str, answer: &str) -> Result<String, LlmError> {
        let llm = MockLlm::new(Endpoint::InProcess, answer);
        apply_edit(&llm, selection, instruction, &CancelToken::new())
    }

    // The defect polish had, here too: a refusal typed over the user's text.
    #[test]
    fn a_refusal_leaves_the_selection_alone() {
        for answer in [
            "I am a foundation model developed by Apple. I cannot fulfill this request.",
            "I'm sorry, but I can't assist with that.",
            "Sure! Here's the formal version: Dear team, the launch moves to May.",
        ] {
            assert!(
                matches!(
                    edited("hey all, launch moves to may", "make it formal", answer),
                    Err(LlmError::BadResponse(_))
                ),
                "{answer}"
            );
        }
    }

    // Only the model's own talk is refused: an edit may rewrite every word, and may speak of AI
    // when the text or the instruction did.
    #[test]
    fn an_edit_that_rewrites_everything_gets_through() {
        for (selection, instruction, answer) in [
            (
                "hey all, launch moves to may",
                "make it formal",
                "Dear team, the launch has been moved to May.",
            ),
            (
                "the meeting is at noon",
                "translate into Spanish",
                "La reunión es al mediodía.",
            ),
            (
                "As an AI researcher I study language models",
                "add a comma",
                "As an AI researcher, I study language models.",
            ),
            // The text's own words, with an article or in the singular.
            (
                "dictation is slow",
                "make it a full sentence",
                "The dictation is slow.",
            ),
            (
                "language models is good at this",
                "fix the grammar",
                "A language model is good at this.",
            ),
            (
                "we cant do friday",
                "say it was about the language model demo",
                "We can't do Friday's language model demo.",
            ),
        ] {
            assert_eq!(
                edited(selection, instruction, answer).as_deref(),
                Ok(answer),
                "{instruction}"
            );
        }
    }

    /// A model that records the request it was sent and answers with a scripted reply.
    struct Scripted {
        reply: &'static str,
        sent: std::sync::Mutex<Option<LlmRequest>>,
    }

    impl Llm for Scripted {
        fn info(&self) -> ink_core::LlmInfo {
            MockLlm::new(Endpoint::InProcess, "").info()
        }

        fn complete(
            &self,
            request: &LlmRequest,
            _cancel: &CancelToken,
        ) -> Result<ink_core::LlmResponse, LlmError> {
            *self.sent.lock().unwrap() = Some(request.clone());
            Ok(ink_core::LlmResponse {
                text: self.reply.to_owned(),
            })
        }
    }

    // The local model bench: every candidate gave "delete the last sentence" back unchanged, which
    // the old prompt's catch-all ("if the instruction cannot be applied, reply with the original
    // text unchanged") invited. The prompt now says to apply edits that remove text, and keeps
    // the unchanged reply for instructions that are not an edit or cannot be done. What a real
    // model makes of the wording is measured, not unit-tested; this pins the wording.
    #[test]
    fn the_prompt_asks_for_edits_that_remove_text_to_be_applied() {
        let llm = Scripted {
            reply: "We shipped the feature. It took three weeks.",
            sent: std::sync::Mutex::new(None),
        };
        let selection = "We shipped the feature. It took three weeks. The team is happy.";
        assert_eq!(
            apply_edit(
                &llm,
                selection,
                "delete the last sentence",
                &CancelToken::new()
            )
            .unwrap(),
            "We shipped the feature. It took three weeks."
        );
        let sent = llm.sent.lock().unwrap().take().unwrap();
        let system = sent.system.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            system.contains("Apply the instruction even when it removes, moves or shortens text"),
            "{system}"
        );
        assert!(system.contains("\"delete the last sentence\""), "{system}");
        // The way out is narrowed, not gone, and the catch-all is gone.
        assert!(
            system.contains(
                "unchanged only when the instruction is not a change to the text, cannot be done \
                 to it, or would be harmful"
            ),
            "{system}"
        );
        assert!(
            !system.contains("If the instruction cannot be applied"),
            "{system}"
        );
        assert_eq!(
            sent.user,
            build_user_message(selection, "delete the last sentence")
        );
    }

    #[test]
    fn an_empty_answer_leaves_the_selection_alone() {
        let llm = MockLlm::new(Endpoint::InProcess, "\"\"");
        assert!(matches!(
            apply_edit(&llm, "keep me", "delete nothing", &CancelToken::new()),
            Err(LlmError::BadResponse(_))
        ));
    }
}
