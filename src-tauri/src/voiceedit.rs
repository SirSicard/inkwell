//! Voice editing: select text, hold a key, say what to change, get the rewrite.
//!
//! Every competitor that has this calls it something different (Command Mode,
//! Voice Edit, Transform) and they all agree on the shape: the selection is the
//! subject, speech is the instruction, and the result replaces the selection.
//! It is the one converged feature Inkwell did not have.
//!
//! This is deliberately BYOK-only. Rewriting text to order is a language-model
//! job, and Inkwell has no model of its own to do it with, so without a key the
//! feature says so rather than degrading into something that looks like it
//! worked.

use crate::llm;

/// Instruction to the model. Written to be boring on purpose: the failure mode
/// that matters is a model that explains, apologises, or wraps the answer in
/// quotes, because whatever comes back is pasted straight over the user's text.
pub const EDIT_SYSTEM_PROMPT: &str = "\
You rewrite text according to an instruction. Reply with the rewritten text and \
nothing else: no preamble, no explanation, no quotation marks around the result, \
no markdown fences. Preserve the original language unless told otherwise. If the \
instruction cannot be applied, reply with the original text unchanged.";

/// Pack the selection and the spoken instruction into one user message.
///
/// Delimited rather than concatenated, because an instruction like "delete the
/// last sentence" is indistinguishable from text to be edited unless the
/// boundary is explicit.
pub fn build_user_message(selection: &str, instruction: &str) -> String {
    format!(
        "Instruction:\n{}\n\nText:\n{}",
        instruction.trim(),
        selection
    )
}

/// Strip the wrappers models add even when told not to.
///
/// Belt and braces next to the system prompt: a stray ```/``` pair or a pair of
/// surrounding quotes would otherwise be pasted into the user's document, and
/// the cost of being wrong here is one unwrapped layer, not corruption.
pub fn clean_response(text: &str) -> String {
    let mut out = text.trim();

    if out.starts_with("```") {
        // Drop the opening fence and its language tag, then the closing fence.
        if let Some(rest) = out.split_once('\n').map(|(_, r)| r) {
            out = rest;
        }
        if let Some(stripped) = out.trim_end().strip_suffix("```") {
            out = stripped;
        }
        out = out.trim();
    }

    // Only unwrap quotes that enclose the whole response, and only when the
    // original did not obviously want them: "He said "hi"" must survive.
    let unwrapped = out
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .filter(|inner| !inner.contains('"'));
    if let Some(inner) = unwrapped {
        out = inner;
    }

    out.to_string()
}

/// Ask the configured provider to apply `instruction` to `selection`.
pub async fn apply_edit(
    provider: &str,
    api_key: &str,
    model: Option<String>,
    selection: &str,
    instruction: &str,
) -> Result<String, String> {
    let cfg = llm::ProviderConfig {
        provider: provider.to_string(),
        api_key: api_key.to_string(),
        custom_url: None,
        model,
    };
    let llm = llm::build_provider(cfg);
    let user = build_user_message(selection, instruction);
    let result = llm.complete(EDIT_SYSTEM_PROMPT, &user).await?;

    let cleaned = clean_response(&result.text);
    if cleaned.is_empty() {
        // Pasting an empty string would delete the user's selection and look
        // like the feature ate their text.
        return Err("The model returned nothing, so the selection was left alone".to_string());
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_and_text_stay_distinguishable() {
        let m = build_user_message("Hello world", "make it formal");
        assert!(m.contains("Instruction:\nmake it formal"));
        assert!(m.contains("Text:\nHello world"));
    }

    #[test]
    fn leading_whitespace_in_the_selection_is_preserved() {
        // Indentation is meaningful in code and lists; trimming the selection
        // would silently reformat it.
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
        // The model was asked to keep a quotation; unwrapping here would
        // corrupt the result.
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
}
