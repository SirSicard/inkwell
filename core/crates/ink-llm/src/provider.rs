//! Bring-your-own-key providers: OpenAI, Groq, OpenRouter and any OpenAI-compatible server on
//! `/chat/completions`, and Anthropic on `/v1/messages`. Ported from Inkwell 0.2's `llm.rs`.
//!
//! A call runs in this order, and each step can end it with nothing sent:
//! 1. the cancel token;
//! 2. the local-only guard, before the keychain is even asked (a refused call raises no prompt);
//! 3. the key, read from the keychain only now, when it is about to be used. No key is
//!    [`LlmError::NoKey`]; a refusal is [`LlmError::KeychainDenied`];
//! 4. the cancel token again (a keychain prompt can take a while);
//! 5. the request, through the shared [`Transport`];
//! 6. the cancel token once more: an answer that arrives after cancellation is dropped.
//!
//! An HTTP error is [`LlmError::Http`] with the status alone. Inkwell 0.2 kept the provider's
//! error `code` (`model_decommissioned`, `invalid_api_key`); the contract's error has no field
//! for it, and the body is never read, because providers echo the request in it.
//!
//! Cancelling cannot interrupt a request already on the wire: the client is blocking, and the
//! transport's read timeout bounds the wait.

use std::sync::Arc;

use ink_core::{CancelToken, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse};
use serde_json::{Value, json};

use crate::guard::{EndpointError, EndpointUrl, LocalOnly};
use crate::json::{bad, bad_field};
use crate::keys::{ApiKey, KeyStore, KeyStoreError};
use crate::transport::{HttpRequest, Transport, TransportError};

/// A bring-your-own-key provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
    /// OpenAI.
    OpenAi,
    /// Groq (OpenAI-compatible).
    Groq,
    /// Anthropic.
    Anthropic,
    /// OpenRouter (OpenAI-compatible).
    OpenRouter,
    /// Any OpenAI-compatible server the user names, usually a local one (Ollama, llama.cpp,
    /// LM Studio). Its key is optional.
    Custom,
}

impl Provider {
    /// Every provider, in Inkwell 0.2's preference order.
    pub const ALL: [Self; 5] = [
        Self::OpenAi,
        Self::Groq,
        Self::Anthropic,
        Self::OpenRouter,
        Self::Custom,
    ];

    /// The id: the keychain account its key is stored under (as in 0.2) and [`LlmInfo`]'s
    /// provider.
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Groq => "groq",
            Self::Anthropic => "anthropic",
            Self::OpenRouter => "openrouter",
            Self::Custom => "custom",
        }
    }

    /// The provider with this id.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    /// The base URL used when the configuration names none.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Groq => "https://api.groq.com/openai/v1",
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            // Ollama's default, as in 0.2.
            Self::Custom => "http://localhost:11434/v1",
        }
    }

    /// The model used when the configuration names none. These are 0.2's defaults except
    /// Anthropic's, whose 0.2 default id did not exist; the settings screen lets users choose.
    pub fn default_model(self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-4o-mini",
            Self::Groq => "llama-3.3-70b-versatile",
            Self::Anthropic => "claude-haiku-4-5",
            Self::OpenRouter => "openai/gpt-4o-mini",
            Self::Custom => "llama3",
        }
    }

    /// Whether a call needs a key. A custom server usually runs without one.
    pub fn needs_key(self) -> bool {
        self != Self::Custom
    }

    /// Whether to ask for JSON mode (`response_format: json_object`) when a task wants
    /// structured output. Not for custom servers: some reject the parameter outright, and every
    /// task validates the answer's shape anyway.
    fn json_mode(self) -> bool {
        matches!(self, Self::OpenAi | Self::Groq | Self::OpenRouter)
    }
}

/// Which provider, which model, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByokConfig {
    /// The provider.
    pub provider: Provider,
    /// The model id; the provider's default when `None`.
    pub model: Option<String>,
    /// The base URL; the provider's default when `None`. For [`Provider::Custom`] this is the
    /// server the user named. For the others it can point at a compatible proxy.
    pub base_url: Option<String>,
}

