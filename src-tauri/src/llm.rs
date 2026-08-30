use async_trait::async_trait;
use serde_json::json;

/// Uniform result from LLM polish
/// Turn a failed HTTP response into an error string that is safe to log, show
/// and hand to a caller.
///
/// The body is deliberately **not** included. Providers echo the offending
/// input back in validation errors, and the input here is the user's dictation
/// or whatever they had selected in another application. Returning the raw body
/// put that text into `Err(String)`, which `pipeline.rs` then wrote to the
/// on-disk log and emitted to the frontend, where `App.tsx` renders
/// `voice-edit-error` in a toast. Same class of bug as the one this session
/// already fixed once in the disfluency logger, arrived at from the other end.
///
/// What survives is the part that actually diagnoses the failure and cannot
/// contain user text: the HTTP status, and the provider's own machine-readable
/// `code`/`type` enums when it sends them. "model_decommissioned" and
/// "invalid_api_key" are what a user needs; the echoed sentence is not.
fn safe_api_error(label: &str, status: reqwest::StatusCode, body: &str) -> String {
    let tag = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            let e = v.get("error")?;
            let code = e.get("code").and_then(|c| c.as_str());
            let kind = e.get("type").and_then(|t| t.as_str());
            match (code, kind) {
                (Some(c), _) => Some(c.to_string()),
                (None, Some(t)) => Some(t.to_string()),
                _ => None,
            }
        });

    // The full body is a debug-build-only diagnostic, through the same gate as
    // every other piece of user-derived text in this app.
    log::debug!("{} raw error body: {}", label, crate::redact(body));

    match tag {
        Some(t) => format!("{} {} ({})", label, status.as_u16(), t),
        None => format!("{} {}", label, status.as_u16()),
    }
}

pub struct PolishResult {
    pub text: String,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<PolishResult, String>;
}

// ---------------------------------------------------------------------------
// OpenAI-compatible (OpenAI, Groq, Cerebras, OpenRouter, all on /v1/chat/completions)
// ---------------------------------------------------------------------------

pub struct OpenAICompatible {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[async_trait]
impl LlmProvider for OpenAICompatible {
    async fn complete(&self, system: &str, user: &str) -> Result<PolishResult, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&json!({
                "model": self.model,
                "messages": [
                    { "role": "system", "content": system },
                    { "role": "user",   "content": user }
                ],
                "max_tokens": 1024,
                "temperature": 0.3
            }))
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(safe_api_error("API error", status, &body));
        }

        let body: serde_json::Value = resp.json().await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(PolishResult { text })
    }
}

// ---------------------------------------------------------------------------
// Anthropic (different auth header + different SSE format)
// ---------------------------------------------------------------------------

pub struct AnthropicProvider {
    pub model: String,
    pub api_key: String,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, system: &str, user: &str) -> Result<PolishResult, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&json!({
                "model": self.model,
                "system": system,
                "messages": [{ "role": "user", "content": user }],
                "max_tokens": 1024
            }))
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(safe_api_error("Anthropic API error", status, &body));
        }

        let body: serde_json::Value = resp.json().await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        let text = body["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(PolishResult { text })
    }
}

// ---------------------------------------------------------------------------
// Factory: build provider from stored config
// ---------------------------------------------------------------------------

pub struct ProviderConfig {
    pub provider: String,  // "openai" | "groq" | "anthropic" | "openrouter" | "custom"
    pub api_key: String,
    pub custom_url: Option<String>,
    pub model: Option<String>,
}

/// Providers a BYOK key can be stored under, in preference order.
pub const PROVIDERS: &[&str] = &["openai", "groq", "anthropic", "openrouter", "custom"];

/// Keychain service name for every stored key. One constant because the writer
/// and the existence check must agree: were they to drift, saving a key and
/// asking whether one exists would look at different items and the UI would
/// disagree with reality with nothing to show for it.
pub const KEYRING_SERVICE: &str = "inkwell";

/// Look up an API key from the OS keyring. Keys are keyring-only, never on disk.
///
/// On macOS this is not a cheap read: the system asks the user to authorize
/// access to the item unless the calling binary is already in its ACL, so every
/// call is a potential password dialog. Call it when the key is about to be
/// used, never to find out whether one exists.
pub fn api_key_for(provider: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, provider)
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|k| !k.is_empty())
}

/// Whether a key is stored for `provider`, without reading it.
///
/// macOS gates access to a keychain item's *secret*, not to its metadata, so an
/// attributes-only query answers the boolean with no dialog. That distinction is
/// the whole bug: asking `get_password` in order to learn a yes/no put a system
/// authorization prompt in front of the user for information it then threw away.
#[cfg(target_os = "macos")]
pub fn has_api_key(provider: &str) -> bool {
    use security_framework::item::{ItemClass, ItemSearchOptions};

    // load_attributes without load_data is what keeps this promptless: ask for
    // the secret here and the dialog comes straight back.
    ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(KEYRING_SERVICE)
        .account(provider)
        .load_attributes(true)
        .search()
        .map(|found| !found.is_empty())
        .unwrap_or(false)
}

/// Non-macOS platforms have no promptless existence query worth the complexity:
/// Windows' credential store does not prompt on read, and secret-service unlocks
/// once per session rather than per item.
#[cfg(not(target_os = "macos"))]
pub fn has_api_key(provider: &str) -> bool {
    api_key_for(provider).is_some()
}

/// Providers with a key stored, in preference order. Safe to call on a UI path.
pub fn configured_providers() -> Vec<String> {
    PROVIDERS
        .iter()
        .filter(|p| has_api_key(p))
        .map(|p| p.to_string())
        .collect()
}

