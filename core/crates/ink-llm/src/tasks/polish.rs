//! AI polish: clean up a dictation (fillers, false starts, punctuation) without changing what was
//! said. Ported from Inkwell 0.2's `polish.rs` and `llm.rs`.

use ink_core::{CancelToken, Llm, LlmError, LlmRequest};

use super::ask;
use crate::json::bad;

/// The default instruction; the user can replace it in settings.
pub const DEFAULT_POLISH_PROMPT: &str = "\
Clean up this speech-to-text transcription. The input comes from a dictation app and may contain:
- Filler words (um, uh, like, you know)
- False starts and repeated words
- Missing or wrong punctuation
- Misheard words or names

Rules:
- Fix grammar, punctuation, and capitalization
- Remove filler words and false starts
- Keep the speaker's original meaning, tone, and word choices
- Do NOT add, remove, or rephrase content
- Do NOT add greetings, sign-offs, or commentary
- Do NOT split into paragraphs (input is short dictation, not long-form)
- Return ONLY the cleaned text, nothing else";

/// The answer's token budget. Dictations are short; 0.2 used the same.
const MAX_TOKENS: u32 = 1024;

/// The request for polishing `text` under `prompt` (the default prompt when `prompt` is blank).
pub fn request(prompt: &str, text: &str) -> LlmRequest {
    let system = if prompt.trim().is_empty() {
        DEFAULT_POLISH_PROMPT
    } else {
        prompt
    };
    LlmRequest {
        system: system.to_owned(),
        user: text.to_owned(),
        max_tokens: MAX_TOKENS,
        temperature: 0.3,
        json_schema: None,
    }
}

/// **Worker.** Polishes `text`. An empty answer is an error, not an empty paste: the caller keeps
/// the unpolished text rather than inserting nothing.
pub fn polish(
    llm: &dyn Llm,
    prompt: &str,
    text: &str,
    cancel: &CancelToken,
) -> Result<String, LlmError> {
    let answer = ask(llm, &request(prompt, text), cancel)?;
    let cleaned = answer.trim();
    if cleaned.is_empty() {
        return Err(bad("polish", "the answer was empty"));
    }
    Ok(cleaned.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::Endpoint;
    use ink_core::mock::MockLlm;

    #[test]
    fn the_prompt_and_the_text_go_in_their_own_fields() {
        let r = request("", "um so the the meeting is at noon");
        assert_eq!(r.system, DEFAULT_POLISH_PROMPT);
        assert_eq!(r.user, "um so the the meeting is at noon");
        assert!(r.json_schema.is_none());

        let custom = request("Make it formal.", "hi");
        assert_eq!(custom.system, "Make it formal.");
    }

    #[test]
    fn the_answer_is_trimmed() {
        let llm = MockLlm::new(Endpoint::InProcess, "  So the meeting is at noon.\n");
        assert_eq!(
            polish(&llm, "", "synthetic", &CancelToken::new()).unwrap(),
            "So the meeting is at noon."
        );
    }

    #[test]
    fn an_empty_answer_is_an_error_not_an_empty_paste() {
        let llm = MockLlm::new(Endpoint::InProcess, "   ");
        assert!(matches!(
            polish(&llm, "", "synthetic", &CancelToken::new()),
            Err(LlmError::BadResponse(_))
        ));
    }

    #[test]
    fn a_cancelled_polish_makes_no_call() {
        let llm = MockLlm::new(Endpoint::InProcess, "text");
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            polish(&llm, "", "synthetic", &cancel),
            Err(LlmError::Cancelled)
        );
        assert_eq!(llm.calls(), 0);
    }
}
