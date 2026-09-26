//! The jobs a language model does, each generic over `&dyn Llm`, so the same code runs on a
//! bring-your-own-key provider or a model in this process.
//!
//! **Every task validates the answer's shape against its own task.** An earlier implementation
//! checked every structured answer against the summary's shape, so the commitment judge's
//! answers, which have a different shape, were all thrown away as malformed and the feature
//! silently produced nothing. Here each task owns its parser, its JSON schema sits beside it, and
//! tests feed each task the other tasks' answers.
//!
//! | Task | Answer |
//! |---|---|
//! | [`polish`] | Plain text: the dictation, cleaned up. |
//! | [`voice_edit`] | Plain text: the selection, rewritten. |
//! | [`summary`] | JSON: headline, body, decisions, actions, open questions. |
//! | [`commitments`] | JSON per candidate: one of six classes, with a verbatim quote. |
//! | [`dedup`] | JSON per pair: whether two commitments are the same obligation. |
//!
//! Every task is a **worker**-thread call and passes the cancel token to the model.

pub mod commitments;
pub mod dedup;
pub mod due;
pub mod polish;
pub mod summary;
pub mod transcript;
pub mod voice_edit;

pub use commitments::RecordContext;
pub use due::RecordTime;

use ink_core::{CancelToken, Llm, LlmError, LlmRequest};

/// Runs one request and hands back the text, checking the token first so a cancelled task makes
/// no call.
pub(crate) fn ask(
    llm: &dyn Llm,
    request: &LlmRequest,
    cancel: &CancelToken,
) -> Result<String, LlmError> {
    if cancel.is_cancelled() {
        return Err(LlmError::Cancelled);
    }
    llm.complete(request, cancel).map(|r| r.text)
}

/// Whether `quote` appears verbatim in `text`: the anti-hallucination gate the commitment judge
/// and the summary share. Surrounding whitespace and quotation marks are ignored; everything else
/// must match exactly, case included.
pub fn quote_found(quote: &str, text: &str) -> bool {
    let quote = quote
        .trim()
        .trim_matches(['"', '\u{201c}', '\u{201d}'])
        .trim();
    !quote.is_empty() && text.contains(quote)
}

/// Whether an error ends a whole task rather than one item of it. A malformed answer is one
/// item's problem; a refusal, a missing key or a dead network is every item's, and asking again
/// would only repeat it (or, for the keychain, prompt again).
pub(crate) fn is_fatal(error: &LlmError) -> bool {
    !matches!(error, LlmError::BadResponse(_))
}