pub fn build_provider(cfg: ProviderConfig) -> Box<dyn LlmProvider> {
    match cfg.provider.as_str() {
        "anthropic" => Box::new(AnthropicProvider {
            model: cfg.model.unwrap_or_else(|| "claude-haiku-4-20250514".to_string()),
            api_key: cfg.api_key,
        }),
        "groq" => Box::new(OpenAICompatible {
            base_url: "https://api.groq.com/openai/v1".to_string(),
            model: cfg.model.unwrap_or_else(|| "llama-3.3-70b-versatile".to_string()),
            api_key: cfg.api_key,
        }),
        "openrouter" => Box::new(OpenAICompatible {
            base_url: "https://openrouter.ai/api/v1".to_string(),
            model: cfg.model.unwrap_or_else(|| "openai/gpt-4o-mini".to_string()),
            api_key: cfg.api_key,
        }),
        "custom" => Box::new(OpenAICompatible {
            base_url: cfg.custom_url.unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
            model: cfg.model.unwrap_or_else(|| "llama3".to_string()),
            api_key: cfg.api_key,
        }),
        _ => Box::new(OpenAICompatible {  // default: openai
            base_url: "https://api.openai.com/v1".to_string(),
            model: cfg.model.unwrap_or_else(|| "gpt-4o-mini".to_string()),
            api_key: cfg.api_key,
        }),
    }
}

// ---------------------------------------------------------------------------
// Default polish prompt
// ---------------------------------------------------------------------------

pub const DEFAULT_POLISH_PROMPT: &str =
    "Clean up this speech-to-text transcription. The input comes from a dictation app and may contain:\
     \n- Filler words (um, uh, like, you know)\
     \n- False starts and repeated words\
     \n- Missing or wrong punctuation\
     \n- Misheard words or names\
     \n\
     \nRules:\
     \n- Fix grammar, punctuation, and capitalization\
     \n- Remove filler words and false starts\
     \n- Keep the speaker's original meaning, tone, and word choices\
     \n- Do NOT add, remove, or rephrase content\
     \n- Do NOT add greetings, sign-offs, or commentary\
     \n- Do NOT split into paragraphs (input is short dictation, not long-form)\
     \n- Return ONLY the cleaned text, nothing else";

#[cfg(test)]
mod keychain_tests {
    use super::*;

    /// The bug this guards: `has_api_key` must see an item that `save_api_key`
    /// wrote, and must see it *without* reading the secret. If the service name
    /// or item class ever drift apart between writer and checker, the app saves a
    /// key and then reports that no key is configured, which is exactly the state
    /// a user cannot debug.
    ///
    /// Uses its own account name so it can never touch a real provider entry.
    #[test]
    fn existence_check_agrees_with_what_was_stored() {
        let account = "inkwell-selftest-provider";
        let entry = match keyring::Entry::new(KEYRING_SERVICE, account) {
            Ok(e) => e,
            Err(_) => return, // no credential store on this machine, nothing to assert
        };
        let _ = entry.delete_credential();

        assert!(
            !has_api_key(account),
            "reported a key before one was stored"
        );

        if entry.set_password("test-value-not-a-real-key").is_err() {
            return; // headless CI with no unlocked keychain
        }
        assert!(has_api_key(account), "did not see the key it just stored");

        let _ = entry.delete_credential();
        assert!(!has_api_key(account), "still saw the key after deleting it");
    }
}


#[cfg(test)]
mod error_leak_tests {
    use super::safe_api_error;
    use reqwest::StatusCode;

    /// A provider that echoes the request back in its error body must not be
    /// able to put the user's words into an error string. This is behavioural
    /// rather than a source lint on purpose: the leak was not a bad log call,
    /// it was a value that was unsafe to log anywhere, and only the value's
    /// own construction site can fix that.
    #[test]
    fn the_response_body_never_reaches_the_caller() {
        let echoed = r#"{"error":{"message":"Invalid value for 'input': my bank password is hunter2","type":"invalid_request_error","code":"invalid_value"}}"#;
        let out = safe_api_error("API error", StatusCode::BAD_REQUEST, echoed);
        assert!(!out.contains("hunter2"), "leaked the echoed input: {out}");
        assert!(!out.contains("bank"), "leaked the echoed input: {out}");
        assert!(!out.contains("Invalid value"), "leaked the message: {out}");
    }

    /// ...but the part that actually tells the user what to fix survives, or
    /// the fix would undo this session's other one: an error nobody can act on
    /// is the reason AI Polish failed silently for weeks.
    #[test]
    fn the_diagnosable_part_survives() {
        let decommissioned = r#"{"error":{"message":"The model `llama-3.3-70b-versatile` has been decommissioned","type":"invalid_request_error","code":"model_decommissioned"}}"#;
        let out = safe_api_error("API error", StatusCode::BAD_REQUEST, decommissioned);
        assert!(out.contains("400"), "lost the status: {out}");
        assert!(out.contains("model_decommissioned"), "lost the code: {out}");
    }

    /// `type` is the fallback when a provider sends no `code`.
    #[test]
    fn falls_back_to_the_error_type() {
        let body = r#"{"error":{"message":"nope","type":"authentication_error"}}"#;
        let out = safe_api_error("API error", StatusCode::UNAUTHORIZED, body);
        assert!(out.contains("authentication_error"), "got {out}");
        assert!(!out.contains("nope"));
    }

    /// A non-JSON body (an HTML error page from a proxy, say) must degrade to
    /// the status alone rather than passing the page through.
    #[test]
    fn a_non_json_body_degrades_to_the_status() {
        let out = safe_api_error("API error", StatusCode::BAD_GATEWAY, "<html>secret</html>");
        assert_eq!(out, "API error 502");
    }
}
