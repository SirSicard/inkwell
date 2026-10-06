//! A language model's chat format, beyond what llama.cpp's built-in templates write: keeping
//! reasoning out of an answer, and turning a hybrid thinking model's thinking off
//! ([`ChatQuirks::no_think`]).
//!
//! The llama.cpp adapter applies the model's template through llama.cpp's built-in formats, not
//! Jinja (llama-cpp-2's `common` is off), so a template's `enable_thinking=false` cannot be passed.
//! What Qwen3's own template writes for it is an empty think block at the start of the answer:
//! `<|im_start|>assistant\n<think>\n\n</think>\n\n`. So the prompt gets the same block after the
//! assistant's turn opens ([`with_no_think`]). As a safety net for every model, a think block at the
//! start of an answer is taken off it ([`strip_think`]). Plain string work, compiled in every build
//! so CI tests it without a model.
//!
//! [`ChatQuirks::no_think`]: crate::ChatQuirks::no_think

/// The empty think block Qwen3's template writes when thinking is off.
pub const NO_THINK_PREFILL: &str = "<think>\n\n</think>\n\n";

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

/// `prompt`, which ends where the assistant's answer begins (the template applied with the
/// assistant's turn opened), with the empty think block appended.
pub fn with_no_think(mut prompt: String) -> String {
    prompt.push_str(NO_THINK_PREFILL);
    prompt
}

/// Why an answer could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkError {
    /// The answer opened a think block and never closed it: it is all reasoning, with no answer
    /// to give.
    Unclosed,
}

/// `answer` without a think block at its start (and the whitespace after it); the answer as it is
/// when it starts with none. A think block that is opened and never closed is an error: reasoning
/// must never be taken for the answer, and so typed into the user's text.
pub fn strip_think(answer: &str) -> Result<&str, ThinkError> {
    let Some(rest) = answer.trim_start().strip_prefix(THINK_OPEN) else {
        return Ok(answer);
    };
    let (_, after) = rest.split_once(THINK_CLOSE).ok_or(ThinkError::Unclosed)?;
    Ok(after.trim_start())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefill_is_what_qwen3s_template_writes_with_thinking_off() {
        // Qwen/Qwen3-1.7B's chat template, `add_generation_prompt` with `enable_thinking=false`.
        let opened = "<|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\n".to_owned();
        assert_eq!(
            with_no_think(opened),
            "<|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
        );
    }

    #[test]
    fn a_leading_think_block_is_taken_off_and_nothing_else_is() {
        assert_eq!(strip_think("<think>\n\n</think>\n\nHello."), Ok("Hello."));
        assert_eq!(
            strip_think("\n<think>Let me see. </think> Hello, world."),
            Ok("Hello, world.")
        );
        assert_eq!(strip_think("Hello."), Ok("Hello."));
        // Only a block at the start: the same words later are the user's.
        assert_eq!(
            strip_think("I typed <think> and </think> here."),
            Ok("I typed <think> and </think> here.")
        );
        // The answer's own leading space is kept when there is no block.
        assert_eq!(strip_think("  Hello."), Ok("  Hello."));
        assert_eq!(strip_think(""), Ok(""));
    }

    #[test]
    fn a_think_block_never_closed_is_an_error_not_an_answer() {
        assert_eq!(
            strip_think("<think>Reasoning that ran out of budget"),
            Err(ThinkError::Unclosed)
        );
    }
}
