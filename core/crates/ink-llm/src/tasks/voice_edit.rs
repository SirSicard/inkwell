//! Voice editing: select text, hold a key, say what to change, and the rewrite replaces the
//! selection. The selection is the subject, speech is the instruction. Ported from Inkwell 0.2's
//! `voiceedit.rs`.

use ink_core::{CancelToken, Llm, LlmError, LlmRequest};

use super::ask;
use crate::json::bad;

/// The instruction to the model. Boring on purpose: whatever comes back is pasted straight over
/// the user's text, so the failure that matters is a model that explains, apologises, or wraps
/// the answer in quotes.
pub const EDIT_SYSTEM_PROMPT: &str = "\
You rewrite text according to an instruction. Reply with the rewritten text and \
nothing else: no preamble, no explanation, no quotation marks around the result, \
no markdown fences. Preserve the original language unless told otherwise. If the \
instruction cannot be applied, reply with the original text unchanged.";

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
/// delete the selection and look as if the feature ate the user's text.
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

    #[test]
    fn an_empty_answer_leaves_the_selection_alone() {
        let llm = MockLlm::new(Endpoint::InProcess, "\"\"");
        assert!(matches!(
            apply_edit(&llm, "keep me", "delete nothing", &CancelToken::new()),
            Err(LlmError::BadResponse(_))
        ));
    }
}
