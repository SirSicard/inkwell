//! Language models: polish, voice editing, summaries and commitments.
//!
//! `ink-llm` implements [`Llm`] for the bring-your-own-key providers and for local models (S1.6).
//! The local-only guard (architecture rule 6) lives there and reads [`Llm::info`]'s endpoint.

use crate::error::LlmError;
use crate::threading::CancelToken;

/// Where a model runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// Inside this process (llama.cpp, Foundation Models).
    InProcess,
    /// An HTTP server on this machine. The URL has a loopback host.
    Loopback(String),
    /// Anywhere else. Refused in local-only mode.
    Remote(String),
}

impl Endpoint {
    /// Whether the text stays on this machine.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::InProcess | Self::Loopback(_))
    }
}

/// What a model is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmInfo {
    /// The provider id (for example `anthropic`, `openai-compatible`, `local`).
    pub provider: String,
    /// The model id.
    pub model: String,
    /// Where it runs.
    pub endpoint: Endpoint,
}

/// One request. It holds user text, so it is never logged (I5).
#[derive(Clone, Debug, PartialEq)]
pub struct LlmRequest {
    /// The system prompt.
    pub system: String,
    /// The user message.
    pub user: String,
    /// The answer's token budget.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// A JSON schema the answer must match, for structured tasks (commitments). The caller still
    /// validates the answer's shape against its own task.
    pub json_schema: Option<String>,
}

/// One answer. It holds generated text, so it is never logged either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmResponse {
    /// The text.
    pub text: String,
}

/// A language model.
pub trait Llm: Send + Sync {
    /// **Any thread except realtime.** What this model is and where it runs.
    fn info(&self) -> LlmInfo;

    /// **Worker.** Blocks until the answer arrives, the call fails, or `cancel` is set. When the
    /// keychain refuses access to a key, nothing is sent ([`LlmError::KeychainDenied`]).
    fn complete(&self, request: &LlmRequest, cancel: &CancelToken)
    -> Result<LlmResponse, LlmError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_in_process_and_loopback_are_local() {
        assert!(Endpoint::InProcess.is_local());
        assert!(Endpoint::Loopback("http://127.0.0.1:8080/v1".into()).is_local());
        assert!(!Endpoint::Remote("https://api.example.com/v1".into()).is_local());
    }
}
