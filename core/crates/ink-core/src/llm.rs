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

    /// Where it is, for a message: the URL or the remote's name, or "this process".
    pub fn describe(&self) -> String {
        match self {
            Self::InProcess => "this process".to_owned(),
            Self::Loopback(url) | Self::Remote(url) => url.clone(),
        }
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

    /// **Worker.** [`complete`](Self::complete), only if `allow` accepts the model that answers.
    /// For a caller that must know where the text goes (dictation polish sends only where the user
    /// consented): the check is on the model called, never on one looked up a moment earlier.
    /// Refused, nothing is sent ([`LlmError::NotAllowed`]).
    ///
    /// A model that picks among others at each call overrides this, to check the one it picked;
    /// a wrapper around one model forwards it. The default suits a model that is always itself.
    fn complete_if(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
        allow: &dyn Fn(&LlmInfo) -> bool,
    ) -> Result<LlmResponse, LlmError> {
        let info = self.info();
        if !allow(&info) {
            return Err(LlmError::NotAllowed {
                endpoint: info.endpoint.describe(),
            });
        }
        self.complete(request, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::mock::MockLlm;

    fn request() -> LlmRequest {
        LlmRequest {
            system: String::new(),
            user: "hi".into(),
            max_tokens: 8,
            temperature: 0.0,
            json_schema: None,
        }
    }

    #[test]
    fn complete_if_sends_only_what_the_check_allows() {
        let llm = MockLlm::new(Endpoint::Remote("https://api.example.com/v1".into()), "ok");
        let refused = llm.complete_if(&request(), &CancelToken::new(), &|i| i.endpoint.is_local());
        assert_eq!(
            refused,
            Err(LlmError::NotAllowed {
                endpoint: "https://api.example.com/v1".into()
            })
        );
        assert_eq!(llm.calls(), 0, "a refused call sends nothing");
        let allowed = llm.complete_if(&request(), &CancelToken::new(), &|_| true);
        assert_eq!(allowed.map(|r| r.text), Ok("ok".to_owned()));
        assert_eq!(llm.calls(), 1);
    }

    #[test]
    fn only_in_process_and_loopback_are_local() {
        assert!(Endpoint::InProcess.is_local());
        assert!(Endpoint::Loopback("http://127.0.0.1:8080/v1".into()).is_local());
        assert!(!Endpoint::Remote("https://api.example.com/v1".into()).is_local());
    }
}