impl ByokConfig {
    /// A provider with its default model and URL.
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            model: None,
            base_url: None,
        }
    }
}

/// A bring-your-own-key model.
pub struct ByokLlm {
    provider: Provider,
    model: String,
    /// Parsed once, here; every call and [`Llm::info`] read the same parse.
    base: EndpointUrl,
    keys: Arc<dyn KeyStore>,
    transport: Arc<dyn Transport>,
    local_only: LocalOnly,
}

impl ByokLlm {
    /// Builds a provider. Pass [`UreqTransport::shared`](crate::UreqTransport::shared) as the
    /// transport outside tests, so every provider shares one client.
    ///
    /// Refuses a base URL the guard cannot parse, and plain `http` to another machine for a
    /// provider that needs a key ([`EndpointError::PlainHttp`]). A custom server over plain
    /// `http` elsewhere is allowed, but its key is never read or sent.
    pub fn new(
        config: ByokConfig,
        keys: Arc<dyn KeyStore>,
        transport: Arc<dyn Transport>,
        local_only: LocalOnly,
    ) -> Result<Self, EndpointError> {
        let provider = config.provider;
        let base = EndpointUrl::parse(
            config
                .base_url
                .as_deref()
                .unwrap_or(provider.default_base_url()),
        )?;
        if provider.needs_key() && !base.is_https() && !base.is_loopback() {
            return Err(EndpointError::PlainHttp);
        }
        let model = config
            .model
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| provider.default_model().to_owned());
        Ok(Self {
            provider,
            model,
            base,
            keys,
            transport,
            local_only,
        })
    }

    /// Whether a key may travel to this endpoint: over `https`, or to this machine.
    fn key_may_travel(&self) -> bool {
        self.base.is_https() || self.base.is_loopback()
    }

    fn key(&self) -> Result<Option<ApiKey>, LlmError> {
        if !self.key_may_travel() {
            // Only a custom server gets here (`new` refuses the rest); it is called without one.
            return Ok(None);
        }
        match self.keys.read_key(self.provider.id()) {
            Ok(key) => Ok(Some(key)),
            Err(KeyStoreError::NotFound) if !self.provider.needs_key() => Ok(None),
            Err(KeyStoreError::NotFound) => Err(LlmError::NoKey),
            Err(_) => Err(LlmError::KeychainDenied),
        }
    }

    /// The HTTP request for `request`: URL, headers and body, per provider.
    fn http_request(&self, request: &LlmRequest, key: Option<&ApiKey>) -> HttpRequest {
        let temperature = round_temperature(request.temperature);
        let mut headers = vec![("Content-Type", "application/json".to_owned())];
        let (url, body) = if self.provider == Provider::Anthropic {
            if let Some(key) = key {
                headers.push(("x-api-key", key.expose().to_owned()));
            }
            headers.push(("anthropic-version", "2023-06-01".to_owned()));
            let mut body = json!({
                "model": self.model,
                "messages": [{ "role": "user", "content": request.user }],
                "max_tokens": request.max_tokens,
                "temperature": temperature,
            });
            if !request.system.is_empty() {
                body["system"] = Value::from(request.system.as_str());
            }
            (self.base.join("/v1/messages"), body)
        } else {
            if let Some(key) = key {
                headers.push(("Authorization", format!("Bearer {}", key.expose())));
            }
            let mut messages = Vec::with_capacity(2);
            if !request.system.is_empty() {
                messages.push(json!({ "role": "system", "content": request.system }));
            }
            messages.push(json!({ "role": "user", "content": request.user }));
            let mut body = json!({
                "model": self.model,
                "messages": messages,
                "max_tokens": request.max_tokens,
                "temperature": temperature,
            });
            if request.json_schema.is_some() && self.provider.json_mode() {
                body["response_format"] = json!({ "type": "json_object" });
            }
            (self.base.join("/chat/completions"), body)
        };
        HttpRequest {
            url,
            headers,
            body: body.to_string().into_bytes(),
            loopback_only: self.local_only.is_on(),
        }
    }

    /// The answer's text, per provider.
    fn parse_answer(&self, body: &[u8]) -> Result<String, LlmError> {
        const TASK: &str = "provider answer";
        let value: Value =
            serde_json::from_slice(body).map_err(|_| bad(TASK, "the answer is not JSON"))?;
        if self.provider == Provider::Anthropic {
            let blocks = value
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| bad_field(TASK, "content", "is missing or not an array"))?;
            let texts: Vec<&str> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            if texts.is_empty() {
                return Err(bad_field(TASK, "content", "holds no text block"));
            }
            Ok(texts.concat())
        } else {
            value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| bad_field(TASK, "choices[0].message.content", "is missing"))
        }
    }
}

impl Llm for ByokLlm {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: self.provider.id().to_owned(),
            model: self.model.clone(),
            endpoint: self.base.endpoint(),
        }
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        let endpoint = self.base.endpoint();
        self.local_only.check(&endpoint)?;
        let key = self.key()?;
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        let http = self.http_request(request, key.as_ref());
        drop(key);
        let response = self.transport.post(&http).map_err(|e| match e {
            TransportError::NotLoopback => LlmError::LocalOnly {
                endpoint: self.base.to_string(),
            },
            TransportError::TooLarge => LlmError::BadResponse(e.describe().to_owned()),
            other => LlmError::Network(other.describe().to_owned()),
        })?;
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        if !(200..300).contains(&response.status) {
            return Err(LlmError::Http {
                status: response.status,
            });
        }
        Ok(LlmResponse {
            text: self.parse_answer(&response.body)?,
        })
    }
}

/// `f32` to a short JSON number: `0.3`, not `0.30000001192092896`.
fn round_temperature(t: f32) -> f64 {
    (f64::from(t) * 1000.0).round() / 1000.0
}

/// The first provider in `configured`, in [`Provider::ALL`]'s preference order rather than the
/// order the user saved keys in. Unknown ids are ignored, so a hand-edited setting cannot name a
/// provider that cannot be built.
pub fn preferred_provider<S: AsRef<str>>(configured: &[S]) -> Option<Provider> {
    Provider::ALL
        .into_iter()
        .find(|p| configured.iter().any(|c| c.as_ref() == p.id()))
}

/// Providers with a key stored, in preference order, asked without reading any key (so no
/// keychain prompt). Safe on a settings screen. A provider whose check fails is left out.
pub fn configured_providers(keys: &dyn KeyStore) -> Vec<Provider> {
    Provider::ALL
        .into_iter()
        .filter(|p| keys.has_key(p.id()) == Ok(true))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn preference_order_beats_save_order() {
        assert_eq!(
            preferred_provider(&ids(&["groq", "openai"])),
            Some(Provider::OpenAi)
        );
    }

    #[test]
    fn a_single_provider_is_chosen_whatever_it_is() {
        assert_eq!(preferred_provider(&ids(&["groq"])), Some(Provider::Groq));
        assert_eq!(
            preferred_provider(&ids(&["custom"])),
            Some(Provider::Custom)
        );
    }

    #[test]
    fn no_providers_means_none() {
        assert_eq!(preferred_provider::<String>(&[]), None);
    }

    #[test]
    fn unknown_provider_ids_are_ignored() {
        assert_eq!(preferred_provider(&ids(&["not-a-provider"])), None);
    }

    #[test]
    fn ids_round_trip() {
        for p in Provider::ALL {
            assert_eq!(Provider::from_id(p.id()), Some(p));
        }
        assert_eq!(Provider::from_id("OpenAI"), None);
    }

    #[test]
    fn temperature_is_a_short_number() {
        assert_eq!(round_temperature(0.3), 0.3);
        assert_eq!(round_temperature(0.0), 0.0);
    }
}
